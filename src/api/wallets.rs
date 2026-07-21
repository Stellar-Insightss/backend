use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use crate::database::Database;
use crate::error::{ApiError, ApiResult};
use crate::services::price_feed::PriceFeedClient;
use crate::validation::validate_stellar_address;

const DEFAULT_ACTIVITY_WINDOW_DAYS: i64 = 365;
const MAX_ACTIVITY_WINDOW_DAYS: i64 = 365;

const DEFAULT_TRANSFERS_LIMIT: i64 = 20;
const MAX_TRANSFERS_LIMIT: i64 = 100;

/// Bound on how many of the address's most recent transfers are considered
/// when ranking by USD value. Keeps the query and the price-feed conversion
/// pass cheap; addresses with more history than this get ranked over their
/// most recent `TRANSFER_CANDIDATE_POOL` transfers rather than their entire
/// lifetime.
const TRANSFER_CANDIDATE_POOL: i64 = 1000;

#[derive(Clone)]
pub struct WalletState {
    pub db: Arc<Database>,
    pub price_feed: Arc<PriceFeedClient>,
}

// ---------------------------------------------------------------------------
// Activity calendar
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityDay {
    /// UTC calendar date (YYYY-MM-DD)
    #[schema(example = "2026-07-20")]
    pub date: String,
    /// Number of transactions touching this address on that day
    #[schema(example = 12)]
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityCalendarResponse {
    pub address: String,
    pub activity: Vec<ActivityDay>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ActivityCalendarQuery {
    /// Trailing window size in days (1-365, default 365)
    #[param(example = 365)]
    pub days: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ActivityDayRow {
    day: String,
    count: i64,
}

/// Get per-day transaction activity for a wallet
///
/// Returns a count of transactions touching the address (as source or
/// destination) for each UTC calendar day in the requested window, suitable
/// for rendering an activity calendar/heatmap.
#[utoipa::path(
    get,
    path = "/api/v1/wallets/{address}/activity-calendar",
    params(
        ("address" = String, Path, description = "Stellar account address (G...)"),
        ActivityCalendarQuery
    ),
    responses(
        (status = 200, description = "Per-day activity counts", body = ActivityCalendarResponse),
        (status = 400, description = "Invalid Stellar address"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Wallets"
)]
pub async fn get_activity_calendar(
    State(state): State<WalletState>,
    Path(address): Path<String>,
    Query(params): Query<ActivityCalendarQuery>,
) -> ApiResult<Json<ActivityCalendarResponse>> {
    validate_stellar_address(&address)?;
    let days = params
        .days
        .unwrap_or(DEFAULT_ACTIVITY_WINDOW_DAYS)
        .clamp(1, MAX_ACTIVITY_WINDOW_DAYS);

    // date(created_at) truncates the stored RFC3339 timestamp to a UTC
    // calendar date, matching the boundary convention used elsewhere in the
    // wallet dashboard endpoints.
    let rows = sqlx::query_as::<_, ActivityDayRow>(
        r"
        SELECT date(created_at) AS day, COUNT(*) AS count
        FROM payments
        WHERE (source_account = ?1 OR destination_account = ?1)
          AND date(created_at) >= date('now', ?2)
        GROUP BY day
        ORDER BY day ASC
        ",
    )
    .bind(&address)
    .bind(format!("-{days} days"))
    .fetch_all(state.db.pool())
    .await
    .map_err(|e| {
        ApiError::internal(
            "wallet_activity_calendar_query_failed",
            format!("Failed to load activity calendar: {e}"),
        )
    })?;

    let activity = rows
        .into_iter()
        .map(|r| ActivityDay {
            date: r.day,
            count: r.count,
        })
        .collect();

    Ok(Json(ActivityCalendarResponse { address, activity }))
}

// ---------------------------------------------------------------------------
// Largest transfers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    In,
    Out,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LargestTransfer {
    /// Payment/operation id
    pub id: String,
    /// Transaction hash
    pub transaction_hash: String,
    /// The other party in the transfer
    #[schema(example = "GA...")]
    pub counterparty: String,
    /// Whether the transfer was incoming or outgoing relative to the queried address
    pub direction: TransferDirection,
    /// Raw amount in the asset's own units
    #[schema(example = 25000.0)]
    pub amount: f64,
    /// Asset code (e.g. "XLM", "USDC")
    pub asset_code: String,
    /// Asset issuer, `None` for native XLM
    pub asset_issuer: Option<String>,
    /// Amount converted to USD via the price feed, used to rank "largest"
    #[schema(example = 3125.5)]
    pub amount_usd: f64,
    /// RFC3339 timestamp
    pub timestamp: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LargestTransfersResponse {
    pub address: String,
    pub transfers: Vec<LargestTransfer>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LargestTransfersQuery {
    /// Number of transfers to return (1-100, default 20)
    #[param(example = 20)]
    pub limit: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct PaymentCandidateRow {
    id: String,
    transaction_hash: String,
    source_account: String,
    destination_account: String,
    asset_type: String,
    asset_code: Option<String>,
    asset_issuer: Option<String>,
    amount: f64,
    created_at: String,
}

/// Builds the price-feed asset identifier (e.g. `"XLM:native"` or
/// `"USDC:GISSUER..."`) for a payment row.
fn asset_identifier(asset_type: &str, code: Option<&str>, issuer: Option<&str>) -> String {
    if asset_type == "native" {
        "XLM:native".to_string()
    } else {
        format!(
            "{}:{}",
            code.unwrap_or("UNKNOWN"),
            issuer.unwrap_or("unknown")
        )
    }
}

/// Get the largest transfers for a wallet by USD value
///
/// Returns the address's largest transfers (incoming or outgoing), ranked by
/// value converted to a common USD basis via the price feed -- comparing raw
/// asset amounts directly would be meaningless across assets of very
/// different unit value (e.g. a $10,000 XLM transfer vs. a $10 USDC
/// transfer). Assets with no known price mapping are excluded from the
/// ranking, since "largest" is undefined for them without a conversion rate.
#[utoipa::path(
    get,
    path = "/api/v1/wallets/{address}/largest-transfers",
    params(
        ("address" = String, Path, description = "Stellar account address (G...)"),
        LargestTransfersQuery
    ),
    responses(
        (status = 200, description = "Largest transfers by USD value", body = LargestTransfersResponse),
        (status = 400, description = "Invalid Stellar address"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Wallets"
)]
pub async fn get_largest_transfers(
    State(state): State<WalletState>,
    Path(address): Path<String>,
    Query(params): Query<LargestTransfersQuery>,
) -> ApiResult<Json<LargestTransfersResponse>> {
    validate_stellar_address(&address)?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_TRANSFERS_LIMIT)
        .clamp(1, MAX_TRANSFERS_LIMIT);

    let candidates = sqlx::query_as::<_, PaymentCandidateRow>(
        r"
        SELECT id, transaction_hash, source_account, destination_account,
               asset_type, asset_code, asset_issuer, amount, created_at
        FROM payments
        WHERE source_account = ?1 OR destination_account = ?1
        ORDER BY created_at DESC
        LIMIT ?2
        ",
    )
    .bind(&address)
    .bind(TRANSFER_CANDIDATE_POOL)
    .fetch_all(state.db.pool())
    .await
    .map_err(|e| {
        ApiError::internal(
            "wallet_largest_transfers_query_failed",
            format!("Failed to load transfer candidates: {e}"),
        )
    })?;

    let asset_ids: Vec<String> = {
        let mut seen = HashSet::new();
        candidates
            .iter()
            .map(|c| {
                asset_identifier(
                    &c.asset_type,
                    c.asset_code.as_deref(),
                    c.asset_issuer.as_deref(),
                )
            })
            .filter(|id| seen.insert(id.clone()))
            .collect()
    };

    let prices = state.price_feed.get_prices(&asset_ids).await;

    let mut transfers: Vec<LargestTransfer> = candidates
        .into_iter()
        .filter_map(|c| {
            let asset_id = asset_identifier(
                &c.asset_type,
                c.asset_code.as_deref(),
                c.asset_issuer.as_deref(),
            );
            let price = *prices.get(&asset_id)?;
            let amount_usd = c.amount * price;

            let direction = if c.source_account == address {
                TransferDirection::Out
            } else {
                TransferDirection::In
            };
            let counterparty = if direction == TransferDirection::Out {
                c.destination_account
            } else {
                c.source_account
            };

            Some(LargestTransfer {
                id: c.id,
                transaction_hash: c.transaction_hash,
                counterparty,
                direction,
                amount: c.amount,
                asset_code: c.asset_code.unwrap_or_else(|| "XLM".to_string()),
                asset_issuer: c.asset_issuer,
                amount_usd,
                timestamp: c.created_at,
            })
        })
        .collect();

    transfers.sort_by(|a, b| b.amount_usd.total_cmp(&a.amount_usd));
    transfers.truncate(limit as usize);

    Ok(Json(LargestTransfersResponse { address, transfers }))
}

pub fn routes(db: Arc<Database>, price_feed: Arc<PriceFeedClient>) -> Router {
    let state = WalletState { db, price_feed };
    Router::new()
        .route("/:address/activity-calendar", get(get_activity_calendar))
        .route("/:address/largest-transfers", get(get_largest_transfers))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_identifier_native() {
        assert_eq!(asset_identifier("native", None, None), "XLM:native");
    }

    #[test]
    fn test_asset_identifier_credit_asset() {
        assert_eq!(
            asset_identifier("credit_alphanum4", Some("USDC"), Some("GISSUER")),
            "USDC:GISSUER"
        );
    }

    #[test]
    fn test_asset_identifier_missing_code_falls_back() {
        assert_eq!(
            asset_identifier("credit_alphanum4", None, None),
            "UNKNOWN:unknown"
        );
    }
}
