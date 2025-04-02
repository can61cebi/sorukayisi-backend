use actix_web::{web, Error, HttpRequest, HttpResponse};
use actix_ws::{Message, MessageStream, Session};
use chrono::Utc;
use futures_util::StreamExt;
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use sqlx::{Pool, Postgres};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time;
use uuid::Uuid;

use crate::db::models::{ConnectionType, GameStatus, LeaderboardEntry};

// Bağlantı durumları
#[derive(Debug, PartialEq, Clone, Copy)]
enum ConnectionState {
    Lobby,
    Game,
    Question,   // Aktif bir soru gösteriliyor
    Review,     // Cevap ve liderlik tablosu gösteriliyor
    Ended,
}

// Uygulama durumu
pub struct AppState {
    active_connections: Arc<Mutex<HashMap<String, WebSocketConnection>>>, // session_id -> connection
    games: Arc<Mutex<HashMap<String, GameState>>>,                       // game_code -> GameState
    db_pool: Arc<Pool<Postgres>>,
    next_user_id: Arc<AtomicUsize>,
}

// WebSocket bağlantısını takip etmek için yapı
struct WebSocketConnection {
    user_id: Option<i32>,
    player_id: Option<i32>,
    game_id: Option<i32>,
    game_code: Option<String>,
    connection_type: ConnectionType,
    session: Option<Session>,
    last_seen: Instant,
}

// Oyun durumu
struct GameState {
    id: i32,
    code: String,
    host_session_id: String,
    host_id: i32,
    question_set_id: i32,
    players: HashMap<String, PlayerState>, // session_id -> PlayerState
    current_question: i32,
    state: ConnectionState,
    started_at: Option<Instant>,
    ended_at: Option<Instant>,
    question_timer: Option<Instant>,       // Mevcut sorunun başlangıç zamanı
    question_duration: Option<Duration>,   // Mevcut sorunun süresi
    total_questions: i32,                  // Toplam soru sayısı
}

// Oyuncu durumu
struct PlayerState {
    player_id: i32,
    user_id: Option<i32>,
    session_id: String,
    nickname: String,
    score: i32,
    answers: HashMap<i32, PlayerAnswer>,   // question_id -> PlayerAnswer
    is_active: bool,
    joined_at: Instant,
    last_seen: Instant,
    last_answer_time: Option<Instant>,     // Son cevabın verildiği zaman
}

// Oyuncu cevabı
struct PlayerAnswer {
    question_id: i32,
    answer: Option<String>,
    is_correct: bool,
    response_time_ms: i32,
    points_earned: i32,
}

impl AppState {
    pub fn new(db_pool: Pool<Postgres>) -> Self {
        AppState {
            active_connections: Arc::new(Mutex::new(HashMap::new())),
            games: Arc::new(Mutex::new(HashMap::new())),
            db_pool: Arc::new(db_pool),
            next_user_id: Arc::new(AtomicUsize::new(1)),
        }
    }

    // Oyundaki tüm oyunculara mesaj gönderme
    pub async fn broadcast_to_game(&self, game_code: &str, message: &str) {
        debug!("Broadcast to game: {}, message: {}", game_code, message);
        
        let active_connections = self.active_connections.lock().await;
        let games = self.games.lock().await;
        
        if let Some(game) = games.get(game_code) {
            for session_id in game.players.keys() {
                if let Some(conn) = active_connections.get(session_id) {
                    if let Some(session) = &conn.session {
                        // Here we need a mutable session
                        let mut session_clone = session.clone();
                        if let Err(e) = session_clone.text(message.to_string()).await {
                            error!("Mesaj gönderme hatası: {}", e);
                        }
                    }
                }
            }
            
            // Oyun sahibine de mesaj gönder
            if let Some(conn) = active_connections.get(&game.host_session_id) {
                if let Some(session) = &conn.session {
                    // Here we need a mutable session
                    let mut session_clone = session.clone();
                    if let Err(e) = session_clone.text(message.to_string()).await {
                        error!("Host'a mesaj gönderme hatası: {}", e);
                    }
                }
            }
        }
    }
    
    // Belirli bir oyuncuya mesaj gönderme
    pub async fn send_to_player(&self, session_id: &str, message: &str) {
        let active_connections = self.active_connections.lock().await;
        
        if let Some(conn) = active_connections.get(session_id) {
            if let Some(session) = &conn.session {
                // Here we need a mutable session
                let mut session_clone = session.clone();
                if let Err(e) = session_clone.text(message.to_string()).await {
                    error!("Oyuncuya mesaj gönderme hatası: {}", e);
                }
            }
        }
    }
    
    // Oyun durumunu kontrol etme ve gerekirse zamanlayıcıyı çalıştırma
    pub async fn check_game_timers(&self) {
        let mut games_to_advance = Vec::new();
        
        // Kilidi mümkün olduğunca kısa tutmak için önce kontrol et, sonra işlem yap
        {
            let games = self.games.lock().await;
            
            for (code, game) in games.iter() {
                // Soru gösteriliyorsa ve süre dolduysa
                if game.state == ConnectionState::Question && game.question_timer.is_some() && game.question_duration.is_some() {
                    let now = Instant::now();
                    let start_time = game.question_timer.unwrap();
                    let duration = game.question_duration.unwrap();
                    
                    if now.duration_since(start_time) >= duration {
                        games_to_advance.push(code.clone());
                    }
                }
            }
        }
        
        // Şimdi kilidi bıraktık, oyunları ilerletebiliriz
        for game_code in games_to_advance {
            if let Err(e) = self.show_question_result(&game_code).await {
                error!("Soru sonucu gösterilirken hata oluştu: {}", e);
            }
        }
    }
    
