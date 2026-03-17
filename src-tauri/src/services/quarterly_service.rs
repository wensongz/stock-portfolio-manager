use crate::db::Database;
use crate::models::quarterly::{
    CategoryComparison, ComparisonOverview, HoldingChangeItem, HoldingChanges, HoldingNoteHistory,
    MarketComparison, QuarterComparison, QuarterlyHoldingSnapshot, QuarterlyNotesSummary,
    QuarterlySnapshot, QuarterlySnapshotDetail, QuarterlyTrends,
};
use crate::services::exchange_rate_service::{convert_currency, get_cached_rates, ExchangeRateCache};
use crate::services::quote_service::{fetch_quotes_batch_cached_with_providers, QuoteCache};
use crate::services::quote_provider_service;
use chrono::{Datelike, NaiveDate, Utc};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Quarter helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the quarter string (e.g., "2025-Q1") for a given date.
pub fn date_to_quarter(date: NaiveDate) -> String {
    let q = (date.month() - 1) / 3 + 1;
    format!("{}-Q{}", date.year(), q)
}

/// Returns the last day of the quarter.
pub fn quarter_end_date(year: i32, q: u32) -> NaiveDate {
    match q {
        1 => NaiveDate::from_ymd_opt(year, 3, 31).unwrap(),
        2 => NaiveDate::from_ymd_opt(year, 6, 30).unwrap(),
        3 => NaiveDate::from_ymd_opt(year, 9, 30).unwrap(),
        4 => NaiveDate::from_ymd_opt(year, 12, 31).unwrap(),
        _ => unreachable!(),
    }
}

/// Parse a quarter string like "2025-Q1" into (year, quarter_number).
pub fn parse_quarter(s: &str) -> Result<(i32, u32), String> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid quarter format: '{}'", s));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| format!("Invalid year in quarter '{}'", s))?;
    let q_str = parts[1];
    if q_str.len() != 2 || !q_str.starts_with('Q') {
        return Err(format!("Invalid quarter part in '{}'", s));
    }
    let q: u32 = q_str[1..]
        .parse()
        .map_err(|_| format!("Invalid quarter number in '{}'", s))?;
    if !(1..=4).contains(&q) {
        return Err(format!("Quarter number must be 1-4, got {}", q));
    }
    Ok((year, q))
}

