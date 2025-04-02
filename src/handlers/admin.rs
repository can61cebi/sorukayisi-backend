use actix_web::{web, HttpResponse, Responder};
use log::{error, info};
use sqlx::{Pool, Postgres};

use crate::db::models::{ApproveUserDto, Claims};
use crate::services::email::EmailService;

// Onay bekleyen öğretmenleri listele
pub async fn list_pending_teachers(
    pool: web::Data<Pool<Postgres>>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    // Sadece adminler erişebilir
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Bu işlem için admin yetkisi gerekiyor"
        }));
    }
    
    // Onay bekleyen öğretmenleri getir
    let teachers = sqlx::query!(
        r#"
        SELECT id, username, email, created_at
        FROM users
        WHERE role = 'teacher' AND is_approved = false AND is_email_verified = true
        ORDER BY created_at
        "#
    )
    .fetch_all(&**pool)
    .await;
    
    match teachers {
        Ok(teachers) => {
            HttpResponse::Ok().json(serde_json::json!({
                "pending_teachers": teachers.iter().map(|t| {
                    serde_json::json!({
                        "id": t.id,
                        "username": t.username,
                        "email": t.email,
                        "created_at": t.created_at
                    })
                }).collect::<Vec<_>>()
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Öğretmen listesi alınamadı"
            }))
        }
    }
}

// Öğretmen onaylama/reddetme
pub async fn approve_teacher(
    pool: web::Data<Pool<Postgres>>,
    approval: web::Json<ApproveUserDto>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    // Sadece adminler erişebilir
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Bu işlem için admin yetkisi gerekiyor"
        }));
    }
    
    // Kullanıcının öğretmen olup olmadığını kontrol et
    let user = sqlx::query!(
        r#"
        SELECT id, username, email, role
        FROM users
        WHERE id = $1
        "#,
        approval.user_id
    )
    .fetch_optional(&**pool)
    .await;
    
    match user {
        Ok(Some(user)) => {
            if user.role != "teacher" {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Bu kullanıcı öğretmen değil"
                }));
            }
            
            // Öğretmeni onayla/reddet
            let result = sqlx::query!(
                "UPDATE users SET is_approved = $1 WHERE id = $2",
                approval.approve,
                approval.user_id
            )
            .execute(&**pool)
            .await;
            
            match result {
                Ok(_) => {
                    // Kullanıcıya bildirim e-postası gönder
                    let email_service = EmailService::new();
                    let _ = email_service
                        .send_teacher_approval_email(
                            &user.email,
                            &user.username,
                            approval.approve,
                        )
                        .await;
                    
                    info!(
                        "Öğretmen {} {}",
                        user.username,
                        if approval.approve { "onaylandı" } else { "reddedildi" }
                    );
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": format!(
                            "Öğretmen {} {}",
                            user.username,
                            if approval.approve { "onaylandı" } else { "reddedildi" }
                        )
                    }))
                }
                Err(e) => {
                    error!("Öğretmen onaylama hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Öğretmen onaylanamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Kullanıcı bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Öğretmen onaylanamadı"
            }))
        }
    }
}

// Tüm kullanıcıları listele (admin için)
pub async fn list_all_users(
    pool: web::Data<Pool<Postgres>>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    // Sadece adminler erişebilir
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Bu işlem için admin yetkisi gerekiyor"
        }));
    }
    
    // Tüm kullanıcıları getir
    let users = sqlx::query!(
        r#"
        SELECT id, username, email, role, is_approved, is_email_verified, created_at, last_login
        FROM users
        ORDER BY created_at DESC
        "#
    )
    .fetch_all(&**pool)
    .await;
    
    match users {
        Ok(users) => {
            HttpResponse::Ok().json(serde_json::json!({
                "users": users.iter().map(|u| {
                    serde_json::json!({
                        "id": u.id,
                        "username": u.username,
                        "email": u.email,
                        "role": u.role,
                        "is_approved": u.is_approved,
                        "is_email_verified": u.is_email_verified,
                        "created_at": u.created_at,
                        "last_login": u.last_login
                    })
                }).collect::<Vec<_>>()
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Kullanıcı listesi alınamadı"
            }))
        }
    }
}

