#!/bin/bash
set -e

echo "Soru Kayısı veritabanı kurulumu başlatılıyor..."

# PostgreSQL kullanıcı ve veritabanı kontrolü
if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_roles WHERE rolname='sorukayisi'" | grep -q 1; then
    echo "PostgreSQL kullanıcısı oluşturuluyor: sorukayisi"
    sudo -u postgres psql -c "CREATE USER sorukayisi WITH PASSWORD 'soru61kayisi';"
else
    echo "PostgreSQL kullanıcısı zaten var: sorukayisi"
fi

if ! sudo -u postgres psql -tAc "SELECT 1 FROM pg_database WHERE datname='sorukayisi_db'" | grep -q 1; then
    echo "Veritabanı oluşturuluyor: sorukayisi_db"
    sudo -u postgres psql -c "CREATE DATABASE sorukayisi_db OWNER sorukayisi;"
else
    echo "Veritabanı zaten var: sorukayisi_db"
fi

# PostgreSQL kullanıcısına gerekli izinler veriliyor
echo "PostgreSQL kullanıcısına izinler veriliyor..."
sudo -u postgres psql << EOF
GRANT ALL PRIVILEGES ON DATABASE sorukayisi_db TO sorukayisi;
\c sorukayisi_db
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO sorukayisi;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO sorukayisi;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL PRIVILEGES ON TABLES TO sorukayisi;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL PRIVILEGES ON SEQUENCES TO sorukayisi;
EOF

# Şema dosyası oluşturma
echo "Şema dosyası oluşturuluyor..."
cat > /tmp/schema.sql << 'EOL'
-- Aktif kullanıcılar tablosu (mevcut tabloyu koruyalım)
CREATE TABLE IF NOT EXISTS active_users (
    id SERIAL PRIMARY KEY,
    session_id VARCHAR(255) UNIQUE NOT NULL,
    last_seen TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Kullanıcılar tablosu
CREATE TABLE IF NOT EXISTS users (
    id SERIAL PRIMARY KEY,
    username VARCHAR(100) UNIQUE NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL CHECK (email LIKE '%edu.tr' OR email LIKE '%edu'),
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(20) NOT NULL CHECK (role IN ('admin', 'teacher', 'student')),
    is_approved BOOLEAN DEFAULT FALSE,
    is_email_verified BOOLEAN DEFAULT FALSE,
    verification_token VARCHAR(255),
    reset_token VARCHAR(255),
    reset_token_expires_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    last_login TIMESTAMP WITH TIME ZONE
);

-- Admin kullanıcısı kontrolü ve oluşturma
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM users WHERE username = 'cancebi') THEN
        INSERT INTO users (username, email, password_hash, role, is_approved, is_email_verified)
        VALUES ('cancebi', 'can@edu.tr', '$2a$12$k8Y6JHT9vFx5aHrbjlU9heJGiVjZxrBPHOYWEbXoumtJ3Q3PRxJkW', 'admin', TRUE, TRUE);
    END IF;
END
$$;

-- Soru setleri tablosu
CREATE TABLE IF NOT EXISTS question_sets (
    id SERIAL PRIMARY KEY,
    creator_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title VARCHAR(255) NOT NULL,
    description TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Sorular tablosu
CREATE TABLE IF NOT EXISTS questions (
    id SERIAL PRIMARY KEY,
    question_set_id INTEGER NOT NULL REFERENCES question_sets(id) ON DELETE CASCADE,
    question_text TEXT NOT NULL,
    option_a TEXT NOT NULL,
    option_b TEXT NOT NULL,
    option_c TEXT NOT NULL,
    option_d TEXT NOT NULL,
    correct_option CHAR(1) NOT NULL CHECK (correct_option IN ('A', 'B', 'C', 'D')),
    points INTEGER DEFAULT 100,
    time_limit INTEGER DEFAULT 30,
    position INTEGER NOT NULL
);

-- Oyunlar tablosu
CREATE TABLE IF NOT EXISTS games (
    id SERIAL PRIMARY KEY,
    code VARCHAR(6) UNIQUE NOT NULL,
    question_set_id INTEGER NOT NULL REFERENCES question_sets(id),
    host_id INTEGER NOT NULL REFERENCES users(id),
    status VARCHAR(20) NOT NULL CHECK (status IN ('lobby', 'active', 'completed')),
    current_question INTEGER DEFAULT 0,
    started_at TIMESTAMP WITH TIME ZONE,
    ended_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Oyuncular tablosu
CREATE TABLE IF NOT EXISTS players (
    id SERIAL PRIMARY KEY,
    game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
    nickname VARCHAR(100) NOT NULL,
    score INTEGER DEFAULT 0,
    session_id VARCHAR(255) NOT NULL,
    is_active BOOLEAN DEFAULT TRUE,
    joined_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Oyuncu cevapları tablosu
CREATE TABLE IF NOT EXISTS player_answers (
    id SERIAL PRIMARY KEY,
    player_id INTEGER NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    answer CHAR(1) CHECK (answer IN ('A', 'B', 'C', 'D', 'X')),
    is_correct BOOLEAN NOT NULL,
    response_time_ms INTEGER,
    points_earned INTEGER DEFAULT 0,
    answered_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Aktif WebSocket bağlantıları
CREATE TABLE IF NOT EXISTS active_connections (
    id SERIAL PRIMARY KEY,
    session_id VARCHAR(255) UNIQUE NOT NULL,
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    game_id INTEGER REFERENCES games(id) ON DELETE CASCADE,
    player_id INTEGER REFERENCES players(id) ON DELETE CASCADE,
    connection_type VARCHAR(20) NOT NULL CHECK (connection_type IN ('host', 'player', 'viewer')),
    last_seen TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- İndeksler
CREATE INDEX IF NOT EXISTS idx_active_users_last_seen ON active_users(last_seen);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_role ON users(role);
CREATE INDEX IF NOT EXISTS idx_question_sets_creator ON question_sets(creator_id);
CREATE INDEX IF NOT EXISTS idx_questions_set_id ON questions(question_set_id);
CREATE INDEX IF NOT EXISTS idx_games_code ON games(code);
CREATE INDEX IF NOT EXISTS idx_games_host ON games(host_id);
CREATE INDEX IF NOT EXISTS idx_players_game ON players(game_id);
CREATE INDEX IF NOT EXISTS idx_players_user ON players(user_id);
CREATE INDEX IF NOT EXISTS idx_player_answers_player ON player_answers(player_id);
CREATE INDEX IF NOT EXISTS idx_player_answers_question ON player_answers(question_id);
CREATE INDEX IF NOT EXISTS idx_active_connections_session ON active_connections(session_id);
CREATE INDEX IF NOT EXISTS idx_active_connections_user ON active_connections(user_id);
CREATE INDEX IF NOT EXISTS idx_active_connections_game ON active_connections(game_id);
EOL

# Şemayı veritabanına uygulama
echo "Şema uygulanıyor..."
sudo -u postgres psql -d sorukayisi_db -f /tmp/schema.sql

echo "Geçici şema dosyası temizleniyor..."
rm /tmp/schema.sql

echo "Veritabanı kurulumu tamamlandı!"