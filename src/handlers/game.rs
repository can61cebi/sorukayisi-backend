use actix_web::{web, HttpRequest, HttpResponse, Responder};
use chrono::Utc;
use log::{debug, error, info};
use sqlx::{Pool, Postgres};
use sqlx::types::BigDecimal;
use uuid::Uuid;

use crate::db::models::{Claims, CreateGameDto, GameStatus, JoinGameDto, LeaderboardEntry, SubmitAnswerDto, PlayerStatistics, QuestionStatistics};
use crate::services::email::EmailService;
use crate::utils::security::generate_game_code;

// BigDecimal değerlerini f64'e dönüştürmek için yardımcı fonksiyon
fn bigdecimal_to_f64(value: Option<BigDecimal>) -> f64 {
    match value {
        Some(bd) => bd.to_string().parse::<f64>().unwrap_or(0.0),
        None => 0.0
    }
}

// Yeni oyun oluştur
pub async fn create_game(
    pool: web::Data<Pool<Postgres>>,
    game_dto: web::Json<CreateGameDto>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    
    // Kullanıcı rolünü kontrol et
    if claims.role != "teacher" && claims.role != "admin" {
        return HttpResponse::Forbidden().json(serde_json::json!({
            "error": "Sadece öğretmenler oyun oluşturabilir"
        }));
    }
    
    // Soru setinin varlığını kontrol et
    let question_set = sqlx::query!(
        "SELECT id, title, creator_id FROM question_sets WHERE id = $1",
        game_dto.question_set_id
    )
    .fetch_optional(&**pool)
    .await;
    
    match question_set {
        Ok(Some(set)) => {
            // Soru setinin bu kullanıcıya ait olup olmadığını kontrol et
            if set.creator_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu soru seti size ait değil"
                }));
            }

            // Soru setinde soru var mı kontrol et
            let question_count = sqlx::query!(
                "SELECT COUNT(*) as count FROM questions WHERE question_set_id = $1",
                set.id
            )
            .fetch_one(&**pool)
            .await;

            if let Ok(count) = question_count {
                if count.count.unwrap_or(0) == 0 {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "Bu soru setinde hiç soru yok"
                    }));
                }
            }
            
            // Benzersiz oyun kodu oluştur
            let game_code = generate_game_code();
            
            // Oyunu veritabanına ekle
            let game_result = sqlx::query!(
                r#"
                INSERT INTO games (code, question_set_id, host_id, status, created_at)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING id, code, created_at
                "#,
                game_code,
                game_dto.question_set_id,
                user_id,
                GameStatus::Lobby.to_string().to_lowercase(),
                Utc::now()
            )
            .fetch_one(&**pool)
            .await;
            
            match game_result {
                Ok(game) => {
                    // Kullanıcıya oyun bağlantısını e-posta ile gönder
                    let user = sqlx::query!(
                        "SELECT email, username FROM users WHERE id = $1",
                        user_id
                    )
                    .fetch_one(&**pool)
                    .await;
                    
                    if let Ok(user) = user {
                        let email_service = EmailService::new();
                        let _ = email_service.send_game_invitation(
                            &user.email,
                            &user.username,
                            &game.code,
                            &set.title,
                        ).await;
                    }
                    
                    HttpResponse::Created().json(serde_json::json!({
                        "id": game.id,
                        "code": game.code,
                        "question_set_id": game_dto.question_set_id,
                        "status": "lobby",
                        "created_at": game.created_at
                    }))
                }
                Err(e) => {
                    error!("Oyun oluşturulurken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Oyun oluşturulamadı"
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
                "error": "Oyun oluşturulamadı"
            }))
        }
    }
}

