use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::{distributions::Alphanumeric, Rng};
use uuid::Uuid;

use crate::{config::CONFIG, db::models::Claims};

// Şifre hashleme
pub fn hash_password(password: &str) -> Result<String, anyhow::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)?
        .to_string();
    Ok(password_hash)
}

// Şifre doğrulama
pub fn verify_password(password: &str, hash: &str) -> Result<bool, anyhow::Error> {
    let parsed_hash = PasswordHash::new(hash)?;
    let result = Argon2::default().verify_password(password.as_bytes(), &parsed_hash);
    Ok(result.is_ok())
}

// JWT token oluşturma
pub fn generate_jwt(user_id: i32, role: &str) -> Result<String, anyhow::Error> {
    let expiration = Utc::now()
        .checked_add_signed(Duration::seconds(CONFIG.jwt_expiration))
        .expect("Invalid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: user_id.to_string(),
        role: role.to_string(),
        exp: expiration,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(CONFIG.jwt_secret.as_bytes()),
    )?;

    Ok(token)
}

// JWT token çözme
pub fn decode_jwt(token: &str) -> Result<Claims, anyhow::Error> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(CONFIG.jwt_secret.as_bytes()),
        &Validation::default(),
    )?;

    Ok(token_data.claims)
}

// Doğrulama tokeni oluşturma
pub fn generate_verification_token() -> String {
    Uuid::new_v4().to_string()
}

// Rastgele kod oluşturma (oyun kodları için)
pub fn generate_game_code() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect::<String>()
        .to_uppercase()
}

// Öğretmen onay tokeni oluşturma
pub fn generate_approval_token() -> String {
    Uuid::new_v4().to_string()
}

// Şifre sıfırlama tokeni oluşturma
pub fn generate_reset_token() -> String {
    Uuid::new_v4().to_string()
}