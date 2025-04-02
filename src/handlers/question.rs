use actix_web::{web, HttpResponse, Responder};
use chrono::Utc;
use log::{error, info};
use sqlx::{Pool, Postgres};

use crate::db::models::{Claims, CreateQuestionDto, CreateQuestionSetDto};

// Yeni soru seti oluştur
pub async fn create_question_set(
    pool: web::Data<Pool<Postgres>>,
    set_dto: web::Json<CreateQuestionSetDto>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Kullanıcı rolünü kontrol et
    if claims.role != "teacher" && claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Sadece öğretmenler soru seti oluşturabilir"
        }));
    }
    
    // Soru setini veritabanına ekle
    let result = sqlx::query!(
        r#"
        INSERT INTO question_sets (creator_id, title, description, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, created_at
        "#,
        user_id,
        set_dto.title,
        set_dto.description,
        Utc::now(),
        Utc::now()
    )
    .fetch_one(&**pool)
    .await;
    
    match result {
        Ok(record) => {
            info!(
                "Soru seti oluşturuldu: {} (user_id: {})",
                set_dto.title, user_id
            );
            
            HttpResponse::Created().json(serde_json::json!({
                "id": record.id,
                "title": set_dto.title,
                "description": set_dto.description,
                "created_at": record.created_at
            }))
        }
        Err(e) => {
            error!("Soru seti oluşturulurken hata: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru seti oluşturulamadı"
            }))
        }
    }
}

// Soru ekle
pub async fn create_question(
    pool: web::Data<Pool<Postgres>>,
    question_dto: web::Json<CreateQuestionDto>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Kullanıcı rolünü kontrol et
    if claims.role != "teacher" && claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Sadece öğretmenler soru ekleyebilir"
        }));
    }
    
    // Soru setinin bu kullanıcıya ait olup olmadığını kontrol et
    let question_set = sqlx::query!(
        "SELECT creator_id FROM question_sets WHERE id = $1",
        question_dto.question_set_id
    )
    .fetch_optional(&**pool)
    .await;
    
    match question_set {
        Ok(Some(set)) => {
            if set.creator_id != user_id {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu soru seti size ait değil"
                }));
            }
            
            // Doğru cevap kontrolü
            let correct_option = question_dto.correct_option.to_uppercase();
            if !["A", "B", "C", "D"].contains(&correct_option.as_str()) {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Doğru cevap A, B, C veya D olmalıdır"
                }));
            }
            
            // Varsayılan değerleri belirle
            let points = question_dto.points.unwrap_or(100);
            let time_limit = question_dto.time_limit.unwrap_or(30);
            
            // Soruyu veritabanına ekle
            let result = sqlx::query!(
                r#"
                INSERT INTO questions 
                (question_set_id, question_text, option_a, option_b, option_c, option_d,
                correct_option, points, time_limit, position)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                RETURNING id
                "#,
                question_dto.question_set_id,
                question_dto.question_text,
                question_dto.option_a,
                question_dto.option_b,
                question_dto.option_c,
                question_dto.option_d,
                correct_option,
                points,
                time_limit,
                question_dto.position
            )
            .fetch_one(&**pool)
            .await;
            
            match result {
                Ok(record) => {
                    // Soru seti güncelleme zamanını güncelle
                    let _ = sqlx::query!(
                        "UPDATE question_sets SET updated_at = $1 WHERE id = $2",
                        Utc::now(),
                        question_dto.question_set_id
                    )
                    .execute(&**pool)
                    .await;
                    
                    info!(
                        "Soru eklendi: id={}, soru seti={}",
                        record.id, question_dto.question_set_id
                    );
                    
                    HttpResponse::Created().json(serde_json::json!({
                        "id": record.id,
                        "question_set_id": question_dto.question_set_id,
                        "question_text": question_dto.question_text,
                        "option_a": question_dto.option_a,
                        "option_b": question_dto.option_b,
                        "option_c": question_dto.option_c,
                        "option_d": question_dto.option_d,
                        "correct_option": correct_option,
                        "points": points,
                        "time_limit": time_limit,
                        "position": question_dto.position
                    }))
                }
                Err(e) => {
                    error!("Soru eklenirken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Soru eklenemedi"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Soru seti bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru eklenemedi"
            }))
        }
    }
}