/// Returns the previous quarter string (e.g., "2025-Q1" -> "2024-Q4", "2025-Q3" -> "2025-Q2").
pub fn previous_quarter(s: &str) -> Result<String, String> {
    let (year, q) = parse_quarter(s)?;
    if q == 1 {
        Ok(format!("{}-Q4", year - 1))
    } else {
        Ok(format!("{}-Q{}", year, q - 1))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Create a quarterly snapshot. `quarter` defaults to the current quarter if None.
pub async fn create_quarterly_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quarter: Option<String>,
) -> Result<QuarterlySnapshot, String> {
    let today = Utc::now().date_naive();
    let quarter_str = quarter.unwrap_or_else(|| date_to_quarter(today));
    let (year, q) = parse_quarter(&quarter_str)?;
    let end_date = quarter_end_date(year, q);
    // Use today or quarter end, whichever is earlier, as snapshot date
    let snapshot_date = if today < end_date { today } else { end_date };
    let snapshot_date_str = snapshot_date.format("%Y-%m-%d").to_string();

    // Load all holdings
    struct HoldingRow {
        id: String,
        account_id: String,
        account_name: String,
        symbol: String,
        name: String,
        market: String,
        category_name: String,
        category_color: String,
        shares: f64,
        avg_cost: f64,
    }

    let rows: Vec<HoldingRow> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT h.id, h.account_id, COALESCE(a.name, '') AS account_name,
                        h.symbol, h.name, h.market,
                        COALESCE(c.name, '未分类') AS category_name,
                        COALESCE(c.color, '#8B8B8B') AS category_color,
                        h.shares, h.avg_cost
                 FROM holdings h
                 LEFT JOIN accounts a ON h.account_id = a.id
                 LEFT JOIN categories c ON h.category_id = c.id
                 WHERE h.shares > 0
                 ORDER BY h.market, h.symbol",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map([], |row| {
                Ok(HoldingRow {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    account_name: row.get(2)?,
                    symbol: row.get(3)?,
                    name: row.get(4)?,
                    market: row.get(5)?,
                    category_name: row.get(6)?,
                    category_color: row.get(7)?,
                    shares: row.get(8)?,
                    avg_cost: row.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    if rows.is_empty() {
        return Err("No holdings found to snapshot".to_string());
    }

    // Fetch prices: try to get close prices from daily_holding_snapshots around the snapshot date,
    // fall back to live quotes.
    let price_map = get_prices_for_date(db, quote_cache, &rows.iter().map(|r| r.symbol.clone()).collect(), snapshot_date).await?;

    // Fetch exchange rates
    let rates = get_cached_rates(cache).await.unwrap_or_else(|_| {
        crate::models::quote::ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: Utc::now().to_rfc3339(),
        }
    });

    let mut us_value = 0.0f64;
    let mut us_cost = 0.0f64;
    let mut cn_value = 0.0f64;
    let mut cn_cost = 0.0f64;
    let mut hk_value = 0.0f64;
    let mut hk_cost = 0.0f64;

    let mut holding_rows_for_insert: Vec<(HoldingRow, f64, f64, f64)> = Vec::new();

    for row in rows {
        let close_price = *price_map.get(&row.symbol).unwrap_or(&0.0);
        let market_value = row.shares * close_price;
        let cost_value = row.shares * row.avg_cost;

        match row.market.as_str() {
            "US" => {
                us_value += market_value;
                us_cost += cost_value;
            }
            "CN" => {
                cn_value += market_value;
                cn_cost += cost_value;
            }
            "HK" => {
                hk_value += market_value;
                hk_cost += cost_value;
            }
            _ => {}
        }
        holding_rows_for_insert.push((row, close_price, market_value, cost_value));
    }

    // Convert all to USD for totals
    let total_value = us_value
        + convert_currency(cn_value, "CNY", "USD", &rates)
        + convert_currency(hk_value, "HKD", "USD", &rates);
    let total_cost = us_cost
        + convert_currency(cn_cost, "CNY", "USD", &rates)
        + convert_currency(hk_cost, "HKD", "USD", &rates);
    let total_pnl = total_value - total_cost;

    let rates_json = serde_json::to_string(&rates).unwrap_or_default();
    let snapshot_id = uuid::Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let holding_count = holding_rows_for_insert.len();

    // Persist (upsert: delete existing snapshot for this quarter, then insert — wrapped in transaction)
    {
        let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        // Delete existing snapshot for this quarter
        let old_id: Option<String> = tx
            .query_row(
                "SELECT id FROM quarterly_snapshots WHERE quarter = ?1",
                rusqlite::params![quarter_str],
                |row| row.get(0),
            )
            .ok();
        if let Some(oid) = old_id {
            tx.execute(
                "DELETE FROM quarterly_holding_snapshots WHERE quarterly_snapshot_id = ?1",
                rusqlite::params![oid],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM quarterly_snapshots WHERE id = ?1",
                rusqlite::params![oid],
            )
            .map_err(|e| e.to_string())?;
        }

        // Insert new snapshot
        tx.execute(
            "INSERT INTO quarterly_snapshots
             (id, quarter, snapshot_date, total_value, total_cost, total_pnl,
              us_value, us_cost, cn_value, cn_cost, hk_value, hk_cost,
              exchange_rates, overall_notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, ?14)",
            rusqlite::params![
                snapshot_id, quarter_str, snapshot_date_str,
                total_value, total_cost, total_pnl,
                us_value, us_cost, cn_value, cn_cost, hk_value, hk_cost,
                rates_json, created_at
            ],
        )
        .map_err(|e| e.to_string())?;

        // Insert holding snapshots
        for (row, close_price, market_value, cost_value) in &holding_rows_for_insert {
            let pnl = market_value - cost_value;
            let pnl_percent = if *cost_value != 0.0 {
                pnl / cost_value * 100.0
            } else {
                0.0
            };
            let weight = if total_value != 0.0 {
                let mv_usd = match row.market.as_str() {
                    "CN" => convert_currency(*market_value, "CNY", "USD", &rates),
                    "HK" => convert_currency(*market_value, "HKD", "USD", &rates),
                    _ => *market_value,
                };
                mv_usd / total_value * 100.0
            } else {
                0.0
            };
            let holding_snap_id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO quarterly_holding_snapshots
                 (id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                  category_name, category_color, shares, avg_cost, close_price, market_value,
                  cost_value, pnl, pnl_percent, weight, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, NULL)",
                rusqlite::params![
                    holding_snap_id, snapshot_id,
                    row.account_id, row.account_name,
                    row.symbol, row.name, row.market,
                    row.category_name, row.category_color,
                    row.shares, row.avg_cost, close_price, market_value,
                    cost_value, pnl, pnl_percent, weight
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(QuarterlySnapshot {
        id: snapshot_id,
        quarter: quarter_str,
        snapshot_date: snapshot_date_str,
        total_value,
        total_cost,
        total_pnl,
        us_value,
        us_cost,
        cn_value,
        cn_cost,
        hk_value,
        hk_cost,
        exchange_rates: rates_json,
        overall_notes: None,
        created_at,
        holding_count,
    })
}

/// Fetch prices for a list of symbols at or before the given date.
/// Falls back to live quotes if no historical data available.
async fn get_prices_for_date(
    db: &Database,
    quote_cache: &QuoteCache,
    symbols: &Vec<String>,
    date: NaiveDate,
) -> Result<HashMap<String, f64>, String> {
    let date_str = date.format("%Y-%m-%d").to_string();
    let mut price_map: HashMap<String, f64> = HashMap::new();

    // Try to get prices from daily_holding_snapshots at or before the date
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        for symbol in symbols {
            let price: Option<f64> = conn
                .query_row(
                    "SELECT close_price FROM daily_holding_snapshots
                     WHERE symbol = ?1 AND date <= ?2 AND close_price > 0
                     ORDER BY date DESC LIMIT 1",
                    rusqlite::params![symbol, date_str],
                    |row| row.get(0),
                )
                .ok();
            if let Some(p) = price {
                if p > 0.0 {
                    price_map.insert(symbol.clone(), p);
                }
            }
        }
    }

    // For symbols without historical prices, fetch live quotes
    let missing: Vec<String> = symbols
        .iter()
        .filter(|s| !price_map.contains_key(*s))
        .cloned()
        .collect();

    if !missing.is_empty() {
        // We need to know the market for each symbol - look it up
        let market_map: HashMap<String, String> = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            let mut result = HashMap::new();
            for symbol in &missing {
                let market: Option<String> = conn
                    .query_row(
                        "SELECT market FROM holdings WHERE symbol = ?1 LIMIT 1",
                        rusqlite::params![symbol],
                        |row| row.get(0),
                    )
                    .ok();
                if let Some(m) = market {
                    result.insert(symbol.clone(), m);
                }
            }
            result
        };
        let sym_market_pairs: Vec<(String, String)> = missing
            .iter()
            .filter_map(|s| {
                market_map
                    .get(s)
                    .map(|m| (s.clone(), m.clone()))
            })
            .collect();
        if !sym_market_pairs.is_empty() {
            let quotes = {
                let config = quote_provider_service::get_quote_provider_config(db)?;
                fetch_quotes_batch_cached_with_providers(quote_cache, sym_market_pairs, &config.us_provider, &config.hk_provider, true).await?
            };
            for q in quotes {
                price_map.insert(q.symbol, q.current_price);
            }
        }
    }

    Ok(price_map)
}

/// Get all quarterly snapshots ordered by quarter descending.
pub fn get_quarterly_snapshots(db: &Database) -> Result<Vec<QuarterlySnapshot>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT qs.id, qs.quarter, qs.snapshot_date, qs.total_value, qs.total_cost, qs.total_pnl,
                    qs.us_value, qs.us_cost, qs.cn_value, qs.cn_cost, qs.hk_value, qs.hk_cost,
                    qs.exchange_rates, qs.overall_notes, qs.created_at,
                    COUNT(qhs.id) AS holding_count
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
                        COUNT(qhs.id) AS holding_count
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

    // Compute holding changes vs previous quarter
    let prev_q = previous_quarter(&snapshot.quarter).ok();
    let holding_changes = prev_q.as_ref().and_then(|pq| {
        load_holdings_for_quarter(db, pq).ok().map(|prev_holdings| {
            compute_holding_changes(&prev_holdings, &holdings)
        })
    });
    let previous_quarter = if holding_changes.is_some() { prev_q } else { None };

    Ok(QuarterlySnapshotDetail { snapshot, holdings, holding_changes, previous_quarter })
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

/// Refresh closing prices in a quarterly snapshot.
/// - Past quarter: prices are updated to the last trading day of that quarter.
/// - Current quarter: prices are updated to current live quotes.
pub async fn refresh_quarterly_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    snapshot_id: &str,
) -> Result<QuarterlySnapshotDetail, String> {
    // 1. Read existing snapshot header
    let (quarter_str, exchange_rates_json, _overall_notes) = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT quarter, exchange_rates, overall_notes FROM quarterly_snapshots WHERE id = ?1",
            rusqlite::params![snapshot_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .map_err(|e| format!("Snapshot not found: {}", e))?
    };

    // 2. Read all holding snapshots
    struct HoldingInfo {
        id: String,
        symbol: String,
        market: String,
        shares: f64,
        avg_cost: f64,
    }
    let holdings: Vec<HoldingInfo> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, symbol, market, shares, avg_cost
                 FROM quarterly_holding_snapshots
                 WHERE quarterly_snapshot_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map(rusqlite::params![snapshot_id], |row| {
            Ok(HoldingInfo {
                id: row.get(0)?,
                symbol: row.get(1)?,
                market: row.get(2)?,
                shares: row.get(3)?,
                avg_cost: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    if holdings.is_empty() {
        return get_quarterly_snapshot_detail(db, snapshot_id);
    }

    // 3. Determine if current or past quarter
    let today = Utc::now().date_naive();
    let current_quarter = date_to_quarter(today);
    let (year, q) = parse_quarter(&quarter_str)?;
    let end_date = quarter_end_date(year, q);
    let is_current = quarter_str == current_quarter;

    // 4. Fetch new prices
    let symbols: Vec<String> = holdings.iter().map(|h| h.symbol.clone()).collect();
    let price_map = if is_current {
        // Current quarter: fetch live quotes
        let sym_market_pairs: Vec<(String, String)> = holdings
            .iter()
            .map(|h| (h.symbol.clone(), h.market.clone()))
            .collect();
        let mut pm: HashMap<String, f64> = HashMap::new();
        if !sym_market_pairs.is_empty() {
            let config = quote_provider_service::get_quote_provider_config(db)?;
            let quotes = fetch_quotes_batch_cached_with_providers(
                quote_cache,
                sym_market_pairs,
                &config.us_provider,
                &config.hk_provider,
                true,
            )
            .await?;
            for q in quotes {
                pm.insert(q.symbol, q.current_price);
            }
        }
        pm
    } else {
        // Past quarter: fetch prices at quarter end date
        get_prices_for_date(db, quote_cache, &symbols, end_date).await?
    };

    // 5. Refresh exchange rates for current quarter, keep existing for past
    let rates: crate::models::quote::ExchangeRates = if is_current {
        get_cached_rates(cache).await.unwrap_or_else(|_| {
            // Fall back to stored rates
            serde_json::from_str(&exchange_rates_json).unwrap_or(crate::models::quote::ExchangeRates {
                usd_cny: 7.2,
                usd_hkd: 7.8,
                cny_hkd: 7.8 / 7.2,
                updated_at: Utc::now().to_rfc3339(),
            })
        })
    } else {
        // Keep existing exchange rates for past quarters
        serde_json::from_str(&exchange_rates_json).unwrap_or(crate::models::quote::ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: Utc::now().to_rfc3339(),
        })
    };

    // 6. Recalculate values
    let mut us_value = 0.0f64;
    let mut us_cost = 0.0f64;
    let mut cn_value = 0.0f64;
    let mut cn_cost = 0.0f64;
    let mut hk_value = 0.0f64;
    let mut hk_cost = 0.0f64;

    struct UpdateRow {
        id: String,
        market: String,
        close_price: f64,
        market_value: f64,
        cost_value: f64,
        pnl: f64,
        pnl_percent: f64,
    }

    let mut updates: Vec<UpdateRow> = Vec::new();

    for h in &holdings {
        let close_price = *price_map.get(&h.symbol).unwrap_or(&0.0);
        let market_value = h.shares * close_price;
        let cost_value = h.shares * h.avg_cost;
        let pnl = market_value - cost_value;
        let pnl_percent = if cost_value != 0.0 {
            pnl / cost_value * 100.0
        } else {
            0.0
        };

        match h.market.as_str() {
            "US" => {
                us_value += market_value;
                us_cost += cost_value;
            }
            "CN" => {
                cn_value += market_value;
                cn_cost += cost_value;
            }
            "HK" => {
                hk_value += market_value;
                hk_cost += cost_value;
            }
            _ => {}
        }

        updates.push(UpdateRow {
            id: h.id.clone(),
            market: h.market.clone(),
            close_price,
            market_value,
            cost_value,
            pnl,
            pnl_percent,
        });
    }

    let total_value = us_value
        + convert_currency(cn_value, "CNY", "USD", &rates)
        + convert_currency(hk_value, "HKD", "USD", &rates);
    let total_cost = us_cost
        + convert_currency(cn_cost, "CNY", "USD", &rates)
        + convert_currency(hk_cost, "HKD", "USD", &rates);
    let total_pnl = total_value - total_cost;

    let rates_json = serde_json::to_string(&rates).unwrap_or_default();
    let snapshot_date_str = if is_current {
        today.format("%Y-%m-%d").to_string()
    } else {
        end_date.format("%Y-%m-%d").to_string()
    };

    // 7. Update DB in a transaction
    {
        let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        // Update holding snapshots
        for u in &updates {
            let weight = if total_value != 0.0 {
                let mv_usd = match u.market.as_str() {
                    "CN" => convert_currency(u.market_value, "CNY", "USD", &rates),
                    "HK" => convert_currency(u.market_value, "HKD", "USD", &rates),
                    _ => u.market_value,
                };
                mv_usd / total_value * 100.0
            } else {
                0.0
            };

            tx.execute(
                "UPDATE quarterly_holding_snapshots
                 SET close_price = ?1, market_value = ?2, cost_value = ?3,
                     pnl = ?4, pnl_percent = ?5, weight = ?6
                 WHERE id = ?7",
                rusqlite::params![
                    u.close_price, u.market_value, u.cost_value,
                    u.pnl, u.pnl_percent, weight, u.id
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        // Update snapshot header
        tx.execute(
            "UPDATE quarterly_snapshots
             SET snapshot_date = ?1, total_value = ?2, total_cost = ?3, total_pnl = ?4,
                 us_value = ?5, us_cost = ?6, cn_value = ?7, cn_cost = ?8,
                 hk_value = ?9, hk_cost = ?10, exchange_rates = ?11
             WHERE id = ?12",
            rusqlite::params![
                snapshot_date_str, total_value, total_cost, total_pnl,
                us_value, us_cost, cn_value, cn_cost,
                hk_value, hk_cost, rates_json, snapshot_id
            ],
        )
        .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
    }

    // 8. Return updated detail
    get_quarterly_snapshot_detail(db, snapshot_id)
}

/// Find quarters that have no snapshot, from the first transaction quarter to the current quarter.
pub fn check_missing_snapshots(db: &Database) -> Result<Vec<String>, String> {
    // Find the earliest transaction date
    let earliest: Option<String> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT MIN(DATE(traded_at)) FROM transactions",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten()
    };

    let Some(earliest_str) = earliest else {
        return Ok(vec![]);
    };

    let earliest_date = NaiveDate::parse_from_str(&earliest_str, "%Y-%m-%d")
        .map_err(|e| format!("Bad date: {}", e))?;
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

/// Compare two quarterly snapshots.
pub fn compare_quarters(
    db: &Database,
    quarter1: &str,
    quarter2: &str,
) -> Result<QuarterComparison, String> {
    let (snap1, h1) = load_snapshot_for_quarter(db, quarter1)?;
    let (snap2, h2) = load_snapshot_for_quarter(db, quarter2)?;

    // Overview
    let value_change = snap2.total_value - snap1.total_value;
    let value_change_percent = if snap1.total_value != 0.0 {
        value_change / snap1.total_value * 100.0
    } else {
        0.0
    };
    let overview = ComparisonOverview {
        q1_total_value: snap1.total_value,
        q2_total_value: snap2.total_value,
        value_change,
        value_change_percent,
        q1_total_cost: snap1.total_cost,
        q2_total_cost: snap2.total_cost,
        q1_pnl: snap1.total_pnl,
        q2_pnl: snap2.total_pnl,
        q1_holding_count: h1.len(),
        q2_holding_count: h2.len(),
    };

    // By market
    let by_market = compute_market_comparison(&h1, &h2);

    // By category
    let by_category = compute_category_comparison(&h1, &h2);

    // Holding changes
    let holding_changes = compute_holding_changes(&h1, &h2);

    Ok(QuarterComparison {
        quarter1: quarter1.to_string(),
        quarter2: quarter2.to_string(),
        overview,
        by_market,
        by_category,
        holding_changes,
    })
}

fn load_snapshot_for_quarter(
    db: &Database,
    quarter: &str,
) -> Result<(QuarterlySnapshot, Vec<QuarterlyHoldingSnapshot>), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let snapshot_id: String = conn
        .query_row(
            "SELECT id FROM quarterly_snapshots WHERE quarter = ?1",
            rusqlite::params![quarter],
            |row| row.get(0),
        )
        .map_err(|_| format!("No snapshot found for quarter '{}'", quarter))?;

    let snapshot = conn
        .query_row(
            "SELECT id, quarter, snapshot_date, total_value, total_cost, total_pnl,
                    us_value, us_cost, cn_value, cn_cost, hk_value, hk_cost,
                    exchange_rates, overall_notes, created_at
             FROM quarterly_snapshots WHERE id = ?1",
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
                    holding_count: 0,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                    category_name, category_color, shares, avg_cost, close_price,
                    market_value, cost_value, pnl, pnl_percent, weight, notes
             FROM quarterly_holding_snapshots
             WHERE quarterly_snapshot_id = ?1",
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

    Ok((snapshot, holdings))
}

/// Load only the holdings for a given quarter. Used for computing holding changes.
fn load_holdings_for_quarter(
    db: &Database,
    quarter: &str,
) -> Result<Vec<QuarterlyHoldingSnapshot>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let snapshot_id: String = conn
        .query_row(
            "SELECT id FROM quarterly_snapshots WHERE quarter = ?1",
            rusqlite::params![quarter],
            |row| row.get(0),
        )
        .map_err(|_| format!("No snapshot found for quarter '{}'", quarter))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                    category_name, category_color, shares, avg_cost, close_price,
                    market_value, cost_value, pnl, pnl_percent, weight, notes
             FROM quarterly_holding_snapshots
             WHERE quarterly_snapshot_id = ?1",
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

    Ok(holdings)
}

fn compute_market_comparison(
    h1: &[QuarterlyHoldingSnapshot],
    h2: &[QuarterlyHoldingSnapshot],
) -> Vec<MarketComparison> {
    let markets = ["US", "CN", "HK"];
    markets
        .iter()
        .map(|m| {
            let q1_value: f64 = h1.iter().filter(|h| h.market == *m).map(|h| h.market_value).sum();
            let q1_cost: f64 = h1.iter().filter(|h| h.market == *m).map(|h| h.cost_value).sum();
            let q2_value: f64 = h2.iter().filter(|h| h.market == *m).map(|h| h.market_value).sum();
            let q2_cost: f64 = h2.iter().filter(|h| h.market == *m).map(|h| h.cost_value).sum();
            let value_change = q2_value - q1_value;
            let value_change_percent = if q1_value != 0.0 {
                value_change / q1_value * 100.0
            } else {
                0.0
            };
            MarketComparison {
                market: m.to_string(),
                q1_value,
                q2_value,
                value_change,
                value_change_percent,
                q1_cost,
                q2_cost,
                q1_pnl: q1_value - q1_cost,
                q2_pnl: q2_value - q2_cost,
            }
        })
        .collect()
}

fn compute_category_comparison(
    h1: &[QuarterlyHoldingSnapshot],
    h2: &[QuarterlyHoldingSnapshot],
) -> Vec<CategoryComparison> {
    let mut all_categories: Vec<(String, String)> = Vec::new();
    for h in h1.iter().chain(h2.iter()) {
        if !all_categories.iter().any(|(n, _)| n == &h.category_name) {
            all_categories.push((h.category_name.clone(), h.category_color.clone()));
        }
    }

    all_categories
        .into_iter()
        .map(|(cat_name, cat_color)| {
            let q1_value: f64 = h1
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.market_value)
                .sum();
            let q1_cost: f64 = h1
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.cost_value)
                .sum();
            let q2_value: f64 = h2
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.market_value)
                .sum();
            let q2_cost: f64 = h2
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.cost_value)
                .sum();
            let value_change = q2_value - q1_value;
            let value_change_percent = if q1_value != 0.0 {
                value_change / q1_value * 100.0
            } else {
                0.0
            };
            CategoryComparison {
                category_name: cat_name,
                category_color: cat_color,
                q1_value,
                q2_value,
                value_change,
                value_change_percent,
                q1_cost,
                q2_cost,
                q1_pnl: q1_value - q1_cost,
                q2_pnl: q2_value - q2_cost,
            }
        })
        .collect()
}

fn compute_holding_changes(
    h1: &[QuarterlyHoldingSnapshot],
    h2: &[QuarterlyHoldingSnapshot],
) -> HoldingChanges {
    // Build maps: symbol -> holding
    let map1: HashMap<&str, &QuarterlyHoldingSnapshot> =
        h1.iter().map(|h| (h.symbol.as_str(), h)).collect();
    let map2: HashMap<&str, &QuarterlyHoldingSnapshot> =
        h2.iter().map(|h| (h.symbol.as_str(), h)).collect();

    let mut new_holdings = Vec::new();
    let mut closed_holdings = Vec::new();
    let mut increased = Vec::new();
    let mut decreased = Vec::new();
    let mut unchanged = Vec::new();

    // Holdings in q2
    for (sym, h2_hold) in &map2 {
        if let Some(h1_hold) = map1.get(sym) {
            let shares_change = h2_hold.shares - h1_hold.shares;
            let value_change = h2_hold.market_value - h1_hold.market_value;
            let item = HoldingChangeItem {
                symbol: sym.to_string(),
                name: h2_hold.name.clone(),
                market: h2_hold.market.clone(),
                category_name: h2_hold.category_name.clone(),
                q1_shares: Some(h1_hold.shares),
                q2_shares: Some(h2_hold.shares),
                q1_value: Some(h1_hold.market_value),
                q2_value: Some(h2_hold.market_value),
                shares_change,
                value_change,
            };
            if shares_change > 1e-9 {
                increased.push(item);
            } else if shares_change < -1e-9 {
                decreased.push(item);
            } else {
                unchanged.push(item);
            }
        } else {
            new_holdings.push(HoldingChangeItem {
                symbol: sym.to_string(),
                name: h2_hold.name.clone(),
                market: h2_hold.market.clone(),
                category_name: h2_hold.category_name.clone(),
                q1_shares: None,
                q2_shares: Some(h2_hold.shares),
                q1_value: None,
                q2_value: Some(h2_hold.market_value),
                shares_change: h2_hold.shares,
                value_change: h2_hold.market_value,
            });
        }
    }

    // Holdings in q1 but not q2 (closed)
    for (sym, h1_hold) in &map1 {
        if !map2.contains_key(sym) {
            closed_holdings.push(HoldingChangeItem {
                symbol: sym.to_string(),
                name: h1_hold.name.clone(),
                market: h1_hold.market.clone(),
                category_name: h1_hold.category_name.clone(),
                q1_shares: Some(h1_hold.shares),
                q2_shares: None,
                q1_value: Some(h1_hold.market_value),
                q2_value: None,
                shares_change: -h1_hold.shares,
                value_change: -h1_hold.market_value,
            });
        }
    }

    HoldingChanges {
        new_holdings,
        closed_holdings,
        increased,
        decreased,
        unchanged,
    }
}

/// Update the notes for a specific holding in a quarterly snapshot.
pub fn update_holding_notes(
    db: &Database,
    snapshot_id: &str,
    symbol: &str,
    notes: &str,
) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let rows = conn
        .execute(
            "UPDATE quarterly_holding_snapshots SET notes = ?1
             WHERE quarterly_snapshot_id = ?2 AND symbol = ?3",
            rusqlite::params![notes, snapshot_id, symbol],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}

/// Get the notes history for a specific symbol across all quarterly snapshots.
pub fn get_holding_notes_history(
    db: &Database,
    symbol: &str,
) -> Result<Vec<HoldingNoteHistory>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT qs.quarter, qs.snapshot_date, qhs.shares, qhs.avg_cost,
                    qhs.close_price, qhs.pnl_percent, COALESCE(qhs.notes, '')
             FROM quarterly_holding_snapshots qhs
             JOIN quarterly_snapshots qs ON qhs.quarterly_snapshot_id = qs.id
             WHERE qhs.symbol = ?1
             ORDER BY qs.quarter DESC",
        )
        .map_err(|e| e.to_string())?;

    let history = stmt
        .query_map(rusqlite::params![symbol], |row| {
            Ok(HoldingNoteHistory {
                quarter: row.get(0)?,
                snapshot_date: row.get(1)?,
                shares: row.get(2)?,
                avg_cost: row.get(3)?,
                close_price: row.get(4)?,
                pnl_percent: row.get(5)?,
                notes: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(history)
}

/// Update the overall notes for a quarterly snapshot.
pub fn update_quarterly_notes(
    db: &Database,
    snapshot_id: &str,
    notes: &str,
) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let rows = conn
        .execute(
            "UPDATE quarterly_snapshots SET overall_notes = ?1 WHERE id = ?2",
            rusqlite::params![notes, snapshot_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}

/// Get a list of all quarterly notes summaries.
pub fn get_quarterly_notes_history(db: &Database) -> Result<Vec<QuarterlyNotesSummary>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, quarter, snapshot_date, COALESCE(overall_notes, ''), total_value, total_pnl
             FROM quarterly_snapshots
             ORDER BY quarter DESC",
        )
        .map_err(|e| e.to_string())?;

    let summaries = stmt
        .query_map([], |row| {
            Ok(QuarterlyNotesSummary {
                snapshot_id: row.get(0)?,
                quarter: row.get(1)?,
                snapshot_date: row.get(2)?,
                overall_notes: row.get(3)?,
                total_value: row.get(4)?,
                total_pnl: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(summaries)
}

/// Get multi-quarter trend data.
pub fn get_quarterly_trends(db: &Database) -> Result<QuarterlyTrends, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Get all snapshots ordered by quarter
    let mut stmt = conn
        .prepare(
            "SELECT id, quarter, total_value, total_cost, total_pnl,
                    us_value, cn_value, hk_value
             FROM quarterly_snapshots
             ORDER BY quarter ASC",
        )
        .map_err(|e| e.to_string())?;

    struct SnapRow {
        id: String,
        quarter: String,
        total_value: f64,
        total_cost: f64,
        total_pnl: f64,
        us_value: f64,
        cn_value: f64,
        hk_value: f64,
    }

    let snap_rows: Vec<SnapRow> = stmt
        .query_map([], |row| {
            Ok(SnapRow {
                id: row.get(0)?,
                quarter: row.get(1)?,
                total_value: row.get(2)?,
                total_cost: row.get(3)?,
                total_pnl: row.get(4)?,
                us_value: row.get(5)?,
                cn_value: row.get(6)?,
                hk_value: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if snap_rows.is_empty() {
        return Ok(QuarterlyTrends {
            quarters: vec![],
            total_values: vec![],
            total_costs: vec![],
            total_pnls: vec![],
            market_values: HashMap::new(),
            category_values: HashMap::new(),
            holding_counts: vec![],
        });
    }

    let quarters: Vec<String> = snap_rows.iter().map(|r| r.quarter.clone()).collect();
    let total_values: Vec<f64> = snap_rows.iter().map(|r| r.total_value).collect();
    let total_costs: Vec<f64> = snap_rows.iter().map(|r| r.total_cost).collect();
    let total_pnls: Vec<f64> = snap_rows.iter().map(|r| r.total_pnl).collect();

    let mut market_values: HashMap<String, Vec<f64>> = HashMap::new();
    market_values.insert(
        "US".to_string(),
        snap_rows.iter().map(|r| r.us_value).collect(),
    );
    market_values.insert(
        "CN".to_string(),
        snap_rows.iter().map(|r| r.cn_value).collect(),
    );
    market_values.insert(
        "HK".to_string(),
        snap_rows.iter().map(|r| r.hk_value).collect(),
    );

    // Get category breakdown per snapshot
    let all_cats: Vec<(String, String)> = {
        let mut cats: Vec<(String, String)> = Vec::new();
        let mut cat_stmt = conn
            .prepare(
                "SELECT DISTINCT category_name, category_color FROM quarterly_holding_snapshots",
            )
            .map_err(|e| e.to_string())?;
        cat_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .for_each(|c| {
                if !cats.iter().any(|(n, _)| n == &c.0) {
                    cats.push(c);
                }
            });
        cats
    };

    let mut category_values: HashMap<String, Vec<f64>> = HashMap::new();
    for (cat_name, _) in &all_cats {
        let mut values = Vec::new();
        for snap in &snap_rows {
            let sum: f64 = {
                conn.query_row(
                    "SELECT COALESCE(SUM(market_value), 0) FROM quarterly_holding_snapshots
                     WHERE quarterly_snapshot_id = ?1 AND category_name = ?2",
                    rusqlite::params![snap.id, cat_name],
                    |row| row.get(0),
                )
                .unwrap_or(0.0)
            };
            values.push(sum);
        }
        category_values.insert(cat_name.clone(), values);
    }

    // Holding counts per snapshot
    let holding_counts: Vec<usize> = snap_rows
        .iter()
        .map(|snap| {
            conn.query_row(
                "SELECT COUNT(*) FROM quarterly_holding_snapshots WHERE quarterly_snapshot_id = ?1",
                rusqlite::params![snap.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
        })
        .collect();

    Ok(QuarterlyTrends {
        quarters,
        total_values,
        total_costs,
        total_pnls,
        market_values,
        category_values,
        holding_counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
