use std::time::Duration;

use data_encoding::BASE32;
use stellar_analysis_backend::rate_limit::policy::{
    MuxedAccountRateLimiter, MuxedRateLimitConfig, MuxedRateLimitTier,
};

const VERSION_MUXED_ACCOUNT: u8 = 12 << 3;
const CRC16_POLY: u16 = 0x1021;

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ CRC16_POLY
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn muxed_address(account_id: [u8; 32], muxed_id: u64) -> String {
    let mut raw = [0u8; 43];
    raw[0] = VERSION_MUXED_ACCOUNT;
    raw[1..33].copy_from_slice(&account_id);
    raw[33..41].copy_from_slice(&muxed_id.to_be_bytes());
    let checksum = crc16(&raw[..41]).to_le_bytes();
    raw[41..43].copy_from_slice(&checksum);
    BASE32.encode(&raw)
}

#[test]
fn one_noisy_muxed_account_does_not_exhaust_sibling_budgets() {
    let mut limiter = MuxedAccountRateLimiter::new(MuxedRateLimitConfig {
        per_muxed_capacity: 2,
        per_muxed_refill_per_second: 0.0,
        per_base_capacity: 10,
        per_base_refill_per_second: 0.0,
    });
    let account = [11u8; 32];
    let now = Duration::ZERO;
    let noisy = muxed_address(account, 1);

    assert!(limiter.check(&noisy, now).unwrap().allowed);
    assert!(limiter.check(&noisy, now).unwrap().allowed);

    let noisy_rejection = limiter.check(&noisy, now).unwrap();
    assert!(!noisy_rejection.allowed);
    assert_eq!(
        noisy_rejection.hit_tier,
        Some(MuxedRateLimitTier::MuxedAddress)
    );
    assert_eq!(noisy_rejection.rejection_tier(), Some("muxed_address"));

    for id in 2..=6 {
        let sibling = limiter.check(&muxed_address(account, id), now).unwrap();
        assert!(sibling.allowed, "sibling muxed id {id} should remain available");
    }
}