// Kullanıcının soru setlerini getir
pub async fn get_question_sets(
    pool: web::Data<Pool<Postgres>>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Kullanıcının tüm soru setlerini getir
    let sets = sqlx::query!(
        r#"
        SELECT id, title, description, created_at, updated_at
        FROM question_sets
        WHERE creator_id = $1
        ORDER BY updated_at DESC
        "#,
        user_id
    )
    .fetch_all(&**pool)
    .await;
    
    match sets {
        Ok(sets) => {
            // Her soru seti için soru sayısını getir
            let mut result = Vec::new();
            
            for set in sets {
                let question_count = sqlx::query!(
                    "SELECT COUNT(*) as count FROM questions WHERE question_set_id = $1",
                    set.id
                )
                .fetch_one(&**pool)
                .await;
                
                let count = question_count.map(|c| c.count.unwrap_or(0)).unwrap_or(0);
                
                result.push(serde_json::json!({
                    "id": set.id,
                    "title": set.title,
                    "description": set.description,
                    "created_at": set.created_at,
                    "updated_at": set.updated_at,
                    "question_count": count
                }));
            }
            
            HttpResponse::Ok().json(serde_json::json!({
                "question_sets": result
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru setleri alınamadı"
            }))
        }
    }
}

// Soru setini detayları ile getir
pub async fn get_question_set(
    pool: web::Data<Pool<Postgres>>,
    set_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kere kullan ve sakla
    let set_id_inner = set_id.into_inner();
    
    // Soru setini getir
    let set = sqlx::query!(
        r#"
        SELECT id, creator_id, title, description, created_at, updated_at
        FROM question_sets
        WHERE id = $1
        "#,
        set_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match set {
        Ok(Some(set)) => {
            // Soru setinin bu kullanıcıya ait olup olmadığını kontrol et
            if set.creator_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu soru setine erişim izniniz yok"
                }));
            }
            
            // Soruları getir
            let questions = sqlx::query!(
                r#"
                SELECT id, question_text, option_a, option_b, option_c, option_d,
                       correct_option, points, time_limit, position
                FROM questions
                WHERE question_set_id = $1
                ORDER BY position
                "#,
                set.id
            )
            .fetch_all(&**pool)
            .await;
            
            match questions {
                Ok(questions) => {
                    // Soruları JSON formatına çevir
                    let questions_json: Vec<serde_json::Value> = questions
                        .iter()
                        .map(|q| {
                            serde_json::json!({
                                "id": q.id,
                                "question_text": q.question_text,
                                "option_a": q.option_a,
                                "option_b": q.option_b,
                                "option_c": q.option_c,
                                "option_d": q.option_d,
                                "correct_option": q.correct_option,
                                "points": q.points,
                                "time_limit": q.time_limit,
                                "position": q.position
                            })
                        })
                        .collect();
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "id": set.id,
                        "title": set.title,
                        "description": set.description,
                        "created_at": set.created_at,
                        "updated_at": set.updated_at,
                        "questions": questions_json,
                        "question_count": questions.len()
                    }))
                }
                Err(e) => {
                    error!("Veritabanı sorgu hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Sorular alınamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Soru seti bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru seti alınamadı"
            }))
        }
    }
}

// Soru seti sil
pub async fn delete_question_set(
    pool: web::Data<Pool<Postgres>>,
    set_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kere kullan ve sakla
    let set_id_inner = set_id.into_inner();
    
    // Soru setini getir
    let set = sqlx::query!(
        "SELECT creator_id FROM question_sets WHERE id = $1",
        set_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match set {
        Ok(Some(set)) => {
            // Soru setinin bu kullanıcıya ait olup olmadığını kontrol et
            if set.creator_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu soru setini silme izniniz yok"
                }));
            }
            
            // Soru setini ve ilişkili soruları sil (cascade)
            let result = sqlx::query!(
                "DELETE FROM question_sets WHERE id = $1",
                set_id_inner
            )
            .execute(&**pool)
            .await;
            
            match result {
                Ok(_) => {
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "Soru seti başarıyla silindi"
                    }))
                }
                Err(e) => {
                    error!("Soru seti silinirken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Soru seti silinemedi"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Soru seti bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru seti silinemedi"
            }))
        }
    }
}

