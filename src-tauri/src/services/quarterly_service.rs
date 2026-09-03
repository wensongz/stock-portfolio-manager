use crate::db::Database;
use crate::models::quarterly::{
    CategoryComparison, ComparisonOverview, HoldingChangeItem, HoldingChanges, HoldingNoteHistory,
    MarketComparison, QuarterComparison, QuarterlyHoldingSnapshot, QuarterlySnapshot,
    QuarterlySnapshotDetail, QuarterlyTrends, StockTransactionGroup,
};
use crate::models::transaction::Transaction;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::{QuoteCache, QuoteServiceState};
use chrono::{Datelike, NaiveDate, Utc};
use std::collections::{HashMap, HashSet};

#[path = "quarterly/comparison.rs"]
mod comparison;
#[path = "quarterly/dates.rs"]
mod dates;
#[path = "quarterly/notes.rs"]
mod notes;
#[path = "quarterly/rebuild.rs"]
mod rebuild;
#[path = "quarterly/transactions.rs"]
mod transactions;
#[path = "quarterly/trends.rs"]
mod trends;

pub use comparison::compare_quarters;
use comparison::load_holdings_for_quarter_from_snapshot;
pub use dates::{
    date_to_quarter, parse_quarter, previous_quarter, quarter_end_date, quarter_start_date,
};
pub use notes::{get_holding_notes_history, update_holding_notes, update_quarterly_notes};
pub use transactions::get_quarterly_transactions;
pub use trends::get_quarterly_trends;

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Create a quarterly snapshot. `quarter` defaults to the current quarter if None.
pub async fn create_quarterly_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    quarter: Option<String>,
) -> Result<QuarterlySnapshot, String> {
    let quarter = quarter.unwrap_or_else(|| date_to_quarter(Utc::now().date_naive()));
    rebuild::rebuild_quarterly_snapshot(db, cache, quote_cache, quote_state, &quarter, None).await
}

