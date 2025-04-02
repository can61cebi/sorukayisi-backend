use actix_web::{web, HttpResponse, Responder};
use log::{error, info};
use sqlx::{Pool, Postgres};
use sqlx::types::BigDecimal;

use crate::db::models::Claims;

// BigDecimal değerlerini f64'e dönüştürmek için yardımcı fonksiyon
fn bigdecimal_to_f64(value: Option<BigDecimal>) -> f64 {
    match value {
        Some(bd) => bd.to_string().parse::<f64>().unwrap_or(0.0),
        None => 0.0
    }
}

// Oyuncu bilgilerini getir
pub async fn get_player_info(
    pool: web::Data<Pool<Postgres>>,
    player_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kez kullanıp saklayalım
    let player_id_inner = player_id.into_inner();
    
    // Oyuncu bilgilerini getir
    let player = sqlx::query!(
        r#"
        SELECT p.id, p.game_id, p.user_id, p.nickname, p.score, p.is_active,
               g.code as game_code, g.status as game_status, 
               u.username as username
        FROM players p
        JOIN games g ON p.game_id = g.id
        LEFT JOIN users u ON p.user_id = u.id
        WHERE p.id = $1
        "#,
        player_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match player {
        Ok(Some(player)) => {
            // Kullanıcı yetkisini kontrol et (kullanıcının kendisi, oyun sahibi veya admin görebilir)
            if player.user_id.is_some() && player.user_id.unwrap() != user_id && claims.role != "admin" {
                // Oyun sahibi mi kontrol et
                let is_host = sqlx::query!(
                    "SELECT host_id FROM games WHERE id = $1",
                    player.game_id
                )
                .fetch_optional(&**pool)
                .await
                .map(|g| g.map(|h| h.host_id == user_id))
                .unwrap_or(None)
                .unwrap_or(false);
                
                if !is_host {
                    return HttpResponse::Forbidden().json(serde_json::json!({
                        "error": "Bu oyuncu bilgilerine erişim izniniz yok"
                    }));
                }
            }
            
            HttpResponse::Ok().json(serde_json::json!({
                "id": player.id,
                "game_id": player.game_id,
                "game_code": player.game_code,
                "game_status": player.game_status,
                "user_id": player.user_id,
                "username": player.username,
                "nickname": player.nickname,
                "score": player.score,
                "is_active": player.is_active,
                "is_guest": player.user_id.is_none()
            }))
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyuncu bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyuncu bilgileri alınamadı"
            }))
        }
    }
}

