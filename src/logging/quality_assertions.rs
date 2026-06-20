/// Production-safety assertions for the logging subsystem.
///
/// These functions are called from tests and the quality-check CI step to
/// verify that sensitive data cannot appear in log output and that
/// production log-level filtering is enforced.
use once_cell::sync::Lazy;
use regex::Regex;

/// Patterns that must never appear verbatim in production log lines.
static SENSITIVE_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Stellar secret keys (S + 55 base32 chars)
        Regex::new(r"S[A-Z2-7]{55}").unwrap(),
        // Raw JWTs
        Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap(),
        // Mnemonic seed phrases (12 or 24 space-separated lowercase words)
        Regex::new(r"(?:[a-z]{3,8}\s){11}[a-z]{3,8}").unwrap(),
        // Private key hex strings (64 hex chars)
        Regex::new(r"\b[0-9a-fA-F]{64}\b").unwrap(),
    ]
});

/// Debug-level keywords that must not appear when `RUST_LOG` is set to `info`
/// or higher in production.
static DEBUG_KEYWORDS: &[&str] = &[
    "DEBUG",
    "TRACE",
    "[debug]",
    "[trace]",
];

/// Check that a log line does not contain any raw sensitive values.
///
/// Returns `Ok(())` when the line is clean, or `Err` with a description of
/// the violation found.
pub fn assert_no_sensitive_data(line: &str) -> Result<(), String> {
    for pattern in SENSITIVE_PATTERNS.iter() {
        if pattern.is_match(line) {
            return Err(format!(
                "Log line contains sensitive data matching pattern `{}`: {}",
                pattern.as_str(),
                &line[..line.len().min(120)]
            ));
        }
    }
    Ok(())
}

/// Check that a log line produced at `info` level contains no debug/trace
/// noise — i.e. the log level filter is working correctly.
pub fn assert_no_debug_output(line: &str) -> Result<(), String> {
    for keyword in DEBUG_KEYWORDS {
        if line.contains(keyword) {
            return Err(format!(
                "Production log line contains debug keyword `{keyword}`: {}",
                &line[..line.len().min(120)]
            ));
        }
    }
    Ok(())
}

/// Validate an entire slice of log lines for production safety.
///
/// Calls both [`assert_no_sensitive_data`] and [`assert_no_debug_output`] on
/// every line and collects all violations rather than short-circuiting.
pub fn assert_log_lines_production_safe(lines: &[&str]) -> Result<(), Vec<String>> {
    let violations: Vec<String> = lines
        .iter()
        .enumerate()
        .flat_map(|(i, line)| {
            let mut errs = Vec::new();
            if let Err(e) = assert_no_sensitive_data(line) {
                errs.push(format!("Line {i}: {e}"));
            }
            if let Err(e) = assert_no_debug_output(line) {
                errs.push(format!("Line {i}: {e}"));
            }
            errs
        })
        .collect();

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_info_line_passes() {
        let line = r#"{"level":"INFO","msg":"HTTP request completed","path":"/api/v1/corridors"}"#;
        assert!(assert_no_sensitive_data(line).is_ok());
        assert!(assert_no_debug_output(line).is_ok());
    }

    #[test]
    fn stellar_secret_key_is_detected() {
        let line = "User secret: SBCXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX";
        assert!(assert_no_sensitive_data(line).is_err());
    }

    #[test]
    fn raw_jwt_is_detected() {
        let line = "token=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.SflKxwRJSMeKKF2QT4fwpMeJf36";
        assert!(assert_no_sensitive_data(line).is_err());
    }

    #[test]
    fn private_key_hex_is_detected() {
        let line = "key=deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        assert!(assert_no_sensitive_data(line).is_err());
    }

    #[test]
    fn debug_keyword_in_production_log_is_detected() {
        let line = "[debug] Processing payment corridor";
        assert!(assert_no_debug_output(line).is_err());
    }

    #[test]
    fn redacted_stellar_address_passes() {
        // redact_account() produces "GXXX...XXXX" which is safe
        let line = r#"{"level":"INFO","account":"GABX...WXYZ","msg":"balance fetched"}"#;
        assert!(assert_no_sensitive_data(line).is_ok());
    }

    #[test]
    fn batch_validation_collects_all_violations() {
        let lines = [
            r#"{"level":"INFO","msg":"ok"}"#,
            "secret=SBCXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
            "[debug] internal detail",
        ];
        let result = assert_log_lines_production_safe(&lines);
        assert!(result.is_err());
        let violations = result.unwrap_err();
        assert_eq!(violations.len(), 2);
    }

    #[test]
    fn batch_validation_passes_for_clean_lines() {
        let lines = [
            r#"{"level":"INFO","msg":"request received"}"#,
            r#"{"level":"WARN","msg":"rate limit approaching","remaining":5}"#,
            r#"{"level":"ERROR","msg":"corridor not found","corridor_id":"USD-EUR"}"#,
        ];
        assert!(assert_log_lines_production_safe(&lines).is_ok());
    }
}
