<div align="center">

# ⚙️ Stellar Analysis — Backend

**Rust analytics engine for real-time Stellar payment reliability.**

[![Rust](https://img.shields.io/badge/Rust-Axum-DE3F24?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![PostgreSQL](https://img.shields.io/badge/DB-PostgreSQL%20%2F%20SQLite-4169E1?logo=postgresql&logoColor=white)](https://www.postgresql.org)
[![OpenTelemetry](https://img.shields.io/badge/Observability-OpenTelemetry-425CC7?logo=opentelemetry&logoColor=white)](https://opentelemetry.io)

</div>

---

## What it does

Ingests Stellar network activity (via RPC/Horizon), computes corridor and anchor reliability metrics, and serves them over REST, GraphQL, and WebSockets — with caching, rate limiting, alerting, and full observability built in.

## Prerequisites

- Rust (stable)
- PostgreSQL (production) or SQLite (development, default)
- Redis (caching, rate limiting)
- [Vault](https://www.vaultproject.io) for secrets in production (see `docs/SECRETS_MANAGEMENT.md` in the [core repo](https://github.com/Stellar-Analysis/frontend))

## Setup

1. Copy the environment template and fill in required values:

   ```bash
   cp .env.example .env
   ```

   At minimum, set `JWT_SECRET`, `ENCRYPTION_KEY`, and `SEP10_SERVER_PUBLIC_KEY` — the server refuses to start with placeholder values.

2. Run database migrations:

   ```bash
   ./scripts/migrate.sh
   ```

3. Start the server:

   ```bash
   cargo run
   ```

   The server listens on `SERVER_HOST:SERVER_PORT` (default `127.0.0.1:8080`).

### Docker

```bash
docker build -t stellar-analysis-backend .
docker run --env-file .env -p 8080:8080 stellar-analysis-backend
```

The container entrypoint (`entrypoint.sh`) runs pending migrations before starting the server.

## Project layout

| Path | Contents |
|---|---|
| `src/api/` | REST endpoint handlers (anchors, corridors, alerts, auth, achievements, ...) |
| `src/graphql/` | GraphQL schema and resolvers |
| `src/rpc/` | Stellar RPC/Horizon client, rate limiting, circuit breaker |
| `src/ingestion/` | Network data ingestion pipelines |
| `src/auth/`, `auth_middleware.rs` | SEP-10 Stellar auth and JWT session handling |
| `src/cache/` | Redis-backed caching and invalidation |
| `src/jobs/` | Background jobs (corridor/anchor refresh, price feed, cache cleanup) |
| `src/observability/` | OpenTelemetry tracing, health checks |
| `src/logging/` | Structured (JSON) logging, ELK/Logstash forwarding |
| `src/webhooks/`, `src/telegram/` | Outbound alert delivery |
| `src/vault/` | Vault-backed secrets integration |
| `migrations/` | SQL migrations (32 to date) |
| `scripts/` | Migration, backup, and smoke-test scripts |

## Testing

```bash
cargo test
```

Integration/load tests live in `tests/` and `load-tests/`.

## Outbound webhooks

An authenticated `POST /api/webhooks` (or `POST /api/v1/webhooks`) returns the
subscription fields plus its signing `secret` once, with `Cache-Control:
no-store`. Save this secret in the receiving integration when you register.
List, detail, and delivery-status responses do not return it.

Each queued event has a persistent UUID. Its serialized JSON envelope (`id`,
`event`, `timestamp`, `data`) is saved before delivery and reused byte for byte
on retries, including after a dispatcher restart. Consumers should deduplicate
using `X-Zapier-Delivery-ID`, which is the same value as the envelope's `id` and
the `event_id` returned by `POST /api/webhooks/{id}/test`.

Requests also include `X-Zapier-Event`, `X-Zapier-Timestamp` (the event creation
time in Unix seconds), and `X-Zapier-Signature`. The signature is
`sha256=<hex HMAC-SHA256>` over the exact raw request body, keyed with that
subscription's secret. Verify the raw bytes before parsing JSON or acting on
an event. Re-serializing JSON before checking the signature can change the
signed bytes. The creation timestamp remains stable across retries; use the
event ID to deduplicate an already accepted event.

Any non-2xx response or transport failure counts as a failed attempt. Redirects
are treated as failures without following another URL, and failure records do
not include the receiver's response body. Delivery becomes terminal after
**three recorded failures**, including the initial request, with a ten-second
timeout on each request. Retry due times are stored
in SQLite: the delays after the first and second failures are five and ten
seconds. The exponential delay is capped at 300 seconds if the attempt limit
is increased. The dispatcher polls every five seconds, so actual delivery can
occur later than the due time. The third failure becomes terminal `failed`.
Inactive or missing subscriptions also become terminal without another send.

A thirty-second reservation prevents another worker from immediately sending
the same due event and makes events recoverable after a worker stops. Delivery
is at least once: an accepted HTTP request whose database update fails can be
sent again with the same ID and exact body. Crashes and failed database writes
can therefore cause more physical sends than the recorded failure count.
Reservation tokens prevent a resumed, expired worker from overwriting a newer
worker's result. Successful delivery and the
subscription's `last_fired_at` are recorded in one transaction; database
failures are reported by the dispatcher instead of reported as success.

Authenticated subscribers can query
`GET /api/webhooks/{webhook_id}/events/{event_id}` for `status` (`pending`,
`delivered`, or `failed`), `retries` (failed attempts, retained after success),
`last_error`, `created_at`, `next_attempt_at`, `last_attempt_at`, and
`delivered_at`. The latter three timestamps use Unix seconds and are nullable;
while a worker holds an event, `next_attempt_at` is its reservation expiry.
Only the subscription's owner can access the record. Secrets and event data
are excluded from this response.
The same resource is available at
`GET /api/v1/webhooks/{webhook_id}/events/{event_id}` with a valid access-token
`Authorization: Bearer ...` header.

Migration `035_webhook_delivery_retries.sql` preserves existing raw payloads
and recorded errors, marks previously stranded exhausted events as `failed`,
and freezes legacy envelopes from their stored ID and creation time on their
next delivery. Attempts sent before this migration used a different ID and
cannot be retroactively deduplicated with their new, stable ID.

Run the focused dispatcher, signature, migration, and API tests with:

```bash
cargo test --test webhook_delivery_regression_tests --test webhook_event_service_tests
```

## Observability

- **Logs** — set `LOGSTASH_ENABLED=true` to forward structured logs to an ELK stack (see `docker-compose.elk.yml` in the core repo)
- **Traces** — set `OTEL_ENABLED=true` to export traces to Jaeger (`docker-compose.jaeger.yml`)
- **API docs** — OpenAPI schema generated via `utoipa` (`src/openapi.rs`)

## Related repos

- [contracts](https://github.com/Stellar-Analysis/contracts) — Soroban contracts this backend indexes
- [frontend](https://github.com/Stellar-Analysis/frontend/tree/main/frontend) — dashboard consuming this API
- [mobile](https://github.com/Stellar-Analysis/mobile) — mobile client