// Oyuncunun cevap istatistiklerini getir
pub async fn get_player_stats(
    pool: web::Data<Pool<Postgres>>,
    player_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kez kullanıp saklayalım
    let player_id_inner = player_id.into_inner();
    
    // Oyuncu bilgilerini getir
    let player = sqlx::query!(
        "SELECT p.user_id, p.game_id, g.host_id FROM players p JOIN games g ON p.game_id = g.id WHERE p.id = $1",
        player_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match player {
        Ok(Some(player)) => {
            // Kullanıcı yetkisini kontrol et (kullanıcının kendisi, oyun sahibi veya admin görebilir)
            if player.user_id.is_some() && player.user_id.unwrap() != user_id && player.host_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu oyuncu istatistiklerine erişim izniniz yok"
                }));
            }
            
            // Oyuncu cevap istatistiklerini getir
            let stats = sqlx::query!(
                r#"
                SELECT 
                    COUNT(*) FILTER (WHERE is_correct = true) as "correct_count!",
                    COUNT(*) FILTER (WHERE is_correct = false) as "incorrect_count!",
                    ROUND(AVG(response_time_ms)) as "avg_response_time",
                    SUM(points_earned) as "total_points",
                    MAX(points_earned) as "max_points"
                FROM player_answers
                WHERE player_id = $1
                "#,
                player_id_inner
            )
            .fetch_one(&**pool)
            .await;
            
            match stats {
                Ok(stats) => {
                    // Soru bazında istatistikler
                    let questions = sqlx::query!(
                        r#"
                        SELECT 
                            pa.question_id, q.question_text, pa.answer, pa.is_correct, 
                            pa.response_time_ms, pa.points_earned,
                            q.correct_option
                        FROM player_answers pa
                        JOIN questions q ON pa.question_id = q.id
                        WHERE pa.player_id = $1
                        ORDER BY pa.answered_at
                        "#,
                        player_id_inner
                    )
                    .fetch_all(&**pool)
                    .await;
                    
                    let total_questions = stats.correct_count + stats.incorrect_count;
                    let accuracy = if total_questions > 0 {
                        (stats.correct_count as f64 / total_questions as f64 * 100.0).round()
                    } else {
                        0.0
                    };
                    
                    match questions {
                        Ok(question_stats) => {
                            // Performans değerlendirmesi
                            let performance_rating = if total_questions > 0 {
                                // Doğruluk oranı, yanıt süresi ve puan faktörlerine göre performans hesapla
                                let accuracy_factor = stats.correct_count as f64 / total_questions as f64;
                                
                                // Burada avg_time tanımlanmalı!
                                let avg_time = bigdecimal_to_f64(stats.avg_response_time.clone());
                                let time_factor = if avg_time > 0.0 {
                                    (10000.0 - avg_time.min(10000.0)) / 10000.0  // 10 saniye ve altı daha yüksek puan
                                } else {
                                    0.5 // Varsayılan
                                };
                                
                                let avg_points = if stats.correct_count > 0 {
                                    stats.total_points.unwrap_or(0) as f64 / stats.correct_count as f64 / 1000.0
                                } else {
                                    0.0
                                };
                                
                                // Puanları birleştir (0-10 arası)
                                let score = (accuracy_factor * 0.6 + time_factor * 0.2 + avg_points * 0.2) * 10.0;
                                
                                // Performans derecesi (A+, A, B+, B, C+, C, D, F)
                                if score >= 9.5 {
                                    "A+"
                                } else if score >= 8.5 {
                                    "A"
                                } else if score >= 7.5 {
                                    "B+"
                                } else if score >= 6.5 {
                                    "B"
                                } else if score >= 5.5 {
                                    "C+"
                                } else if score >= 4.5 {
                                    "C"
                                } else if score >= 3.5 {
                                    "D"
                                } else {
                                    "F"
                                }
                            } else {
                                "N/A"
                            };
                            
                            // Gelişim alanları
                            let areas_for_improvement = if total_questions > 0 {
                                let mut areas = Vec::new();
                                
                                if accuracy < 50.0 {
                                    areas.push("Doğruluk oranınız düşük. Konuları daha iyi anlamak için çalışmanız yararlı olabilir.");
                                }
                                
                                let avg_time = bigdecimal_to_f64(stats.avg_response_time.clone());
                                if avg_time > 5000.0 {
                                    areas.push("Yanıt süreniz yavaş. Daha hızlı cevap vermek için pratik yapabilirsiniz.");
                                }
                                
                                if areas.is_empty() {
                                    areas.push("Harika gidiyorsunuz! Performansınızı sürdürmeye devam edin.");
                                }
                                
                                areas
                            } else {
                                vec!["Henüz yeterli veri yok."]
                            };
                            
                            HttpResponse::Ok().json(serde_json::json!({
                                "summary": {
                                    "correct_count": stats.correct_count,
                                    "incorrect_count": stats.incorrect_count,
                                    "accuracy": accuracy,
                                    "avg_response_time_ms": bigdecimal_to_f64(stats.avg_response_time.clone()),
                                    "total_points": stats.total_points,
                                    "max_points": stats.max_points,
                                    "total_questions": total_questions,
                                    "performance_rating": performance_rating,
                                    "areas_for_improvement": areas_for_improvement
                                },
                                "questions": question_stats.iter().map(|q| {
                                    serde_json::json!({
                                        "question_id": q.question_id,
                                        "question_text": q.question_text,
                                        "answer": q.answer,
                                        "correct_answer": q.correct_option,
                                        "is_correct": q.is_correct,
                                        "response_time_ms": q.response_time_ms,
                                        "points_earned": q.points_earned
                                    })
                                }).collect::<Vec<_>>()
                            }))
                        }
                        Err(e) => {
                            error!("Soru istatistikleri alınamadı: {}", e);
                            HttpResponse::InternalServerError().json(serde_json::json!({
                                "error": "Soru istatistikleri alınamadı"
                            }))
                        }
                    }
                }
                Err(e) => {
                    error!("İstatistikler alınamadı: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Oyuncu istatistikleri alınamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyuncu bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyuncu bilgileri alınamadı"
            }))
        }
    }
}