    // Soru sonucunu göster
    pub async fn show_question_result(&self, game_code: &str) -> Result<(), anyhow::Error> {
        let mut games = self.games.lock().await;
        
        if let Some(game) = games.get_mut(game_code) {
            // Oyun durumunu "Review" olarak güncelle
            game.state = ConnectionState::Review;
            
            // Mevcut sorunun doğru cevabını veritabanından al
            let question_id = sqlx::query!(
                r#"
                SELECT id, correct_option
                FROM questions
                WHERE question_set_id = $1 AND position = $2
                "#,
                game.question_set_id,
                game.current_question
            )
            .fetch_one(&*self.db_pool)
            .await?;
            
            // Liderlik tablosunu hesapla
            let leaderboard = self.get_leaderboard(game_code).await?;
            
            // Sonuçları tüm oyunculara bildir
            let result_message = json!({
                "type": "question_end",
                "question_id": question_id.id,
                "correct_option": question_id.correct_option,
                "leaderboard": leaderboard
            }).to_string();
            
            drop(games); // Kilidi bırak, çünkü broadcast_to_game'de yeniden alınacak
            self.broadcast_to_game(game_code, &result_message).await;
        }
        
        Ok(())
    }
    
    // Liderlik tablosunu getir
    pub async fn get_leaderboard(&self, game_code: &str) -> Result<Vec<LeaderboardEntry>, anyhow::Error> {
        let games = self.games.lock().await;
        
        if let Some(game) = games.get(game_code) {
            // Veritabanından oyuncuları puanlarına göre sıralanmış olarak getir
            let players = sqlx::query!(
                r#"
                SELECT id, nickname, score, user_id IS NULL as is_guest
                FROM players
                WHERE game_id = $1 AND is_active = true
                ORDER BY score DESC
                LIMIT 100
                "#,
                game.id
            )
            .fetch_all(&*self.db_pool)
            .await?;
            
            let leaderboard: Vec<LeaderboardEntry> = players
                .iter()
                .map(|p| LeaderboardEntry {
                    player_id: p.id,
                    nickname: p.nickname.clone(),
                    score: p.score.unwrap_or(0),
                    is_guest: p.is_guest.unwrap_or(false),
                })
                .collect();
            
            Ok(leaderboard)
        } else {
            Err(anyhow::anyhow!("Oyun bulunamadı"))
        }
    }
}

// WebSocket handlers
pub async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let user_id = app_state.next_user_id.fetch_add(1, Ordering::Relaxed);
    let active_connections = app_state.active_connections.clone();
    let games = app_state.games.clone();
    let db_pool = app_state.db_pool.clone();
    let session_id = Uuid::new_v4().to_string();

    let (response, session, msg_stream) = actix_ws::handle(&req, stream)?;

    info!(
        "Yeni WebSocket bağlantısı: user_id={}, session_id={}",
        user_id, session_id
    );

    // Veritabanına aktif bağlantıyı ekle
    match sqlx::query!(
        r#"
        INSERT INTO active_connections (session_id, connection_type, last_seen)
        VALUES ($1, $2, $3)
        "#,
        session_id,
        ConnectionType::Viewer.to_string().to_lowercase(),
        Utc::now()
    )
    .execute(&*db_pool)
    .await
    {
        Ok(_) => {
            info!("Aktif bağlantı veritabanına eklendi: {}", session_id);
        }
        Err(e) => {
            error!(
                "Aktif bağlantı veritabanına eklenirken hata oluştu: {}",
                e
            );
        }
    }

    // Aktif kullanıcılar listesine ekle
    {
        let mut connections = active_connections.lock().await;
        connections.insert(session_id.clone(), WebSocketConnection {
            user_id: None,
            player_id: None,
            game_id: None,
            game_code: None,
            connection_type: ConnectionType::Viewer,
            session: Some(session.clone()),
            last_seen: Instant::now(),
        });
    }

    // WebSocket bağlantısını ayrı bir task'ta işle
    actix_web::rt::spawn(websocket_task(
        session,
        msg_stream,
        session_id,
        user_id,
        active_connections,
        games,
        db_pool,
        app_state.clone(),
    ));

    Ok(response)
}

