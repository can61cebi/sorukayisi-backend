use lazy_static::lazy_static;
use std::env;

// Uygulamanın tüm konfigürasyon ayarları
pub struct Config {
    pub database_url: String,
    pub server_addr: String,
    pub jwt_secret: String,
    pub jwt_expiration: i64,
    pub email_from: String,
    pub email_server: String,
    pub email_username: String,
    pub email_password: String,
    pub recaptcha_secret_key: String,
    pub frontend_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            server_addr: env::var("SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            jwt_secret: env::var("JWT_SECRET").expect("JWT_SECRET must be set"),
            jwt_expiration: env::var("JWT_EXPIRATION")
                .unwrap_or_else(|_| "86400".to_string())
                .parse::<i64>()
                .expect("JWT_EXPIRATION must be a number"),
            email_from: env::var("EMAIL_FROM").expect("EMAIL_FROM must be set"),
            email_server: env::var("EMAIL_SERVER").expect("EMAIL_SERVER must be set"),
            email_username: env::var("EMAIL_USERNAME").expect("EMAIL_USERNAME must be set"),
            email_password: env::var("EMAIL_PASSWORD").expect("EMAIL_PASSWORD must be set"),
            recaptcha_secret_key: env::var("RECAPTCHA_SECRET_KEY").expect("RECAPTCHA_SECRET_KEY must be set"),
            frontend_url: env::var("FRONTEND_URL").expect("FRONTEND_URL must be set"),
        }
    }
}

lazy_static! {
    pub static ref CONFIG: Config = Config::from_env();
}

// Ortam değişkenlerini yükler
pub fn load_config() {
    dotenv::dotenv().ok();
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    lazy_static::initialize(&CONFIG);
    
    // Kritik değişkenleri kontrol et
    let _ = &CONFIG.database_url;
    let _ = &CONFIG.jwt_secret;
}