// Kullanıcının oyun geçmişini getir
pub async fn get_user_game_history(
    pool: web::Data<Pool<Postgres>>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Kullanıcının oynadığı oyunların listesini getir
    let games = sqlx::query!(
        r#"
        SELECT 
            p.id as player_id, p.game_id, p.nickname, p.score, p.joined_at,
            g.code as game_code, g.status as game_status, g.started_at, g.ended_at,
            qs.title as question_set_title,
            u.username as host_username,
            (SELECT COUNT(*) FROM player_answers WHERE player_id = p.id) as answer_count,
            (SELECT COUNT(*) FROM player_answers WHERE player_id = p.id AND is_correct = true) as correct_count
        FROM players p
        JOIN games g ON p.game_id = g.id
        JOIN question_sets qs ON g.question_set_id = qs.id
        JOIN users u ON g.host_id = u.id
        WHERE p.user_id = $1
        ORDER BY p.joined_at DESC
        "#,
        user_id
    )
    .fetch_all(&**pool)
    .await;
    
    match games {
        Ok(games) => {
            let game_history = games.iter().map(|g| {
                let total_answers = g.answer_count.unwrap_or(0);
                let correct_answers = g.correct_count.unwrap_or(0);
                let accuracy = if total_answers > 0 {
                    (correct_answers as f64 / total_answers as f64 * 100.0).round()
                } else {
                    0.0
                };
                
                serde_json::json!({
                    "player_id": g.player_id,
                    "game_id": g.game_id,
                    "game_code": g.game_code,
                    "nickname": g.nickname,
                    "score": g.score,
                    "question_set_title": g.question_set_title,
                    "host_username": g.host_username,
                    "game_status": g.game_status,
                    "started_at": g.started_at,
                    "ended_at": g.ended_at,
                    "joined_at": g.joined_at,
                    "stats": {
                        "total_answers": total_answers,
                        "correct_answers": correct_answers,
                        "accuracy": accuracy
                    }
                })
            }).collect::<Vec<_>>();
            
            // Toplam istatistikler
            let total_games = game_history.len();
            let completed_games = games.iter().filter(|g| g.game_status == "completed").count();
            let total_score = games.iter().map(|g| g.score.unwrap_or(0)).sum::<i32>();
            let avg_score = if total_games > 0 {
                total_score as f64 / total_games as f64
            } else {
                0.0
            };
            
            // Toplam doğru/yanlış cevaplar
            let total_answers: i64 = games.iter().map(|g| g.answer_count.unwrap_or(0)).sum();
            let correct_answers: i64 = games.iter().map(|g| g.correct_count.unwrap_or(0)).sum();
            let overall_accuracy = if total_answers > 0 {
                (correct_answers as f64 / total_answers as f64 * 100.0).round()
            } else {
                0.0
            };
            
            HttpResponse::Ok().json(serde_json::json!({
                "user_id": user_id,
                "summary": {
                    "total_games": total_games,
                    "completed_games": completed_games,
                    "total_score": total_score,
                    "avg_score": avg_score,
                    "total_answers": total_answers,
                    "correct_answers": correct_answers,
                    "overall_accuracy": overall_accuracy
                },
                "games": game_history
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyun geçmişi alınamadı"
            }))
        }
    }
}

// Oyundan ayrıl
pub async fn leave_game(
    pool: web::Data<Pool<Postgres>>,
    player_id: web::Path<i32>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Path parametresini bir kez kullanıp saklayalım
    let player_id_inner = player_id.into_inner();
    
    // Oyuncu bilgilerini getir
    let player = sqlx::query!(
        "SELECT user_id, game_id FROM players WHERE id = $1",
        player_id_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match player {
        Ok(Some(player)) => {
            // Kullanıcı yetkisini kontrol et
            if player.user_id.is_some() && player.user_id.unwrap() != user_id {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu oyuncuyu oyundan çıkarma izniniz yok"
                }));
            }
            
            // Oyuncuyu pasif olarak işaretle
            let result = sqlx::query!(
                "UPDATE players SET is_active = false WHERE id = $1",
                player_id_inner
            )
            .execute(&**pool)
            .await;
            
            match result {
                Ok(_) => {
                    // Aktif bağlantıyı kaldır
                    let _ = sqlx::query!(
                        "DELETE FROM active_connections WHERE player_id = $1",
                        player_id_inner
                    )
                    .execute(&**pool)
                    .await;
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "Oyundan ayrıldınız"
                    }))
                }
                Err(e) => {
                    error!("Oyundan ayrılırken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Oyundan ayrılırken bir hata oluştu"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyuncu bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyuncu bilgileri alınamadı"
            }))
        }
    }
}