async fn websocket_task(
    mut session: Session,
    mut msg_stream: MessageStream,
    session_id: String,
    user_id: usize,
    active_connections: Arc<Mutex<HashMap<String, WebSocketConnection>>>,
    games: Arc<Mutex<HashMap<String, GameState>>>,
    db_pool: Arc<Pool<Postgres>>,
    app_state: web::Data<AppState>,
) {
    // Kullanıcı için hoş geldin mesajı gönder
    if let Err(e) = session
        .text(
            json!({
                "type": "welcome",
                "session_id": session_id,
                "message": "WebSocket bağlantısı kuruldu"
            })
            .to_string(),
        )
        .await
    {
        error!("Hoş geldin mesajı gönderme hatası: {}", e);
    }

    // İlk bağlantı bilgilerini gönder
    let active_count = {
        let connections = active_connections.lock().await;
        connections.len()
    };

    if let Err(e) = session
        .text(
            json!({
                "type": "counter",
                "count": active_count
            })
            .to_string(),
        )
        .await
    {
        error!("Aktif kullanıcı sayısı mesajı gönderme hatası: {}", e);
    }

    // Heartbeat için değişkenler
    let mut last_heartbeat = Instant::now();
    let mut interval = time::interval(Duration::from_secs(1));  // 1 saniye aralıklarla kontrol et
    const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
    const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

    // Ana mesaj işleme döngüsü
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Heartbeat kontrolü
                if Instant::now().duration_since(last_heartbeat) > CLIENT_TIMEOUT {
                    warn!("İstemci zaman aşımı: user_id={}, session_id={}", user_id, session_id);
                    break;
                }

                // Her 10 saniyede bir ping gönder
                if Instant::now().duration_since(last_heartbeat) > HEARTBEAT_INTERVAL {
                    if let Err(e) = session.ping(b"").await {
                        error!("Ping gönderme hatası: {}", e);
                        break;
                    }
                }

                // Veritabanında last_seen'i güncelle
                if let Err(e) = sqlx::query!(
                    "UPDATE active_connections SET last_seen = $1 WHERE session_id = $2",
                    Utc::now(),
                    session_id
                )
                .execute(&*db_pool)
                .await
                {
                    error!("Last seen güncellenirken hata oluştu: {}", e);
                }

                // Aktif kullanıcı sayısını gönder
                let active_count = {
                    let connections = active_connections.lock().await;
                    connections.len()
                };

                if let Err(e) = session
                    .text(
                        json!({
                            "type": "counter",
                            "count": active_count
                        })
                        .to_string(),
                    )
                    .await
                {
                    error!("Aktif kullanıcı sayısı mesajı gönderme hatası: {}", e);
                }
                
                // Oyun zamanlayıcılarını kontrol et
                app_state.check_game_timers().await;
            }
            result = msg_stream.next() => match result {
                Some(Ok(msg)) => {
                    last_heartbeat = Instant::now();
                    
                    // Bağlantı bilgisini güncelle
                    {
                        let mut connections = active_connections.lock().await;
                        if let Some(conn) = connections.get_mut(&session_id) {
                            conn.last_seen = Instant::now();
                        }
                    }

                    match msg {
                        Message::Text(text) => {
                            debug!("Metin mesajı alındı: {}", text);
                            
                            // JSON mesajını ayrıştır
                            let msg_result: Result<Value, serde_json::Error> = serde_json::from_str(&text);
                            
                            match msg_result {
                                Ok(msg_value) => {
                                    // Mesaj tipine göre işle
                                    if let Some(msg_type) = msg_value.get("type").and_then(|t| t.as_str()) {
                                        match msg_type {
                                            "ping" => {
                                                // Pong yanıtı gönder
                                                if let Err(e) = session
                                                    .text(json!({"type": "pong", "timestamp": Utc::now().timestamp()}).to_string())
                                                    .await
                                                {
                                                    error!("Pong yanıtı gönderme hatası: {}", e);
                                                }
                                            }
                                            "join_lobby" => {
                                                // Oyun lobisine katılım isteği
                                                if let (Some(game_code), Some(nickname)) = (
                                                    msg_value.get("game_code").and_then(|g| g.as_str()),
                                                    msg_value.get("nickname").and_then(|n| n.as_str())
                                                ) {
                                                    handle_join_lobby(&mut session, &db_pool, game_code, nickname, &session_id, &app_state).await;
                                                }
                                            }
                                            "start_game" => {
                                                // Oyun başlatma isteği
                                                if let Some(game_code) = msg_value.get("game_code").and_then(|g| g.as_str()) {
                                                    handle_start_game(&mut session, &db_pool, game_code, &session_id, &app_state).await;
                                                }
                                            }
                                            "submit_answer" => {
                                                // Cevap gönderme isteği
                                                if let (Some(question_id), Some(answer), Some(response_time)) = (
                                                    msg_value.get("question_id").and_then(|q| q.as_i64()),
                                                    msg_value.get("answer").and_then(|a| a.as_str()),
                                                    msg_value.get("response_time_ms").and_then(|r| r.as_i64()),
                                                ) {
                                                    handle_submit_answer(&mut session, &db_pool, question_id as i32, answer, response_time as i32, &session_id, &app_state).await;
                                                }
                                            }
                                            "next_question" => {
                                                // Bir sonraki soru isteği
                                                if let Some(game_code) = msg_value.get("game_code").and_then(|g| g.as_str()) {
                                                    handle_next_question(&mut session, &db_pool, game_code, &session_id, &app_state).await;
                                                }
                                            }
                                            "reconnect" => {
                                                // Yeniden bağlanma isteği
                                                if let Some(old_session_id) = msg_value.get("old_session_id").and_then(|s| s.as_str()) {
                                                    handle_reconnect(&mut session, &db_pool, old_session_id, &session_id, &app_state).await;
                                                }
                                            }
                                            // Diğer mesaj tipleri burada işlenebilir
                                            _ => {
                                                warn!("Bilinmeyen mesaj tipi: {}", msg_type);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("JSON ayrıştırma hatası: {}", e);
                                }
                            }
                        }
                        Message::Binary(_) => {
                            debug!("İkili mesaj alındı");
                        }
                        Message::Ping(bytes) => {
                            if let Err(e) = session.pong(&bytes).await {
                                error!("Pong yanıtı gönderme hatası: {}", e);
                                break;
                            }
                        }
                        Message::Pong(_) => {
                            // Pong yanıtı alındı, heartbeat zamanını güncelle
                        }
                        Message::Close(reason) => {
                            info!("Kapatma isteği alındı: {:?}", reason);
                            break;
                        }
                        Message::Continuation(_) => {
                            // Devam çerçeveleri gerekirse burada işlenebilir
                        }
                        Message::Nop => {}
                    }
                }
                Some(Err(e)) => {
                    error!("WebSocket hatası: {}", e);
                    break;
                }
                None => break,
            },
            else => break,
        }
    }

    // Bağlantı kapandı, temizlik yap
    info!(
        "WebSocket bağlantısı kapatılıyor: user_id={}, session_id={}",
        user_id, session_id
    );

    // Aktif bağlantıları temizle
    {
        let mut connections = active_connections.lock().await;
        connections.remove(&session_id);
    }

    // Veritabanından aktif bağlantıyı kaldır
    if let Err(e) = sqlx::query!(
        "DELETE FROM active_connections WHERE session_id = $1",
        session_id
    )
    .execute(&*db_pool)
    .await
    {
        error!(
            "Aktif bağlantı veritabanından kaldırılırken hata oluştu: {}",
            e
        );
    }

    // Oyun lobisinden oyuncuyu kaldır
    {
        let mut games_lock = games.lock().await;
        // Oyuncunun bulunduğu oyunu bul
        let mut game_to_update = None;
        
        for (code, game) in games_lock.iter_mut() {
            if game.players.contains_key(&session_id) {
                // Oyuncuyu pasif olarak işaretle
                if let Some(player) = game.players.get_mut(&session_id) {
                    player.is_active = false;
                    game_to_update = Some(code.clone());
                }
                break;
            }
        }
        
        // Eğer bu oyuncu bir oyunun host'u ise, oyunu sonlandır
        if let Some(game_code) = game_to_update {
            if let Some(game) = games_lock.get(&game_code) {
                if game.host_session_id == session_id {
                    // Oyunu sonlandır ve tüm oyunculara bildir
                    info!("Host ayrıldı, oyun sonlandırılıyor: {}", game_code);
                    
                    // Oyun durumunu veritabanında güncelle
                    let _ = sqlx::query!(
                        "UPDATE games SET status = 'completed', ended_at = $1 WHERE code = $2",
                        Utc::now(),
                        game_code
                    )
                    .execute(&*db_pool)
                    .await;
                    
                    // Tüm oyunculara bildir
                    drop(games_lock); // Kilidi bırak
                    let _ = app_state.broadcast_to_game(&game_code, &json!({
                        "type": "game_end",
                        "reason": "host_left",
                        "message": "Sunucu bağlantısı kesildi, oyun sonlandırıldı"
                    }).to_string()).await;
                    return;
                }
            }
        }
    }

    // WebSocket oturumunu kapat
    let _ = session.close(None).await;

    info!(
        "WebSocket bağlantısı kapatıldı: user_id={}, session_id={}",
        user_id, session_id
    );
}

