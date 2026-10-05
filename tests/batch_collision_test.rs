use std::{
    collections::{HashMap, HashSet},
    error::Error,
};

use serde_json::{json, Value};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use stellar_analysis_backend::{
    cursor_pagination::{CompoundCursor, CursorPageMeta},
    pagination_queries::{list_anchors, list_ledger_transactions, snapshot_page, store_snapshot},
};

const CREATED_AT: &str = "2026-01-22T10:00:00.123Z";
const BEFORE: [u8; 5] = [9, 1, 7, 0, 5];
const AFTER: [u8; 6] = [8, 2, 10, 4, 6, 3];
const LIMIT: i64 = 3;
const SNAPSHOT_SCOPE: &str = "corridors:USD:active";

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
struct AnchorRow {
    id: String,
}

async fn seed(pool: &SqlitePool, ids: &[u8]) -> Result<(), sqlx::Error> {
    for id in ids {
        sqlx::query(
            "INSERT INTO anchors (id, name, stellar_account, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(format!("anchor-{id:02}"))
        .bind(format!("Anchor {id}"))
        .bind(format!("account-{id:02}"))
        .bind(CREATED_AT)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO transactions (hash, ledger_sequence, source_account, created_at) \
             VALUES (?, 1, ?, ?)",
        )
        .bind(format!("tx-{id:02}"))
        .bind(format!("account-{id:02}"))
        .bind(CREATED_AT)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn read_page(
    pool: &SqlitePool,
    scope: &str,
    cursor: Option<&str>,
    first_snapshot: Option<&str>,
) -> Result<(Vec<String>, CursorPageMeta), Box<dyn Error>> {
    Ok(match scope {
        "anchors" => {
            let page = list_anchors::<AnchorRow>(pool, LIMIT, cursor).await?;
            (
                page.data.into_iter().map(|row| row.id).collect(),
                page.pagination,
            )
        }
        "transactions" => {
            let page = list_ledger_transactions(pool, LIMIT, cursor).await?;
            (
                page.data.into_iter().map(|row| row.hash).collect(),
                page.pagination,
            )
        }
        _ => {
            let page = snapshot_page::<Value>(pool, scope, LIMIT, cursor, first_snapshot).await?;
            let ids = page
                .data
                .iter()
                .map(|row| {
                    row["id"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or("snapshot payload lacks an ID")
                })
                .collect::<Result<Vec<_>, _>>()?;
            (ids, page.pagination)
        }
    })
}

fn assert_seek_index(rows: Vec<sqlx::sqlite::SqliteRow>, index: &str) -> Result<(), sqlx::Error> {
    let plan = rows
        .iter()
        .map(|row| row.try_get::<String, _>("detail"))
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    assert!(
        plan.contains(index),
        "Expected {index} in query plan:\n{plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE"),
        "Unexpected pagination sort:\n{plan}"
    );
    Ok(())
}

#[tokio::test]
async fn identical_timestamps_preserve_every_row_across_live_selectors_and_rpc_snapshot(
) -> Result<(), Box<dyn Error>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
    for migration in [
        include_str!("../migrations/001_create_anchors.sql"),
        include_str!("../migrations/002_create_metrics_corridors_snapshots.sql"),
        include_str!("../migrations/007_create_ledger_ingestion_tables.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await?;
    }
    sqlx::query("INSERT INTO ledgers (sequence, hash, close_time) VALUES (1, 'ledger-1', ?)")
        .bind(CREATED_AT)
        .execute(&pool)
        .await?;
    seed(&pool, &BEFORE).await?;
    sqlx::raw_sql(include_str!(
        "../migrations/035_collision_proof_pagination.sql"
    ))
    .execute(&pool)
    .await?;
    let backfilled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pagination_keys")
        .fetch_one(&pool)
        .await?;
    assert_eq!(backfilled, (BEFORE.len() * 2) as i64);
    seed(&pool, &AFTER).await?;

    let timestamp_ms = chrono::DateTime::parse_from_rfc3339(CREATED_AT)?.timestamp_millis();
    let total = BEFORE.len() + AFTER.len();
    let snapshot = store_snapshot(
        &pool,
        SNAPSHOT_SCOPE,
        (0..total)
            .rev()
            .map(|id| {
                let row_id = format!("corridor-{id:02}");
                (
                    timestamp_ms,
                    row_id.clone(),
                    json!({ "id": row_id, "status": "active" }),
                )
            })
            .collect::<Vec<_>>(),
    )
    .await?;

    for scope in ["anchors", "transactions", SNAPSHOT_SCOPE] {
        let is_snapshot = scope == SNAPSHOT_SCOPE;
        let snapshot_id = is_snapshot.then_some(snapshot.as_str());
        let expected: Vec<String> = if is_snapshot {
            (0..total).map(|id| format!("corridor-{id:02}")).collect()
        } else {
            let prefix = if scope == "anchors" { "anchor" } else { "tx" };
            BEFORE
                .iter()
                .chain(AFTER.iter())
                .map(|id| format!("{prefix}-{id:02}"))
                .collect()
        };
        let key_rows: Vec<(String, i64, i64)> = if is_snapshot {
            sqlx::query_as(
                "SELECT row_id, timestamp_ms, sequence FROM pagination_snapshot_rows \
                            WHERE snapshot_id = ?",
            )
            .bind(&snapshot)
            .fetch_all(&pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT row_id, timestamp_ms, sequence FROM pagination_keys \
                            WHERE resource = ?",
            )
            .bind(scope)
            .fetch_all(&pool)
            .await?
        };
        assert_eq!(
            key_rows.len(),
            total,
            "{scope} missed backfill or trigger registration"
        );
        let ceiling = key_rows
            .iter()
            .map(|row| row.2)
            .max()
            .ok_or("missing pagination keys")?;
        let keys: HashMap<_, _> = key_rows
            .into_iter()
            .map(|(id, ts, seq)| (id, (ts, seq)))
            .collect();
        let mut cursor: Option<String> = None;
        let mut seen = Vec::new();
        let mut unique = HashSet::new();
        let mut previous_key = None;
        let page_count = total.div_ceil(LIMIT as usize);
        assert!(page_count > 2);

        for page_number in 0..page_count {
            let (ids, meta) = read_page(
                &pool,
                scope,
                cursor.as_deref(),
                if page_number == 0 { snapshot_id } else { None },
            )
            .await?;
            assert_eq!(
                ids.len(),
                (total - seen.len()).min(LIMIT as usize),
                "{scope}"
            );
            assert_eq!(meta.total, total as i64, "{scope} total changed");
            assert_eq!(meta.limit, LIMIT);
            assert_eq!(meta.cursor.as_deref(), cursor.as_deref());
            assert_eq!(meta.has_next, page_number + 1 < page_count, "{scope}");
            assert_eq!(meta.next_cursor.is_some(), meta.has_next, "{scope}");
            for id in &ids {
                assert!(unique.insert(id.clone()), "{scope} repeated {id}");
                let key = *keys.get(id).ok_or("returned ID has no pagination key")?;
                assert_eq!(
                    key.0, timestamp_ms,
                    "{scope} timestamp normalization changed"
                );
                assert!(key.1 > 0 && key.1 <= ceiling);
                if let Some(previous) = previous_key {
                    assert!(key > previous, "{scope} tuple keys did not advance");
                }
                previous_key = Some(key);
            }
            if let Some(next) = &meta.next_cursor {
                let boundary = CompoundCursor::decode(next, scope)?;
                assert_eq!(
                    Some((boundary.ts, boundary.id)),
                    previous_key,
                    "{scope} cursor must identify the last emitted row"
                );
                assert_eq!(boundary.ceiling, ceiling);
                assert_eq!(boundary.snapshot.as_deref(), snapshot_id);
            }
            seen.extend(ids);
            cursor = meta.next_cursor;
        }
        assert_eq!(seen, expected, "{scope} lost rows or changed their order");
        assert_eq!(unique.len(), total);
        assert!(
            cursor.is_none(),
            "{scope} advertised continuation beyond the last page"
        );

        let (ts, id) = previous_key.ok_or("no final row")?;
        let terminal = CompoundCursor {
            v: 1,
            scope: scope.to_owned(),
            ts,
            id,
            ceiling,
            total: total as i64,
            snapshot: snapshot_id.map(str::to_owned),
        }
        .encode()?;
        let (ids, meta) = read_page(&pool, scope, Some(&terminal), None).await?;
        assert!(ids.is_empty(), "{scope} returned rows past the final tuple");
        assert_eq!(meta.total, total as i64);
        assert!(!meta.has_next);
        assert!(meta.next_cursor.is_none());

        if scope == "anchors" {
            let rows = sqlx::query(
                "EXPLAIN QUERY PLAN \
                SELECT r.*, p.timestamp_ms AS page_timestamp, p.sequence AS page_sequence \
                FROM pagination_keys p JOIN anchors r ON r.id = p.row_id \
                WHERE p.resource = ? AND p.sequence <= ? AND (p.timestamp_ms, p.sequence) > (?, ?) \
                ORDER BY p.timestamp_ms ASC, p.sequence ASC LIMIT ?",
            )
            .bind(scope)
            .bind(ceiling)
            .bind(timestamp_ms)
            .bind(0_i64)
            .bind(LIMIT + 1)
            .fetch_all(&pool)
            .await?;
            assert_seek_index(rows, "idx_pagination_keys_seek")?;
        } else if is_snapshot {
            let rows = sqlx::query(
                "EXPLAIN QUERY PLAN \
                SELECT payload, timestamp_ms AS page_timestamp, sequence AS page_sequence \
                FROM pagination_snapshot_rows WHERE snapshot_id = ? \
                AND (timestamp_ms, sequence) > (?, ?) \
                ORDER BY timestamp_ms ASC, sequence ASC LIMIT ?",
            )
            .bind(&snapshot)
            .bind(timestamp_ms)
            .bind(0_i64)
            .bind(LIMIT + 1)
            .fetch_all(&pool)
            .await?;
            assert_seek_index(rows, "idx_pagination_snapshot_rows_seek")?;
        }
    }
    Ok(())
}
