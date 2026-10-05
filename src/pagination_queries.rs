//! Indexed cursor selectors used by the live list handlers.
//!
//! Database pages freeze insert membership with a sequence ceiling. Corridor
//! pages retain the complete filtered RPC payload so a moving RPC window or a
//! changed metric cannot remove rows halfway through traversal.

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, FromRow, QueryBuilder, Row, Sqlite, SqlitePool};
use utoipa::ToSchema;

use crate::cursor_pagination::{CompoundCursor, CursorPaginatedResponse};

const SNAPSHOT_TTL_MS: i64 = 15 * 60 * 1000;

#[derive(Debug, thiserror::Error)]
pub enum PaginationError {
    #[error("Invalid cursor: {0}")]
    InvalidCursor(String),
    #[error("This result snapshot has expired; restart from the first page")]
    ExpiredSnapshot,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn parse_cursor(
    encoded: Option<&str>,
    scope: &str,
) -> Result<Option<CompoundCursor>, PaginationError> {
    encoded
        .map(|value| CompoundCursor::decode(value, scope).map_err(PaginationError::InvalidCursor))
        .transpose()
}

/// Stored ledger-ingestion records. This deliberately excludes pending signing
/// requests, their XDR payloads and collected signatures.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct LedgerTransaction {
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: Option<String>,
    pub fee: Option<i64>,
    pub operation_count: Option<i64>,
    pub successful: Option<bool>,
    pub created_at: Option<String>,
}

pub async fn list_anchors<T>(
    pool: &SqlitePool,
    limit: i64,
    cursor: Option<&str>,
) -> Result<CursorPaginatedResponse<T>, PaginationError>
where
    T: for<'row> FromRow<'row, SqliteRow> + Serialize,
{
    database_page(pool, "anchors", "anchors", "id", limit, cursor).await
}

pub async fn list_ledger_transactions(
    pool: &SqlitePool,
    limit: i64,
    cursor: Option<&str>,
) -> Result<CursorPaginatedResponse<LedgerTransaction>, PaginationError> {
    database_page(pool, "transactions", "transactions", "hash", limit, cursor).await
}

// Table and identity names are private constants from the two callers above.
// Query values, including all cursor fields, are always bound parameters.
async fn database_page<T>(
    pool: &SqlitePool,
    resource: &str,
    table: &str,
    identity: &str,
    limit: i64,
    encoded: Option<&str>,
) -> Result<CursorPaginatedResponse<T>, PaginationError>
where
    T: for<'row> FromRow<'row, SqliteRow> + Serialize,
{
    crate::cursor_pagination::validate_page_request(limit, 0)
        .map_err(PaginationError::InvalidCursor)?;
    let cursor = parse_cursor(encoded, resource)?;
    if cursor
        .as_ref()
        .is_some_and(|value| value.snapshot.is_some())
    {
        return Err(PaginationError::InvalidCursor(
            "Unexpected RPC snapshot".to_string(),
        ));
    }
    let mut transaction = pool.begin().await?;
    let ceiling = match &cursor {
        Some(value) => value.ceiling,
        None => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(sequence), 0) FROM pagination_keys WHERE resource = ?",
            )
            .bind(resource)
            .fetch_one(&mut *transaction)
            .await?
        }
    };
    let joined = format!(
        " FROM pagination_keys p JOIN {table} r ON r.{identity} = p.row_id \
         WHERE p.resource = "
    );
    let total = if let Some(value) = &cursor {
        value.total
    } else {
        let mut count = QueryBuilder::<Sqlite>::new(format!("SELECT COUNT(*){joined}"));
        count
            .push_bind(resource)
            .push(" AND p.sequence <= ")
            .push_bind(ceiling);
        count
            .build_query_scalar::<i64>()
            .fetch_one(&mut *transaction)
            .await?
    };

    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT r.*, p.timestamp_ms AS page_timestamp, p.sequence AS page_sequence{joined}"
    ));
    query
        .push_bind(resource)
        .push(" AND p.sequence <= ")
        .push_bind(ceiling);
    if let Some(value) = &cursor {
        query
            .push(" AND (p.timestamp_ms, p.sequence) > (")
            .push_bind(value.ts)
            .push(", ")
            .push_bind(value.id)
            .push(")");
    }
    query
        .push(" ORDER BY p.timestamp_ms ASC, p.sequence ASC LIMIT ")
        .push_bind(limit + 1);
    let rows = query.build().fetch_all(&mut *transaction).await?;
    transaction.commit().await?;
    page_from_rows(
        rows,
        total,
        limit,
        encoded,
        resource,
        ceiling,
        None,
        |row| Ok(T::from_row(row)?),
    )
}

