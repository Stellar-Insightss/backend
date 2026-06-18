#!/usr/bin/env bash
# Backend smoke test
# Verifies: env file exists, required vars are set, cargo builds, health endpoint responds.
# Usage: bash backend/scripts/smoke-test.sh
# Exit 0 = pass, exit 1 = fail

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PASS=0
FAIL=0

ok()   { echo "[PASS] $*"; PASS=$((PASS+1)); }
fail() { echo "[FAIL] $*"; FAIL=$((FAIL+1)); }
info() { echo "[INFO] $*"; }

# ── 1. .env exists ────────────────────────────────────────────────────────────
if [[ -f "$BACKEND_DIR/.env" ]]; then
  ok ".env file exists"
else
  fail ".env not found — run: cp backend/.env.example backend/.env and fill in values"
fi

# ── 2. Required env vars are set and not placeholder ──────────────────────────
if [[ -f "$BACKEND_DIR/.env" ]]; then
  # shellcheck source=/dev/null
  set -a; source "$BACKEND_DIR/.env"; set +a
fi

check_var() {
  local var="$1"
  local val="${!var:-}"
  if [[ -z "$val" ]]; then
    fail "$var is not set"
  elif [[ "$val" == *"CHANGE_ME"* ]]; then
    fail "$var still contains placeholder value"
  else
    ok "$var is set"
  fi
}

check_var DATABASE_URL
check_var JWT_SECRET
check_var ENCRYPTION_KEY
check_var SEP10_SERVER_PUBLIC_KEY
check_var REDIS_URL

# ── 3. Rust toolchain present ─────────────────────────────────────────────────
if command -v cargo &>/dev/null; then
  ok "cargo found: $(cargo --version)"
else
  fail "cargo not found — install Rust from https://rustup.rs"
fi

# ── 4. Cargo build (debug, quiet) ─────────────────────────────────────────────
info "Running cargo check (this may take a moment on first run)…"
if cargo check --manifest-path "$BACKEND_DIR/Cargo.toml" --quiet 2>&1; then
  ok "cargo check passed"
else
  fail "cargo check failed — run 'cargo build' in backend/ for details"
fi

# ── 5. sqlx-cli present ───────────────────────────────────────────────────────
if command -v sqlx &>/dev/null; then
  ok "sqlx-cli found: $(sqlx --version 2>&1 | head -1)"
else
  fail "sqlx-cli not found — run: cargo install sqlx-cli --no-default-features --features postgres,sqlite"
fi

# ── 6. Health endpoint (if server is already running) ────────────────────────
HEALTH_URL="http://${SERVER_HOST:-127.0.0.1}:${SERVER_PORT:-8080}/health"
info "Probing $HEALTH_URL (skipped if server not running)…"
if command -v curl &>/dev/null; then
  HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" --max-time 3 "$HEALTH_URL" 2>/dev/null || echo "000")
  if [[ "$HTTP_STATUS" == "200" ]]; then
    ok "backend /health returned HTTP 200"
  elif [[ "$HTTP_STATUS" == "000" ]]; then
    info "/health unreachable — server not running (start with 'cargo run' to verify)"
  else
    fail "/health returned HTTP $HTTP_STATUS (expected 200)"
  fi
else
  info "curl not found — skipping live health check"
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "────────────────────────────────────────"
echo "Backend smoke test: $PASS passed, $FAIL failed"
echo "────────────────────────────────────────"

if [[ $FAIL -eq 0 ]]; then
  echo "[PASS] backend smoke test"
  exit 0
else
  echo "[FAIL] backend smoke test — fix the issues above and re-run"
  exit 1
fi
