use actix_cors::Cors;
use actix_web::{middleware::Logger, web, App, HttpServer};
use log::info;
use sqlx::postgres::PgPoolOptions;

mod config;
mod db;
mod errors;
mod handlers;
mod middleware;
mod services;
mod utils;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Konfigürasyonu yükle
    config::load_config();
    
    // Veritabanı bağlantısı kur
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config::CONFIG.database_url)
        .await
        .expect("Veritabanına bağlanılamadı");
    
    // Aktif kullanıcıları temizle (sunucu yeniden başlatıldığında)
    sqlx::query!("DELETE FROM active_connections")
        .execute(&pool)
        .await
        .expect("Aktif bağlantılar temizlenemedi");
    
    info!("Veritabanı bağlantısı başarıyla kuruldu");
    
    // WebSocket durumunu başlat
    let ws_state = handlers::websocket::AppState::new(pool.clone());
    let ws_data = web::Data::new(ws_state);
    
    // Sunucuyu başlat
    info!("Sunucu başlatılıyor: {}", &config::CONFIG.server_addr);
    
    HttpServer::new(move || {
        // CORS yapılandırması
        let cors = Cors::default()
            .allowed_origin(&config::CONFIG.frontend_url)
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE"])
            .allowed_headers(vec!["Content-Type", "Authorization", "X-Recaptcha-Token"])
            .max_age(3600);
        
        App::new()
            .wrap(Logger::default())
            .wrap(cors)
            .wrap(middleware::JwtAuth)
            // reCAPTCHA doğrulayıcısını etkinleştir
            .wrap(middleware::RecaptchaValidator)
            // WebSocket paylaşılan durumunu ekle
            .app_data(ws_data.clone())
            .app_data(web::Data::new(pool.clone()))
            .configure(handlers::configure_routes)
    })
    .bind(&config::CONFIG.server_addr)?
    .run()
    .await
}