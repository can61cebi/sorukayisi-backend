use actix_web::{web, HttpResponse, Responder};
use chrono::{Duration, Utc};
use log::{error, info};
use sqlx::{Pool, Postgres};

use crate::db::models::{Claims, CreateUserDto, LoginDto, UserRole};
use crate::services::email::EmailService;
use crate::utils::security::{
    generate_jwt, generate_reset_token, generate_verification_token, hash_password, verify_password,
};
use crate::utils::validation;

// Kullanıcı kayıt işleyicisi
pub async fn register(
    pool: web::Data<Pool<Postgres>>,
    user_dto: web::Json<CreateUserDto>,
) -> impl Responder {
    // Alan doğrulamalarını yap
    if !validation::validate_email(&user_dto.email) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "E-posta adresi .edu.tr veya .edu ile bitmelidir"
        }));
    }

    if !validation::validate_username(&user_dto.username) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Kullanıcı adı geçersiz. 3-30 karakter arasında olmalı ve sadece harf, rakam ve alt çizgi içermelidir."
        }));
    }

    if !validation::validate_password(&user_dto.password) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Şifre en az 8 karakter uzunluğunda olmalıdır."
        }));
    }

    // E-posta adresinin zaten kayıtlı olup olmadığını kontrol et
    let existing_user = sqlx::query!(
        "SELECT id FROM users WHERE email = $1",
        user_dto.email
    )
    .fetch_optional(&**pool)
    .await;

    if let Ok(Some(_)) = existing_user {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Bu e-posta adresi zaten kullanımda"
        }));
    }

    // Kullanıcı adının zaten kayıtlı olup olmadığını kontrol et
    let existing_username = sqlx::query!(
        "SELECT id FROM users WHERE username = $1",
        user_dto.username
    )
    .fetch_optional(&**pool)
    .await;

    if let Ok(Some(_)) = existing_username {
        return HttpResponse::Conflict().json(serde_json::json!({
            "error": "Bu kullanıcı adı zaten kullanımda"
        }));
    }

    // Misafirler için ** öneki kontrol et
    if user_dto.username.starts_with("**") {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Kullanıcı adı '**' ile başlayamaz (bu prefix misafir kullanıcılar için ayrılmıştır)"
        }));
    }

    // Şifreyi hashle
    let password_hash = match hash_password(&user_dto.password) {
        Ok(hash) => hash,
        Err(e) => {
            error!("Şifre hashleme hatası: {}", e);
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Kayıt işlemi başarısız oldu"
            }));
        }
    };

    // Doğrulama tokeni oluştur
    let verification_token = generate_verification_token();

    // Kullanıcıyı veritabanına ekle
    let role = user_dto.role.clone();
    let is_approved = match &role {
        UserRole::Student => true, // Öğrenci hesapları otomatik onaylanır
        UserRole::Teacher => false, // Öğretmen hesapları admin onayı gerektirir
        UserRole::Admin => false, // Admin hesapları oluşturulamaz (hardcoded)
    };

    let result = sqlx::query!(
        r#"
        INSERT INTO users (username, email, password_hash, role, is_approved, is_email_verified, verification_token, created_at)
        VALUES ($1, $2, $3, $4, $5, false, $6, $7)
        RETURNING id
        "#,
        user_dto.username,
        user_dto.email,
        password_hash,
        role.to_string().to_lowercase(),
        is_approved,
        verification_token,
        Utc::now()
    )
    .fetch_one(&**pool)
    .await;

    match result {
        Ok(record) => {
            // E-posta doğrulama mesajı gönder
            let email_service = EmailService::new();
            match email_service
                .send_verification_email(&user_dto.email, &user_dto.username, &verification_token)
                .await
            {
                Ok(_) => {
                    info!(
                        "Kullanıcı başarıyla kaydedildi ve doğrulama e-postası gönderildi: {}",
                        user_dto.email
                    );
                }
                Err(e) => {
                    error!(
                        "Doğrulama e-postası gönderilemedi ({}): {}",
                        user_dto.email, e
                    );
                    // E-posta gönderilemese bile kullanıcı kaydedilir
                }
            }

            // Başarılı yanıt
            HttpResponse::Created().json(serde_json::json!({
                "id": record.id,
                "username": user_dto.username,
                "email": user_dto.email,
                "role": role.to_string().to_lowercase(),
                "is_approved": is_approved,
                "is_email_verified": false,
                "message": "Kullanıcı başarıyla kaydedildi. Lütfen e-posta adresinizi doğrulayın."
            }))
        }
        Err(e) => {
            error!("Kullanıcı kaydedilirken hata oluştu: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Kayıt işlemi başarısız oldu"
            }))
        }
    }
}

