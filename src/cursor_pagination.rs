//! Cursor-based pagination for efficient large result sets.
//!
//! # Wire format
//!
//! ```json
//! {
//!   "data": [ ... ],
//!   "pagination": {
//!     "limit": 50,
//!     "total": 312,
//!     "cursor": "eyJpZCI6IDEyMzQ1fQ==",
//!     "has_next": true,
//!     "next_cursor": "eyJpZCI6IDEyMzk1fQ=="
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Cursor-based pagination metadata for efficient traversal.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CursorPageMeta {
    /// Maximum number of items returned per page.
    #[schema(example = 50)]
    pub limit: i64,
    /// Total number of items available.
    #[schema(example = 312)]
    pub total: i64,
    /// Opaque cursor pointing to the current page.
    #[schema(example = "eyJpZCI6IDEyMzQ1fQ==")]
    pub cursor: Option<String>,
    /// Whether more results are available after this page.
    #[schema(example = true)]
    pub has_next: bool,
    /// Cursor to use for fetching the next page.
    #[schema(example = "eyJpZCI6IDEyMzk1fQ==")]
    pub next_cursor: Option<String>,
}

impl CursorPageMeta {
    /// Create pagination metadata for a cursor-based response.
    #[must_use]
    pub fn new(total: i64, limit: i64, cursor: Option<String>, has_next: bool, next_cursor: Option<String>) -> Self {
        Self {
            limit,
            total,
            cursor,
            has_next,
            next_cursor,
        }
    }
}

/// Cursor-based paginated response envelope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CursorPaginatedResponse<T: Serialize> {
    /// The page of items.
    pub data: Vec<T>,
    /// Cursor pagination metadata.
    pub pagination: CursorPageMeta,
}

impl<T: Serialize> CursorPaginatedResponse<T> {
    /// Wrap items with cursor pagination metadata.
    #[must_use]
    pub fn new(
        data: Vec<T>,
        total: i64,
        limit: i64,
        cursor: Option<String>,
        has_next: bool,
        next_cursor: Option<String>,
    ) -> Self {
        Self {
            data,
            pagination: CursorPageMeta::new(total, limit, cursor, has_next, next_cursor),
        }
    }
}

/// Encode cursor from last item ID (generic implementation).
pub fn encode_cursor(id: &str) -> String {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD.encode(id)
}

/// Decode cursor back to last item ID.
pub fn decode_cursor(cursor: &str) -> Result<String, String> {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD
        .decode(cursor)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| "Invalid cursor format".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_encode_decode_roundtrip() {
        let id = "12345";
        let encoded = encode_cursor(id);
        let decoded = decode_cursor(&encoded).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn invalid_cursor_decode() {
        let result = decode_cursor("!!!invalid base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn cursor_page_meta_has_next() {
        let meta = CursorPageMeta::new(100, 50, Some("cursor1".to_string()), true, Some("cursor2".to_string()));
        assert!(meta.has_next);
        assert_eq!(meta.limit, 50);
        assert_eq!(meta.total, 100);
    }

    #[test]
    fn cursor_paginated_response_wraps_data() {
        let items = vec![1u32, 2, 3];
        let resp = CursorPaginatedResponse::new(
            items.clone(),
            100,
            50,
            Some("cursor1".to_string()),
            true,
            Some("cursor2".to_string()),
        );
        assert_eq!(resp.data, items);
        assert!(resp.pagination.has_next);
    }
}