// Oyuna katıl
pub async fn join_game(
    pool: web::Data<Pool<Postgres>>,
    join_dto: web::Json<JoinGameDto>,
    claims: Option<web::ReqData<Claims>>,
) -> impl Responder {
    // Oyunun varlığını ve durumunu kontrol et
    let game = sqlx::query!(
        "SELECT id, status FROM games WHERE code = $1",
        join_dto.game_code
    )
    .fetch_optional(&**pool)
    .await;
    
    match game {
        Ok(Some(game)) => {
            if game.status != "lobby" {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Bu oyun artık katılıma açık değil"
                }));
            }
            
            let user_id = claims.as_ref().map(|c| c.sub.parse::<i32>().unwrap_or_default());
            let session_id = Uuid::new_v4().to_string();
            
            // Oyuncu bilgilerini hazırla
            let nickname = match (user_id, &join_dto.nickname) {
                (Some(id), _) => {
                    // Kayıtlı kullanıcı - kullanıcı adını veritabanından al
                    let user_result = sqlx::query!(
                        "SELECT username FROM users WHERE id = $1",
                        id
                    )
                    .fetch_one(&**pool)
                    .await;
                    
                    match user_result {
                        Ok(user) => user.username,
                        Err(_) => return HttpResponse::InternalServerError().json(serde_json::json!({
                            "error": "Kullanıcı bilgileri alınamadı"
                        }))
                    }
                }
                (None, Some(nickname)) => {
                    // Misafir kullanıcı - verilen takma adı kullan, ** ekle
                    if !nickname.starts_with("**") {
                        format!("**{}", nickname)
                    } else {
                        nickname.clone()
                    }
                }
                (None, None) => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "Misafir kullanıcılar için takma ad zorunludur"
                    }))
                }
            };
            
            // Takma adın oyunda benzersiz olup olmadığını kontrol et
            let existing_player = sqlx::query!(
                "SELECT id FROM players WHERE game_id = $1 AND nickname = $2",
                game.id,
                nickname
            )
            .fetch_optional(&**pool)
            .await;
            
            if let Ok(Some(_)) = existing_player {
                return HttpResponse::Conflict().json(serde_json::json!({
                    "error": "Bu takma ad zaten kullanılıyor"
                }));
            }
            
            // Oyuncuyu veritabanına ekle
            let player_result = sqlx::query!(
                r#"
                INSERT INTO players (game_id, user_id, nickname, session_id, joined_at)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING id
                "#,
                game.id,
                user_id,
                nickname,
                session_id,
                Utc::now()
            )
            .fetch_one(&**pool)
            .await;
            
            match player_result {
                Ok(player) => {
                    // Aktif bağlantıyı güncelle - oyuncu bağlantısı olarak işaretle
                    let _ = sqlx::query!(
                        r#"
                        INSERT INTO active_connections (session_id, user_id, game_id, player_id, connection_type, last_seen)
                        VALUES ($1, $2, $3, $4, 'player', $5)
                        "#,
                        session_id,
                        user_id,
                        game.id,
                        player.id,
                        Utc::now()
                    )
                    .execute(&**pool)
                    .await;
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "player_id": player.id,
                        "game_id": game.id,
                        "session_id": session_id,
                        "nickname": nickname,
                        "is_guest": user_id.is_none(),
                        "message": "Lobby'ye başarıyla katıldınız. Oyun başlayana kadar bekleyin."
                    }))
                }
                Err(e) => {
                    error!("Oyuna katılırken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Oyuna katılınamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyun bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyuna katılınamadı"
            }))
        }
    }
}

// Oyunu başlat
pub async fn start_game(
    pool: web::Data<Pool<Postgres>>,
    game_code: web::Path<String>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    let game_code_inner = game_code.into_inner();
    
    // Oyunu bul ve host'un bu kullanıcı olup olmadığını kontrol et
    let game = sqlx::query!(
        "SELECT id, host_id, status FROM games WHERE code = $1",
        game_code_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match game {
        Ok(Some(game)) => {
            if game.host_id != user_id {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Sadece oyun sahibi oyunu başlatabilir"
                }));
            }
            
            if game.status != "lobby" {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Bu oyun zaten başlatılmış veya tamamlanmış"
                }));
            }
            
            // Oyun durumunu güncelle
            let update_result = sqlx::query!(
                r#"
                UPDATE games
                SET status = $1, started_at = $2
                WHERE id = $3
                "#,
                GameStatus::Active.to_string().to_lowercase(),
                Utc::now(),
                game.id
            )
            .execute(&**pool)
            .await;
            
            match update_result {
                Ok(_) => {
                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "Oyun başlatıldı",
                        "game_id": game.id,
                        "status": "active",
                        "started_at": Utc::now()
                    }))
                }
                Err(e) => {
                    error!("Oyun başlatılırken hata: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Oyun başlatılamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyun bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyun başlatılamadı"
            }))
        }
    }
}