// Kullanıcı girişi işleyicisi
pub async fn login(
    pool: web::Data<Pool<Postgres>>,
    login_dto: web::Json<LoginDto>,
) -> impl Responder {
    // Kullanıcıyı e-posta adresi ile bul
    let user = sqlx::query!(
        r#"
        SELECT id, username, email, password_hash, role, is_approved, is_email_verified
        FROM users
        WHERE email = $1
        "#,
        login_dto.email
    )
    .fetch_optional(&**pool)
    .await;

    match user {
        Ok(Some(user)) => {
            // Şifreyi doğrula
            match verify_password(&login_dto.password, &user.password_hash) {
                Ok(true) => {
                    // E-posta doğrulaması kontrolü
                    if !user.is_email_verified.unwrap_or(false) {
                        return HttpResponse::Unauthorized().json(serde_json::json!({
                            "error": "Lütfen e-posta adresinizi doğrulayın"
                        }));
                    }

                    // Öğretmen onayı kontrol et
                    if user.role == "teacher" && !user.is_approved.unwrap_or(false) {
                        return HttpResponse::Forbidden().json(serde_json::json!({
                            "error": "Öğretmen hesabınız henüz onaylanmadı"
                        }));
                    }

                    // Son giriş zamanını güncelle
                    let _ = sqlx::query!(
                        "UPDATE users SET last_login = $1 WHERE id = $2",
                        Utc::now(),
                        user.id
                    )
                    .execute(&**pool)
                    .await;

                    // JWT token oluştur
                    match generate_jwt(user.id, &user.role) {
                        Ok(token) => {
                            info!("Kullanıcı giriş yaptı: {}", user.email);
                            HttpResponse::Ok().json(serde_json::json!({
                                "token": token,
                                "user": {
                                    "id": user.id,
                                    "username": user.username,
                                    "email": user.email,
                                    "role": user.role,
                                }
                            }))
                        }
                        Err(e) => {
                            error!("Token oluşturma hatası: {}", e);
                            HttpResponse::InternalServerError().json(serde_json::json!({
                                "error": "Giriş işlemi başarısız oldu"
                            }))
                        }
                    }
                }
                Ok(false) => {
                    HttpResponse::Unauthorized().json(serde_json::json!({
                        "error": "Geçersiz e-posta veya şifre"
                    }))
                }
                Err(e) => {
                    error!("Şifre doğrulama hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Giriş işlemi başarısız oldu"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "Geçersiz e-posta veya şifre"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Giriş işlemi başarısız oldu"
            }))
        }
    }
}

// E-posta doğrulama işleyicisi
pub async fn verify_email(
    pool: web::Data<Pool<Postgres>>,
    token: web::Path<String>,
) -> impl Responder {
    // Tokeni kullanarak kullanıcıyı bul
    let token_inner = token.into_inner();
    let user = sqlx::query!(
        "SELECT id, username, email FROM users WHERE verification_token = $1",
        token_inner
    )
    .fetch_optional(&**pool)
    .await;

    match user {
        Ok(Some(user)) => {
            // Kullanıcıyı doğrulanmış olarak işaretle
            let result = sqlx::query!(
                "UPDATE users SET is_email_verified = true, verification_token = NULL WHERE id = $1",
                user.id
            )
            .execute(&**pool)
            .await;

            match result {
                Ok(_) => {
                    info!("E-posta doğrulandı: {}", user.email);
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "E-posta adresiniz başarıyla doğrulandı. Şimdi giriş yapabilirsiniz."
                    }))
                }
                Err(e) => {
                    error!("E-posta doğrulama güncellemesi başarısız oldu: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "E-posta doğrulama başarısız oldu"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Geçersiz veya süresi dolmuş doğrulama tokeni"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "E-posta doğrulama başarısız oldu"
            }))
        }
    }
}