/// Get all quarterly snapshots ordered by quarter descending.
pub fn get_quarterly_snapshots(db: &Database) -> Result<Vec<QuarterlySnapshot>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT qs.id, qs.quarter, qs.snapshot_date, qs.total_value, qs.total_cost, qs.total_pnl,
                    qs.us_value, qs.us_cost, qs.cn_value, qs.cn_cost, qs.hk_value, qs.hk_cost,
                    qs.exchange_rates, qs.overall_notes, qs.created_at,
                    COUNT(DISTINCT qhs.symbol) AS holding_count
             FROM quarterly_snapshots qs
             LEFT JOIN quarterly_holding_snapshots qhs ON qhs.quarterly_snapshot_id = qs.id
             GROUP BY qs.id
             ORDER BY qs.quarter DESC",
        )
        .map_err(|e| e.to_string())?;

    let snapshots = stmt
        .query_map([], |row| {
            Ok(QuarterlySnapshot {
                id: row.get(0)?,
                quarter: row.get(1)?,
                snapshot_date: row.get(2)?,
                total_value: row.get(3)?,
                total_cost: row.get(4)?,
                total_pnl: row.get(5)?,
                us_value: row.get(6)?,
                us_cost: row.get(7)?,
                cn_value: row.get(8)?,
                cn_cost: row.get(9)?,
                hk_value: row.get(10)?,
                hk_cost: row.get(11)?,
                exchange_rates: row.get(12)?,
                overall_notes: row.get(13)?,
                created_at: row.get(14)?,
                holding_count: row.get::<_, i64>(15)? as usize,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(snapshots)
}

/// Get detailed snapshot with holding list.
pub fn get_quarterly_snapshot_detail(
    db: &Database,
    snapshot_id: &str,
) -> Result<QuarterlySnapshotDetail, String> {
    let (snapshot, holdings) = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;

        // Get snapshot header
        let snapshot = conn
            .query_row(
                "SELECT qs.id, qs.quarter, qs.snapshot_date, qs.total_value, qs.total_cost, qs.total_pnl,
                        qs.us_value, qs.us_cost, qs.cn_value, qs.cn_cost, qs.hk_value, qs.hk_cost,
                        qs.exchange_rates, qs.overall_notes, qs.created_at,
                        COUNT(DISTINCT qhs.symbol) AS holding_count
                 FROM quarterly_snapshots qs
                 LEFT JOIN quarterly_holding_snapshots qhs ON qhs.quarterly_snapshot_id = qs.id
                 WHERE qs.id = ?1
                 GROUP BY qs.id",
                rusqlite::params![snapshot_id],
                |row| {
                    Ok(QuarterlySnapshot {
                        id: row.get(0)?,
                        quarter: row.get(1)?,
                        snapshot_date: row.get(2)?,
                        total_value: row.get(3)?,
                        total_cost: row.get(4)?,
                        total_pnl: row.get(5)?,
                        us_value: row.get(6)?,
                        us_cost: row.get(7)?,
                        cn_value: row.get(8)?,
                        cn_cost: row.get(9)?,
                        hk_value: row.get(10)?,
                        hk_cost: row.get(11)?,
                        exchange_rates: row.get(12)?,
                        overall_notes: row.get(13)?,
                        created_at: row.get(14)?,
                        holding_count: row.get::<_, i64>(15)? as usize,
                    })
                },
            )
            .map_err(|e| format!("Snapshot not found: {}", e))?;

        // Get holdings
        let mut stmt = conn
            .prepare(
                "SELECT id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                        category_name, category_color, shares, avg_cost, close_price,
                        market_value, cost_value, pnl, pnl_percent, weight, notes
                 FROM quarterly_holding_snapshots
                 WHERE quarterly_snapshot_id = ?1
                 ORDER BY market, symbol",
            )
            .map_err(|e| e.to_string())?;

        let holdings = stmt
            .query_map(rusqlite::params![snapshot_id], |row| {
                Ok(QuarterlyHoldingSnapshot {
                    id: row.get(0)?,
                    quarterly_snapshot_id: row.get(1)?,
                    account_id: row.get(2)?,
                    account_name: row.get(3)?,
                    symbol: row.get(4)?,
                    name: row.get(5)?,
                    market: row.get(6)?,
                    category_name: row.get(7)?,
                    category_color: row.get(8)?,
                    shares: row.get(9)?,
                    avg_cost: row.get(10)?,
                    close_price: row.get(11)?,
                    market_value: row.get(12)?,
                    cost_value: row.get(13)?,
                    pnl: row.get(14)?,
                    pnl_percent: row.get(15)?,
                    weight: row.get(16)?,
                    notes: row.get(17)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        (snapshot, holdings)
    }; // conn lock released here

    // Compute holding changes: compare Q1 snapshot with (Q1 snapshot + Q2 transactions).
    // This avoids fragile full-history recalculation that depends on accurate backfill data.
    let prev_q = previous_quarter(&snapshot.quarter).ok();
    let holding_changes = prev_q.as_ref().and_then(|pq| {
        let prev_holdings = load_holdings_for_quarter_from_snapshot(db, pq).ok()?;
        let q_txns = get_quarterly_transactions(db, snapshot_id).ok()?;

        // Aggregate Q2 snapshot holdings by uppercase symbol for market values
        let mut q2_agg: HashMap<String, (f64, f64)> = HashMap::new();
        for h in &holdings {
            if h.symbol.starts_with("$CASH-") {
                continue;
            }
            let key = h.symbol.to_uppercase();
            q2_agg
                .entry(key)
                .and_modify(|(s, v)| {
                    *s += h.shares;
                    *v += h.market_value;
                })
                .or_insert((h.shares, h.market_value));
        }

        // Build Q2 positions: Q1 + Q2 net transactions per symbol
        // Aggregate txns by uppercase symbol
        let mut txn_net: HashMap<String, (f64, f64)> = HashMap::new();
        for g in &q_txns {
            let key = g.symbol.to_uppercase();
            let entry = txn_net.entry(key).or_default();
            entry.0 += g.total_buy_shares - g.total_sell_shares; // net shares change
            entry.1 += g.total_buy_amount - g.total_sell_amount; // net value change
        }

        // Aggregate Q1 holdings by uppercase symbol
        let mut q1_agg: HashMap<String, (f64, f64, String, String, String)> = HashMap::new();
        for h in &prev_holdings {
            if h.symbol.starts_with("$CASH-") {
                continue;
            }
            let key = h.symbol.to_uppercase();
            q1_agg
                .entry(key)
                .and_modify(|(s, v, _, _, _)| {
                    *s += h.shares;
                    *v += h.market_value;
                })
                .or_insert_with(|| {
                    (
                        h.shares,
                        h.market_value,
                        h.name.clone(),
                        h.market.clone(),
                        h.category_name.clone(),
                    )
                });
        }

        // Q2 = Q1 + txn_net
        // Look up market + category for symbols not in Q1 snapshot
        let new_sym_info: HashMap<String, (String, String, String, String)> = {
            let conn = db.conn.lock().ok()?;
            let mut map: HashMap<String, (String, String, String, String)> = HashMap::new();
            for sym_upper in txn_net.keys() {
                if q1_agg.contains_key(sym_upper) {
                    continue;
                }
                if let Ok((name, market, cat_name, cat_color)) = conn.query_row(
                    "SELECT h.name, h.market,
                            COALESCE(c.name, '未分类'),
                            COALESCE(c.color, '#8B8B8B')
                     FROM holdings h
                     LEFT JOIN categories c ON h.category_id = c.id
                     WHERE UPPER(h.symbol) = ?1
                     ORDER BY h.shares DESC
                     LIMIT 1",
                    rusqlite::params![sym_upper],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                ) {
                    map.insert(sym_upper.clone(), (name, market, cat_name, cat_color));
                }
            }
            map
        };

        // Merge all symbols from Q1 and txn_net
        let mut all_syms: HashSet<String> = HashSet::new();
        for k in q1_agg.keys() {
            all_syms.insert(k.clone());
        }
        for k in txn_net.keys() {
            all_syms.insert(k.clone());
        }

        let mut new_holdings = Vec::new();
        let mut closed_holdings = Vec::new();
        let mut increased = Vec::new();
        let mut decreased = Vec::new();
        let mut unchanged = Vec::new();

        for sym_upper in &all_syms {
            let q1 = q1_agg.get(sym_upper);
            let net = txn_net.get(sym_upper).copied().unwrap_or((0.0, 0.0));

            let q1_shares = q1.map(|q| q.0).unwrap_or(0.0);
            let q1_value = q1.map(|q| q.1).unwrap_or(0.0);
            let q2_shares = q1_shares + net.0;
            let q2_value = q2_agg
                .get(sym_upper)
                .map(|q| q.1)
                .unwrap_or_else(|| q1_value + net.1);

            if q2_shares <= 0.0 {
                // Fully closed in Q2: was in Q1, now zero
                if let Some(q1_item) = q1 {
                    closed_holdings.push(HoldingChangeItem {
                        symbol: sym_upper.clone(),
                        name: q1_item.2.clone(),
                        market: q1_item.3.clone(),
                        category_name: q1_item.4.clone(),
                        q1_shares: Some(q1_shares),
                        q2_shares: None,
                        q1_value: Some(q1_value),
                        q2_value: None,
                        shares_change: -q1_shares,
                        value_change: -q1_value,
                    });
                }
                continue;
            }

            let name = q1
                .map(|q| q.2.clone())
                .or_else(|| new_sym_info.get(sym_upper).map(|i| i.0.clone()))
                .unwrap_or_else(|| sym_upper.clone());
            let market = q1
                .map(|q| q.3.clone())
                .or_else(|| new_sym_info.get(sym_upper).map(|i| i.1.clone()))
                .unwrap_or_else(|| "US".to_string());
            let cat = q1
                .map(|q| q.4.clone())
                .or_else(|| new_sym_info.get(sym_upper).map(|i| i.2.clone()))
                .unwrap_or_else(|| "未分类".to_string());

            let item = HoldingChangeItem {
                symbol: sym_upper.clone(),
                name,
                market,
                category_name: cat,
                q1_shares: if q1.is_some() { Some(q1_shares) } else { None },
                q2_shares: Some(q2_shares),
                q1_value: if q1.is_some() { Some(q1_value) } else { None },
                q2_value: Some(q2_value),
                shares_change: net.0,
                value_change: if q1.is_some() {
                    q2_value - q1_value
                } else {
                    q2_value
                },
            };

            if q1.is_none() {
                new_holdings.push(item);
            } else if net.0 > 1e-9 {
                increased.push(item);
            } else if net.0 < -1e-9 {
                decreased.push(item);
            } else {
                unchanged.push(item);
            }
        }

        // Sort CN → HK → US, then symbol
        fn market_order(m: &str) -> u8 {
            match m {
                "CN" => 1,
                "HK" => 2,
                _ => 3,
            }
        }
        let sort_list = |list: &mut Vec<HoldingChangeItem>| {
            list.sort_by(|a, b| {
                market_order(&a.market)
                    .cmp(&market_order(&b.market))
                    .then_with(|| a.symbol.cmp(&b.symbol))
            });
        };
        sort_list(&mut new_holdings);
        sort_list(&mut closed_holdings);
        sort_list(&mut increased);
        sort_list(&mut decreased);
        sort_list(&mut unchanged);

        Some(HoldingChanges {
            new_holdings,
            closed_holdings,
            increased,
            decreased,
            unchanged,
        })
    });
    let previous_quarter = if holding_changes.is_some() {
        prev_q
    } else {
        None
    };

    Ok(QuarterlySnapshotDetail {
        snapshot,
        holdings,
        holding_changes,
        previous_quarter,
    })
}

/// Delete a quarterly snapshot and its holding details.
pub fn delete_quarterly_snapshot(db: &Database, snapshot_id: &str) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM quarterly_holding_snapshots WHERE quarterly_snapshot_id = ?1",
        rusqlite::params![snapshot_id],
    )
    .map_err(|e| e.to_string())?;
    let rows = conn
        .execute(
            "DELETE FROM quarterly_snapshots WHERE id = ?1",
            rusqlite::params![snapshot_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}

/// Rebuild a quarterly snapshot through the same canonical path used by creation.
pub async fn refresh_quarterly_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    snapshot_id: &str,
) -> Result<QuarterlySnapshotDetail, String> {
    let quarter = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        conn.query_row(
            "SELECT quarter FROM quarterly_snapshots WHERE id = ?1",
            rusqlite::params![snapshot_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Snapshot not found: {error}"))?
    };
    rebuild::rebuild_quarterly_snapshot(
        db,
        cache,
        quote_cache,
        quote_state,
        &quarter,
        Some(snapshot_id),
    )
    .await?;
    get_quarterly_snapshot_detail(db, snapshot_id)
}

/// Find quarters that have no snapshot, from the first transaction quarter to the current quarter.
pub fn check_missing_snapshots(db: &Database) -> Result<Vec<String>, String> {
    // Find the earliest transaction date
    let earliest: Option<String> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row("SELECT MIN(traded_at) FROM transactions", [], |row| {
            row.get::<_, Option<String>>(0)
        })
        .map_err(|error| error.to_string())?
    };

    let Some(earliest_str) = earliest else {
        return Ok(vec![]);
    };

    let date_part = earliest_str
        .get(..10)
        .ok_or_else(|| format!("Bad transaction date: {earliest_str}"))?;
    let earliest_date = NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
        .map_err(|e| format!("Bad transaction date '{earliest_str}': {e}"))?;
    let today = Utc::now().date_naive();

    // Collect all quarters from earliest to current
    let mut all_quarters: Vec<String> = Vec::new();
    let mut year = earliest_date.year();
    let mut q = (earliest_date.month() - 1) / 3 + 1;
    let current_q = (today.month() - 1) / 3 + 1;
    let current_year = today.year();

    loop {
        all_quarters.push(format!("{}-Q{}", year, q));
        if year == current_year && q == current_q {
            break;
        }
        q += 1;
        if q > 4 {
            q = 1;
            year += 1;
        }
    }

    // Get existing snapshot quarters
    let existing: std::collections::HashSet<String> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT quarter FROM quarterly_snapshots")
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        mapped
            .collect::<Result<std::collections::HashSet<String>, _>>()
            .map_err(|e| e.to_string())?
    };

    let missing: Vec<String> = all_quarters
        .into_iter()
        .filter(|q| !existing.contains(q))
        .collect();

    Ok(missing)
}

/// Check if the current quarter already has a snapshot. If not, create one and return it.
/// Returns `None` when a snapshot for the current quarter already exists.
pub async fn ensure_current_quarter_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
) -> Result<Option<QuarterlySnapshot>, String> {
    let today = Utc::now().date_naive();
    let current_quarter = date_to_quarter(today);

    // Check whether a snapshot already exists for the current quarter
    let exists: bool = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT COUNT(*) FROM quarterly_snapshots WHERE quarter = ?1",
            rusqlite::params![current_quarter],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|error| error.to_string())?
    };

    if exists {
        return Ok(None);
    }

    // No snapshot yet for this quarter — create one automatically
    let snapshot =
        create_quarterly_snapshot(db, cache, quote_cache, quote_state, Some(current_quarter))
            .await?;
    Ok(Some(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_helpers_cover_calendar_boundaries() {
        assert_eq!(
            date_to_quarter(NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()),
            "2025-Q1"
        );
        assert_eq!(
            date_to_quarter(NaiveDate::from_ymd_opt(2025, 4, 1).unwrap()),
            "2025-Q2"
        );
        assert_eq!(parse_quarter("2025-Q4").unwrap(), (2025, 4));
        assert_eq!(
            quarter_start_date(2025, 3),
            NaiveDate::from_ymd_opt(2025, 7, 1).unwrap()
        );
        assert_eq!(
            quarter_end_date(2024, 1),
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap()
        );
    }

    #[test]
    fn test_previous_quarter() {
        assert_eq!(previous_quarter("2025-Q2").unwrap(), "2025-Q1");
        assert_eq!(previous_quarter("2025-Q3").unwrap(), "2025-Q2");
        assert_eq!(previous_quarter("2025-Q4").unwrap(), "2025-Q3");
        assert_eq!(previous_quarter("2025-Q1").unwrap(), "2024-Q4");
        assert_eq!(previous_quarter("2000-Q1").unwrap(), "1999-Q4");
    }

    #[test]
    fn test_previous_quarter_invalid() {
        assert!(previous_quarter("invalid").is_err());
        assert!(previous_quarter("2025-Q5").is_err());
        assert!(previous_quarter("2025-Q0").is_err());
    }

    #[test]
    fn missing_snapshot_scan_propagates_malformed_transaction_dates() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES ('acct', 'Account', 'US', '2025-01-01', '2025-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transactions
             (id, account_id, symbol, name, market, transaction_type, shares, price,
              total_amount, commission, currency, traded_at, created_at)
             VALUES ('tx', 'acct', 'AAPL', 'Apple', 'US', 'OPEN', 1, 1, 1, 0,
                     'USD', X'FF', '2025-01-01')",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(check_missing_snapshots(&db).is_err());
    }
}