// Oyun mesajları için handler fonksiyonları
async fn handle_join_lobby(
    session: &mut Session,
    db_pool: &Pool<Postgres>,
    game_code: &str,
    nickname: &str,
    session_id: &str,
    app_state: &web::Data<AppState>,
) {
    info!("Oyun lobisine katılma isteği: game_code={}, nickname={}", game_code, nickname);
    
    // Oyunun varlığını kontrol et
    let game = sqlx::query!(
        "SELECT id, status FROM games WHERE code = $1",
        game_code
    )
    .fetch_optional(db_pool)
    .await;
    
    match game {
        Ok(Some(game)) => {
            // Oyun durumunu kontrol et
            if game.status != "lobby" {
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Bu oyun artık katılıma açık değil"
                    })
                    .to_string(),
                )
                .await;
                return;
            }
            
            // Kullanıcı ID'sini al (varsa)
            let user_id = sqlx::query!(
                "SELECT user_id FROM active_connections WHERE session_id = $1",
                session_id
            )
            .fetch_optional(db_pool)
            .await
            .ok()
            .flatten()
            .and_then(|r| r.user_id);
            
            // Misafir oyuncu kontrolü ve nickname oluşturma
            let is_guest = user_id.is_none(); // Oturum açmış kullanıcı yoksa misafir
            let display_name = if is_guest {
                if !nickname.starts_with("**") {
                    format!("**{}", nickname)
                } else {
                    nickname.to_string()
                }
            } else {
                nickname.to_string() // Oturum açmış kullanıcıların isimlerine dokunma
            };
            
            // Nickname benzersizliğini kontrol et
            let existing_player = sqlx::query!(
                "SELECT id FROM players WHERE game_id = $1 AND nickname = $2",
                game.id,
                display_name
            )
            .fetch_optional(db_pool)
            .await;
            
            if let Ok(Some(_)) = existing_player {
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Bu takma ad zaten kullanılıyor"
                    })
                    .to_string(),
                )
                .await;
                return;
            }
            
            // Oyuncuyu ekle
            let player_result = sqlx::query!(
                r#"
                INSERT INTO players (game_id, user_id, nickname, session_id, joined_at)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING id
                "#,
                game.id,
                user_id,
                display_name,
                session_id,
                Utc::now()
            )
            .fetch_one(db_pool)
            .await;
            
            match player_result {
                Ok(player) => {
                    // Bağlantı tipini güncelle
                    let _ = sqlx::query!(
                        r#"
                        UPDATE active_connections 
                        SET user_id = $1, game_id = $2, player_id = $3, connection_type = 'player'
                        WHERE session_id = $4
                        "#,
                        user_id,
                        game.id,
                        player.id,
                        session_id
                    )
                    .execute(db_pool)
                    .await;
                    
                    // AppState'deki active_connections'ı güncelle
                    {
                        let mut connections = app_state.active_connections.lock().await;
                        if let Some(conn) = connections.get_mut(session_id) {
                            conn.user_id = user_id;
                            conn.player_id = Some(player.id);
                            conn.game_id = Some(game.id);
                            conn.game_code = Some(game_code.to_string());
                            conn.connection_type = ConnectionType::Player;
                        }
                    }
                    
                    // Oyun durumuna oyuncuyu ekle
                    {
                        let mut games = app_state.games.lock().await;
                        if !games.contains_key(game_code) {
                            // Oyun state'ini oluştur
                            let total_questions = sqlx::query!(
                                "SELECT COUNT(*) as count FROM questions WHERE question_set_id = (SELECT question_set_id FROM games WHERE id = $1)",
                                game.id
                            )
                            .fetch_one(db_pool)
                            .await
                            .map(|r| r.count.unwrap_or(0) as i32)
                            .unwrap_or(0);
                            
                            let host_info = sqlx::query!(
                                "SELECT host_id, question_set_id FROM games WHERE id = $1",
                                game.id
                            )
                            .fetch_one(db_pool)
                            .await;
                            
                            if let Ok(host) = host_info {
                                // Oyun host'unun session ID'sini bul
                                let host_session = sqlx::query!(
                                    "SELECT session_id FROM active_connections WHERE user_id = $1 AND game_id = $2",
                                    host.host_id,
                                    game.id
                                )
                                .fetch_optional(db_pool)
                                .await
                                .ok()
                                .flatten()
                                .map(|r| r.session_id)
                                .unwrap_or_else(|| "unknown".to_string());
                                
                                games.insert(game_code.to_string(), GameState {
                                    id: game.id,
                                    code: game_code.to_string(),
                                    host_session_id: host_session,
                                    host_id: host.host_id,
                                    question_set_id: host.question_set_id,
                                    players: HashMap::new(),
                                    current_question: -1, // Henüz başlamamış
                                    state: ConnectionState::Lobby,
                                    started_at: None,
                                    ended_at: None,
                                    question_timer: None,
                                    question_duration: None,
                                    total_questions,
                                });
                            }
                        }
                        
                        // Oyuna oyuncuyu ekle
                        if let Some(game_state) = games.get_mut(game_code) {
                            game_state.players.insert(session_id.to_string(), PlayerState {
                                player_id: player.id,
                                user_id,
                                session_id: session_id.to_string(),
                                nickname: display_name.clone(),
                                score: 0,
                                answers: HashMap::new(),
                                is_active: true,
                                joined_at: Instant::now(),
                                last_seen: Instant::now(),
                                last_answer_time: None,
                            });
                        }
                    }
                    
                    // Oyuncuya katılım onayı gönder
                    let _ = session.text(
                        json!({
                            "type": "join_success",
                            "player_id": player.id,
                            "game_code": game_code,
                            "nickname": display_name,
                            "is_guest": is_guest
                        })
                        .to_string(),
                    )
                    .await;
                    
                    // Lobideki oyuncuları getir
                    let players = sqlx::query!(
                        r#"
                        SELECT p.id, p.nickname, p.user_id IS NULL as is_guest
                        FROM players p
                        WHERE p.game_id = $1 AND p.is_active = true
                        "#,
                        game.id
                    )
                    .fetch_all(db_pool)
                    .await;
                    
                    if let Ok(players) = players {
                        let player_list: Vec<serde_json::Value> = players
                            .iter()
                            .map(|p| {
                                json!({
                                    "player_id": p.id,
                                    "nickname": p.nickname,
                                    "is_guest": p.is_guest.unwrap_or(false)
                                })
                            })
                            .collect();
                        
                        // Tüm lobiye yeni oyuncu bildirimini gönder
                        let lobby_update = json!({
                            "type": "lobby_update",
                            "game_code": game_code,
                            "players": player_list
                        })
                        .to_string();
                        
                        let _ = app_state.broadcast_to_game(game_code, &lobby_update).await;
                    }
                }
                Err(e) => {
                    error!("Oyuncu kaydedilirken hata: {}", e);
                    let _ = session.text(
                        json!({
                            "type": "error",
                            "message": "Oyuna katılırken bir hata oluştu"
                        })
                        .to_string(),
                    )
                    .await;
                }
            }
        }
        Ok(None) => {
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Oyun bulunamadı"
                })
                .to_string(),
            )
            .await;
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Oyuna katılırken bir hata oluştu"
                })
                .to_string(),
            )
            .await;
        }
    }
}

