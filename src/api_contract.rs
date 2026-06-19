//! Unified API response and error contract.
//!
//! All API responses follow this contract for consistency across clients.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Standard API response envelope for all endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum ApiResponse<T: Serialize> {
    /// Successful response with data.
    Success(SuccessResponse<T>),
    /// Error response.
    Error(ErrorResponseContract),
}

/// Successful response structure.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SuccessResponse<T: Serialize> {
    /// Response status ("success")
    pub status: String,
    /// HTTP status code
    pub code: u16,
    /// Response data
    pub data: T,
    /// Optional metadata about the request
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ResponseMetadata>,
}

impl<T: Serialize> SuccessResponse<T> {
    /// Create a successful response.
    pub fn new(data: T, code: u16) -> Self {
        Self {
            status: "success".to_string(),
            code,
            data,
            metadata: None,
        }
    }

    /// Add metadata to the response.
    #[must_use]
    pub fn with_metadata(mut self, metadata: ResponseMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Error response structure following the error contract.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponseContract {
    /// Response status ("error")
    pub status: String,
    /// HTTP status code
    pub code: u16,
    /// Error details
    pub error: ErrorDetail,
}

/// Error detail information.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorDetail {
    /// Machine-readable error code
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Additional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// Request ID for tracing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Optional response metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ResponseMetadata {
    /// Request ID for tracing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Server timestamp in ISO 8601 format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// API version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl ResponseMetadata {
    /// Create new response metadata.
    pub fn new() -> Self {
        Self {
            request_id: None,
            timestamp: None,
            version: None,
        }
    }

    /// Add request ID.
    #[must_use]
    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }

    /// Add timestamp.
    #[must_use]
    pub fn with_timestamp(mut self, timestamp: String) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Add API version.
    #[must_use]
    pub fn with_version(mut self, version: String) -> Self {
        self.version = Some(version);
        self
    }
}

impl Default for ResponseMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorResponseContract {
    /// Create an error response.
    pub fn new(code: u16, error_code: String, message: String) -> Self {
        Self {
            status: "error".to_string(),
            code,
            error: ErrorDetail {
                code: error_code,
                message,
                details: None,
                request_id: None,
            },
        }
    }

    /// Add details to the error.
    #[must_use]
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.error.details = Some(details);
        self
    }

    /// Add request ID.
    #[must_use]
    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.error.request_id = Some(request_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_response_creation() {
        let data = serde_json::json!({"id": 1, "name": "test"});
        let response = SuccessResponse::new(data.clone(), 200);
        assert_eq!(response.status, "success");
        assert_eq!(response.code, 200);
        assert_eq!(response.data, data);
    }

    #[test]
    fn error_response_creation() {
        let error = ErrorResponseContract::new(404, "NOT_FOUND".to_string(), "Resource not found".to_string());
        assert_eq!(error.status, "error");
        assert_eq!(error.code, 404);
        assert_eq!(error.error.code, "NOT_FOUND");
    }

    #[test]
    fn error_response_with_details() {
        let details = serde_json::json!({"field": "email"});
        let error = ErrorResponseContract::new(400, "VALIDATION_ERROR".to_string(), "Invalid input".to_string())
            .with_details(details.clone());
        assert_eq!(error.error.details, Some(details));
    }

    #[test]
    fn response_metadata_builder() {
        let metadata = ResponseMetadata::new()
            .with_request_id("123".to_string())
            .with_timestamp("2025-06-19T00:00:00Z".to_string())
            .with_version("v1".to_string());

        assert_eq!(metadata.request_id, Some("123".to_string()));
        assert!(metadata.timestamp.is_some());
        assert_eq!(metadata.version, Some("v1".to_string()));
    }

    #[test]
    fn success_response_with_metadata() {
        let data = serde_json::json!({"id": 1});
        let metadata = ResponseMetadata::new().with_request_id("req-123".to_string());
        let response = SuccessResponse::new(data, 200).with_metadata(metadata);

        assert!(response.metadata.is_some());
        assert_eq!(response.metadata.unwrap().request_id, Some("req-123".to_string()));
    }
}
