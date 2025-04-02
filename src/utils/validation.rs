use regex::Regex;
use lazy_static::lazy_static;

lazy_static! {
    static ref EMAIL_REGEX: Regex = Regex::new(
        r"^([a-z0-9_+]([a-z0-9_+.]*[a-z0-9_+])?)@([a-z0-9]+([\-\.]{1}[a-z0-9]+)*\.[a-z]{2,6})"
    ).unwrap();
    
    static ref USERNAME_REGEX: Regex = Regex::new(
        r"^[a-zA-Z0-9_]{3,30}$"
    ).unwrap();
    
    static ref PASSWORD_REGEX: Regex = Regex::new(
        r"^.{8,100}$"
    ).unwrap();
    
    static ref GAME_CODE_REGEX: Regex = Regex::new(
        r"^[A-Z0-9]{6}$"
    ).unwrap();
}

// Email formatı kontrolü
pub fn validate_email(email: &str) -> bool {
    if !EMAIL_REGEX.is_match(email) {
        return false;
    }
    
    // Edu domain kontrolü
    let domain = email.split('@').nth(1).unwrap_or("");
    domain.ends_with(".edu.tr") || domain.ends_with(".edu")
}

// Kullanıcı adı kontrolü
pub fn validate_username(username: &str) -> bool {
    // Misafir öneki kontrolü
    if username.starts_with("**") {
        return false;
    }
    
    USERNAME_REGEX.is_match(username)
}

// Şifre kontrolü
pub fn validate_password(password: &str) -> bool {
    PASSWORD_REGEX.is_match(password)
}

// Oyun kodu kontrolü
pub fn validate_game_code(code: &str) -> bool {
    GAME_CODE_REGEX.is_match(code)
}

// Web adresi kontrolü
pub fn validate_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_validate_email() {
        assert!(validate_email("example@university.edu.tr"));
        assert!(validate_email("example@university.edu"));
        assert!(!validate_email("example@university.com"));
        assert!(!validate_email("example@invalid"));
        assert!(!validate_email("invalid-email"));
    }
    
    #[test]
    fn test_validate_username() {
        assert!(validate_username("validuser"));
        assert!(validate_username("valid_user_123"));
        assert!(!validate_username("**guest"));
        assert!(!validate_username("ab")); // too short
        assert!(!validate_username("invalid username")); // contains space
    }
    
    #[test]
    fn test_validate_password() {
        assert!(validate_password("password123"));
        assert!(validate_password("secureP@ssw0rd"));
        assert!(!validate_password("short")); // too short
    }
    
    #[test]
    fn test_validate_game_code() {
        assert!(validate_game_code("ABC123"));
        assert!(validate_game_code("DEFG45"));
        assert!(!validate_game_code("abc123")); // lowercase
        assert!(!validate_game_code("AB12")); // too short
        assert!(!validate_game_code("ABCDEF1")); // too long
    }
    
    #[test]
    fn test_validate_url() {
        assert!(validate_url("https://example.com"));
        assert!(validate_url("http://localhost:3000"));
        assert!(!validate_url("ftp://example.com"));
        assert!(!validate_url("example.com"));
    }
}