//! Validation for Stellar TOML metadata.
//!
//! Ensures generated or fetched TOML payloads meet required structure and constraints.

use crate::services::stellar_toml::{StellarToml, CurrencyInfo, Principal};
use anyhow::{anyhow, Result};

/// Stellar TOML validation rules.
#[derive(Debug, Clone)]
pub struct StellarTomlValidator {
    /// Require organization name
    pub require_org_name: bool,
    /// Require organization URL
    pub require_org_url: bool,
    /// Require support email
    pub require_support_email: bool,
    /// Require network passphrase for test networks
    pub require_network_passphrase: bool,
    /// Maximum organization description length
    pub max_description_length: usize,
}

impl Default for StellarTomlValidator {
    fn default() -> Self {
        Self {
            require_org_name: true,
            require_org_url: true,
            require_support_email: true,
            require_network_passphrase: true,
            max_description_length: 5000,
        }
    }
}

impl StellarTomlValidator {
    /// Create a new validator with default rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a lenient validator (fewer requirements).
    pub fn lenient() -> Self {
        Self {
            require_org_name: false,
            require_org_url: false,
            require_support_email: false,
            require_network_passphrase: false,
            max_description_length: 10000,
        }
    }

    /// Validate a Stellar TOML object.
    pub fn validate(&self, toml: &StellarToml) -> Result<()> {
        // Check required fields
        if self.require_org_name && toml.organization_name.is_none() {
            return Err(anyhow!("organization_name is required"));
        }

        if self.require_org_url && toml.organization_url.is_none() {
            return Err(anyhow!("organization_url is required"));
        }

        if self.require_support_email && toml.organization_support_email.is_none() {
            return Err(anyhow!("organization_support_email is required"));
        }

        if self.require_network_passphrase && toml.network_passphrase.is_none() {
            return Err(anyhow!("network_passphrase is required"));
        }

        // Validate organization fields
        if let Some(ref org_name) = toml.organization_name {
            self.validate_string_field(org_name, "organization_name", 1, 255)?;
        }

        if let Some(ref org_url) = toml.organization_url {
            self.validate_url(org_url, "organization_url")?;
        }

        if let Some(ref org_logo) = toml.organization_logo {
            self.validate_url(org_logo, "organization_logo")?;
        }

        if let Some(ref description) = toml.organization_description {
            if description.len() > self.max_description_length {
                return Err(anyhow!(
                    "organization_description exceeds maximum length of {}",
                    self.max_description_length
                ));
            }
        }

        if let Some(ref email) = toml.organization_official_email {
            self.validate_email(email, "organization_official_email")?;
        }

        if let Some(ref email) = toml.organization_support_email {
            self.validate_email(email, "organization_support_email")?;
        }

        // Validate currencies
        if let Some(ref currencies) = toml.currencies {
            self.validate_currencies(currencies)?;
        }

        // Validate principals
        if let Some(ref principals) = toml.principals {
            self.validate_principals(principals)?;
        }

        Ok(())
    }

    /// Validate currencies array.
    fn validate_currencies(&self, currencies: &[CurrencyInfo]) -> Result<()> {
        if currencies.is_empty() {
            return Err(anyhow!("currencies array cannot be empty if present"));
        }

        // Check for duplicate currency codes
        let mut codes = std::collections::HashSet::new();
        for currency in currencies {
            if !codes.insert(currency.code.clone()) {
                return Err(anyhow!("Duplicate currency code: {}", currency.code));
            }

            // Validate currency code (3 letters + numbers)
            if currency.code.len() > 12 || currency.code.is_empty() {
                return Err(anyhow!(
                    "Invalid currency code length: {}",
                    currency.code
                ));
            }

            // Validate issuer if present
            if let Some(ref issuer) = currency.issuer {
                self.validate_stellar_account(issuer, &currency.code)?;
            }

            // Validate display decimals
            if let Some(decimals) = currency.display_decimals {
                if decimals < 0 || decimals > 18 {
                    return Err(anyhow!(
                        "Invalid display_decimals for {}: must be 0-18",
                        currency.code
                    ));
                }
            }

            // Validate image URL if present
            if let Some(ref image) = currency.image {
                self.validate_url(image, "currency.image")?;
            }
        }

        Ok(())
    }

    /// Validate principals array.
    fn validate_principals(&self, principals: &[Principal]) -> Result<()> {
        if principals.is_empty() {
            return Err(anyhow!("principals array cannot be empty if present"));
        }

        for (idx, principal) in principals.iter().enumerate() {
            if principal.name.is_none()
                && principal.email.is_none()
                && principal.keybase.is_none()
                && principal.twitter.is_none()
                && principal.github.is_none()
            {
                return Err(anyhow!(
                    "principal[{}] must have at least one contact field",
                    idx
                ));
            }

            if let Some(ref email) = principal.email {
                self.validate_email(email, &format!("principal[{}].email", idx))?;
            }
        }

        Ok(())
    }

