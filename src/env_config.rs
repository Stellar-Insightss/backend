//! Environment configuration validation and loading
//!
//! This module provides validation for required environment variables
//! and ensures the application fails fast with clear error messages
//! if critical configuration is missing.
//!
//! ## Fail-Fast Startup Behaviour
//!
//! [`validate_env`] is called **before** any network connections are opened.
//! If validation fails the process exits with a descriptive error, preventing
//! a misconfigured backend from silently serving bad data.

use anyhow::Result;
use std::env;

/// Required environment variables that must be set
const REQUIRED_VARS: &[&str] = &[
    "DATABASE_URL",
    "ENCRYPTION_KEY",
    "JWT_SECRET",
    "APP_ENV",
    "VAULT_ADDR",
    "VAULT_TOKEN",
];

/// Known placeholder values that must never appear in real deployments
const PLACEHOLDER_PREFIXES: &[&str] = &[
    "CHANGE_ME",
    "REPLACE_ME",
    "YOUR_SECRET",
    "GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX", // 51-char Stellar placeholder prefix
    "SXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX", // 51-char Stellar secret placeholder prefix
];

/// Environment variables that should be validated if present
const VALIDATED_VARS: &[(&str, fn(&str) -> bool)] = &[
    ("SERVER_PORT", validate_port),
    ("DB_POOL_MAX_CONNECTIONS", validate_positive_number),
    ("DB_POOL_MIN_CONNECTIONS", validate_positive_number),
    ("RPC_MAX_RECORDS_PER_REQUEST", validate_positive_number),
    ("RPC_MAX_TOTAL_RECORDS", validate_positive_number),
    ("RPC_PAGINATION_DELAY_MS", validate_positive_number),
    ("REQUEST_TIMEOUT_SECONDS", validate_request_timeout),
    ("SLOW_QUERY_THRESHOLD_MS", validate_slow_query_threshold),
    ("JWT_SECRET", validate_jwt_secret),
    ("APP_ENV", validate_app_env),
    ("VAULT_ADDR", validate_url_format),
    ("STELLAR_RPC_URL_MAINNET", validate_stellar_rpc_url),
    ("STELLAR_RPC_URL_TESTNET", validate_stellar_rpc_url),
    ("STELLAR_HORIZON_URL_MAINNET", validate_url_format),
    ("STELLAR_HORIZON_URL_TESTNET", validate_url_format),
    ("STELLAR_NETWORK", validate_stellar_network),
];

/// Check if a value matches a known placeholder pattern
fn is_placeholder(value: &str) -> bool {
    PLACEHOLDER_PREFIXES.iter().any(|prefix| value.starts_with(prefix))
}

/// Validates all required environment variables are set and contain no placeholders.
///
/// Called during startup — the process **must not proceed** if this returns an error.
pub fn validate_env() -> Result<()> {
    let mut errors = Vec::new();

    // Check required variables
    for var in REQUIRED_VARS {
        match env::var(var) {
            Err(_) => {
                errors.push(format!(
                    "Missing required environment variable: {var}. \
                     See .env.example for configuration guidance."
                ));
            }
            Ok(value) if is_placeholder(&value) => {
                errors.push(format!(
                    "Environment variable {var} is set to a placeholder value '{value}'. \
                     Replace it with a real value before starting the server."
                ));
            }
            Ok(_) => {}
        }
    }

    // Validate format of present variables
    for (var, validator) in VALIDATED_VARS {
        if let Ok(value) = env::var(var) {
            if !validator(&value) {
                errors.push(format!(
                    "Invalid value for environment variable {var}: '{value}'. \
                     See .env.example for the expected format."
                ));
            }
        }
    }

    // Specific, actionable validation for JWT_SECRET
    if let Ok(jwt_secret) = env::var("JWT_SECRET") {
        let is_production = env::var("APP_ENV").unwrap_or_default() == "production";

        if jwt_secret == "CHANGE_ME_generate_with_openssl_rand_base64_48" {
            errors.push(
                "JWT_SECRET is set to the placeholder value. \
                This is a critical security risk. \
                Generate a secure secret with: openssl rand -base64 48"
                    .to_string(),
            );
        } else if is_production && jwt_secret.len() < 48 {
            errors.push(format!(
                "JWT_SECRET is too short ({} characters) for production. \
                Must be at least 48 characters. \
                Generate a secure secret with: openssl rand -base64 48",
                jwt_secret.len()
            ));
        } else if jwt_secret.len() < 32 {
            errors.push(format!(
                "JWT_SECRET is too short ({} characters). \
                Must be at least 32 characters. \
                Generate a secure secret with: openssl rand -base64 48",
                jwt_secret.len()
            ));
        }
    }

    // SEP10_SERVER_PUBLIC_KEY must be a valid Stellar public key when set
    if let Ok(sep10_key) = env::var("SEP10_SERVER_PUBLIC_KEY") {
        if !validate_stellar_public_key(&sep10_key) {
            errors.push(
                "SEP10_SERVER_PUBLIC_KEY is not a valid Stellar public key. \
                It must start with 'G', be exactly 56 characters, use base32 encoding (A-Z, 2-7), \
                and must not be a placeholder. \
                Generate one with: stellar keys generate"
                    .to_string(),
            );
        }
    }

    // Cross-validate network consistency
    if let Err(network_errors) = validate_network_consistency() {
        errors.extend(network_errors);
    }

    if !errors.is_empty() {
        anyhow::bail!(
            "Environment configuration errors ({} issue(s)):\n  - {}",
            errors.len(),
            errors.join("\n  - ")
        );
    }

    Ok(())
}

