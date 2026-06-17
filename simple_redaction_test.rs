/// Simple standalone test for basic redaction functionality
/// This demonstrates core redaction logic without external dependencies

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

pub fn auto_redact_string(input: &str) -> String {
    let mut result = input.to_string();
    
    // Simple pattern matching for Stellar accounts (G + 55 uppercase alphanumeric)
    if let Some(start) = result.find("G") {
        let potential_account = &result[start..];
        if potential_account.len() >= 56 {
            let account_part = &potential_account[..56];
            if account_part.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
                result = result.replace(account_part, "G****[REDACTED]");
            }
        }
    }
    
    // Simple pattern matching for Stellar secrets (S + 55 uppercase alphanumeric)
    if let Some(start) = result.find("S") {
        let potential_secret = &result[start..];
        if potential_secret.len() >= 56 {
            let secret_part = &potential_secret[..56];
            if secret_part.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
                result = result.replace(secret_part, "S****[REDACTED_SECRET]");
            }
        }
    }
    
    // Simple email redaction
    if let Some(at_pos) = result.find("@") {
        let words: Vec<&str> = result.split_whitespace().collect();
        for word in words {
            if word.contains("@") && word.contains(".") {
                result = result.replace(word, "****@[REDACTED]");
            }
        }
    }
    
    result
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
    fn test_redacted_wrapper() {
        let sensitive = "secret_value";
        let redacted = Redacted(sensitive);
        assert_eq!(format!("{:?}", redacted), "[REDACTED]");
        assert_eq!(format!("{}", redacted), "[REDACTED]");
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
    fn test_realistic_log_redaction() {
        let log_content = r#"{
            "timestamp": "2024-01-01T00:00:00Z",
            "level": "INFO",
            "message": "User logged in",
            "user_id": "user_12345",
            "stellar_account": "GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R",
            "email": "user@example.com",
            "safe_data": "this should remain"
        }"#;
        
        let redacted = auto_redact_string(log_content);
        
        // Should preserve safe data
        assert!(redacted.contains("user_12345"));
        assert!(redacted.contains("this should remain"));
        assert!(redacted.contains("2024-01-01T00:00:00Z"));
        
        // Should redact sensitive values
        assert!(redacted.contains("G****[REDACTED]"));
        assert!(redacted.contains("****@[REDACTED]"));
        
        // Should not contain original sensitive data
        assert!(!redacted.contains("GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R"));
        assert!(!redacted.contains("user@example.com"));
    }
}

fn main() {
    println!("Running redaction tests...");
    
    // Run tests manually
    tests::test_redact_stellar_accounts();
    println!("✓ Stellar account redaction works");
    
    tests::test_redact_stellar_secret_keys();
    println!("✓ Stellar secret redaction works");
    
    tests::test_redact_jwt_tokens();
    println!("✓ JWT token redaction works");
    
    tests::test_auto_redact_mixed_content();
    println!("✓ Auto redaction of mixed content works");
    
    tests::test_redacted_wrapper();
    println!("✓ Redacted wrapper works");
    
    tests::test_no_false_positives();
    println!("✓ No false positives in redaction");
    
    tests::test_realistic_log_redaction();
    println!("✓ Realistic log redaction works");
    
    println!("\n🎉 All redaction tests passed!");
    println!("✅ Secure logging and redaction protocols are working correctly!");
}