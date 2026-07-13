<div align="center">

# ⚙️ Stellar Insights — Backend

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
- [Vault](https://www.vaultproject.io) for secrets in production (see `docs/SECRETS_MANAGEMENT.md` in the [core repo](https://github.com/Stellar-Insightss/Stellar-inights))

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
docker build -t stellar-insights-backend .
docker run --env-file .env -p 8080:8080 stellar-insights-backend
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

## Observability

- **Logs** — set `LOGSTASH_ENABLED=true` to forward structured logs to an ELK stack (see `docker-compose.elk.yml` in the core repo)
- **Traces** — set `OTEL_ENABLED=true` to export traces to Jaeger (`docker-compose.jaeger.yml`)
- **API docs** — OpenAPI schema generated via `utoipa` (`src/openapi.rs`)

## Related repos

- [contracts](https://github.com/Stellar-Insightss/contracts) — Soroban contracts this backend indexes
- [frontend](https://github.com/Stellar-Insightss/Stellar-inights/tree/main/frontend) — dashboard consuming this API
- [mobile](https://github.com/Stellar-Insightss/mobile) — mobile client