/// Validates that network-related environment variables are mutually consistent.
///
/// For example: if `STELLAR_NETWORK=mainnet` is set but the passphrase looks like
/// a testnet passphrase, that is a misconfiguration we must reject.
pub fn validate_network_consistency() -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    let network = env::var("STELLAR_NETWORK").unwrap_or_default();
    let passphrase = env::var("STELLAR_NETWORK_PASSPHRASE").unwrap_or_default();

    if !network.is_empty() && !passphrase.is_empty() {
        let is_mainnet_network = network.eq_ignore_ascii_case("mainnet");
        let is_testnet_passphrase = passphrase.contains("Test SDF");
        let is_mainnet_passphrase = passphrase.contains("Public Global Stellar");

        if is_mainnet_network && is_testnet_passphrase {
            errors.push(
                "Network mismatch: STELLAR_NETWORK=mainnet but STELLAR_NETWORK_PASSPHRASE \
                 contains a testnet passphrase. \
                 For mainnet use: 'Public Global Stellar Network ; September 2015'"
                    .to_string(),
            );
        }

        if !is_mainnet_network && network.eq_ignore_ascii_case("testnet") && is_mainnet_passphrase {
            errors.push(
                "Network mismatch: STELLAR_NETWORK=testnet but STELLAR_NETWORK_PASSPHRASE \
                 contains a mainnet passphrase. \
                 For testnet use: 'Test SDF Network ; September 2015'"
                    .to_string(),
            );
        }
    }

    // Validate that RPC URLs are consistent with the declared network
    if let Ok(rpc_mainnet) = env::var("STELLAR_RPC_URL_MAINNET") {
        if rpc_mainnet.contains("testnet") {
            errors.push(
                "STELLAR_RPC_URL_MAINNET appears to point to a testnet endpoint (URL contains 'testnet'). \
                 Verify this is intentional."
                    .to_string(),
            );
        }
    }

    if let Ok(rpc_testnet) = env::var("STELLAR_RPC_URL_TESTNET") {
        if !rpc_testnet.contains("testnet") && !rpc_testnet.contains("localhost") && !rpc_testnet.contains("127.0.0.1") {
            errors.push(
                "STELLAR_RPC_URL_TESTNET does not appear to point to a testnet endpoint. \
                 Ensure STELLAR_RPC_URL_TESTNET is set to a testnet RPC URL."
                    .to_string(),
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Logs all configured environment variables (without sensitive values)
pub fn log_env_config() {
    tracing::info!("Environment configuration:");

    // Database
    if let Ok(db_url) = env::var("DATABASE_URL") {
        let sanitized = sanitize_database_url(&db_url);
        tracing::info!("  DATABASE_URL: {}", sanitized);
    }

    // Server
    log_var("SERVER_HOST");
    log_var("SERVER_PORT");
    log_var("RUST_LOG");

    // Redis
    if let Ok(redis_url) = env::var("REDIS_URL") {
        let sanitized = sanitize_url(&redis_url);
        tracing::info!("  REDIS_URL: {}", sanitized);
    }

    // Network
    log_var("STELLAR_NETWORK");
    log_var("RPC_MOCK_MODE");

    // Pool config
    log_var("DB_POOL_MAX_CONNECTIONS");
    log_var("DB_POOL_MIN_CONNECTIONS");
    log_var("DB_POOL_CONNECT_TIMEOUT_SECONDS");
    log_var("DB_POOL_IDLE_TIMEOUT_SECONDS");
    log_var("DB_POOL_MAX_LIFETIME_SECONDS");

    // Request timeout
    log_var("REQUEST_TIMEOUT_SECONDS");

    // CORS
    log_var("CORS_ALLOWED_ORIGINS");

    // Slack Bot
    if let Ok(slack_url) = env::var("SLACK_WEBHOOK_URL") {
        let sanitized = sanitize_url(&slack_url);
        tracing::info!("  SLACK_WEBHOOK_URL: {}", sanitized);
    }

    // Price feed (don't log API key)
    log_var("PRICE_FEED_PROVIDER");
    if env::var("PRICE_FEED_API_KEY").is_ok() {
        tracing::info!("  PRICE_FEED_API_KEY: [REDACTED]");
    }

    // RPC Pagination
    log_var("RPC_MAX_RECORDS_PER_REQUEST");
    log_var("RPC_MAX_TOTAL_RECORDS");
    log_var("RPC_PAGINATION_DELAY_MS");

    // Telegram
    if env::var("TELEGRAM_BOT_TOKEN").is_ok() {
        tracing::info!("  TELEGRAM_BOT_TOKEN: [REDACTED]");
    }
}

/// Helper to log a single environment variable
fn log_var(name: &str) {
    if let Ok(value) = env::var(name) {
        tracing::info!("  {}: {}", name, value);
    }
}

/// Sanitize database URL to hide credentials
fn sanitize_database_url(url: &str) -> String {
    if url.starts_with("sqlite:") {
        return url.to_string();
    }

    // For postgres/mysql URLs, hide password
    if let Some(at_pos) = url.rfind('@') {
        if let Some(colon_pos) = url[..at_pos].rfind(':') {
            if let Some(scheme_end) = url.find("://") {
                let scheme = &url[..scheme_end + 3];
                let user = &url[scheme_end + 3..colon_pos];
                let host_and_db = &url[at_pos..];
                return format!("{scheme}{user}:****{host_and_db}");
            }
        }
    }

    "[REDACTED]".to_string()
}

/// Sanitize generic URL to hide credentials
fn sanitize_url(url: &str) -> String {
    if let Some(at_pos) = url.rfind('@') {
        if let Some(scheme_end) = url.find("://") {
            let scheme = &url[..scheme_end + 3];
            let host_and_path = &url[at_pos + 1..];
            return format!("{scheme}****@{host_and_path}");
        }
    }
    url.to_string()
}

/// Validate port number
fn validate_port(value: &str) -> bool {
    value.parse::<u16>().map(|p| p > 0).unwrap_or(false)
}

/// Validate positive number
fn validate_positive_number(value: &str) -> bool {
    value.parse::<u32>().map(|n| n > 0).unwrap_or(false)
}

/// Validate JWT secret
/// Must not be the placeholder value and should be at least 32 characters
fn validate_jwt_secret(value: &str) -> bool {
    // Check if it's the placeholder value
    if value == "CHANGE_ME_generate_with_openssl_rand_base64_48" {
        return false;
    }

    // Ensure minimum length of 32 characters for security
    value.len() >= 32
}

/// Validate REQUEST_TIMEOUT_SECONDS: must be in range [1, 300]
fn validate_request_timeout(value: &str) -> bool {
    value
        .parse::<u64>()
        .map(|n| (1..=300).contains(&n))
        .unwrap_or(false)
}

/// Validate SLOW_QUERY_THRESHOLD_MS: must be in range [1, 60000]
fn validate_slow_query_threshold(value: &str) -> bool {
    value
        .parse::<u64>()
        .map(|n| (1..=60_000).contains(&n))
        .unwrap_or(false)
}

/// Validate Stellar public key format
/// Must start with 'G' and be exactly 56 characters (Ed25519 public key in base32)
fn validate_stellar_public_key(value: &str) -> bool {
    if !value.starts_with('G') || value.len() != 56 {
        return false;
    }

    // Check if it's not the placeholder value
    if value == "GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX" {
        return false;
    }

    // Validate base32 characters (A-Z, 2-7)
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
}

/// Validate APP_ENV: must be development, test, or production
fn validate_app_env(value: &str) -> bool {
    matches!(value, "development" | "test" | "production")
}

/// Validate URL format
fn validate_url_format(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

/// Validate Stellar RPC URL — must be a proper HTTPS URL (non-placeholder)
fn validate_stellar_rpc_url(value: &str) -> bool {
    if !validate_url_format(value) {
        return false;
    }
    if is_placeholder(value) {
        return false;
    }
    true
}

/// Validate STELLAR_NETWORK: must be "mainnet" or "testnet"
fn validate_stellar_network(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "mainnet" | "testnet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_sqlite_url() {
        let url = "sqlite:./stellar_insights.db";
        assert_eq!(sanitize_database_url(url), url);
    }

    #[test]
    fn test_sanitize_postgres_url() {
        let url = "postgresql://user:secret123@localhost:5432/db";
        let sanitized = sanitize_database_url(url);
        assert_eq!(sanitized, "postgresql://user:****@localhost:5432/db");
        assert!(!sanitized.contains("secret123"));
    }

    #[test]
    fn test_sanitize_redis_url() {
        let url = "redis://user:pass@localhost:6379";
        let sanitized = sanitize_url(url);
        assert_eq!(sanitized, "redis://****@localhost:6379");
        assert!(!sanitized.contains("pass"));
    }

    #[test]
    fn test_validate_port() {
        assert!(validate_port("8080"));
        assert!(validate_port("80"));
        assert!(validate_port("65535"));
        assert!(!validate_port("0"));
        assert!(!validate_port("70000"));
        assert!(!validate_port("abc"));
        assert!(!validate_port("-1"));
    }

    #[test]
    fn test_validate_jwt_secret() {
        // Valid secrets
        assert!(validate_jwt_secret("a".repeat(32).as_str()));
        assert!(validate_jwt_secret(
            "this_is_a_very_secure_jwt_secret_key_12345"
        ));

        // Invalid - placeholder
        assert!(!validate_jwt_secret(
            "CHANGE_ME_generate_with_openssl_rand_base64_48"
        ));

        // Invalid - too short
        assert!(!validate_jwt_secret("short"));
        assert!(!validate_jwt_secret("only_31_chars_long_x"));
    }

    #[test]
    fn test_validate_positive_number() {
        assert!(validate_positive_number("1"));
        assert!(validate_positive_number("100"));
        assert!(!validate_positive_number("0"));
        assert!(!validate_positive_number("-1"));
        assert!(!validate_positive_number("abc"));
    }

    #[test]
    fn test_validate_env_rejects_jwt_placeholder() {
        std::env::set_var("DATABASE_URL", "sqlite://test.db");
        std::env::set_var("ENCRYPTION_KEY", "a".repeat(32));
        std::env::set_var(
            "JWT_SECRET",
            "CHANGE_ME_generate_with_openssl_rand_base64_48",
        );

        let result = validate_env();
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("placeholder"),
            "Error should mention 'placeholder', got: {msg}"
        );
        assert!(
            msg.contains("openssl rand -base64 48"),
            "Error should include generation command, got: {msg}"
        );

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("ENCRYPTION_KEY");
        std::env::remove_var("JWT_SECRET");
    }

    #[test]
    fn test_validate_env_rejects_short_jwt_secret() {
        std::env::set_var("DATABASE_URL", "sqlite://test.db");
        std::env::set_var("ENCRYPTION_KEY", "a".repeat(32));
        std::env::set_var("JWT_SECRET", "tooshort");

        let result = validate_env();
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("too short"),
            "Error should mention 'too short', got: {msg}"
        );
        assert!(
            msg.contains("32 characters"),
            "Error should mention minimum length, got: {msg}"
        );

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("ENCRYPTION_KEY");
        std::env::remove_var("JWT_SECRET");
    }

    #[test]
    fn test_validate_env_accepts_valid_jwt_secret() {
        std::env::set_var("DATABASE_URL", "sqlite://test.db");
        std::env::set_var("ENCRYPTION_KEY", "a".repeat(32));
        std::env::set_var("JWT_SECRET", "a".repeat(48));

        let result = validate_env();
        assert!(result.is_ok(), "Should accept a valid JWT secret");

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("ENCRYPTION_KEY");
        std::env::remove_var("JWT_SECRET");
    }

    #[test]
    fn test_validate_stellar_network_valid() {
        assert!(validate_stellar_network("mainnet"));
        assert!(validate_stellar_network("testnet"));
        assert!(validate_stellar_network("MAINNET"));
        assert!(validate_stellar_network("Testnet"));
    }

    #[test]
    fn test_validate_stellar_network_invalid() {
        assert!(!validate_stellar_network("stagenet"));
        assert!(!validate_stellar_network("devnet"));
        assert!(!validate_stellar_network(""));
        assert!(!validate_stellar_network("main"));
    }

    #[test]
    fn test_validate_stellar_rpc_url_valid() {
        assert!(validate_stellar_rpc_url("https://soroban-testnet.stellar.org"));
        assert!(validate_stellar_rpc_url("https://stellar.api.onfinality.io/public"));
        assert!(validate_stellar_rpc_url("http://localhost:8000"));
    }

    #[test]
    fn test_validate_stellar_rpc_url_invalid() {
        assert!(!validate_stellar_rpc_url("not-a-url"));
        assert!(!validate_stellar_rpc_url("ftp://example.com"));
        assert!(!validate_stellar_rpc_url("CHANGE_ME_rpc_url"));
    }

    #[test]
    fn test_is_placeholder_detection() {
        assert!(is_placeholder("CHANGE_ME_generate_with_openssl_rand_base64_48"));
        assert!(is_placeholder("REPLACE_ME_with_real_value"));
        assert!(is_placeholder("YOUR_SECRET_HERE"));
        assert!(!is_placeholder("actual_real_secret_value_12345678901234567890"));
    }

    #[test]
    fn test_network_consistency_mainnet_testnet_mismatch() {
        std::env::set_var("STELLAR_NETWORK", "mainnet");
        std::env::set_var(
            "STELLAR_NETWORK_PASSPHRASE",
            "Test SDF Network ; September 2015",
        );

        let result = validate_network_consistency();
        assert!(result.is_err(), "Should catch mainnet/testnet mismatch");
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("Network mismatch")));

        std::env::remove_var("STELLAR_NETWORK");
        std::env::remove_var("STELLAR_NETWORK_PASSPHRASE");
    }

    #[test]
    fn test_network_consistency_valid_mainnet() {
        std::env::set_var("STELLAR_NETWORK", "mainnet");
        std::env::set_var(
            "STELLAR_NETWORK_PASSPHRASE",
            "Public Global Stellar Network ; September 2015",
        );

        let result = validate_network_consistency();
        assert!(result.is_ok(), "Valid mainnet config should pass");

        std::env::remove_var("STELLAR_NETWORK");
        std::env::remove_var("STELLAR_NETWORK_PASSPHRASE");
    }

    #[test]
    fn test_network_consistency_valid_testnet() {
        std::env::set_var("STELLAR_NETWORK", "testnet");
        std::env::set_var(
            "STELLAR_NETWORK_PASSPHRASE",
            "Test SDF Network ; September 2015",
        );

        let result = validate_network_consistency();
        assert!(result.is_ok(), "Valid testnet config should pass");

        std::env::remove_var("STELLAR_NETWORK");
        std::env::remove_var("STELLAR_NETWORK_PASSPHRASE");
    }

    #[test]
    fn test_rpc_url_mainnet_testnet_cross_contamination() {
        std::env::set_var(
            "STELLAR_RPC_URL_MAINNET",
            "https://soroban-testnet.stellar.org",
        );

        let result = validate_network_consistency();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("STELLAR_RPC_URL_MAINNET")));

        std::env::remove_var("STELLAR_RPC_URL_MAINNET");
    }

    #[test]
    fn test_env_rejects_placeholder_required_var() {
        std::env::set_var("DATABASE_URL", "CHANGE_ME_db_url");
        std::env::set_var("ENCRYPTION_KEY", "a".repeat(32));
        std::env::set_var("JWT_SECRET", "a".repeat(48));
        std::env::set_var("APP_ENV", "development");
        std::env::set_var("VAULT_ADDR", "https://vault.example.com");
        std::env::set_var("VAULT_TOKEN", "a".repeat(16));

        let result = validate_env();
        assert!(result.is_err(), "Should reject placeholder DATABASE_URL");

        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("ENCRYPTION_KEY");
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("APP_ENV");
        std::env::remove_var("VAULT_ADDR");
        std::env::remove_var("VAULT_TOKEN");
    }
}