// Soruyu sil
pub async fn delete_question(
    pool: web::Data<Pool<Postgres>>,
    question_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kere kullan ve sakla
    let question_id_inner = question_id.into_inner();
    
    // Soruyu ve ilişkili soru setini getir
    let question = sqlx::query!(
        r#"
        SELECT q.id, qs.creator_id, q.question_set_id
        FROM questions q
        JOIN question_sets qs ON q.question_set_id = qs.id
        WHERE q.id = $1
        "#,
        question_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match question {
        Ok(Some(question)) => {
            // Soru setinin bu kullanıcıya ait olup olmadığını kontrol et
            if question.creator_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu soruyu silme izniniz yok"
                }));
            }
            
            // Soruyu sil
            let result = sqlx::query!(
                "DELETE FROM questions WHERE id = $1",
                question.id
            )
            .execute(&**pool)
            .await;
            
            match result {
                Ok(_) => {
                    // Soru setinin güncellenme zamanını güncelle
                    let _ = sqlx::query!(
                        "UPDATE question_sets SET updated_at = $1 WHERE id = $2",
                        Utc::now(),
                        question.question_set_id
                    )
                    .execute(&**pool)
                    .await;
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "Soru başarıyla silindi"
                    }))
                }
                Err(e) => {
                    error!("Soru silinirken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Soru silinemedi"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Soru bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru silinemedi"
            }))
        }
    }
}

// Soruyu güncelle
pub async fn update_question(
    pool: web::Data<Pool<Postgres>>,
    question_id: web::Path<i32>,
    question_dto: web::Json<CreateQuestionDto>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kere kullan ve sakla
    let question_id_inner = question_id.into_inner();
    
    // Soruyu ve ilişkili soru setini getir
    let question = sqlx::query!(
        r#"
        SELECT q.id, qs.creator_id, q.question_set_id
        FROM questions q
        JOIN question_sets qs ON q.question_set_id = qs.id
        WHERE q.id = $1
        "#,
        question_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match question {
        Ok(Some(question)) => {
            // Soru setinin bu kullanıcıya ait olup olmadığını kontrol et
            if question.creator_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu soruyu güncelleme izniniz yok"
                }));
            }
            
            // Doğru cevap kontrolü
            let correct_option = question_dto.correct_option.to_uppercase();
            if !["A", "B", "C", "D"].contains(&correct_option.as_str()) {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Doğru cevap A, B, C veya D olmalıdır"
                }));
            }
            
            // Varsayılan değerleri belirle
            let points = question_dto.points.unwrap_or(100);
            let time_limit = question_dto.time_limit.unwrap_or(30);
            
            // Soruyu güncelle
            let result = sqlx::query!(
                r#"
                UPDATE questions 
                SET question_text = $1, option_a = $2, option_b = $3, option_c = $4, option_d = $5,
                    correct_option = $6, points = $7, time_limit = $8, position = $9
                WHERE id = $10
                RETURNING id
                "#,
                question_dto.question_text,
                question_dto.option_a,
                question_dto.option_b,
                question_dto.option_c,
                question_dto.option_d,
                correct_option,
                points,
                time_limit,
                question_dto.position,
                question.id
            )
            .fetch_one(&**pool)
            .await;
            
            match result {
                Ok(_) => {
                    // Soru setinin güncellenme zamanını güncelle
                    let _ = sqlx::query!(
                        "UPDATE question_sets SET updated_at = $1 WHERE id = $2",
                        Utc::now(),
                        question.question_set_id
                    )
                    .execute(&**pool)
                    .await;
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "id": question.id,
                        "question_set_id": question.question_set_id,
                        "question_text": question_dto.question_text,
                        "option_a": question_dto.option_a,
                        "option_b": question_dto.option_b,
                        "option_c": question_dto.option_c,
                        "option_d": question_dto.option_d,
                        "correct_option": correct_option,
                        "points": points,
                        "time_limit": time_limit,
                        "position": question_dto.position
                    }))
                }
                Err(e) => {
                    error!("Soru güncellenirken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Soru güncellenemedi"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Soru bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Soru güncellenemedi"
            }))
        }
    }
}