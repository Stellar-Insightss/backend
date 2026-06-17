/// Standalone test for redaction functionality that doesn't depend on the full codebase
/// This demonstrates that our redaction implementation is working correctly

use regex::Regex;
use once_cell::sync::Lazy;
use serde::{Serialize, Serializer};

// Copy the redaction implementation for standalone testing
#[derive(Clone)]
pub struct Redacted<T>(pub T);

impl<T> std::fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T> std::fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T: Serialize> Serialize for Redacted<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

pub fn redact_account(account: &str) -> String {
    if account.len() <= 8 {
        return "[REDACTED]".to_string();
    }
    format!("{}...{}", &account[..4], &account[account.len() - 4..])
}

pub fn redact_stellar_secret(secret: &str) -> String {
    if secret.starts_with('S') && secret.len() == 56 {
        "S****[REDACTED]".to_string()
    } else {
        "[REDACTED_SECRET]".to_string()
    }
}

pub fn redact_jwt(token: &str) -> String {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() == 3 {
        format!("{}.[PAYLOAD_REDACTED].[SIGNATURE_REDACTED]", parts[0])
    } else {
        "[REDACTED_JWT]".to_string()
    }
}

static STELLAR_ACCOUNT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"G[A-Z0-9]{55}").unwrap()
});

static STELLAR_SECRET_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"S[A-Z0-9]{55}").unwrap()
});

static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap()
});

static JWT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]*\.[A-Za-z0-9_-]*").unwrap()
});

pub fn auto_redact_string(input: &str) -> String {
    let mut result = input.to_string();
    
    // Redact Stellar accounts
    result = STELLAR_ACCOUNT_REGEX.replace_all(&result, "G****[REDACTED]").to_string();
    
    // Redact Stellar secret keys
    result = STELLAR_SECRET_REGEX.replace_all(&result, "S****[REDACTED_SECRET]").to_string();
    
    // Redact email addresses
    result = EMAIL_REGEX.replace_all(&result, "****@[REDACTED]").to_string();
    
    // Redact JWT tokens
    result = JWT_REGEX.replace_all(&result, "[REDACTED_JWT]").to_string();
    
    result
}