fn page_from_rows<T, F>(
    mut rows: Vec<SqliteRow>,
    total: i64,
    limit: i64,
    encoded: Option<&str>,
    scope: &str,
    ceiling: i64,
    snapshot: Option<&str>,
    decode: F,
) -> Result<CursorPaginatedResponse<T>, PaginationError>
where
    T: Serialize,
    F: Fn(&SqliteRow) -> Result<T, PaginationError>,
{
    let has_next = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if has_next {
        rows.last()
            .map(|row| {
                Ok::<_, PaginationError>(
                    CompoundCursor {
                        v: 1,
                        scope: scope.to_string(),
                        ts: row.try_get("page_timestamp")?,
                        id: row.try_get("page_sequence")?,
                        ceiling,
                        total,
                        snapshot: snapshot.map(str::to_string),
                    }
                    .encode()?,
                )
            })
            .transpose()?
    } else {
        None
    };
    let data = rows.iter().map(decode).collect::<Result<Vec<_>, _>>()?;
    Ok(CursorPaginatedResponse::new(
        data,
        total,
        limit,
        encoded.map(str::to_string),
        has_next,
        next_cursor,
    ))
}

/// Retain a complete RPC result in one transaction before returning its first page.
/// Each tuple contains (observed timestamp in ms, emitted public ID, payload).
pub async fn store_snapshot<T: Serialize>(
    pool: &SqlitePool,
    scope: &str,
    mut rows: Vec<(i64, String, T)>,
) -> Result<String, PaginationError> {
    rows.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().timestamp_millis();
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "DELETE FROM pagination_snapshot_rows WHERE snapshot_id IN \
        (SELECT id FROM pagination_snapshots WHERE expires_at_ms <= ?)",
    )
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM pagination_snapshots WHERE expires_at_ms <= ?")
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO pagination_snapshots (id, scope, created_at_ms, expires_at_ms) \
        VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(scope)
    .bind(now)
    .bind(now + SNAPSHOT_TTL_MS)
    .execute(&mut *transaction)
    .await?;
    for (timestamp_ms, row_id, payload) in rows {
        sqlx::query(
            "INSERT INTO pagination_snapshot_rows \
            (snapshot_id, timestamp_ms, row_id, payload) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(timestamp_ms)
        .bind(row_id)
        .bind(serde_json::to_string(&payload)?)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(id)
}

/// Continue a retained RPC result. A missing/expired snapshot is an explicit error;
/// callers must never replace it with a newly aggregated RPC result.
pub async fn snapshot_page<T: Serialize + DeserializeOwned>(
    pool: &SqlitePool,
    scope: &str,
    limit: i64,
    encoded: Option<&str>,
    first_snapshot: Option<&str>,
) -> Result<CursorPaginatedResponse<T>, PaginationError> {
    crate::cursor_pagination::validate_page_request(limit, 0)
        .map_err(PaginationError::InvalidCursor)?;
    let cursor = parse_cursor(encoded, scope)?;
    let snapshot = match (&cursor, first_snapshot) {
        (Some(value), None) => value.snapshot.as_deref(),
        (None, Some(id)) => Some(id),
        _ => None,
    }
    .ok_or_else(|| {
        PaginationError::InvalidCursor("Missing or ambiguous result snapshot".to_string())
    })?;
    let mut transaction = pool.begin().await?;
    let valid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pagination_snapshots \
        WHERE id = ? AND scope = ? AND expires_at_ms > ?",
    )
    .bind(snapshot)
    .bind(scope)
    .bind(Utc::now().timestamp_millis())
    .fetch_one(&mut *transaction)
    .await?;
    if valid != 1 {
        return Err(PaginationError::ExpiredSnapshot);
    }
    let (total, ceiling): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(MAX(sequence), 0) \
        FROM pagination_snapshot_rows WHERE snapshot_id = ?",
    )
    .bind(snapshot)
    .fetch_one(&mut *transaction)
    .await?;
    if cursor
        .as_ref()
        .is_some_and(|value| value.ceiling != ceiling || value.total != total)
    {
        return Err(PaginationError::InvalidCursor(
            "Snapshot ceiling changed".to_string(),
        ));
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT payload, timestamp_ms AS page_timestamp, sequence AS page_sequence \
         FROM pagination_snapshot_rows WHERE snapshot_id = ",
    );
    query.push_bind(snapshot);
    if let Some(value) = &cursor {
        query
            .push(" AND (timestamp_ms, sequence) > (")
            .push_bind(value.ts)
            .push(", ")
            .push_bind(value.id)
            .push(")");
    }
    query
        .push(" ORDER BY timestamp_ms ASC, sequence ASC LIMIT ")
        .push_bind(limit + 1);
    let rows = query.build().fetch_all(&mut *transaction).await?;
    transaction.commit().await?;
    page_from_rows(
        rows,
        total,
        limit,
        encoded,
        scope,
        ceiling,
        Some(snapshot),
        |row| {
            let payload: String = row.try_get("payload")?;
            Ok(serde_json::from_str(&payload)?)
        },
    )
}