async fn handle_start_game(
    session: &mut Session,
    db_pool: &Pool<Postgres>,
    game_code: &str,
    session_id: &str,
    app_state: &web::Data<AppState>,
) {
    // Oyun ve host kontrolü
    let game = sqlx::query!(
        r#"
        SELECT g.id, g.host_id, g.status, g.question_set_id,
               ac.user_id
        FROM games g
        JOIN active_connections ac ON ac.session_id = $1
        WHERE g.code = $2
        "#,
        session_id,
        game_code
    )
    .fetch_optional(db_pool)
    .await;

    match game {
        Ok(Some(g)) => {
            // Sadece host oyunu başlatabilir
            if g.user_id != Some(g.host_id) {
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Sadece oyun sahibi oyunu başlatabilir"
                    })
                    .to_string(),
                )
                .await;
                return;
            }

            if g.status != "lobby" {
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Bu oyun zaten başlatılmış veya sonlanmış"
                    })
                    .to_string(),
                )
                .await;
                return;
            }

            // Oyun durumunu güncelle
            let update_result = sqlx::query!(
                r#"
                UPDATE games SET status = $1, started_at = $2
                WHERE id = $3
                "#,
                GameStatus::Active.to_string().to_lowercase(),
                Utc::now(),
                g.id
            )
            .execute(db_pool)
            .await;

            if let Err(e) = update_result {
                error!("Oyun başlatılırken hata: {}", e);
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Oyun başlatılırken bir hata oluştu"
                    })
                    .to_string(),
                )
                .await;
                return;
            }

            // Oyun durumunu bellekte güncelle
            {
                let mut games = app_state.games.lock().await;
                if let Some(game_state) = games.get_mut(game_code) {
                    game_state.state = ConnectionState::Game;
                    game_state.started_at = Some(Instant::now());
                }
            }

            // Tüm oyunculara oyunun başladığını bildir
            let start_message = json!({
                "type": "game_started",
                "game_code": game_code,
                "message": "Oyun başlatıldı, ilk soru için hazırlanın!"
            })
            .to_string();

            let _ = app_state.broadcast_to_game(game_code, &start_message).await;

            // İlk soruyu yükle
            handle_next_question(session, db_pool, game_code, session_id, app_state).await;
        }
        Ok(None) => {
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Oyun bulunamadı"
                })
                .to_string(),
            )
            .await;
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Oyun başlatılırken bir hata oluştu"
                })
                .to_string(),
            )
            .await;
        }
    }
}

