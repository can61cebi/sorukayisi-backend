use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    error::ErrorUnauthorized,
    http::header,
    Error, HttpMessage,
};
use futures_util::future::{ready, Ready};
use log::{debug, error};
use std::future::{Future};
use std::pin::Pin;

use crate::utils::security::decode_jwt;

// JWT Kimlik Doğrulama Middleware
pub struct JwtAuth;

impl<S, B> Transform<S, ServiceRequest> for JwtAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = JwtAuthMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(JwtAuthMiddleware { service }))
    }
}

pub struct JwtAuthMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for JwtAuthMiddleware<S>
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
        // Authorization header'ını kontrol et
        let auth_header = req.headers().get(header::AUTHORIZATION);
        
        let auth_token = match auth_header {
            Some(header) => {
                let header_str = match header.to_str() {
                    Ok(s) => s,
                    Err(_) => {
                        return Box::pin(async move {
                            Err(ErrorUnauthorized("Geçersiz yetkilendirme başlığı"))
                        });
                    }
                };
                
                // "Bearer " önekini kontrol et
                if !header_str.starts_with("Bearer ") {
                    return Box::pin(async move {
                        Err(ErrorUnauthorized("Geçersiz yetkilendirme başlığı formatı"))
                    });
                }
                
                header_str[7..].to_string() // "Bearer " önekini kaldır
            }
            None => {
                // Bazı yollar için token gerektirmeyen (public routes) yolları kontrol et
                let path = req.path();
                
                if path.starts_with("/api/auth/login") 
                   || path.starts_with("/api/auth/register")
                   || path.starts_with("/api/auth/verify")
                   || path.starts_with("/api/health")
                   || path.starts_with("/ws")
                   || path.starts_with("/health")
                   || path == "/api/game/join" // Misafir oyuncular için
                {
                    // Bu yollar için token gerekmiyor, normal akışa devam et
                    return Box::pin(self.service.call(req));
                }
                
                return Box::pin(async move {
                    Err(ErrorUnauthorized("Yetkilendirme başlığı eksik"))
                });
            }
        };
        
        // JWT token'ı doğrula
        let claims = match decode_jwt(&auth_token) {
            Ok(claims) => claims,
            Err(e) => {
                error!("JWT token doğrulama hatası: {}", e);
                return Box::pin(async move {
                    Err(ErrorUnauthorized("Geçersiz veya süresi dolmuş token"))
                });
            }
        };
        
        // Yetki kontrolü
        // Bu kısımda rol bazlı erişim kontrolleri yapılabilir
        debug!("JWT doğrulandı: user_id={}, role={}", claims.sub, claims.role);
        
        // Claims'i request uzantısına ekle
        req.extensions_mut().insert(claims);
        
        // Servisi çağır
        let fut = self.service.call(req);
        Box::pin(async move {
            let res = fut.await?;
            Ok(res)
        })
    }
}