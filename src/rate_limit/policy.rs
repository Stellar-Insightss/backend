//! Two-tier rate limiting for Stellar muxed accounts.
//!
//! A muxed address has two useful identities: the full M-address and its shared
//! base G-address. Enforcing both prevents muxed-ID rotation from bypassing a
//! limit while keeping one busy sub-account from consuming every sibling's
//! per-muxed budget.

use std::time::Duration;

use thiserror::Error;

use crate::muxed::parse_muxed_address;

use super::bucket::TokenBucket;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuxedRateLimitTier {
    MuxedAddress,
    BaseAccount,
}

impl MuxedRateLimitTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MuxedAddress => "muxed_address",
            Self::BaseAccount => "base_account",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MuxedRateLimitConfig {
    pub per_muxed_capacity: u32,
    pub per_muxed_refill_per_second: f64,
    pub per_base_capacity: u32,
    pub per_base_refill_per_second: f64,
}

impl Default for MuxedRateLimitConfig {
    fn default() -> Self {
        Self {
            per_muxed_capacity: 30,
            per_muxed_refill_per_second: 0.5,
            per_base_capacity: 300,
            per_base_refill_per_second: 5.0,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MuxedRateLimitError {
    #[error("invalid Stellar muxed account address")]
    InvalidMuxedAddress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuxedRateLimitDecision {
    pub allowed: bool,
    pub hit_tier: Option<MuxedRateLimitTier>,
    pub muxed_remaining: u32,
    pub base_remaining: u32,
}

impl MuxedRateLimitDecision {
    /// Stable response value suitable for a JSON rejection field/header.
    #[must_use]
    pub const fn rejection_tier(&self) -> Option<&'static str> {
        match self.hit_tier {
            Some(tier) => Some(tier.as_str()),
            None => None,
        }
    }
}

#[derive(Debug)]
pub struct MuxedAccountRateLimiter {
    per_muxed: TokenBucket<String>,
    per_base: TokenBucket<String>,
}

impl MuxedAccountRateLimiter {
    #[must_use]
    pub fn new(config: MuxedRateLimitConfig) -> Self {
        Self {
            per_muxed: TokenBucket::new(
                config.per_muxed_capacity,
                config.per_muxed_refill_per_second,
            ),
            per_base: TokenBucket::new(
                config.per_base_capacity,
                config.per_base_refill_per_second,
            ),
        }
    }

    /// Enforce full-M-address and base-G-address budgets atomically.
    ///
    /// Both tiers are previewed before either token is consumed, so a request
    /// rejected by the shared base tier does not burn the caller's muxed token.
    pub fn check(
        &mut self,
        address: &str,
        now: Duration,
    ) -> Result<MuxedRateLimitDecision, MuxedRateLimitError> {
        let parsed = parse_muxed_address(address).ok_or(MuxedRateLimitError::InvalidMuxedAddress)?;
        let base = parsed
            .base_account
            .ok_or(MuxedRateLimitError::InvalidMuxedAddress)?;
        let muxed = parsed.muxed_address;

        let muxed_allowed = self.per_muxed.can_take(&muxed, now);
        let base_allowed = self.per_base.can_take(&base, now);

        if !muxed_allowed {
            return Ok(MuxedRateLimitDecision {
                allowed: false,
                hit_tier: Some(MuxedRateLimitTier::MuxedAddress),
                muxed_remaining: self.per_muxed.remaining(&muxed, now),
                base_remaining: self.per_base.remaining(&base, now),
            });
        }

        if !base_allowed {
            return Ok(MuxedRateLimitDecision {
                allowed: false,
                hit_tier: Some(MuxedRateLimitTier::BaseAccount),
                muxed_remaining: self.per_muxed.remaining(&muxed, now),
                base_remaining: self.per_base.remaining(&base, now),
            });
        }

        let muxed_snapshot = self.per_muxed.take(&muxed, now);
        let base_snapshot = self.per_base.take(&base, now);

        Ok(MuxedRateLimitDecision {
            allowed: muxed_snapshot.allowed && base_snapshot.allowed,
            hit_tier: None,
            muxed_remaining: muxed_snapshot.remaining,
            base_remaining: base_snapshot.remaining,
        })
    }
}