async fn handle_submit_answer(
    session: &mut Session,
    db_pool: &Pool<Postgres>,
    question_id: i32,
    answer: &str,
    response_time_ms: i32,
    session_id: &str,
    app_state: &web::Data<AppState>,
) {
    // Oyuncu bilgilerini al
    let player = sqlx::query!(
        r#"
        SELECT p.id, p.game_id, p.nickname, g.code as game_code
        FROM players p 
        JOIN games g ON p.game_id = g.id
        JOIN active_connections ac ON p.session_id = ac.session_id
        WHERE ac.session_id = $1
        "#,
        session_id
    )
    .fetch_optional(db_pool)
    .await;

    match player {
        Ok(Some(p)) => {
            // Sorunun doğru cevabını kontrol et
            let question = sqlx::query!(
                "SELECT correct_option FROM questions WHERE id = $1",
                question_id
            )
            .fetch_optional(db_pool)
            .await;

            match question {
                Ok(Some(q)) => {
                    let is_correct = answer.to_uppercase() == q.correct_option;
                    
                    // Puanı hesapla
                    let points = if is_correct {
                        // Hızlı cevaplar daha çok puan alır
                        let max_points = 1000;
                        let min_points = 100;
                        let max_time_ms = 10000; // 10 saniye
                        
                        let time_factor = (max_time_ms - response_time_ms).max(0) as f64 / max_time_ms as f64;
                        (min_points as f64 + (max_points - min_points) as f64 * time_factor) as i32
                    } else {
                        0
                    };

                    // Cevabı kaydet
                    let answer_result = sqlx::query!(
                        r#"
                        INSERT INTO player_answers 
                        (player_id, question_id, answer, is_correct, response_time_ms, points_earned)
                        VALUES ($1, $2, $3, $4, $5, $6)
                        "#,
                        p.id,
                        question_id,
                        answer.to_uppercase(),
                        is_correct,
                        response_time_ms,
                        points
                    )
                    .execute(db_pool)
                    .await;

                    if let Ok(_) = answer_result {
                        // Oyuncu puanını güncelle
                        let _ = sqlx::query!(
                            "UPDATE players SET score = score + $1 WHERE id = $2",
                            points,
                            p.id
                        )
                        .execute(db_pool)
                        .await;

                        // Oyun durumunu güncelle (bellekte)
                        {
                            let mut games = app_state.games.lock().await;
                            if let Some(game) = games.get_mut(&p.game_code) {
                                if let Some(player_state) = game.players.get_mut(session_id) {
                                    player_state.score += points;
                                    player_state.last_answer_time = Some(Instant::now());
                                    
                                    let answer_obj = PlayerAnswer {
                                        question_id,
                                        answer: Some(answer.to_uppercase()),
                                        is_correct,
                                        response_time_ms,
                                        points_earned: points,
                                    };
                                    
                                    player_state.answers.insert(question_id, answer_obj);
                                }
                            }
                        }

                        // Oyuncuya sonucu bildir
                        let _ = session.text(
                            json!({
                                "type": "answer_received",
                                "question_id": question_id,
                                "your_answer": answer.to_uppercase(),
                                "is_correct": is_correct,
                                "points_earned": points,
                                "message": if is_correct {
                                    format!("Doğru! {} puan kazandınız", points)
                                } else {
                                    "Yanlış cevap".to_string()
                                }
                            })
                            .to_string(),
                        )
                        .await;
                    }
                }
                Ok(None) => {
                    let _ = session.text(
                        json!({
                            "type": "error",
                            "message": "Soru bulunamadı"
                        })
                        .to_string(),
                    )
                    .await;
                }
                Err(e) => {
                    error!("Veritabanı sorgu hatası: {}", e);
                    let _ = session.text(
                        json!({
                            "type": "error",
                            "message": "Cevabınız kaydedilirken bir hata oluştu"
                        })
                        .to_string(),
                    )
                    .await;
                }
            }
        }
        Ok(None) => {
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Aktif oyuncu bulunamadı"
                })
                .to_string(),
            )
            .await;
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Cevabınız kaydedilirken bir hata oluştu"
                })
                .to_string(),
            )
            .await;
        }
    }
}

