use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::fmt;
use std::collections::HashMap;

// Kullanıcı rolleri
#[derive(Debug, Serialize, Deserialize, sqlx::Type, Clone, PartialEq)]
#[sqlx(type_name = "VARCHAR", rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    Teacher,
    Student,
}

// Display trait implementasyonu
impl fmt::Display for UserRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserRole::Admin => write!(f, "admin"),
            UserRole::Teacher => write!(f, "teacher"),
            UserRole::Student => write!(f, "student"),
        }
    }
}

// Kullanıcı modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub is_approved: bool,
    pub is_email_verified: bool,
    #[serde(skip_serializing)]
    pub verification_token: Option<String>,
    #[serde(skip_serializing)]
    pub reset_token: Option<String>,
    #[serde(skip_serializing)]
    pub reset_token_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

// Kullanıcı oluşturma DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateUserDto {
    pub username: String,
    pub email: String,
    pub password: String,
    pub role: UserRole,
}

// Kullanıcı giriş DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoginDto {
    pub email: String,
    pub password: String,
    pub recaptcha_token: String,
}

// JWT Claims
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String, // Kullanıcı ID
    pub role: String, // Kullanıcı rolü
    pub exp: usize, // Son kullanma tarihi
}

// Soru seti modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct QuestionSet {
    pub id: i32,
    pub creator_id: i32,
    pub title: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Soru modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct Question {
    pub id: i32,
    pub question_set_id: i32,
    pub question_text: String,
    pub option_a: String,
    pub option_b: String,
    pub option_c: String,
    pub option_d: String,
    pub correct_option: String,
    pub points: i32,
    pub time_limit: i32,
    pub position: i32,
}

// Oyun durumu
#[derive(Debug, Serialize, Deserialize, sqlx::Type, Clone, PartialEq)]
#[sqlx(type_name = "VARCHAR", rename_all = "lowercase")]
pub enum GameStatus {
    Lobby,
    Active,
    Completed,
}

// Display trait implementasyonu
impl fmt::Display for GameStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameStatus::Lobby => write!(f, "lobby"),
            GameStatus::Active => write!(f, "active"),
            GameStatus::Completed => write!(f, "completed"),
        }
    }
}

// Oyun modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct Game {
    pub id: i32,
    pub code: String,
    pub question_set_id: i32,
    pub host_id: i32,
    pub status: String,
    pub current_question: i32,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

// Oyuncu modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct Player {
    pub id: i32,
    pub game_id: i32,
    pub user_id: Option<i32>,
    pub nickname: String,
    pub score: i32,
    pub session_id: String,
    pub is_active: bool,
    pub joined_at: DateTime<Utc>,
}

// Oyuncu cevabı modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct PlayerAnswer {
    pub id: i32,
    pub player_id: i32,
    pub question_id: i32,
    pub answer: Option<String>,
    pub is_correct: bool,
    pub response_time_ms: Option<i32>,
    pub points_earned: i32,
    pub answered_at: DateTime<Utc>,
}

// WebSocket bağlantı türleri
#[derive(Debug, Serialize, Deserialize, sqlx::Type, Clone, PartialEq)]
#[sqlx(type_name = "VARCHAR", rename_all = "lowercase")]
pub enum ConnectionType {
    Host,
    Player,
    Viewer,
}

// Display trait implementasyonu
impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionType::Host => write!(f, "host"),
            ConnectionType::Player => write!(f, "player"),
            ConnectionType::Viewer => write!(f, "viewer"),
        }
    }
}

// Aktif bağlantı modeli
#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ActiveConnection {
    pub id: i32,
    pub session_id: String,
    pub user_id: Option<i32>,
    pub game_id: Option<i32>,
    pub player_id: Option<i32>,
    pub connection_type: String,
    pub last_seen: DateTime<Utc>,
}

// Kullanıcı Onay DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApproveUserDto {
    pub user_id: i32,
    pub approve: bool,
}

// Soru seti Oluşturma DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateQuestionSetDto {
    pub title: String,
    pub description: Option<String>,
}

