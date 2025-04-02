pub mod admin;
pub mod auth;
pub mod game;
pub mod player;
pub mod question;
pub mod websocket;

// İşleyicileri ve yolları kaydetme fonksiyonu
use actix_web::web;

// Tüm API rotalarını yapılandır
pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    // Auth rotaları
    cfg.service(
        web::scope("/api/auth")
            .route("/register", web::post().to(auth::register))
            .route("/login", web::post().to(auth::login))
            .route("/verify/{token}", web::get().to(auth::verify_email))
            .route("/me", web::get().to(auth::get_current_user))
            .route("/reset-password/request", web::post().to(auth::request_password_reset))
            .route("/reset-password/{token}", web::post().to(auth::reset_password)),
    );

    // Admin rotaları
    cfg.service(
        web::scope("/api/admin")
            .route("/teachers/pending", web::get().to(admin::list_pending_teachers))
            .route("/teachers/approve", web::post().to(admin::approve_teacher))
            .route("/users", web::get().to(admin::list_all_users))
            .route("/users/{id}", web::delete().to(admin::delete_user))
            .route("/stats", web::get().to(admin::get_system_stats)),
    );

    // Soru seti ve soru rotaları
    cfg.service(
        web::scope("/api/question-sets")
            .route("", web::post().to(question::create_question_set))
            .route("", web::get().to(question::get_question_sets))
            .route("/{id}", web::get().to(question::get_question_set))
            .route("/{id}", web::delete().to(question::delete_question_set)),
    );

    cfg.service(
        web::scope("/api/questions")
            .route("", web::post().to(question::create_question))
            .route("/{id}", web::put().to(question::update_question))
            .route("/{id}", web::delete().to(question::delete_question)),
    );

    // Oyun rotaları
    cfg.service(
        web::scope("/api/game")
            .route("", web::post().to(game::create_game))
            .route("/join", web::post().to(game::join_game))
            .route("/{code}", web::get().to(game::get_game))
            .route("/{code}/start", web::post().to(game::start_game))
            .route("/{code}/next", web::post().to(game::next_question))
            .route("/{code}/leaderboard", web::get().to(game::get_leaderboard))
            .route("/{code}/statistics", web::get().to(game::get_game_statistics))  // Yeni eklenen rota
            .route("/answer", web::post().to(game::submit_answer_with_header)),
    );
    
    // Oyuncu rotaları
    cfg.service(
        web::scope("/api/player")
            .route("/{id}", web::get().to(player::get_player_info))
            .route("/{id}/stats", web::get().to(player::get_player_stats))
            .route("/history", web::get().to(player::get_user_game_history))
            .route("/{id}/leave", web::post().to(player::leave_game)),
    );

    // WebSocket rotası
    cfg.route("/ws", web::get().to(websocket::ws_handler));
    
    // Sağlık kontrolü
    cfg.route("/health", web::get().to(|| async { "Health check OK" }));
}