// Liderlik tablosunu getir
pub async fn get_leaderboard(
    pool: web::Data<Pool<Postgres>>,
    game_code: web::Path<String>,
) -> impl Responder {
    let game_code_inner = game_code.into_inner();
    
    // Oyun bilgilerini getir
    let game = sqlx::query!(
        "SELECT id FROM games WHERE code = $1",
        game_code_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match game {
        Ok(Some(game)) => {
            // Oyuncuları puanlarına göre sırala
            let players = sqlx::query!(
                r#"
                SELECT 
                    p.id, 
                    p.nickname, 
                    p.score, 
                    p.user_id IS NULL as is_guest,
                    COUNT(pa.id) as answer_count,
                    COUNT(pa.id) FILTER (WHERE pa.is_correct) as correct_count
                FROM players p
                LEFT JOIN player_answers pa ON p.id = pa.player_id
                WHERE p.game_id = $1 AND p.is_active = true
                GROUP BY p.id, p.nickname, p.score
                ORDER BY p.score DESC
                LIMIT 100
                "#,
                game.id
            )
            .fetch_all(&**pool)
            .await;
            
            match players {
                Ok(players) => {
                    let leaderboard: Vec<LeaderboardEntry> = players
                        .iter()
                        .map(|p| LeaderboardEntry {
                            player_id: p.id,
                            nickname: p.nickname.clone(),
                            score: p.score.unwrap_or(0),
                            is_guest: p.is_guest.unwrap_or(false),
                        })
                        .collect();
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "leaderboard": leaderboard
                    }))
                }
                Err(e) => {
                    error!("Veritabanı sorgu hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Liderlik tablosu alınamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyun bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyun bilgileri alınamadı"
            }))
        }
    }
}

// Cevap gönderme işleyicisi
pub async fn submit_answer_with_header(
    req: HttpRequest,
    pool: web::Data<Pool<Postgres>>,
    answer_dto: web::Json<SubmitAnswerDto>,
) -> HttpResponse {
    // Session ID'yi header'dan al
    let session_id_str = match req.headers().get("session-id") {
        Some(value) => match value.to_str() {
            Ok(str_value) => str_value.to_string(),
            Err(_) => return HttpResponse::BadRequest().json(serde_json::json!({
                "error": "Geçersiz session-id header değeri"
            })),
        },
        None => return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "session-id header eksik"
        })),
    };
    
    // İç fonksiyonu çağır
    submit_answer_internal(pool, answer_dto, session_id_str).await
}

