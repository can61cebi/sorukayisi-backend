// Veritabanı şema yapılandırması
// Bu dosya veritabanı şemasının program içerisindeki tanımlamalarını içerir

use sqlx::postgres::PgPool;
use log::info;

// Veritabanı şemasının doğruluğunu kontrol eden yardımcı fonksiyon
pub async fn check_schema(pool: &PgPool) -> bool {
    // Ana tabloların varlığını kontrol et
    let tables = ["users", "question_sets", "questions", "games", "players", "player_answers", "active_connections"];
    
    for table in tables {
        let result = sqlx::query!(
            r#"
            SELECT EXISTS (
                SELECT FROM information_schema.tables 
                WHERE table_schema = 'public' AND table_name = $1
            ) AS "exists!"
            "#,
            table
        )
        .fetch_one(pool)
        .await;
        
        match result {
            Ok(record) => {
                if !record.exists {
                    info!("Veritabanı şeması eksik: {} tablosu bulunamadı", table);
                    return false;
                }
            },
            Err(e) => {
                info!("Veritabanı şema kontrolü başarısız: {}", e);
                return false;
            }
        }
    }
    
    info!("Veritabanı şema kontrolü başarılı: Tüm tablolar mevcut");
    true
}

// Admin kullanıcısının varlığını kontrol et
pub async fn check_admin_user(pool: &PgPool) -> bool {
    let result = sqlx::query!(
        r#"SELECT COUNT(*) as "count!" FROM users WHERE username = 'cancebi' AND role = 'admin'"#
    )
    .fetch_one(pool)
    .await;
    
    match result {
        Ok(record) => {
            if record.count == 0 {
                info!("Admin kullanıcısı bulunamadı");
                return false;
            }
            info!("Admin kullanıcısı kontrol edildi");
            true
        },
        Err(e) => {
            info!("Admin kullanıcısı kontrolü başarısız: {}", e);
            false
        }
    }
}