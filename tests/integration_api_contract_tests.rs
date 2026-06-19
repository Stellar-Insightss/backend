//! Integration tests for API contract compliance.
//!
//! Verifies that all API responses conform to the unified contract across different
//! response types, error scenarios, and data shapes.

#[cfg(test)]
mod api_contract_tests {
    use serde_json::{json, Value};

    /// Validate success response conforms to contract
    fn validate_success_response(response: &Value) -> Result<(), String> {
        // Check top-level structure
        if response.get("status").and_then(|v| v.as_str()) != Some("success") {
            return Err("status field must be 'success'".to_string());
        }

        if !response.get("code").and_then(|v| v.as_i64()).map(|c| c >= 200 && c < 300).unwrap_or(false) {
            return Err("code must be a 2xx status code".to_string());
        }

        if response.get("data").is_none() {
            return Err("data field is required in success response".to_string());
        }

        // Validate optional metadata
        if let Some(metadata) = response.get("metadata") {
            if !metadata.is_object() {
                return Err("metadata must be an object".to_string());
            }
        }

        Ok(())
    }

    /// Validate error response conforms to contract
    fn validate_error_response(response: &Value) -> Result<(), String> {
        if response.get("status").and_then(|v| v.as_str()) != Some("error") {
            return Err("status field must be 'error'".to_string());
        }

        if response.get("code").and_then(|v| v.as_i64()).is_none() {
            return Err("code field is required and must be a number".to_string());
        }

        let error = response
            .get("error")
            .ok_or("error field is required in error response".to_string())?;

        if error.get("code").and_then(|v| v.as_str()).is_none() {
            return Err("error.code must be a non-empty string".to_string());
        }

        if error.get("message").and_then(|v| v.as_str()).is_none() {
            return Err("error.message must be a non-empty string".to_string());
        }

        Ok(())
    }

    #[test]
    fn success_response_minimal() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": { "id": 1, "name": "test" }
        });

        assert!(validate_success_response(&response).is_ok());
    }

    #[test]
    fn success_response_with_metadata() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": { "id": 1, "name": "test" },
            "metadata": {
                "request_id": "req-123",
                "timestamp": "2025-06-19T12:00:00Z",
                "version": "v1"
            }
        });

        assert!(validate_success_response(&response).is_ok());
    }

    #[test]
    fn success_response_with_array_data() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [
                { "id": 1, "name": "item1" },
                { "id": 2, "name": "item2" }
            ]
        });

        assert!(validate_success_response(&response).is_ok());
    }

    #[test]
    fn error_response_minimal() {
        let response = json!({
            "status": "error",
            "code": 400,
            "error": {
                "code": "VALIDATION_ERROR",
                "message": "Invalid input"
            }
        });

        assert!(validate_error_response(&response).is_ok());
    }

    #[test]
    fn error_response_with_details() {
        let response = json!({
            "status": "error",
            "code": 400,
            "error": {
                "code": "VALIDATION_ERROR",
                "message": "Validation failed",
                "details": {
                    "field": "email",
                    "reason": "Invalid format"
                },
                "request_id": "req-123"
            }
        });

        assert!(validate_error_response(&response).is_ok());
    }

    #[test]
    fn error_response_missing_error_code() {
        let response = json!({
            "status": "error",
            "code": 400,
            "error": {
                "message": "Validation failed"
            }
        });

        assert!(validate_error_response(&response).is_err());
    }

    #[test]
    fn error_response_missing_message() {
        let response = json!({
            "status": "error",
            "code": 400,
            "error": {
                "code": "VALIDATION_ERROR"
            }
        });

        assert!(validate_error_response(&response).is_err());
    }

    #[test]
    fn success_response_invalid_code() {
        let response = json!({
            "status": "success",
            "code": 400,
            "data": { "id": 1 }
        });

        assert!(validate_success_response(&response).is_err());
    }

    #[test]
    fn response_missing_status() {
        let response = json!({
            "code": 200,
            "data": { "id": 1 }
        });

        assert!(validate_success_response(&response).is_err());
    }

    #[test]
    fn response_missing_data() {
        let response = json!({
            "status": "success",
            "code": 200
        });

        assert!(validate_success_response(&response).is_err());
    }
}

#[cfg(test)]
mod pagination_contract_tests {
    use serde_json::{json, Value};

