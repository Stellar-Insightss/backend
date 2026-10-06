-- Keep the serialized delivery envelope and retry schedule across restarts.
-- Existing payloads remain unchanged; the dispatcher freezes legacy envelopes
-- using their stored event ID and creation time before their next delivery.
ALTER TABLE webhook_events ADD COLUMN delivery_payload TEXT;
ALTER TABLE webhook_events ADD COLUMN next_attempt_at INTEGER;
ALTER TABLE webhook_events ADD COLUMN last_attempt_at INTEGER;
ALTER TABLE webhook_events ADD COLUMN delivered_at INTEGER;
ALTER TABLE webhook_events ADD COLUMN delivery_token TEXT;

-- The old dispatcher stranded the third failure as pending, although its query
-- excluded retries >= 3. Preserve the recorded error and make those rows terminal.
UPDATE webhook_events
SET status = 'failed',
    last_error = COALESCE(last_error, 'maximum_attempts_exhausted')
WHERE status = 'pending' AND retries >= 3;

CREATE INDEX IF NOT EXISTS idx_webhook_events_due
ON webhook_events(status, next_attempt_at, created_at);