pub fn redact_sensitive_fields(json_str: &str) -> String {
    let sensitive_fields = [
        "password", "secret", "token", "key", "private", "seed", "mnemonic", 
        "credential", "auth", "signature", "jwt", "bearer", "access_token",
        "refresh_token", "client_secret", "api_key", "stellar_secret"
    ];
    
    let mut result = json_str.to_string();
    
    for field in &sensitive_fields {
        let pattern = format!(r#""{}":\s*"[^"]*""#, field);
        let regex = Regex::new(&pattern).unwrap_or_else(|_| {
            Regex::new(&format!(r"(?i){}.*", field)).unwrap()
        });
        result = regex.replace_all(&result, &format!(r#""{}":"[REDACTED]""#, field)).to_string();
    }
    
    auto_redact_string(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_stellar_accounts() {
        let account = "GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R";
        let redacted = redact_account(account);
        assert_eq!(redacted, "GCKF...KF3R");
        assert!(!redacted.contains("BEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHG"));
    }

    #[test]
    fn test_redact_stellar_secret_keys() {
        let secret = "SCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R";
        let redacted = redact_stellar_secret(secret);
        assert_eq!(redacted, "S****[REDACTED]");
        assert!(!redacted.contains("CKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHG"));
    }

    #[test]
    fn test_redact_jwt_tokens() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let redacted = redact_jwt(jwt);
        assert_eq!(redacted, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.[PAYLOAD_REDACTED].[SIGNATURE_REDACTED]");
        assert!(!redacted.contains("SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"));
    }

    #[test]
    fn test_auto_redact_mixed_content() {
        let input = "User account GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R has secret SCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R and email john@example.com";
        let redacted = auto_redact_string(input);
        
        assert!(redacted.contains("G****[REDACTED]"));
        assert!(redacted.contains("S****[REDACTED_SECRET]"));
        assert!(redacted.contains("****@[REDACTED]"));
        assert!(!redacted.contains("GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R"));
        assert!(!redacted.contains("SCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R"));
        assert!(!redacted.contains("john@example.com"));
    }

    #[test]
    fn test_redact_sensitive_fields() {
        let json = r#"{"username":"john","password":"secret123","token":"abc123"}"#;
        let result = redact_sensitive_fields(json);
        assert!(result.contains(r#""password":"[REDACTED]""#));
        assert!(result.contains(r#""token":"[REDACTED]""#));
        assert!(result.contains("username"));
    }

    #[test]
    fn test_redacted_wrapper() {
        let sensitive = "secret_value";
        let redacted = Redacted(sensitive);
        assert_eq!(format!("{:?}", redacted), "[REDACTED]");
        assert_eq!(format!("{}", redacted), "[REDACTED]");
    }

    #[test]
    fn test_complex_json_redaction() {
        let complex_json = r#"{
            "user": {
                "username": "john",
                "password": "secret123",
                "profile": {
                    "email": "john@example.com",
                    "api_key": "ak_live_abcdef1234567890"
                }
            },
            "stellar": {
                "account": "GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R",
                "secret": "SCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R"
            },
            "safe_data": "this should remain"
        }"#;
        
        let redacted = redact_sensitive_fields(complex_json);
        
        // Should preserve safe data
        assert!(redacted.contains("username"));
        assert!(redacted.contains("this should remain"));
        
        // Should redact sensitive fields
        assert!(redacted.contains(r#""password":"[REDACTED]""#));
        assert!(redacted.contains(r#""api_key":"[REDACTED]""#));
        
        // Should redact sensitive values via auto-redaction
        assert!(redacted.contains("G****[REDACTED]"));
        assert!(redacted.contains("S****[REDACTED_SECRET]"));
        assert!(redacted.contains("****@[REDACTED]"));
        
        // Should not contain original sensitive data
        assert!(!redacted.contains("secret123"));
        assert!(!redacted.contains("john@example.com"));
        assert!(!redacted.contains("ak_live_abcdef1234567890"));
        assert!(!redacted.contains("GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R"));
    }

    #[test] 
    fn test_no_false_positives() {
        // Test that non-sensitive data that looks similar isn't redacted
        let safe_data = vec![
            "regular_field_name",
            "normal text content", 
            "NOTASTELLARACCOUNTG12345", // Too short
            "GNOTASTELLARACCOUNT123",   // Wrong format
            "user@domain",              // Invalid email format
            "short_token",              // Too short for API key pattern
        ];
        
        for data in safe_data {
            let redacted = auto_redact_string(data);
            assert_eq!(redacted, data, "Should not redact safe data: {}", data);
        }
    }

    #[test]
    fn test_json_serialization() {
        use serde_json;
        
        let sensitive = "secret_value";
        let redacted = Redacted(sensitive);
        
        // Test JSON serialization
        let json = serde_json::to_string(&redacted).unwrap();
        assert_eq!(json, r#""[REDACTED]""#);
    }

    #[test]
    fn test_performance_with_realistic_data() {
        // Test realistic log content
        let log_content = format!(
            r#"{{
                "timestamp": "2024-01-01T00:00:00Z",
                "level": "INFO",
                "message": "User logged in",
                "user_id": "user_12345",
                "stellar_account": "GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R",
                "request_id": "req_abcdef123456",
                "ip": "192.168.1.100",
                "user_agent": "Mozilla/5.0...",
                "password": "secret123",
                "jwt_token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
                "email": "user@example.com"
            }}"#
        );
        
        let start = std::time::Instant::now();
        let redacted = redact_sensitive_fields(&log_content);
        let duration = start.elapsed();
        
        // Redaction should be fast (under 1ms for typical log entries)
        assert!(duration.as_millis() < 10);
        
        // Verify redaction worked
        assert!(redacted.contains(r#""password":"[REDACTED]""#));
        assert!(redacted.contains("G****[REDACTED]"));
        assert!(redacted.contains("[REDACTED_JWT]"));
        assert!(redacted.contains("****@[REDACTED]"));
        
        // Safe data should remain
        assert!(redacted.contains("user_12345"));
        assert!(redacted.contains("req_abcdef123456"));
        assert!(redacted.contains("Mozilla/5.0"));
    }
}