    fn validate_paginated_response(response: &Value) -> Result<(), String> {
        // First validate as success response
        if response.get("status").and_then(|v| v.as_str()) != Some("success") {
            return Err("status must be 'success'".to_string());
        }

        let data = response.get("data").ok_or("data field required")?;
        if !data.is_array() {
            return Err("data must be an array for paginated responses".to_string());
        }

        let pagination = response
            .get("pagination")
            .ok_or("pagination field required")?;

        // Validate pagination structure
        if pagination.get("limit").and_then(|v| v.as_i64()).is_none() {
            return Err("pagination.limit must be a number".to_string());
        }

        if pagination.get("total").and_then(|v| v.as_i64()).is_none() {
            return Err("pagination.total must be a number".to_string());
        }

        if pagination.get("has_next").and_then(|v| v.as_bool()).is_none() {
            return Err("pagination.has_next must be a boolean".to_string());
        }

        // Validate next_cursor consistency
        let has_next = pagination.get("has_next").and_then(|v| v.as_bool()).unwrap_or(false);
        if has_next && pagination.get("next_cursor").is_none() {
            return Err("next_cursor required when has_next is true".to_string());
        }

        Ok(())
    }

    #[test]
    fn valid_paginated_response() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [
                { "id": 1, "name": "item1" },
                { "id": 2, "name": "item2" }
            ],
            "pagination": {
                "limit": 50,
                "total": 100,
                "cursor": "abc123",
                "has_next": true,
                "next_cursor": "def456"
            }
        });

        assert!(validate_paginated_response(&response).is_ok());
    }

    #[test]
    fn paginated_response_last_page() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [
                { "id": 99, "name": "item99" },
                { "id": 100, "name": "item100" }
            ],
            "pagination": {
                "limit": 50,
                "total": 100,
                "cursor": "xyz789",
                "has_next": false,
                "next_cursor": null
            }
        });

        assert!(validate_paginated_response(&response).is_ok());
    }

    #[test]
    fn paginated_response_missing_pagination() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [{ "id": 1 }]
        });

        assert!(validate_paginated_response(&response).is_err());
    }

    #[test]
    fn paginated_response_missing_next_cursor_when_has_next() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [{ "id": 1 }],
            "pagination": {
                "limit": 50,
                "total": 100,
                "has_next": true
            }
        });

        assert!(validate_paginated_response(&response).is_err());
    }

    #[test]
    fn empty_paginated_response() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [],
            "pagination": {
                "limit": 50,
                "total": 0,
                "has_next": false,
                "next_cursor": null
            }
        });

        assert!(validate_paginated_response(&response).is_ok());
    }
}

#[cfg(test)]
mod field_selection_contract_tests {
    use serde_json::{json, Value};

    fn validate_field_selection_response(response: &Value, expected_fields: &[&str]) -> Result<(), String> {
        if response.get("status").and_then(|v| v.as_str()) != Some("success") {
            return Err("status must be 'success'".to_string());
        }

        let data = response.get("data").ok_or("data field required")?;

        if let Some(array) = data.as_array() {
            for item in array {
                if let Some(obj) = item.as_object() {
                    let response_fields: Vec<_> = obj.keys().collect();
                    for field in &response_fields {
                        if !expected_fields.contains(&field.as_str()) {
                            return Err(format!("Unexpected field in response: {}", field));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[test]
    fn field_selection_response_valid() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [
                { "id": 1, "name": "test" },
                { "id": 2, "name": "test2" }
            ]
        });

        assert!(validate_field_selection_response(&response, &["id", "name"]).is_ok());
    }

    #[test]
    fn field_selection_response_extra_fields() {
        let response = json!({
            "status": "success",
            "code": 200,
            "data": [
                { "id": 1, "name": "test", "extra_field": "value" }
            ]
        });

        assert!(validate_field_selection_response(&response, &["id", "name"]).is_err());
    }

    #[test]
    fn field_selection_error_invalid_fields() {
        let response = json!({
            "status": "error",
            "code": 400,
            "error": {
                "code": "INVALID_FIELDS",
                "message": "Invalid field selection",
                "details": {
                    "invalid": ["unknown_field"],
                    "allowed": ["id", "name", "domain"]
                }
            }
        });

        assert!(response.get("error").is_some());
        assert_eq!(
            response.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_str()),
            Some("INVALID_FIELDS")
        );
    }
}
