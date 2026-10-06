//! Generic in-memory token bucket used by rate-limit policies.

use std::{collections::HashMap, hash::Hash, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketSnapshot {
    pub allowed: bool,
    pub remaining: u32,
}

#[derive(Debug, Clone, Copy)]
struct BucketState {
    tokens: f64,
    last_refill: Duration,
}

/// Small deterministic token bucket keyed by any hashable identity.
///
/// Callers provide the timestamp so tests and higher-level policies can make
/// atomic decisions across multiple buckets without sleeping.
#[derive(Debug)]
pub struct TokenBucket<K> {
    capacity: u32,
    refill_per_second: f64,
    states: HashMap<K, BucketState>,
}

impl<K> TokenBucket<K>
where
    K: Eq + Hash + Clone,
{
    #[must_use]
    pub fn new(capacity: u32, refill_per_second: f64) -> Self {
        Self {
            capacity,
            refill_per_second: refill_per_second.max(0.0),
            states: HashMap::new(),
        }
    }

    fn state_at(&mut self, key: &K, now: Duration) -> &mut BucketState {
        let capacity = f64::from(self.capacity);
        let refill_per_second = self.refill_per_second;
        let state = self.states.entry(key.clone()).or_insert(BucketState {
            tokens: capacity,
            last_refill: now,
        });

        if now > state.last_refill {
            let elapsed = now.saturating_sub(state.last_refill).as_secs_f64();
            state.tokens = (state.tokens + elapsed * refill_per_second).min(capacity);
            state.last_refill = now;
        }

        state
    }

    pub fn can_take(&mut self, key: &K, now: Duration) -> bool {
        self.state_at(key, now).tokens >= 1.0
    }

    pub fn remaining(&mut self, key: &K, now: Duration) -> u32 {
        self.state_at(key, now).tokens.floor().max(0.0) as u32
    }

    pub fn take(&mut self, key: &K, now: Duration) -> BucketSnapshot {
        let state = self.state_at(key, now);
        if state.tokens < 1.0 {
            return BucketSnapshot {
                allowed: false,
                remaining: state.tokens.floor().max(0.0) as u32,
            };
        }

        state.tokens -= 1.0;
        BucketSnapshot {
            allowed: true,
            remaining: state.tokens.floor().max(0.0) as u32,
        }
    }
}
