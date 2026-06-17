use stellar_insights_backend::logging::redaction::*;

#[cfg(test)]
mod logging_redaction_tests {
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
    fn test_redact_sensitive_field_names() {
        let sensitive_fields = vec![
            "password",
            "secret_key", 
            "api_token",
            "private_key",
            "authorization",
            "credential",
            "jwt_token",
            "bearer_token",
            "stellar_secret",
            "mnemonic_phrase"
        ];

        for field in sensitive_fields {
            let json = format!(r#"{{"username":"john","{}":"sensitive_value","other":"safe"}}"#, field);
            let redacted = redact_sensitive_fields(&json);
            assert!(redacted.contains(r#""username":"john""#), "Should preserve non-sensitive fields");
            assert!(redacted.contains(&format!(r#""{}":"[REDACTED]""#, field)), "Should redact field: {}", field);
            assert!(!redacted.contains("sensitive_value"), "Should not contain original sensitive value");
        }
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
    fn test_redact_mnemonic_phrases() {
        let mnemonic_12 = "abandon ability able about above absent absorb abstract absurd abuse access accident";
        let mnemonic_24 = "abandon ability able about above absent absorb abstract absurd abuse access accident account accuse achieve acid acoustic acquire across act action actor actress actual";
        
        assert_eq!(redact_mnemonic(mnemonic_12), "[12_WORD_MNEMONIC_REDACTED]");
        assert_eq!(redact_mnemonic(mnemonic_24), "[24_WORD_MNEMONIC_REDACTED]");
    }

    #[test]
    fn test_redact_payment_amounts() {
        assert_eq!(redact_amount(1000.0), "~10^3");
        assert_eq!(redact_amount(50.0), "~10^1");  
        assert_eq!(redact_amount(0.1), "~10^-1");
        assert_eq!(redact_amount(0.001), "~10^-3");
        assert_eq!(redact_amount(0.0), "~10^0");
        assert_eq!(redact_amount(-100.0), "~10^0");
    }

    #[test]
    fn test_redact_ip_addresses() {
        // IPv4
        assert_eq!(redact_ip("192.168.1.100"), "192.168.*.*");
        assert_eq!(redact_ip("10.0.0.1"), "10.0.*.*");
        
        // IPv6
        assert_eq!(redact_ip("2001:db8:85a3::8a2e:370:7334"), "2001:****");
        assert_eq!(redact_ip("::1"), ":****");
        
        // Invalid
        assert_eq!(redact_ip("invalid"), "[REDACTED]");
    }

    #[test] 
    fn test_redact_phone_numbers() {
        assert_eq!(redact_phone("+1234567890"), "+12****");
        assert_eq!(redact_phone("+44207123456"), "+44****");
        assert_eq!(redact_phone("1234567890"), "[REDACTED_PHONE]");
        assert_eq!(redact_phone("+1"), "[REDACTED_PHONE]");
    }

    #[test]
    fn test_redact_user_ids() {
        assert_eq!(redact_user_id("user_12345678"), "user_****");
        assert_eq!(redact_user_id("account_abcdef"), "account_****");
        assert_eq!(redact_user_id("longstring"), "long****");
        assert_eq!(redact_user_id("short"), "[REDACTED]");
    }

    #[test]
    fn test_redact_email_addresses() {
        assert_eq!(redact_email("user@example.com"), "****@example.com");
        assert_eq!(redact_email("test@domain.org"), "****@domain.org");
        assert_eq!(redact_email("invalid"), "[REDACTED]");
    }

    #[test]
    fn test_redact_transaction_hashes() {
        let hash = "abcdef1234567890abcdef1234567890abcdef12";
        assert_eq!(redact_hash(hash), "abcd...ef12");
        assert_eq!(redact_hash("short"), "[REDACTED]");
    }

    #[test]
    fn test_redact_api_tokens() {
        let token = "test_api_key_1234567890abcdef1234567890abcdef";
        assert_eq!(redact_token(token), "test****");
        assert_eq!(redact_token("abc"), "[REDACTED]");
    }

    #[test]
    fn test_redacted_wrapper_serialization() {
        use serde_json;
        
        let sensitive = "secret_value";
        let redacted = Redacted(sensitive);
        
        // Test Debug formatting
        assert_eq!(format!("{:?}", redacted), "[REDACTED]");
        
        // Test Display formatting  
        assert_eq!(format!("{}", redacted), "[REDACTED]");
        
        // Test JSON serialization
        let json = serde_json::to_string(&redacted).unwrap();
        assert_eq!(json, r#""[REDACTED]""#);
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
    fn test_secure_display_trait() {
        let account = "GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R";
        let secret = "SCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R";
        
        assert!(account.secure_display().contains("G****[REDACTED]"));
        assert!(secret.secure_display().contains("S****[REDACTED_SECRET]"));
        
        let account_string = account.to_string();
        assert!(account_string.secure_display().contains("G****[REDACTED]"));
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
    fn test_edge_cases() {
        // Empty strings
        assert_eq!(auto_redact_string(""), "");
        assert_eq!(redact_sensitive_fields(""), "");
        
        // Whitespace only
        assert_eq!(auto_redact_string("   "), "   ");
        
        // Mixed case sensitivity
        let mixed_json = r#"{"Password":"secret","TOKEN":"value","Api_Key":"key"}"#;
        let redacted = redact_sensitive_fields(mixed_json);
        assert!(redacted.contains(r#""Password":"[REDACTED]""#));
        assert!(redacted.contains(r#""TOKEN":"[REDACTED]""#));
        assert!(redacted.contains(r#""Api_Key":"[REDACTED]""#));
    }

    #[test]
    fn test_performance_with_large_strings() {
        // Test that redaction works efficiently with large content
        let large_content = "safe data ".repeat(1000) + 
            "GCKFBEIYTKP5ROORWS2HE6XXVV6MQVE6YDJHB5P7C4GGQXJN6ZHGKF3R" +
            &" more safe data ".repeat(1000);
        
        let redacted = auto_redact_string(&large_content);
        assert!(redacted.contains("G****[REDACTED]"));
        assert!(redacted.len() < large_content.len()); // Should be shorter due to redaction
    }
}