// Soru Oluşturma DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateQuestionDto {
    pub question_set_id: i32,
    pub question_text: String,
    pub option_a: String,
    pub option_b: String,
    pub option_c: String,
    pub option_d: String,
    pub correct_option: String,
    pub points: Option<i32>,     // Varsayılan: 100
    pub time_limit: Option<i32>, // Varsayılan: 30 saniye
    pub position: i32,
}

// Oyun Oluşturma DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateGameDto {
    pub question_set_id: i32,
}

// Oyun Katılım DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JoinGameDto {
    pub game_code: String,
    pub nickname: Option<String>, // Misafir oyuncular için
}

// Cevap Gönderme DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubmitAnswerDto {
    pub question_id: i32,
    pub answer: String,
    pub response_time_ms: i32,
}

// WebSocket Mesaj DTO
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum WebSocketMessage {
    // Lobby mesajları
    JoinLobby {
        game_code: String,
        player_id: Option<i32>,
        nickname: Option<String>,
    },
    JoinSuccess {
        player_id: i32,
        game_code: String,
        nickname: String,
        is_guest: bool,
    },
    LobbyUpdate {
        game_code: String,
        players: Vec<PlayerInfo>,
    },
    GameStarted {
        game_code: String,
        message: String,
    },
    
    // Soru mesajları
    QuestionStart {
        question_id: i32,
        question_text: String,
        options: HashMap<String, String>,
        time_limit: i32,
        question_number: i64,
        total_questions: i64,
        correct_option: Option<String>, // Sadece öğretmen için
    },
    SubmitAnswer {
        question_id: i32,
        answer: String,
        response_time_ms: i32,
    },
    AnswerReceived {
        question_id: i32,
        your_answer: String,
        is_correct: bool,
        points_earned: i32,
        message: String,
    },
    QuestionEnd {
        question_id: i32,
        correct_option: String,
        leaderboard: Vec<LeaderboardEntry>,
    },
    
    // Oyun sonu
    GameEnd {
        final_leaderboard: Vec<LeaderboardEntry>,
        player_stats: Vec<PlayerStatistics>,
        message: String,
    },
    
    // Yeniden bağlanma
    Reconnect {
        old_session_id: String,
    },
    ReconnectSuccess {
        player_id: i32,
        game_code: String,
        nickname: String,
        score: i32,
        game_status: String,
        current_question: Option<i32>,
    },
    
    // Sistem mesajları
    Error {
        message: String,
    },
    Counter {
        count: usize,
    },
    Welcome {
        session_id: String,
        message: String,
    },
    Ping,
    Pong {
        timestamp: i64,
    },
}

// WebSocket için basitleştirilmiş oyuncu bilgisi
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerInfo {
    pub player_id: i32,
    pub nickname: String,
    pub is_guest: bool,
}

// Liderlik tablosu girişi
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LeaderboardEntry {
    pub player_id: i32,
    pub nickname: String,
    pub score: i32,
    pub is_guest: bool,
}

// Oyuncu istatistikleri
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerStatistics {
    pub player_id: i32,
    pub nickname: String,
    pub score: i32,
    pub answers: i64,
    pub correct: i64,
    pub accuracy: f64,
    pub avg_response_time_ms: Option<i64>,
}

// Soru istatistiği
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuestionStatistics {
    pub question_id: i32,
    pub question_text: String,
    pub correct_count: i64,
    pub incorrect_count: i64,
    pub total_answers: i64,
    pub accuracy: f64,
    pub avg_response_time_ms: Option<f64>,
    pub difficulty_score: f64, // 0-10 arası, 10 en zor
}

// Oyun istatistikleri
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GameStatistics {
    pub game_id: i32,
    pub game_code: String,
    pub question_set_title: String,
    pub host_username: String,
    pub played_at: DateTime<Utc>,
    pub player_count: i64,
    pub avg_score: f64,
    pub top_players: Vec<LeaderboardEntry>,
    pub question_stats: Vec<QuestionStatistics>,
}