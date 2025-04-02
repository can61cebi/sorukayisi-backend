use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use crate::config::CONFIG;
use log::info;

pub type DbPool = Arc<PgPool>;

pub async fn create_pool() -> DbPool {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&CONFIG.database_url)
        .await
        .expect("Veritabanına bağlanılamadı");
    
    info!("Veritabanı bağlantısı başarıyla kuruldu");
    
    // Veritabanı şemasını kontrol et
    check_database_schema(&pool).await;
    
    Arc::new(pool)
}

async fn check_database_schema(pool: &PgPool) {
    // Gerekli tabloların varlığını kontrol et
    let table_exists = sqlx::query!(
        "SELECT EXISTS (
            SELECT FROM information_schema.tables 
            WHERE table_schema = 'public' 
            AND table_name = 'users'
        ) as exists"
    )
    .fetch_one(pool)
    .await;
    
    match table_exists {
        Ok(result) => {
            if !result.exists.unwrap_or(false) {
                panic!("Veritabanı şeması eksik. Lütfen migrasyon betiğini çalıştırın.");
            }
            
            info!("Veritabanı şeması doğrulandı");
        }
        Err(e) => {
            panic!("Veritabanı şeması kontrol edilemedi: {}", e);
        }
    }
}