async fn handle_next_question(
    session: &mut Session,
    db_pool: &Pool<Postgres>,
    game_code: &str,
    session_id: &str,
    app_state: &web::Data<AppState>,
) {
    // Oyun ve host kontrolü
    let game = sqlx::query!(
        r#"
        SELECT g.id, g.host_id, g.status, g.current_question, g.question_set_id,
               ac.user_id
        FROM games g
        JOIN active_connections ac ON ac.session_id = $1
        WHERE g.code = $2
        "#,
        session_id,
        game_code
    )
    .fetch_optional(db_pool)
    .await;

    match game {
        Ok(Some(g)) => {
            // Sadece host soruyu ilerletebilir
            if g.user_id != Some(g.host_id) {
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Sadece oyun sahibi soruları ilerletebilir"
                    })
                    .to_string(),
                )
                .await;
                return;
            }

            // Bir sonraki soruyu getir
            let next_question = g.current_question.unwrap_or(-1) + 1;
            
            // Soru bilgilerini getir
            let question = sqlx::query!(
                r#"
                SELECT id, question_text, option_a, option_b, option_c, option_d, 
                       correct_option, time_limit, position
                FROM questions
                WHERE question_set_id = $1 AND position = $2
                "#,
                g.question_set_id,
                next_question
            )
            .fetch_optional(db_pool)
            .await;

            // Toplam soru sayısını al
            let total_questions = sqlx::query!(
                "SELECT COUNT(*) as count FROM questions WHERE question_set_id = $1",
                g.question_set_id
            )
            .fetch_one(db_pool)
            .await
            .map(|r| r.count.unwrap_or(0) as i64)
            .unwrap_or(0);

            match question {
                Ok(Some(q)) => {
                    // Oyun durumunu güncelle
                    let _ = sqlx::query!(
                        "UPDATE games SET current_question = $1 WHERE id = $2",
                        next_question,
                        g.id
                    )
                    .execute(db_pool)
                    .await;

                    // Oyun durumunu bellekte güncelle
                    {
                        let mut games = app_state.games.lock().await;
                        if let Some(game_state) = games.get_mut(game_code) {
                            game_state.current_question = next_question;
                            game_state.state = ConnectionState::Question;
                            game_state.question_timer = Some(Instant::now());
                            game_state.question_duration = Some(Duration::from_secs(q.time_limit.unwrap_or(30) as u64));
                        }
                    }

                    // Tüm oyunculara soruyu gönder
                    let question_data = json!({
                        "type": "question_start",
                        "question_id": q.id,
                        "question_text": q.question_text,
                        "options": {
                            "A": q.option_a,
                            "B": q.option_b,
                            "C": q.option_c, 
                            "D": q.option_d
                        },
                        "time_limit": q.time_limit,
                        "question_number": next_question + 1,
                        "total_questions": total_questions
                    });

                    // Sorudan doğru cevabı çıkar (oyunculara gönderilmemeli)
                    let mut question_without_answer = question_data.clone();
                    if let Some(obj) = question_without_answer.as_object_mut() {
                        obj.remove("correct_option");
                    }

                    // Tüm oyunculara soruyu gönder
                    let _ = app_state.broadcast_to_game(game_code, &question_without_answer.to_string()).await;

                    // Host'a doğru cevapla birlikte gönder
                    let _ = session.text(
                        json!({
                            "type": "question_start",
                            "question_id": q.id,
                            "question_text": q.question_text,
                            "options": {
                                "A": q.option_a,
                                "B": q.option_b,
                                "C": q.option_c, 
                                "D": q.option_d
                            },
                            "correct_option": q.correct_option,
                            "time_limit": q.time_limit,
                            "question_number": next_question + 1,
                            "total_questions": total_questions
                        })
                        .to_string(),
                    )
                    .await;
                }
                Ok(None) => {
                    // Soru kalmadı, oyunu bitir
                    let _ = sqlx::query!(
                        r#"
                        UPDATE games SET status = 'completed', ended_at = NOW()
                        WHERE id = $1
                        "#,
                        g.id
                    )
                    .execute(db_pool)
                    .await;

                    // Oyun durumunu bellekte güncelle
                    {
                        let mut games = app_state.games.lock().await;
                        if let Some(game_state) = games.get_mut(game_code) {
                            game_state.state = ConnectionState::Ended;
                            game_state.ended_at = Some(Instant::now());
                        }
                    }

                    // Final skor tablosunu hesapla
                    let leaderboard = app_state.get_leaderboard(game_code).await;

                    if let Ok(leaderboard) = leaderboard {
                        // Oyun sonu performans istatistiklerini hesapla
                        let player_stats = sqlx::query!(
                            r#"
                            SELECT 
                                p.id as player_id,
                                p.nickname,
                                p.score,
                                COUNT(pa.id) as answer_count,
                                COUNT(pa.id) FILTER (WHERE pa.is_correct) as correct_count,
                                ROUND(AVG(pa.response_time_ms)) as avg_response_time
                            FROM players p
                            LEFT JOIN player_answers pa ON p.id = pa.player_id
                            WHERE p.game_id = $1 AND p.is_active = true
                            GROUP BY p.id, p.nickname, p.score
                            ORDER BY p.score DESC
                            "#,
                            g.id
                        )
                        .fetch_all(db_pool)
                        .await;

                        let stats_json = if let Ok(stats) = player_stats {
                            stats.iter().map(|s| {
                                let accuracy = if s.answer_count.unwrap_or(0) > 0 {
                                    (s.correct_count.unwrap_or(0) as f64 / s.answer_count.unwrap_or(0) as f64 * 100.0).round()
                                } else {
                                    0.0
                                };
                                
                                // BigDecimal'ı doğrudan kullanmak yerine bir string ya da sayıya çevir
                                let avg_time_value = match &s.avg_response_time {
                                    Some(bd) => bd.to_string().parse::<f64>().unwrap_or(0.0),
                                    None => 0.0
                                };
                                
                                json!({
                                    "player_id": s.player_id,
                                    "nickname": s.nickname,
                                    "score": s.score,
                                    "answers": s.answer_count,
                                    "correct": s.correct_count,
                                    "accuracy": accuracy,
                                    "avg_response_time_ms": avg_time_value
                                })
                            }).collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        };

                        // Tüm oyunculara sonuçları gönder
                        let _ = app_state.broadcast_to_game(game_code, &json!({
                            "type": "game_end",
                            "final_leaderboard": leaderboard,
                            "player_stats": stats_json,
                            "message": "Oyun tamamlandı, sonuçlar gösteriliyor"
                        }).to_string()).await;
                    }
                }
                Err(e) => {
                    error!("Veritabanı sorgu hatası: {}", e);
                    let _ = session.text(
                        json!({
                            "type": "error",
                            "message": "Bir sonraki soru alınırken bir hata oluştu"
                        })
                        .to_string(),
                    )
                    .await;
                }
            }
        }
        Ok(None) => {
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Oyun bulunamadı"
                })
                .to_string(),
            )
            .await;
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Bir sonraki soruya geçilirken bir hata oluştu"
                })
                .to_string(),
            )
            .await;
        }
    }
}