// Mevcut kullanıcı bilgilerini getir
pub async fn get_current_user(
    pool: web::Data<Pool<Postgres>>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();

    // Kullanıcı bilgilerini getir
    let user = sqlx::query!(
        r#"
        SELECT id, username, email, role, is_approved, is_email_verified, created_at, last_login
        FROM users
        WHERE id = $1
        "#,
        user_id
    )
    .fetch_optional(&**pool)
    .await;

    match user {
        Ok(Some(user)) => {
            HttpResponse::Ok().json(serde_json::json!({
                "id": user.id,
                "username": user.username,
                "email": user.email,
                "role": user.role,
                "is_approved": user.is_approved,
                "is_email_verified": user.is_email_verified,
                "created_at": user.created_at,
                "last_login": user.last_login,
            }))
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Kullanıcı bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Kullanıcı bilgileri alınamadı"
            }))
        }
    }
}

// Şifre sıfırlama isteği işleyicisi
pub async fn request_password_reset(
    pool: web::Data<Pool<Postgres>>,
    email: web::Json<String>,
) -> impl Responder {
    // Kullanıcıyı e-posta ile bul
    let user = sqlx::query!(
        "SELECT id, username, email FROM users WHERE email = $1",
        email.into_inner()
    )
    .fetch_optional(&**pool)
    .await;
    
    match user {
        Ok(Some(user)) => {
            // Sıfırlama tokeni oluştur
            let reset_token = generate_reset_token();
            let expires_at = Utc::now() + Duration::hours(24);
            
            // Tokeni veritabanına kaydet
            let _ = sqlx::query!(
                "UPDATE users SET reset_token = $1, reset_token_expires_at = $2 WHERE id = $3",
                reset_token,
                expires_at,
                user.id
            )
            .execute(&**pool)
            .await;
            
            // E-posta gönder
            let email_service = EmailService::new();
            let _ = email_service.send_password_reset_email(
                &user.email,
                &user.username,
                &reset_token
            ).await;
            
            HttpResponse::Ok().json(serde_json::json!({
                "message": "Şifre sıfırlama talimatları e-posta adresinize gönderildi"
            }))
        }
        _ => {
            // Güvenlik nedeniyle aynı mesajı gösterelim
            HttpResponse::Ok().json(serde_json::json!({
                "message": "Şifre sıfırlama talimatları e-posta adresinize gönderildi"
            }))
        }
    }
}

// Şifre sıfırlama işleyicisi
pub async fn reset_password(
    pool: web::Data<Pool<Postgres>>,
    token: web::Path<String>,
    new_password: web::Json<String>,
) -> impl Responder {
    if !validation::validate_password(&new_password) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "Şifre en az 8 karakter uzunluğunda olmalıdır."
        }));
    }

    // Tokeni kullanarak kullanıcıyı bul
    let token_inner = token.into_inner();
    let user = sqlx::query!(
        "SELECT id FROM users WHERE reset_token = $1 AND reset_token_expires_at > $2",
        token_inner,
        Utc::now()
    )
    .fetch_optional(&**pool)
    .await;

    match user {
        Ok(Some(user)) => {
            // Yeni şifreyi hashle
            let password_hash = match hash_password(&new_password) {
                Ok(hash) => hash,
                Err(e) => {
                    error!("Şifre hashleme hatası: {}", e);
                    return HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Şifre sıfırlama başarısız oldu"
                    }));
                }
            };

            // Kullanıcının şifresini güncelle
            let result = sqlx::query!(
                "UPDATE users SET password_hash = $1, reset_token = NULL, reset_token_expires_at = NULL WHERE id = $2",
                password_hash,
                user.id
            )
            .execute(&**pool)
            .await;

            match result {
                Ok(_) => {
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "Şifreniz başarıyla sıfırlandı. Şimdi giriş yapabilirsiniz."
                    }))
                }
                Err(e) => {
                    error!("Şifre güncelleme hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Şifre sıfırlama başarısız oldu"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Geçersiz veya süresi dolmuş sıfırlama tokeni"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Şifre sıfırlama başarısız oldu"
            }))
        }
    }
}