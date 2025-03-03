use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Error};
use actix_ws::Message;
use sqlx::postgres::PgPoolOptions;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use log::{info, error, warn};
use futures_util::StreamExt;
use uuid::Uuid;
use actix_cors::Cors;
use dotenv::dotenv;
use std::time::{Duration, Instant};
use tokio::time;

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
const MAX_USERS: usize = 100;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct AppState {
    active_users: Arc<Mutex<HashMap<String, usize>>>, // session_id -> user_id
    db_pool: Arc<sqlx::PgPool>,
}

async fn add_active_user(
    pool: &sqlx::PgPool,
    session_id: &str,
) -> Result<i32, sqlx::Error> {
    let result = sqlx::query_scalar!(
        "INSERT INTO active_users (session_id, last_seen) VALUES ($1, CURRENT_TIMESTAMP) RETURNING id",
        session_id
    )
    .fetch_one(pool)
    .await?;

    Ok(result)
}

async fn remove_active_user(
    pool: &sqlx::PgPool,
    session_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM active_users WHERE session_id = $1",
        session_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn update_last_seen(
    pool: &sqlx::PgPool,
    session_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE active_users SET last_seen = CURRENT_TIMESTAMP WHERE session_id = $1",
        session_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn get_active_users_count(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    let result = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM active_users WHERE last_seen > CURRENT_TIMESTAMP - INTERVAL '1 minute'"
    )
    .fetch_one(pool)
    .await?;

    Ok(result.unwrap_or(0))
}

async fn cleanup_stale_connections(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    // Fixed: Don't use COUNT(*) in RETURNING clause
    let result = sqlx::query!(
        "DELETE FROM active_users WHERE last_seen < CURRENT_TIMESTAMP - INTERVAL '1 minute'"
    )
    .execute(pool)
    .await?;

    // Get the number of rows affected by the DELETE operation
    Ok(result.rows_affected() as i64)
}

async fn broadcast_user_count(
    pool: &sqlx::PgPool, 
    session: &mut actix_ws::Session,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(count) = get_active_users_count(pool).await {
        let msg = serde_json::json!({
            "type": "counter",
            "count": std::cmp::min(count as usize, MAX_USERS)
        }).to_string();

        session.text(msg).await?;
    }
    Ok(())
}

async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let user_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);
    let active_users = app_state.active_users.clone();
    let db_pool = app_state.db_pool.clone();
    let session_id = Uuid::new_v4().to_string();
    
    let (res, session, msg_stream) = actix_ws::handle(&req, stream)?;
    
    info!("New WebSocket connection: user_id={}, session_id={}", user_id, session_id);

    // Add user to database
    match add_active_user(&*db_pool, &session_id).await {
        Ok(id) => {
            info!("Added active user to database with id: {}", id);
        }
        Err(e) => {
            error!("Failed to add active user to database: {}", e);
        }
    };

    // Add user to our active users map
    {
        let mut users = active_users.lock().await;
        users.insert(session_id.clone(), user_id);
    }

    // Handle WebSocket connection in separate task
    actix_web::rt::spawn(websocket_handler(
        session,
        msg_stream,
        session_id,
        user_id,
        active_users,
        db_pool,
    ));

    Ok(res)
}

async fn websocket_handler(
    mut session: actix_ws::Session,
    mut msg_stream: actix_ws::MessageStream,
    session_id: String,
    user_id: usize,
    active_users: Arc<Mutex<HashMap<String, usize>>>,
    db_pool: Arc<sqlx::PgPool>,
) {
    // Send initial user count
    if let Err(e) = broadcast_user_count(&*db_pool, &mut session).await {
        error!("Failed to send initial user count: {}", e);
    }

    // Set up the heartbeat interval
    let mut last_heartbeat = Instant::now();
    let mut interval = time::interval(HEARTBEAT_INTERVAL);

    // Send periodic cleanup to remove stale connections
    let cleanup_interval = time::interval(Duration::from_secs(30));
    let db_pool_for_cleanup = db_pool.clone();

    actix_web::rt::spawn(async move {
        let mut cleanup = cleanup_interval;
        loop {
            cleanup.tick().await;
            match cleanup_stale_connections(&*db_pool_for_cleanup).await {
                Ok(removed) => {
                    if removed > 0 {
                        info!("Cleaned up {} stale connections", removed);
                    }
                },
                Err(e) => error!("Error cleaning up stale connections: {}", e),
            }
        }
    });

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Send ping to client
                if Instant::now().duration_since(last_heartbeat) > CLIENT_TIMEOUT {
                    warn!("Client timeout for user_id={}, session_id={}", user_id, session_id);
                    break;
                }

                if let Err(e) = session.ping(b"").await {
                    error!("Failed to send ping: {}", e);
                    break;
                }

                // Update last_seen in the database
                if let Err(e) = update_last_seen(&*db_pool, &session_id).await {
                    error!("Failed to update last_seen: {}", e);
                }

                // Broadcast updated user count
                if let Err(e) = broadcast_user_count(&*db_pool, &mut session).await {
                    error!("Failed to broadcast user count: {}", e);
                }
            }
            Some(result) = msg_stream.next() => {
                match result {
                    Ok(msg) => match msg {
                        Message::Text(text) => {
                            info!("Received text message from {}: {}", session_id, text);
                            last_heartbeat = Instant::now();
                        }
                        Message::Binary(_) => {
                            last_heartbeat = Instant::now();
                        }
                        Message::Ping(bytes) => {
                            last_heartbeat = Instant::now();
                            if let Err(e) = session.pong(&bytes).await {
                                error!("Failed to send pong: {}", e);
                                break;
                            }
                        }
                        Message::Pong(_) => {
                            last_heartbeat = Instant::now();
                        }
                        Message::Close(_) => {
                            info!("Client requested close for session {}", session_id);
                            break;
                        }
                        Message::Continuation(_) => {
                            // Handle continuation frames if needed
                        }
                        Message::Nop => {}
                    },
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                }
            }
            else => break,
        }
    }

    // Connection closed, clean up
    info!("WebSocket connection closing for user_id={}, session_id={}", user_id, session_id);

    // Remove from active users
    {
        let mut users = active_users.lock().await;
        users.remove(&session_id);
    }

    // Remove from database
    if let Err(e) = remove_active_user(&*db_pool, &session_id).await {
        error!("Failed to remove active user from database: {}", e);
    }

    // Close the WebSocket session
    let _ = session.close(None).await;

    info!("WebSocket connection closed: user_id={}, session_id={}", user_id, session_id);
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    
    info!("Starting server initialization...");

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Could not connect to database");

    let pool = Arc::new(pool);

    // Clean all active users on startup
    sqlx::query!("DELETE FROM active_users")
        .execute(&*pool)
        .await
        .expect("Failed to clean active_users table");

    info!("Database connection established successfully");

    let app_state = web::Data::new(AppState {
        active_users: Arc::new(Mutex::new(HashMap::new())),
        db_pool: pool,
    });

    info!("Server starting at http://localhost:8080");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .wrap(cors)
            .app_data(app_state.clone())
            .route("/ws", web::get().to(ws_handler))
            .route("/health", web::get().to(|| async { HttpResponse::Ok().body("Health check OK") }))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}