// Kullanıcı sil
pub async fn delete_user(
    pool: web::Data<Pool<Postgres>>,
    user_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    // Sadece adminler erişebilir
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Bu işlem için admin yetkisi gerekiyor"
        }));
    }
    
    // into_inner'ı bir kez kullanıp saklayalım
    let user_id_inner = user_id.into_inner();
    
    // Admin kullanıcıyı silemez
    if user_id_inner == 1 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Ana admin kullanıcı silinemez"
        }));
    }
    
    // Kullanıcıyı getir
    let user = sqlx::query!(
        "SELECT username FROM users WHERE id = $1",
        user_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match user {
        Ok(Some(user)) => {
            // Kullanıcıyı sil (cascade ile ilişkili tüm veriler silinecek)
            let result = sqlx::query!(
                "DELETE FROM users WHERE id = $1",
                user_id_inner
            )
            .execute(&**pool)
            .await;
            
            match result {
                Ok(_) => {
                    info!("Kullanıcı silindi: {}", user.username);
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": format!("Kullanıcı silindi: {}", user.username)
                    }))
                }
                Err(e) => {
                    error!("Kullanıcı silme hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Kullanıcı silinemedi"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Kullanıcı bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Kullanıcı silinemedi"
            }))
        }
    }
}

// Sistem istatistiklerini getir
pub async fn get_system_stats(
    pool: web::Data<Pool<Postgres>>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    // Sadece adminler erişebilir
    if claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Bu işlem için admin yetkisi gerekiyor"
        }));
    }
    
    // Kullanıcı sayıları
    let user_counts = sqlx::query!(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE role = 'student') as student_count,
            COUNT(*) FILTER (WHERE role = 'teacher') as teacher_count,
            COUNT(*) FILTER (WHERE role = 'teacher' AND is_approved = false) as pending_teacher_count,
            COUNT(*) FILTER (WHERE is_email_verified = false) as unverified_count
        FROM users
        "#
    )
    .fetch_one(&**pool)
    .await;
    
    // Oyun ve soru seti sayıları
    let content_counts = sqlx::query!(
        r#"
        SELECT
            (SELECT COUNT(*) FROM question_sets) as question_set_count,
            (SELECT COUNT(*) FROM questions) as question_count,
            (SELECT COUNT(*) FROM games) as game_count,
            (SELECT COUNT(*) FROM games WHERE status = 'active') as active_game_count,
            (SELECT COUNT(*) FROM players) as player_count
        "#
    )
    .fetch_one(&**pool)
    .await;
    
    // Aktif bağlantı sayısı
    let active_connections = sqlx::query!(
        r#"
        SELECT COUNT(*) as count FROM active_connections
        WHERE last_seen > CURRENT_TIMESTAMP - INTERVAL '1 minute'
        "#
    )
    .fetch_one(&**pool)
    .await;
    
    match (user_counts, content_counts, active_connections) {
        (Ok(users), Ok(content), Ok(connections)) => {
            HttpResponse::Ok().json(serde_json::json!({
                "users": {
                    "total": (users.student_count.unwrap_or(0) + users.teacher_count.unwrap_or(0) + 1), // +1 for admin
                    "students": users.student_count.unwrap_or(0),
                    "teachers": users.teacher_count.unwrap_or(0),
                    "pending_teachers": users.pending_teacher_count.unwrap_or(0),
                    "unverified": users.unverified_count.unwrap_or(0)
                },
                "content": {
                    "question_sets": content.question_set_count.unwrap_or(0),
                    "questions": content.question_count.unwrap_or(0),
                    "games": {
                        "total": content.game_count.unwrap_or(0),
                        "active": content.active_game_count.unwrap_or(0)
                    },
                    "players": content.player_count.unwrap_or(0)
                },
                "system": {
                    "active_connections": connections.count.unwrap_or(0)
                }
            }))
        }
        _ => {
            error!("İstatistikler alınırken hata oluştu");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Sistem istatistikleri alınamadı"
            }))
        }
    }
}