// Cevap gönderme işleminin iç fonksiyonu
async fn submit_answer_internal(
    pool: web::Data<Pool<Postgres>>,
    answer_dto: web::Json<SubmitAnswerDto>,
    session_id: String,
) -> HttpResponse {  
    // Oyuncu ve oyun bilgilerini kontrol et
    let player = sqlx::query!(
        r#"
        SELECT p.id, p.user_id, p.game_id, p.nickname, g.status, g.current_question
        FROM players p
        JOIN games g ON p.game_id = g.id
        WHERE p.session_id = $1 AND p.is_active = true
        "#,
        session_id
    )
    .fetch_optional(&**pool)
    .await;
    
    match player {
        Ok(Some(player)) => {
            if player.status != "active" {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Oyun aktif değil"
                }));
            }
            
            // Mevcut soru kontrolü - doğru soru için cevap gönderiliyor mu?
            let current_question_position = player.current_question.unwrap_or(0);
            let question_position = sqlx::query!(
                "SELECT position FROM questions WHERE id = $1",
                answer_dto.question_id
            )
            .fetch_optional(&**pool)
            .await;
            
            if let Ok(Some(pos)) = question_position {
                if pos.position != current_question_position {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "error": "Bu soru şu anda aktif değil"
                    }));
                }
            } else {
                return HttpResponse::NotFound().json(serde_json::json!({
                    "error": "Soru bulunamadı"
                }));
            }
            
            // Oyuncunun bu soruya daha önce cevap verip vermediğini kontrol et
            let existing_answer = sqlx::query!(
                "SELECT id FROM player_answers WHERE player_id = $1 AND question_id = $2",
                player.id,
                answer_dto.question_id
            )
            .fetch_optional(&**pool)
            .await;
            
            if let Ok(Some(_)) = existing_answer {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Bu soruya zaten cevap verdiniz"
                }));
            }
            
            // Sorunun doğru cevabını bul
            let question = sqlx::query!(
                r#"
                SELECT correct_option, question_set_id FROM questions WHERE id = $1
                "#,
                answer_dto.question_id
            )
            .fetch_optional(&**pool)
            .await;
            
            match question {
                Ok(Some(question)) => {
                    // Sorunun bu oyuna ait olup olmadığını kontrol et
                    let question_set = sqlx::query!(
                        r#"
                        SELECT id FROM games WHERE id = $1 AND question_set_id = $2
                        "#,
                        player.game_id,
                        question.question_set_id
                    )
                    .fetch_optional(&**pool)
                    .await;
                    
                    if question_set.is_err() || question_set.unwrap().is_none() {
                        return HttpResponse::BadRequest().json(serde_json::json!({
                            "error": "Bu soru bu oyuna ait değil"
                        }));
                    }
                    
                    // Cevabın doğru olup olmadığını kontrol et
                    let is_correct = answer_dto.answer.to_uppercase() == question.correct_option;
                    
                    // Puanı hesapla - hız temelli puanlama
                    let points = if is_correct {
                        // Daha hızlı cevaplar daha yüksek puan alır
                        // En fazla 1000 puan, en az 100 puan (10 saniye için)
                        let max_points = 1000;
                        let min_points = 100;
                        let max_time_ms = 10000; // 10 saniye
                        
                        let time_factor = (max_time_ms - answer_dto.response_time_ms).max(0) as f64 / max_time_ms as f64;
                        (min_points as f64 + (max_points - min_points) as f64 * time_factor) as i32
                    } else {
                        0
                    };
                    
                    // Cevabı veritabanına kaydet
                    let answer_result = sqlx::query!(
                        r#"
                        INSERT INTO player_answers
                        (player_id, question_id, answer, is_correct, response_time_ms, points_earned)
                        VALUES ($1, $2, $3, $4, $5, $6)
                        RETURNING id, points_earned
                        "#,
                        player.id,
                        answer_dto.question_id,
                        answer_dto.answer.to_uppercase(),
                        is_correct,
                        answer_dto.response_time_ms,
                        points
                    )
                    .fetch_one(&**pool)
                    .await;
                    
                    match answer_result {
                        Ok(answer) => {
                            // Oyuncu puanını güncelle
                            let _ = sqlx::query!(
                                r#"
                                UPDATE players
                                SET score = score + $1
                                WHERE id = $2
                                "#,
                                answer.points_earned,
                                player.id
                            )
                            .execute(&**pool)
                            .await;
                            
                            HttpResponse::Ok().json(serde_json::json!({
                                "answer_id": answer.id,
                                "is_correct": is_correct,
                                "points_earned": answer.points_earned,
                                "correct_option": question.correct_option,
                                "message": if is_correct {
                                    format!("Doğru! {} puan kazandınız", answer.points_earned.unwrap_or(0))
                                } else {
                                    "Yanlış cevap".to_string()
                                }
                            }))
                        }
                        Err(e) => {
                            error!("Cevap kaydedilirken hata: {}", e);
                            HttpResponse::InternalServerError().json(serde_json::json!({
                                "error": "Cevap gönderilemedi"
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
                        "error": "Cevap gönderilemedi"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::Unauthorized().json(serde_json::json!({
                "error": "Aktif oyuncu bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Cevap gönderilemedi"
            }))
        }
    }
}

// Bir sonraki soruya geç
pub async fn next_question(
    pool: web::Data<Pool<Postgres>>,
    game_code: web::Path<String>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    let game_code_inner = game_code.into_inner();
    
    // Oyun ve host kontrolü
    let game = sqlx::query!(
        r#"
        SELECT g.id, g.host_id, g.status, g.current_question, g.question_set_id
        FROM games g
        WHERE g.code = $1
        "#,
        game_code_inner
    )
    .fetch_optional(&**pool)
    .await;

    match game {
        Ok(Some(g)) => {
            // Sadece host soruyu ilerletebilir
            if g.host_id != user_id {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Sadece oyun sahibi soruları ilerletebilir"
                }));
            }

            if g.status != "active" {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "error": "Oyun aktif değil"
                }));
            }

            // Bir sonraki soruyu getir
            let next_question = g.current_question.unwrap_or(0) + 1;
            
            // Soru bilgilerini getir
            let question = sqlx::query!(
                r#"
                SELECT id, question_text, option_a, option_b, option_c, option_d, 
                       correct_option, time_limit, position
                FROM questions
                WHERE question_set_id = $1 AND position = $2
                "#,
                g.question_set_id,
                next_question
            )
            .fetch_optional(&**pool)
            .await;

            // Toplam soru sayısını al
            let total_questions = sqlx::query!(
                "SELECT COUNT(*) as count FROM questions WHERE question_set_id = $1",
                g.question_set_id
            )
            .fetch_one(&**pool)
            .await
            .map(|r| r.count.unwrap_or(0))
            .unwrap_or(0);

            match question {
                Ok(Some(q)) => {
                    // Oyun durumunu güncelle
                    let _ = sqlx::query!(
                        "UPDATE games SET current_question = $1 WHERE id = $2",
                        next_question,
                        g.id
                    )
                    .execute(&**pool)
                    .await;

                    HttpResponse::Ok().json(serde_json::json!({
                        "question_id": q.id,
                        "question_text": q.question_text,
                        "options": {
                            "A": q.option_a,
                            "B": q.option_b,
                            "C": q.option_c, 
                            "D": q.option_d
                        },
                        "correct_option": q.correct_option,
                        "time_limit": q.time_limit,
                        "question_number": next_question + 1,
                        "total_questions": total_questions
                    }))
                }
                Ok(None) => {
                    // Soru kalmadı, oyunu bitir
                    let _ = sqlx::query!(
                        r#"
                        UPDATE games SET status = 'completed', ended_at = $1
                        WHERE id = $2
                        "#,
                        Utc::now(),
                        g.id
                    )
                    .execute(&**pool)
                    .await;

                    HttpResponse::Ok().json(serde_json::json!({
                        "message": "Oyun tamamlandı",
                        "game_id": g.id,
                        "status": "completed",
                        "ended_at": Utc::now()
                    }))
                }
                Err(e) => {
                    error!("Veritabanı sorgu hatası: {}", e);
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Bir sonraki soru alınamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyun bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyun bilgileri alınamadı"
            }))
        }
    }
}