// Yeniden bağlanma işlevi
async fn handle_reconnect(
    session: &mut Session,
    db_pool: &Pool<Postgres>,
    old_session_id: &str,
    new_session_id: &str,
    app_state: &web::Data<AppState>,
) {
    info!("Yeniden bağlanma isteği: old_session_id={}, new_session_id={}", old_session_id, new_session_id);
    
    // Eski oturumun oyuncu bilgilerini kontrol et
    let player = sqlx::query!(
        r#"
        SELECT p.id, p.game_id, p.user_id, p.nickname, p.score, p.is_active,
               g.code as game_code, g.status, g.current_question
        FROM players p
        JOIN games g ON p.game_id = g.id
        WHERE p.session_id = $1
        "#,
        old_session_id
    )
    .fetch_optional(db_pool)
    .await;
    
    match player {
        Ok(Some(p)) => {
            if !p.is_active.unwrap_or(false) {
                // Oyuncu aktif değilse aktifleştir
                let _ = sqlx::query!(
                    "UPDATE players SET is_active = true, session_id = $1 WHERE id = $2",
                    new_session_id,
                    p.id
                )
                .execute(db_pool)
                .await;
                
                // Aktif bağlantıları güncelle
                let _ = sqlx::query!(
                    r#"
                    UPDATE active_connections
                    SET user_id = $1, game_id = $2, player_id = $3, connection_type = 'player'
                    WHERE session_id = $4
                    "#,
                    p.user_id,
                    p.game_id,
                    p.id,
                    new_session_id
                )
                .execute(db_pool)
                .await;
                
                // AppState'i güncelle
                {
                    let mut connections = app_state.active_connections.lock().await;
                    if let Some(conn) = connections.get_mut(new_session_id) {
                        conn.user_id = p.user_id;
                        conn.player_id = Some(p.id);
                        conn.game_id = Some(p.game_id);
                        conn.game_code = Some(p.game_code.clone());
                        conn.connection_type = ConnectionType::Player;
                    }
                    
                    // Eski bağlantıyı kaldır
                    connections.remove(old_session_id);
                }
                
                // Oyunu güncelle
                {
                    let mut games = app_state.games.lock().await;
                    if let Some(game) = games.get_mut(&p.game_code) {
                        // Eski oyuncuyu kaldır
                        if let Some(player_state) = game.players.remove(old_session_id) {
                            // Yeni session ID ile ekle
                            game.players.insert(new_session_id.to_string(), PlayerState {
                                player_id: p.id,
                                user_id: p.user_id,
                                session_id: new_session_id.to_string(),
                                nickname: p.nickname.clone(),
                                score: p.score.unwrap_or(0),
                                answers: player_state.answers,
                                is_active: true,
                                joined_at: player_state.joined_at,
                                last_seen: Instant::now(),
                                last_answer_time: player_state.last_answer_time,
                            });
                        }
                    }
                }
                
                // Oyuncuya mevcut oyun durumunu gönder
                let _ = session.text(
                    json!({
                        "type": "reconnect_success",
                        "player_id": p.id,
                        "game_code": p.game_code,
                        "nickname": p.nickname,
                        "score": p.score,
                        "game_status": p.status,
                        "current_question": p.current_question
                    })
                    .to_string(),
                )
                .await;
                
                // Oyunun mevcut durumuna göre ek bilgi gönder
                if p.status == "active" {
                    // Mevcut soruyu gönder
                    if let Some(current_q) = p.current_question {
                        let question = sqlx::query!(
                            r#"
                            SELECT id, question_text, option_a, option_b, option_c, option_d, time_limit, position
                            FROM questions
                            WHERE question_set_id = (SELECT question_set_id FROM games WHERE id = $1)
                            AND position = $2
                            "#,
                            p.game_id,
                            current_q
                        )
                        .fetch_optional(db_pool)
                        .await;
                        
                        if let Ok(Some(q)) = question {
                            let _ = session.text(
                                json!({
                                    "type": "current_question",
                                    "question_id": q.id,
                                    "question_text": q.question_text,
                                    "options": {
                                        "A": q.option_a,
                                        "B": q.option_b,
                                        "C": q.option_c, 
                                        "D": q.option_d
                                    },
                                    "time_limit": q.time_limit,
                                    "question_number": q.position + 1
                                })
                                .to_string(),
                            )
                            .await;
                            
                            // Oyuncunun bu soruya cevap verip vermediğini kontrol et
                            let answer = sqlx::query!(
                                "SELECT answer, is_correct, points_earned FROM player_answers WHERE player_id = $1 AND question_id = $2",
                                p.id,
                                q.id
                            )
                            .fetch_optional(db_pool)
                            .await;
                            
                            if let Ok(Some(a)) = answer {
                                // Oyuncu zaten cevap vermiş
                                let _ = session.text(
                                    json!({
                                        "type": "answer_received",
                                        "question_id": q.id,
                                        "your_answer": a.answer,
                                        "is_correct": a.is_correct,
                                        "points_earned": a.points_earned,
                                        "message": if a.is_correct {
                                            format!("Doğru! {} puan kazandınız", a.points_earned.unwrap_or(0))
                                        } else {
                                            "Yanlış cevap".to_string()
                                        }
                                    })
                                    .to_string(),
                                )
                                .await;
                            }
                        }
                    }
                    
                    // Liderlik tablosunu gönder
                    if let Ok(leaderboard) = app_state.get_leaderboard(&p.game_code).await {
                        let _ = session.text(
                            json!({
                                "type": "leaderboard_update",
                                "leaderboard": leaderboard
                            })
                            .to_string(),
                        )
                        .await;
                    }
                }
            } else {
                // Oyuncu zaten aktif
                let _ = session.text(
                    json!({
                        "type": "error",
                        "message": "Bu oturum zaten aktif"
                    })
                    .to_string(),
                )
                .await;
            }
        }
        Ok(None) => {
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Önceki oturum bulunamadı"
                })
                .to_string(),
            )
            .await;
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            let _ = session.text(
                json!({
                    "type": "error",
                    "message": "Yeniden bağlanırken bir hata oluştu"
                })
                .to_string(),
            )
            .await;
        }
    }
}