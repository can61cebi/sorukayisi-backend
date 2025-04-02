use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};
use derive_more::Display;
use serde::{Deserialize, Serialize};
use sqlx::error::Error as SqlxError;
use std::convert::From;

#[derive(Debug, Display)]
pub enum AppError {
    #[display(fmt = "Kimlik doğrulama hatası: {}", _0)]
    AuthError(String),
    
    #[display(fmt = "Yetkilendirme hatası: {}", _0)]
    ForbiddenError(String),
    
    #[display(fmt = "Bulunamadı: {}", _0)]
    NotFoundError(String),
    
    #[display(fmt = "Geçersiz istek: {}", _0)]
    BadRequestError(String),
    
    #[display(fmt = "İç sunucu hatası: {}", _0)]
    InternalError(String),
    
    #[display(fmt = "Veritabanı hatası: {}", _0)]
    DatabaseError(String),
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        let status = self.status_code();
        
        let error_response = ErrorResponse {
            error: self.to_string(),
            status_code: status.as_u16(),
        };
        
        HttpResponse::build(status).json(error_response)
    }
    
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::AuthError(_) => StatusCode::UNAUTHORIZED,
            AppError::ForbiddenError(_) => StatusCode::FORBIDDEN,
            AppError::NotFoundError(_) => StatusCode::NOT_FOUND,
            AppError::BadRequestError(_) => StatusCode::BAD_REQUEST,
            AppError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<SqlxError> for AppError {
    fn from(error: SqlxError) -> Self {
        match error {
            SqlxError::RowNotFound => AppError::NotFoundError("Kayıt bulunamadı".to_string()),
            _ => AppError::DatabaseError(error.to_string()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
    status_code: u16,
}