// Oyun detaylarını getir
pub async fn get_game(
    pool: web::Data<Pool<Postgres>>,
    game_code: web::Path<String>,
) -> impl Responder {
    let game_code_inner = game_code.into_inner();
    
    // Oyun bilgilerini getir
    let game = sqlx::query!(
        r#"
        SELECT g.id, g.code, g.question_set_id, g.host_id, g.status, 
               g.current_question, g.started_at, g.ended_at, g.created_at,
               qs.title as question_set_title,
               u.username as host_username
        FROM games g
        JOIN question_sets qs ON g.question_set_id = qs.id
        JOIN users u ON g.host_id = u.id
        WHERE g.code = $1
        "#,
        game_code_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match game {
        Ok(Some(game)) => {
            // Oyuncu sayısını getir
            let player_count = sqlx::query!(
                "SELECT COUNT(*) as count FROM players WHERE game_id = $1 AND is_active = true",
                game.id
            )
            .fetch_one(&**pool)
            .await;
            
            let player_count = player_count.map(|c| c.count.unwrap_or(0)).unwrap_or(0);
            
            // Soru sayısını getir
            let question_count = sqlx::query!(
                "SELECT COUNT(*) as count FROM questions WHERE question_set_id = $1",
                game.question_set_id
            )
            .fetch_one(&**pool)
            .await;
            
            let question_count = question_count.map(|c| c.count.unwrap_or(0)).unwrap_or(0);
            
            HttpResponse::Ok().json(serde_json::json!({
                "id": game.id,
                "code": game.code,
                "question_set_id": game.question_set_id,
                "question_set_title": game.question_set_title,
                "host_id": game.host_id,
                "host_username": game.host_username,
                "status": game.status,
                "current_question": game.current_question,
                "started_at": game.started_at,
                "ended_at": game.ended_at,
                "created_at": game.created_at,
                "player_count": player_count,
                "question_count": question_count
            }))
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyun bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyun bilgileri alınamadı"
            }))
        }
    }
}