    /// Validate string field length.
    fn validate_string_field(
        &self,
        value: &str,
        field_name: &str,
        min: usize,
        max: usize,
    ) -> Result<()> {
        if value.len() < min {
            return Err(anyhow!(
                "{} is too short (minimum {} characters)",
                field_name,
                min
            ));
        }
        if value.len() > max {
            return Err(anyhow!(
                "{} exceeds maximum length of {} characters",
                field_name,
                max
            ));
        }
        Ok(())
    }

    /// Validate email address format.
    fn validate_email(&self, email: &str, field_name: &str) -> Result<()> {
        if !email.contains('@') || email.len() < 5 || email.len() > 254 {
            return Err(anyhow!("Invalid email format for {}: {}", field_name, email));
        }
        Ok(())
    }

    /// Validate URL format.
    fn validate_url(&self, url: &str, field_name: &str) -> Result<()> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(anyhow!(
                "Invalid URL for {}: must start with http:// or https://",
                field_name
            ));
        }
        if url.len() > 2048 {
            return Err(anyhow!("{} URL is too long", field_name));
        }
        Ok(())
    }

    /// Validate Stellar account address.
    fn validate_stellar_account(&self, account: &str, context: &str) -> Result<()> {
        // Stellar addresses start with 'G' and are 56 characters long (base32)
        if !account.starts_with('G') || account.len() != 56 {
            return Err(anyhow!(
                "Invalid Stellar account for {}: {}",
                context,
                account
            ));
        }

        // Basic validation of base32 characters
        if !account.chars().all(|c| {
            c.is_ascii_uppercase() || c.is_ascii_digit() || c == '='
        }) {
            return Err(anyhow!(
                "Invalid characters in Stellar account for {}",
                context
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_minimal_toml() -> StellarToml {
        StellarToml {
            organization_name: Some("Test Org".to_string()),
            organization_dba: None,
            organization_url: Some("https://test.com".to_string()),
            organization_logo: None,
            organization_description: None,
            organization_physical_address: None,
            organization_phone_number: None,
            organization_keybase: None,
            organization_twitter: None,
            organization_github: None,
            organization_official_email: None,
            organization_support_email: Some("support@test.com".to_string()),
            network_passphrase: Some("Test Network".to_string()),
            currencies: None,
            principals: None,
            documentation: None,
            domain: "test.com".to_string(),
            fetched_at: 0,
        }
    }

    #[test]
    fn valid_minimal_toml() {
        let toml = create_minimal_toml();
        let validator = StellarTomlValidator::new();
        assert!(validator.validate(&toml).is_ok());
    }

    #[test]
    fn missing_required_organization_name() {
        let mut toml = create_minimal_toml();
        toml.organization_name = None;
        let validator = StellarTomlValidator::new();
        assert!(validator.validate(&toml).is_err());
    }

    #[test]
    fn missing_required_support_email() {
        let mut toml = create_minimal_toml();
        toml.organization_support_email = None;
        let validator = StellarTomlValidator::new();
        assert!(validator.validate(&toml).is_err());
    }

    #[test]
    fn invalid_organization_url() {
        let mut toml = create_minimal_toml();
        toml.organization_url = Some("not-a-url".to_string());
        let validator = StellarTomlValidator::new();
        assert!(validator.validate(&toml).is_err());
    }

    #[test]
    fn invalid_email() {
        let mut toml = create_minimal_toml();
        toml.organization_support_email = Some("not-an-email".to_string());
        let validator = StellarTomlValidator::new();
        assert!(validator.validate(&toml).is_err());
    }

    #[test]
    fn duplicate_currency_codes() {
        let mut toml = create_minimal_toml();
        toml.currencies = Some(vec![
            CurrencyInfo {
                code: "USD".to_string(),
                issuer: Some("GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF46Q6F".to_string()),
                display_decimals: Some(2),
                name: Some("US Dollar".to_string()),
                desc: None,
                conditions: None,
                image: None,
                fixed_number: None,
                max_number: None,
                is_unlimited: None,
                is_asset_anchored: None,
                anchor_asset_type: None,
                anchor_asset: None,
                redemption_instructions: None,
                status: None,
            },
            CurrencyInfo {
                code: "USD".to_string(),
                issuer: None,
                display_decimals: None,
                name: None,
                desc: None,
                conditions: None,
                image: None,
                fixed_number: None,
                max_number: None,
                is_unlimited: None,
                is_asset_anchored: None,
                anchor_asset_type: None,
                anchor_asset: None,
                redemption_instructions: None,
                status: None,
            },
        ]);
        let validator = StellarTomlValidator::new();
        assert!(validator.validate(&toml).is_err());
    }

    #[test]
    fn lenient_validator() {
        let mut toml = create_minimal_toml();
        toml.organization_name = None;
        toml.organization_url = None;
        toml.organization_support_email = None;
        let validator = StellarTomlValidator::lenient();
        assert!(validator.validate(&toml).is_ok());
    }
}
