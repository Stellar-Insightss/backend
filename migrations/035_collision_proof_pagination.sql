-- Stable list identities are separate from mutable metrics and caller-facing IDs.
-- SQLite and RFC3339 creation timestamps are normalized to integer milliseconds.
-- Legacy NULL or unparseable timestamps sort at epoch zero. Updating an entity's
-- metrics or creation-time text never moves an already issued pagination key.
CREATE TABLE IF NOT EXISTS pagination_keys (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    resource TEXT NOT NULL,
    row_id TEXT NOT NULL,
    timestamp_ms INTEGER NOT NULL,
    UNIQUE (resource, row_id)
);

CREATE INDEX IF NOT EXISTS idx_pagination_keys_seek
    ON pagination_keys(resource, timestamp_ms, sequence);

INSERT OR IGNORE INTO pagination_keys(resource, row_id, timestamp_ms)
SELECT 'anchors', id,
       COALESCE(CAST(strftime('%s', created_at) AS INTEGER) * 1000
                + CAST(substr(strftime('%f', created_at), 4, 3) AS INTEGER), 0)
FROM anchors
ORDER BY COALESCE(CAST(strftime('%s', created_at) AS INTEGER) * 1000
                  + CAST(substr(strftime('%f', created_at), 4, 3) AS INTEGER), 0),
         rowid;

INSERT OR IGNORE INTO pagination_keys(resource, row_id, timestamp_ms)
SELECT 'transactions', hash,
       COALESCE(CAST(strftime('%s', created_at) AS INTEGER) * 1000
                + CAST(substr(strftime('%f', created_at), 4, 3) AS INTEGER), 0)
FROM transactions
ORDER BY COALESCE(CAST(strftime('%s', created_at) AS INTEGER) * 1000
                  + CAST(substr(strftime('%f', created_at), 4, 3) AS INTEGER), 0),
         rowid;

CREATE TRIGGER IF NOT EXISTS pagination_anchor_insert
AFTER INSERT ON anchors
BEGIN
    INSERT INTO pagination_keys(resource, row_id, timestamp_ms)
    VALUES ('anchors', NEW.id,
            COALESCE(CAST(strftime('%s', NEW.created_at) AS INTEGER) * 1000
                     + CAST(substr(strftime('%f', NEW.created_at), 4, 3) AS INTEGER), 0));
END;

CREATE TRIGGER IF NOT EXISTS pagination_anchor_delete
AFTER DELETE ON anchors
BEGIN
    DELETE FROM pagination_keys WHERE resource = 'anchors' AND row_id = OLD.id;
END;

CREATE TRIGGER IF NOT EXISTS pagination_transaction_insert
AFTER INSERT ON transactions
BEGIN
    INSERT INTO pagination_keys(resource, row_id, timestamp_ms)
    VALUES ('transactions', NEW.hash,
            COALESCE(CAST(strftime('%s', NEW.created_at) AS INTEGER) * 1000
                     + CAST(substr(strftime('%f', NEW.created_at), 4, 3) AS INTEGER), 0));
END;

CREATE TRIGGER IF NOT EXISTS pagination_transaction_delete
AFTER DELETE ON transactions
BEGIN
    DELETE FROM pagination_keys WHERE resource = 'transactions' AND row_id = OLD.hash;
END;

-- A remote corridor read has no durable database creation order. Store its
-- filtered rows once, then seek through that same snapshot until it expires.
CREATE TABLE IF NOT EXISTS pagination_snapshots (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pagination_snapshots_expiration
    ON pagination_snapshots(expires_at_ms);

CREATE TABLE IF NOT EXISTS pagination_snapshot_rows (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_id TEXT NOT NULL REFERENCES pagination_snapshots(id) ON DELETE CASCADE,
    timestamp_ms INTEGER NOT NULL,
    row_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    UNIQUE (snapshot_id, row_id)
);

CREATE INDEX IF NOT EXISTS idx_pagination_snapshot_rows_seek
    ON pagination_snapshot_rows(snapshot_id, timestamp_ms, sequence);