// Oyun İstatistiklerini Getir
pub async fn get_game_statistics(
    pool: web::Data<Pool<Postgres>>,
    game_code: web::Path<String>,
    claims: web::ReqData<Claims>,
) -> impl Responder {
    let user_id = claims.sub.parse::<i32>().unwrap_or_default();
    let game_code_inner = game_code.into_inner();
    
    // Oyun bilgilerini getir
    let game = sqlx::query!(
        r#"
        SELECT g.id, g.host_id, g.status, g.question_set_id, 
               qs.title as question_set_title, u.username as host_username
        FROM games g
        JOIN question_sets qs ON g.question_set_id = qs.id
        JOIN users u ON g.host_id = u.id
        WHERE g.code = $1
        "#,
        game_code_inner
    )
    .fetch_optional(&**pool)
    .await;
    
    match game {
        Ok(Some(game)) => {
            // Sadece oyun sahibi veya admin tüm istatistikleri görebilir
            if game.host_id != user_id && claims.role != "admin" {
                return HttpResponse::Forbidden().json(serde_json::json!({
                    "error": "Bu oyunun istatistiklerini görüntüleme izniniz yok"
                }));
            }
            
            // Oyuncu istatistikleri
            let player_stats = sqlx::query!(
                r#"
                SELECT 
                    p.id as player_id,
                    p.nickname,
                    p.score,
                    COUNT(pa.id) as answer_count,
                    COUNT(pa.id) FILTER (WHERE pa.is_correct) as correct_count,
                    ROUND(AVG(pa.response_time_ms)) as avg_response_time
                FROM players p
                LEFT JOIN player_answers pa ON p.id = pa.player_id
                WHERE p.game_id = $1 AND p.is_active = true
                GROUP BY p.id, p.nickname, p.score
                ORDER BY p.score DESC
                "#,
                game.id
            )
            .fetch_all(&**pool)
            .await;
            
            // Soru istatistikleri
            let question_stats = sqlx::query!(
                r#"
                SELECT 
                    q.id as question_id,
                    q.question_text,
                    q.correct_option,
                    COUNT(pa.id) as answer_count,
                    COUNT(pa.id) FILTER (WHERE pa.is_correct) as correct_count,
                    ROUND(AVG(pa.response_time_ms)) as avg_response_time
                FROM questions q
                LEFT JOIN player_answers pa ON q.id = pa.question_id
                WHERE q.question_set_id = $1 AND pa.player_id IN (
                    SELECT id FROM players WHERE game_id = $2
                )
                GROUP BY q.id, q.question_text, q.correct_option
                ORDER BY q.position
                "#,
                game.question_set_id,
                game.id
            )
            .fetch_all(&**pool)
            .await;
            
            match (player_stats, question_stats) {
                (Ok(players), Ok(questions)) => {
                    let player_statistics: Vec<PlayerStatistics> = players
                        .iter()
                        .map(|p| {
                            let accuracy = if p.answer_count.unwrap_or(0) > 0 {
                                (p.correct_count.unwrap_or(0) as f64 / p.answer_count.unwrap_or(0) as f64 * 100.0).round()
                            } else {
                                0.0
                            };
                            
                            PlayerStatistics {
                                player_id: p.player_id,
                                nickname: p.nickname.clone(),
                                score: p.score.unwrap_or(0),
                                answers: p.answer_count.unwrap_or(0),
                                correct: p.correct_count.unwrap_or(0),
                                accuracy,
                                avg_response_time_ms: p.avg_response_time.as_ref().map(|bd| bigdecimal_to_f64(Some(bd.clone())) as i64),
                            }
                        })
                        .collect();
                    
                    let question_statistics: Vec<QuestionStatistics> = questions
                        .iter()
                        .map(|q| {
                            let total_answers = q.answer_count.unwrap_or(0);
                            let correct_count = q.correct_count.unwrap_or(0);
                            let incorrect_count = total_answers - correct_count;
                            
                            let accuracy = if total_answers > 0 {
                                (correct_count as f64 / total_answers as f64 * 100.0).round()
                            } else {
                                0.0
                            };
                            
                            // Zorluğu hesapla: Cevap sayısı, doğruluk oranı ve yanıt süresine göre 0-10 arası (10 en zor)
                            let difficulty_score = if total_answers > 0 {
                                let accuracy_factor = 1.0 - (correct_count as f64 / total_answers as f64);
                                let time_factor = if let Some(time) = &q.avg_response_time {
                                    let time_value = bigdecimal_to_f64(Some(time.clone()));
                                    (time_value / 10000.0).min(1.0)  // 10 saniye üzeri max zorluk
                                } else {
                                    0.5  // Varsayılan orta zorluk
                                };
                                
                                ((accuracy_factor * 0.7 + time_factor * 0.3) * 10.0).round() / 10.0
                            } else {
                                5.0  // Yanıt yoksa orta zorluk
                            };
                            
                            QuestionStatistics {
                                question_id: q.question_id,
                                question_text: q.question_text.clone(),
                                correct_count,
                                incorrect_count,
                                total_answers,
                                accuracy,
                                avg_response_time_ms: q.avg_response_time.as_ref().map(|t| bigdecimal_to_f64(Some(t.clone()))),
                                difficulty_score,
                            }
                        })
                        .collect();
                    
                    // Genel oyun istatistikleri
                    let total_players = player_statistics.len();
                    let avg_score = if total_players > 0 {
                        player_statistics.iter().map(|p| p.score).sum::<i32>() as f64 / total_players as f64
                    } else {
                        0.0
                    };
                    
                    HttpResponse::Ok().json(serde_json::json!({
                        "game_id": game.id,
                        "game_code": game_code_inner,
                        "question_set_title": game.question_set_title,
                        "host_username": game.host_username,
                        "status": game.status,
                        "player_count": total_players,
                        "avg_score": avg_score,
                        "player_statistics": player_statistics,
                        "question_statistics": question_statistics,
                    }))
                }
                _ => {
                    error!("İstatistikler alınırken hata oluştu");
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "error": "Oyun istatistikleri alınamadı"
                    }))
                }
            }
        }
        Ok(None) => {
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Oyun bulunamadı"
            }))
        }
        Err(e) => {
            error!("Veritabanı sorgu hatası: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Oyun bilgileri alınamadı"
            }))
        }
    }
}