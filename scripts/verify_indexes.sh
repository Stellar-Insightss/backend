#!/bin/bash
# Script to verify performance indexes are created correctly
# Usage: ./scripts/verify_indexes.sh

set -e

DB_FILE="${DATABASE_URL:-sqlite:./stellar_analysis.db}"
DB_PATH="${DB_FILE#sqlite:}"

echo "=========================================="
echo "Performance Index Verification"
echo "=========================================="
echo "Database: $DB_PATH"
echo ""

if [ ! -f "$DB_PATH" ]; then
    echo "❌ Database file not found: $DB_PATH"
    echo "Please run migrations first: sqlx migrate run"
    exit 1
fi

echo "✅ Database file found"
echo ""

# Function to check if index exists
check_index() {
    local index_name=$1
    local table_name=$2
    
    result=$(sqlite3 "$DB_PATH" "SELECT name FROM sqlite_master WHERE type='index' AND name='$index_name';")
    
    if [ -n "$result" ]; then
        echo "✅ $index_name (on $table_name)"
        return 0
    else
        echo "❌ $index_name (on $table_name) - MISSING"
        return 1
    fi
}

echo "Checking Payments Table Indexes:"
echo "-----------------------------------"
check_index "idx_payments_created_at" "payments"
check_index "idx_payments_asset_code" "payments"
check_index "idx_payments_source_account_date" "payments"
check_index "idx_payments_dest_account_date" "payments"
check_index "idx_payments_transaction_hash" "payments"
check_index "idx_payments_source_account" "payments"
check_index "idx_payments_destination_account" "payments"
echo ""

echo "Checking Assets Table Indexes:"
echo "-----------------------------------"
check_index "idx_assets_asset_code" "assets"
check_index "idx_assets_asset_issuer" "assets"
check_index "idx_assets_code_issuer" "assets"
check_index "idx_assets_anchor_id_code" "assets"
check_index "idx_assets_anchor" "assets"
echo ""

echo "Checking Corridors Table Indexes:"
echo "-----------------------------------"
check_index "idx_corridors_source_dest" "corridors"
check_index "idx_corridors_dest_source" "corridors"
check_index "idx_corridors_reliability_score" "corridors"
check_index "idx_corridors_status" "corridors"
check_index "idx_corridors_reliability" "corridors"
echo ""

echo "Checking Corridor Metrics Indexes:"
echo "-----------------------------------"
check_index "idx_corridor_metrics_key_date" "corridor_metrics"
check_index "idx_corridor_metrics_date" "corridor_metrics"
echo ""

echo "Checking Anchor Indexes:"
echo "-----------------------------------"
check_index "idx_anchors_stellar_account" "anchors"
check_index "idx_anchors_reliability_score" "anchors"
check_index "idx_anchors_status" "anchors"
echo ""

echo "Checking Anchor Metrics History Indexes:"
echo "-----------------------------------"
check_index "idx_anchor_metrics_anchor_timestamp" "anchor_metrics_history"
check_index "idx_anchor_metrics_anchor_time" "anchor_metrics_history"
check_index "idx_anchor_metrics_timestamp" "anchor_metrics_history"
echo ""

echo "Checking Network Daily Metrics Indexes:"
echo "-----------------------------------"
check_index "idx_network_daily_metrics_date" "network_daily_metrics"
echo ""

echo "Checking Soroban Daily Metrics Indexes:"
echo "-----------------------------------"
check_index "idx_soroban_daily_metrics_date" "soroban_daily_metrics"
check_index "idx_soroban_daily_metrics_contract" "soroban_daily_metrics"
echo ""

echo "Checking Wallet Balance Snapshots Indexes:"
echo "-----------------------------------"
check_index "idx_wallet_balance_snapshots_unique" "wallet_balance_snapshots"
check_index "idx_wallet_balance_snapshots_address_time" "wallet_balance_snapshots"
echo ""

echo "Checking Daily Stat Snapshots Indexes:"
echo "-----------------------------------"
check_index "idx_daily_stat_snapshots_date" "daily_stat_snapshots"
echo ""

echo "=========================================="
echo "Query Plan Verification"
echo "=========================================="
echo ""

echo "Testing: Payment time-range query"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM payments WHERE created_at > datetime('now', '-1 day');" | grep -i "index\|scan"
echo ""

echo "Testing: Payment by source account"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM payments WHERE source_account = 'GXXXXXX';" | grep -i "index\|scan"
echo ""

echo "Testing: Corridor lookup"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM corridors WHERE source_asset_code = 'USDC' AND destination_asset_code = 'XLM';" | grep -i "index\|scan"
echo ""

echo "Testing: Asset by code"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM assets WHERE asset_code = 'USDC';" | grep -i "index\|scan"
echo ""

echo "Testing: Network daily metrics time-range query (#15-#19)"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM network_daily_metrics WHERE date >= date('now', '-30 days') ORDER BY date DESC;" | grep -i "index\|scan"
echo ""

echo "Testing: Soroban daily metrics by contract (#21-#24)"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM soroban_daily_metrics WHERE contract_id = 'CXXXXXX' AND date >= date('now', '-30 days') ORDER BY date DESC;" | grep -i "index\|scan"
echo ""

echo "Testing: Wallet balance history time-series query (#26)"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM wallet_balance_snapshots WHERE address = 'GXXXXXX' ORDER BY snapshot_at DESC;" | grep -i "index\|scan"
echo ""

echo "Testing: Daily stat snapshot vs-yesterday delta (#3)"
echo "-----------------------------------"
sqlite3 "$DB_PATH" "EXPLAIN QUERY PLAN SELECT * FROM daily_stat_snapshots WHERE date >= date('now', '-7 days') ORDER BY date DESC;" | grep -i "index\|scan"
echo ""

echo "=========================================="
echo "Index Statistics"
echo "=========================================="
echo ""

echo "Total indexes per table:"
sqlite3 "$DB_PATH" "
SELECT 
    tbl_name as table_name,
    COUNT(*) as index_count
FROM sqlite_master 
WHERE type='index' AND tbl_name IN ('payments', 'assets', 'corridors', 'corridor_metrics', 'anchors', 'anchor_metrics_history', 'network_daily_metrics', 'soroban_daily_metrics', 'wallet_balance_snapshots', 'daily_stat_snapshots')
GROUP BY tbl_name
ORDER BY tbl_name;
"
echo ""

echo "=========================================="
echo "Table Row Counts"
echo "=========================================="
echo ""

echo "Payments: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM payments;")"
echo "Assets: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM assets;")"
echo "Corridors: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM corridors;")"
echo "Anchors: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM anchors;")"
echo "Network Daily Metrics: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM network_daily_metrics;")"
echo "Soroban Daily Metrics: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM soroban_daily_metrics;")"
echo "Wallet Balance Snapshots: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM wallet_balance_snapshots;")"
echo "Daily Stat Snapshots: $(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM daily_stat_snapshots;")"
echo ""

echo "=========================================="
echo "Verification Complete!"
echo "=========================================="
echo ""
echo "Next steps:"
echo "1. If any indexes are missing, run: sqlx migrate run"
echo "2. Monitor query performance in application logs"
echo "3. Run load tests to verify improvements"
echo "4. Check for 'USING INDEX' in query plans above"
echo ""
