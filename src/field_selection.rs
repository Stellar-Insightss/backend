//! Field selection support for API responses.
//!
//! Allows clients to request specific fields from API responses using a whitelist approach.
//! Example: `?fields=id,name,created_at`

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Field selection query parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSelector {
    /// Set of requested field names.
    pub fields: Option<HashSet<String>>,
}

impl FieldSelector {
    /// Create a new field selector from comma-separated string.
    pub fn from_query(query: Option<String>) -> Self {
        let fields = query.map(|q| {
            q.split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect::<HashSet<_>>()
        });
        Self { fields }
    }

    /// Check if a field should be included.
    #[must_use]
    pub fn should_include(&self, field: &str) -> bool {
        match &self.fields {
            None => true, // No selection = include all
            Some(fields) => fields.contains(field),
        }
    }

    /// Check if all fields in the provided set are requested.
    #[must_use]
    pub fn includes_all(&self, field_names: &[&str]) -> bool {
        match &self.fields {
            None => true,
            Some(fields) => field_names.iter().all(|f| fields.contains(*f)),
        }
    }
}

/// Field whitelist schema for validation.
#[derive(Debug, Clone)]
pub struct FieldSchema {
    /// Allowed field names for this endpoint.
    allowed_fields: HashSet<String>,
}

impl FieldSchema {
    /// Create a new field schema with allowed fields.
    pub fn new(fields: &[&str]) -> Self {
        Self {
            allowed_fields: fields.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Validate and filter field selector against the schema.
    pub fn validate(&self, selector: &FieldSelector) -> Result<FieldSelector, String> {
        if let Some(fields) = &selector.fields {
            let invalid: Vec<_> = fields
                .iter()
                .filter(|f| !self.allowed_fields.contains(*f))
                .collect();

            if !invalid.is_empty() {
                return Err(format!(
                    "Invalid fields: {}. Allowed: {}",
                    invalid.join(", "),
                    self.allowed_fields.iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        Ok(selector.clone())
    }

    /// Get all allowed fields.
    #[must_use]
    pub fn allowed_fields(&self) -> &HashSet<String> {
        &self.allowed_fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_selector_from_query() {
        let selector = FieldSelector::from_query(Some("id,name,email".to_string()));
        assert!(selector.should_include("id"));
        assert!(selector.should_include("name"));
        assert!(!selector.should_include("password"));
    }

    #[test]
    fn field_selector_no_query_includes_all() {
        let selector = FieldSelector::from_query(None);
        assert!(selector.should_include("any_field"));
        assert!(selector.should_include("another_field"));
    }

    #[test]
    fn field_schema_validation_success() {
        let schema = FieldSchema::new(&["id", "name", "email"]);
        let selector = FieldSelector::from_query(Some("id,name".to_string()));
        assert!(schema.validate(&selector).is_ok());
    }

    #[test]
    fn field_schema_validation_failure() {
        let schema = FieldSchema::new(&["id", "name", "email"]);
        let selector = FieldSelector::from_query(Some("id,invalid_field".to_string()));
        assert!(schema.validate(&selector).is_err());
    }

    #[test]
    fn field_schema_includes_all() {
        let selector = FieldSelector::from_query(Some("id,name,email".to_string()));
        assert!(selector.includes_all(&["id", "name"]));
        assert!(!selector.includes_all(&["id", "name", "password"]));
    }
}
