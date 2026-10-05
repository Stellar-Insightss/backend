use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use stellar_analysis_backend::pagination_queries::{
    list_anchors, list_ledger_transactions, snapshot_page, store_snapshot, PaginationError,
};

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
struct AnchorRow {
    id: String,
}

#[tokio::test]
async fn traversal_survives_concurrent_inserts_updates_and_new_rpc_snapshots(
) -> Result<(), Box<dyn std::error::Error>> {
    const ROWS: i64 = 7;
    const LIMIT: i64 = 2;
    const CREATED: &str = "2026-01-02 03:04:05";
    const SCOPE: &str = "corridors:asset=USDC";

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    for migration in [
        include_str!("../migrations/001_create_anchors.sql"),
        include_str!("../migrations/002_create_metrics_corridors_snapshots.sql"),
        include_str!("../migrations/007_create_ledger_ingestion_tables.sql"),
        include_str!("../migrations/035_collision_proof_pagination.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await?;
    }
    sqlx::query("INSERT INTO ledgers (sequence, hash, close_time) VALUES (1, 'ledger', ?)")
        .bind(CREATED)
        .execute(&pool)
        .await?;
    for index in 0..ROWS {
        sqlx::query(
            "INSERT INTO anchors (id, name, stellar_account, reliability_score, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(format!("anchor-{index}"))
        .bind(format!("Anchor {index}"))
        .bind(format!("account-{index}"))
        .bind(50.0 + index as f64)
        .bind(CREATED)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO transactions (hash, ledger_sequence, source_account, successful, created_at) \
             VALUES (?, 1, ?, 1, ?)",
        )
        .bind(format!("transaction-{index}"))
        .bind(format!("account-{index}"))
        .bind(CREATED)
        .execute(&pool)
        .await?;
    }
    let original_keys: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT resource, row_id, timestamp_ms, sequence FROM pagination_keys ORDER BY sequence",
    )
    .fetch_all(&pool)
    .await?;
    let original_payloads: Vec<Value> = (0..ROWS)
        .map(|index| json!({"id": format!("corridor-{index}"), "volume": index + 1}))
        .collect();
    let snapshot = store_snapshot(
        &pool,
        SCOPE,
        original_payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| {
                (
                    1_767_323_045_000,
                    format!("corridor-{index}"),
                    payload.clone(),
                )
            })
            .collect(),
    )
    .await?;

    let anchor_first = list_anchors::<AnchorRow>(&pool, LIMIT, None).await?;
    let transaction_first = list_ledger_transactions(&pool, LIMIT, None).await?;
    let snapshot_first = snapshot_page::<Value>(&pool, SCOPE, LIMIT, None, Some(&snapshot)).await?;
    let anchor_cursor = anchor_first
        .pagination
        .next_cursor
        .clone()
        .ok_or("expected an anchor continuation")?;
    let transaction_cursor = transaction_first
        .pagination
        .next_cursor
        .clone()
        .ok_or("expected a transaction continuation")?;
    let snapshot_cursor = snapshot_first
        .pagination
        .next_cursor
        .clone()
        .ok_or("expected a corridor continuation")?;
    let anchor_before = list_anchors::<AnchorRow>(&pool, LIMIT, Some(&anchor_cursor)).await?;
    let transaction_before =
        list_ledger_transactions(&pool, LIMIT, Some(&transaction_cursor)).await?;
    let snapshot_before =
        snapshot_page::<Value>(&pool, SCOPE, LIMIT, Some(&snapshot_cursor), None).await?;

    // A separate writer commits between page one and its continuation. This
    // controlled interleaving exercises concurrent traversal without timing sleeps.
    let writer_pool = pool.clone();
    tokio::spawn(async move {
        let mut write = writer_pool.begin().await?;
        for (suffix, timestamp) in [("same", CREATED), ("backdated", "2020-01-02 03:04:05")] {
            sqlx::query(
                "INSERT INTO anchors (id, name, stellar_account, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(format!("late-anchor-{suffix}"))
            .bind(format!("Late {suffix}"))
            .bind(format!("late-account-{suffix}"))
            .bind(timestamp)
            .execute(&mut *write)
            .await?;
            sqlx::query(
                "INSERT INTO transactions (hash, ledger_sequence, created_at) VALUES (?, 1, ?)",
            )
            .bind(format!("late-transaction-{suffix}"))
            .bind(timestamp)
            .execute(&mut *write)
            .await?;
        }
        // Move one already returned row forward and one unseen row backward.
        sqlx::query(
            "UPDATE anchors SET created_at = CASE id WHEN 'anchor-0' \
             THEN '2099-01-01 00:00:00' ELSE '1900-01-01 00:00:00' END, \
             reliability_score = CASE id WHEN 'anchor-0' THEN 100 ELSE 0 END \
             WHERE id IN ('anchor-0', 'anchor-5')",
        )
        .execute(&mut *write)
        .await?;
        sqlx::query(
            "UPDATE transactions SET created_at = CASE hash WHEN 'transaction-0' \
             THEN '2099-01-01 00:00:00' ELSE '1900-01-01 00:00:00' END \
             WHERE hash IN ('transaction-0', 'transaction-5')",
        )
        .execute(&mut *write)
        .await?;
        write.commit().await
    })
    .await??;

    let keys_after: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT resource, row_id, timestamp_ms, sequence FROM pagination_keys \
         WHERE row_id NOT LIKE 'late-%' ORDER BY sequence",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        keys_after, original_keys,
        "updates must preserve both order keys"
    );

    let replacement = store_snapshot(
        &pool,
        SCOPE,
        vec![
            (
                1_767_323_045_000,
                "corridor-1".to_string(),
                json!({"id": "corridor-1", "volume": 999}),
            ),
            (
                1_767_323_045_000,
                "new-corridor".to_string(),
                json!({"id": "new-corridor", "volume": 999}),
            ),
        ],
    )
    .await?;
    assert_ne!(replacement, snapshot);
    let replacement_first =
        snapshot_page::<Value>(&pool, SCOPE, LIMIT, None, Some(&replacement)).await?;
    assert_eq!(replacement_first.pagination.total, 2);
    assert_eq!(replacement_first.data[0]["volume"], json!(999));

    let anchor_replay = list_anchors::<AnchorRow>(&pool, LIMIT, Some(&anchor_cursor)).await?;
    assert_eq!(
        anchor_replay
            .data
            .iter()
            .map(|row| &row.id)
            .collect::<Vec<_>>(),
        anchor_before
            .data
            .iter()
            .map(|row| &row.id)
            .collect::<Vec<_>>()
    );
    let transaction_replay =
        list_ledger_transactions(&pool, LIMIT, Some(&transaction_cursor)).await?;
    assert_eq!(
        transaction_replay
            .data
            .iter()
            .map(|row| &row.hash)
            .collect::<Vec<_>>(),
        transaction_before
            .data
            .iter()
            .map(|row| &row.hash)
            .collect::<Vec<_>>()
    );
    let snapshot_replay =
        snapshot_page::<Value>(&pool, SCOPE, LIMIT, Some(&snapshot_cursor), None).await?;
    assert_eq!(snapshot_replay.data, snapshot_before.data);
    assert_eq!(
        anchor_replay.pagination.next_cursor,
        anchor_before.pagination.next_cursor
    );
    assert_eq!(
        transaction_replay.pagination.next_cursor,
        transaction_before.pagination.next_cursor
    );
    assert_eq!(
        snapshot_replay.pagination.next_cursor,
        snapshot_before.pagination.next_cursor
    );

    let mut anchor_ids: Vec<_> = anchor_first.data.into_iter().map(|row| row.id).collect();
    let mut transaction_ids: Vec<_> = transaction_first
        .data
        .into_iter()
        .map(|row| row.hash)
        .collect();
    let mut payloads = snapshot_first.data;
    let mut anchors_next = Some(anchor_cursor.clone());
    let mut transactions_next = Some(transaction_cursor.clone());
    let mut snapshots_next = Some(snapshot_cursor.clone());
    for _ in 0..ROWS {
        if let Some(cursor) = anchors_next.take() {
            let page = list_anchors::<AnchorRow>(&pool, LIMIT, Some(&cursor)).await?;
            assert_eq!(page.pagination.total, ROWS);
            assert_eq!(
                page.pagination.has_next,
                page.pagination.next_cursor.is_some()
            );
            anchor_ids.extend(page.data.into_iter().map(|row| row.id));
            anchors_next = page.pagination.next_cursor;
        }
        if let Some(cursor) = transactions_next.take() {
            let page = list_ledger_transactions(&pool, LIMIT, Some(&cursor)).await?;
            assert_eq!(page.pagination.total, ROWS);
            assert_eq!(
                page.pagination.has_next,
                page.pagination.next_cursor.is_some()
            );
            transaction_ids.extend(page.data.into_iter().map(|row| row.hash));
            transactions_next = page.pagination.next_cursor;
        }
        if let Some(cursor) = snapshots_next.take() {
            let page = snapshot_page::<Value>(&pool, SCOPE, LIMIT, Some(&cursor), None).await?;
            assert_eq!(page.pagination.total, ROWS);
            assert_eq!(
                page.pagination.has_next,
                page.pagination.next_cursor.is_some()
            );
            payloads.extend(page.data);
            snapshots_next = page.pagination.next_cursor;
        }
    }
    assert!(anchors_next.is_none() && transactions_next.is_none() && snapshots_next.is_none());
    assert_eq!(
        anchor_ids,
        (0..ROWS)
            .map(|index| format!("anchor-{index}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        transaction_ids,
        (0..ROWS)
            .map(|index| format!("transaction-{index}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(payloads, original_payloads);
    assert_eq!(
        list_anchors::<AnchorRow>(&pool, 100, None)
            .await?
            .pagination
            .total,
        ROWS + 2
    );
    assert_eq!(
        list_ledger_transactions(&pool, 100, None)
            .await?
            .pagination
            .total,
        ROWS + 2
    );

    assert!(matches!(
        list_anchors::<AnchorRow>(&pool, LIMIT, Some(&transaction_cursor)).await,
        Err(PaginationError::InvalidCursor(_))
    ));
    assert!(matches!(
        list_ledger_transactions(&pool, LIMIT, Some("!!!")).await,
        Err(PaginationError::InvalidCursor(_))
    ));
    assert!(matches!(
        snapshot_page::<Value>(
            &pool,
            "corridors:asset=XLM",
            LIMIT,
            Some(&snapshot_cursor),
            None
        )
        .await,
        Err(PaginationError::InvalidCursor(_))
    ));
    pool.close().await;
    Ok(())
}
