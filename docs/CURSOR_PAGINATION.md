# Cursor pagination for live collections

The live list handlers use an immutable `(timestamp_ms, sequence)` key. The
sequence is an SQLite `INTEGER PRIMARY KEY AUTOINCREMENT`. Public anchor IDs,
ledger transaction hashes and emitted corridor IDs keep their existing values.

## Requests

The versioned routes are `/api/v1/anchors`, `/api/v1/corridors` and
`/api/v1/ledger/transactions`. The existing router also exposes the corresponding
unversioned routes. Start with `?limit=50` and continue with
`?limit=50&cursor=<pagination.next_cursor>`. Retain the same corridor filters.

The response keeps `data` and `pagination`. Pagination contains `limit`, the
initial `total`, the incoming `cursor`, `has_next`, and `next_cursor`. The next
cursor names the last returned row, not the lookahead row. A page reads at most
one extra row to establish continuation; exhaustion returns no next cursor.

Limits must be 1–100. Nonzero offsets return `400 INVALID_PAGINATION`; clients
must switch to returned cursors. Traversal orders timestamp and sequence
ascending. Anchor reliability and corridor metrics remain payload fields and
do not reorder traversal. The existing corridor `sort_by` parameter does not
replace that immutable ordering.

The new ledger collection reads the existing `transactions` ingestion table.
It does not list pending signing requests, XDR payloads, or signatures. Existing
ingestion can store placeholder transaction records; this endpoint does not
establish complete ledger decoding.

## Cursor encoding

The opaque value is unpadded URL-safe base64 of UTF-8 JSON:

```json
{"v":1,"scope":"anchors","ts":1767342245000,"id":42,"ceiling":999,"total":312,"snapshot":null}
```

`ts` and `id` form the strict seek boundary. `ceiling` excludes later inserts,
including backdated ones. `total` retains the first-page count, avoiding a full
count on every continuation. `scope` identifies the collection; corridor scopes
also include SHA-256 of the filter and sort parameters. `snapshot` identifies
a retained corridor result and is null for database collections.

Unknown fields/versions, malformed encoding or JSON, negative totals, invalid
sequence bounds, wrong scopes and values over 2048 encoded characters are
rejected. Cursors carry traversal state and do not authorize access. Clients
should reuse the returned opaque value rather than constructing it.

## Database ordering

Migration 035 backfills `pagination_keys` from anchors and ingested transactions.
It normalizes SQLite/RFC3339 timestamps to integer milliseconds; legacy null or
unparseable values sort at zero. Insert triggers allocate keys atomically with
source rows. Ordinary metric/timestamp updates preserve keys; delete/recreate
allocates a fresh sequence, keeping the replacement outside older ceilings.

Page one reads ceiling, count and rows in one transaction. Continuations use:

```sql
WHERE p.resource = ? AND p.sequence <= ?
  AND (p.timestamp_ms, p.sequence) > (?, ?)
ORDER BY p.timestamp_ms ASC, p.sequence ASC LIMIT ?
```

The matching index is `idx_pagination_keys_seek(resource,timestamp_ms,sequence)`.
Insert membership and ordering stay stable. Existing payloads may reflect
updates and deleted rows disappear; the initial total is retained. Historical
payload versions and deletion history are outside this guarantee.

## RPC corridor snapshots

The first-page cache miss retains the FULL filtered RPC result in one database
transaction. The existing aggregation, emitted asset-pair IDs and filters remain.
The snapshot is indexed by `(snapshot_id,timestamp_ms,sequence)`. Continuations
read its payloads directly, without fetching payments/trades, recomputing prices
or reevaluating filters against a moving RPC window.

Snapshots expire after 15 minutes. Expired rows are removed when a new snapshot
is stored. Missing/expired snapshots return `400 CURSOR_EXPIRED`; the handler
does not substitute a fresh RPC result. First-page caching is capped at 5 minutes,
and cache keys distinguish scope/cursor. Start a new traversal after expiration.

## Required checks

```sh
cargo test --locked --test batch_collision_test --test concurrent_write_stability_test
```

These two files exercise the production selectors with SQLite and actual
migrations 001/002/007/035: batch collisions, concurrent same-time/backdated
inserts, cursor replay and retained corridor results after a newer snapshot.
They cover selectors/storage, not HTTP composition, live-provider benchmarking,
or sponsor acceptance. Runtime execution status belongs in the PR receipt.
