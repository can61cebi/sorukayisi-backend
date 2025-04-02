use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    error::ErrorUnauthorized,
    Error,
};
use futures_util::{
    future::{ready, Ready},
    FutureExt,
};
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::config::CONFIG;

// reCAPTCHA v3 doğrulama yanıtı
#[derive(Debug, Serialize, Deserialize)]
struct RecaptchaResponse {
    success: bool,
    #[serde(rename = "error-codes")]
    error_codes: Option<Vec<String>>,
    score: Option<f64>,
    action: Option<String>,
}

// reCAPTCHA middleware yapısı
pub struct RecaptchaValidator;

impl<S, B> Transform<S, ServiceRequest> for RecaptchaValidator
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RecaptchaMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RecaptchaMiddleware {
            service: Arc::new(service),
        }))
    }
}

pub struct RecaptchaMiddleware<S> {
    service: Arc<S>,
}

impl<S, B> Service<ServiceRequest> for RecaptchaMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // Sadece belirli rotaları doğrula (kayıt, giriş gibi)
        let path = req.path().to_string();
        
        if !path.starts_with("/api/auth/register") && !path.starts_with("/api/auth/login") {
            // Diğer rotaları atla
            let service = Arc::clone(&self.service);
            return Box::pin(async move {
                service.call(req).await
            });
        }
        
        // Token'ı header'dan al
        let recaptcha_token = match req.headers().get("X-Recaptcha-Token") {
            Some(token) => match token.to_str() {
                Ok(t) => t.to_string(),
                Err(_) => {
                    return Box::pin(async move {
                        Err(ErrorUnauthorized("Geçersiz reCAPTCHA token formatı"))
                    });
                }
            },
            None => {
                debug!("Korumalı yol için reCAPTCHA tokenı bulunamadı: {}", path);
                return Box::pin(async move {
                    Err(ErrorUnauthorized("reCAPTCHA doğrulaması gerekli"))
                });
            }
        };
        
        let secret_key = CONFIG.recaptcha_secret_key.clone();
        let service = Arc::clone(&self.service);
        
        Box::pin(async move {
            // Google API'si ile doğrula
            let client = reqwest::Client::new();
            let response = match client
                .post("https://www.google.com/recaptcha/api/siteverify")
                .form(&[
                    ("secret", &secret_key),
                    ("response", &recaptcha_token),
                ])
                .send()
                .await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!("reCAPTCHA tokenı doğrulanamadı: {}", e);
                        return Err(ErrorUnauthorized("reCAPTCHA tokenı doğrulanamadı"));
                    }
                };
            
            // JSON yanıtını ayrıştır
            let recaptcha_result: Result<RecaptchaResponse, _> = response.json().await;
            
            match recaptcha_result {
                Ok(result) => {
                    if result.success {
                        if let Some(score) = result.score {
                            if score > 0.5 {
                                debug!("reCAPTCHA doğrulaması başarılı, score: {}", score);
                                service.call(req).await
                            } else {
                                warn!("reCAPTCHA score çok düşük: {}", score);
                                Err(ErrorUnauthorized("reCAPTCHA score çok düşük"))
                            }
                        } else {
                            warn!("reCAPTCHA yanıtında score yok");
                            Err(ErrorUnauthorized("Geçersiz reCAPTCHA yanıtı"))
                        }
                    } else {
                        let error_codes = result.error_codes.unwrap_or_default().join(", ");
                        warn!("reCAPTCHA doğrulaması başarısız: {}", error_codes);
                        Err(ErrorUnauthorized(format!("reCAPTCHA doğrulaması başarısız: {}", error_codes)))
                    }
                },
                Err(e) => {
                    error!("reCAPTCHA yanıtı ayrıştırılamadı: {}", e);
                    Err(ErrorUnauthorized("Geçersiz reCAPTCHA yanıtı"))
                }
            }
        })
    }
}

impl<S> Clone for RecaptchaMiddleware<S> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
        }
    }
}