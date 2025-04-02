#!/bin/bash
set -e

echo "Soru Kayısı veritabanı şeması güncelleniyor..."

# Schema güncellemelerini oluştur
cat > /tmp/schema_updates.sql << 'EOL'
-- Sorular tablosunda varsayılan zaman ve puan değerlerini güncelle
ALTER TABLE questions ALTER COLUMN time_limit SET DEFAULT 30;
ALTER TABLE questions ALTER COLUMN points SET DEFAULT 100;

-- Oturum bilgilerini kaydetmek için yeni tablo (yeniden bağlanma için)
CREATE TABLE IF NOT EXISTS session_recovery (
    id SERIAL PRIMARY KEY,
    old_session_id VARCHAR(255) NOT NULL,
    new_session_id VARCHAR(255) NOT NULL,
    player_id INTEGER REFERENCES players(id) ON DELETE CASCADE,
    game_id INTEGER REFERENCES games(id) ON DELETE CASCADE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP + INTERVAL '24 hours'
);

-- Oyunlar tablosuna soru zamanlaması ve diğer değerler için sütunlar
ALTER TABLE games ADD COLUMN IF NOT EXISTS question_started_at TIMESTAMP WITH TIME ZONE;
ALTER TABLE games ADD COLUMN IF NOT EXISTS question_ends_at TIMESTAMP WITH TIME ZONE;
ALTER TABLE games ADD COLUMN IF NOT EXISTS show_results_until TIMESTAMP WITH TIME ZONE;

-- Doğrulama tokeni için indeks ekle
CREATE INDEX IF NOT EXISTS idx_users_verification_token ON users(verification_token);

-- Şifre sıfırlama tokeni için indeks ekle
CREATE INDEX IF NOT EXISTS idx_users_reset_token ON users(reset_token);

-- Oyun kodu için unique indeks
CREATE UNIQUE INDEX IF NOT EXISTS idx_games_code_unique ON games(code);

-- Kullanıcı e-posta ve kullanıcı adı indeksleri
CREATE INDEX IF NOT EXISTS idx_users_username_lower ON users(LOWER(username));
CREATE INDEX IF NOT EXISTS idx_users_email_lower ON users(LOWER(email));

-- Oyuncu istatistikleri görünümü
CREATE OR REPLACE VIEW player_statistics AS
SELECT 
    p.id AS player_id,
    p.nickname,
    p.user_id,
    p.game_id,
    p.score,
    g.code AS game_code,
    COUNT(pa.id) AS answer_count,
    COUNT(pa.id) FILTER (WHERE pa.is_correct) AS correct_count,
    ROUND(AVG(pa.response_time_ms)) AS avg_response_time_ms,
    ROUND(SUM(pa.points_earned) * 100.0 / NULLIF(COUNT(pa.id), 0)) / 100.0 AS avg_points_per_question
FROM 
    players p
    LEFT JOIN player_answers pa ON p.id = pa.player_id
    JOIN games g ON p.game_id = g.id
WHERE 
    p.is_active = true
GROUP BY 
    p.id, p.nickname, p.user_id, p.game_id, p.score, g.code;

-- Soru istatistikleri görünümü
CREATE OR REPLACE VIEW question_statistics AS
SELECT 
    q.id AS question_id,
    q.question_text,
    q.question_set_id,
    q.correct_option,
    COUNT(pa.id) AS answer_count,
    COUNT(pa.id) FILTER (WHERE pa.is_correct) AS correct_count,
    ROUND(AVG(pa.response_time_ms)) AS avg_response_time_ms,
    ROUND((COUNT(pa.id) FILTER (WHERE pa.is_correct) * 100.0) / NULLIF(COUNT(pa.id), 0)) AS accuracy_percentage
FROM 
    questions q
    LEFT JOIN player_answers pa ON q.id = pa.question_id
GROUP BY 
    q.id, q.question_text, q.question_set_id, q.correct_option;

-- Oyun istatistikleri görünümü
CREATE OR REPLACE VIEW game_statistics AS
SELECT 
    g.id AS game_id,
    g.code AS game_code,
    qs.title AS question_set_title,
    u.username AS host_username,
    g.started_at,
    g.ended_at,
    COUNT(DISTINCT p.id) AS player_count,
    ROUND(AVG(p.score)) AS avg_score,
    (SELECT COUNT(*) FROM questions WHERE question_set_id = g.question_set_id) AS question_count
FROM 
    games g
    JOIN question_sets qs ON g.question_set_id = qs.id
    JOIN users u ON g.host_id = u.id
    LEFT JOIN players p ON g.id = p.game_id AND p.is_active = true
GROUP BY 
    g.id, g.code, qs.title, u.username, g.started_at, g.ended_at;
EOL

# Şemayı veritabanına uygulama
echo "Şema güncellemeleri uygulanıyor..."
sudo -u postgres psql -d sorukayisi_db -f /tmp/schema_updates.sql

# Gerekli izinleri ayarla
echo "İzinler ayarlanıyor..."
sudo -u postgres psql << EOF
\c sorukayisi_db
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO sorukayisi;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO sorukayisi;
GRANT ALL PRIVILEGES ON ALL FUNCTIONS IN SCHEMA public TO sorukayisi;
EOF

echo "Geçici şema dosyası temizleniyor..."
rm /tmp/schema_updates.sql

echo "Veritabanı şeması başarıyla güncellendi!"