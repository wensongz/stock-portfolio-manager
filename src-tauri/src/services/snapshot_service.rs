use crate::db::Database;
use crate::models::{DailyHoldingSnapshot, DailyPortfolioValue};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::fetch_quotes_batch;
use chrono::NaiveDate;

/// Take a daily portfolio snapshot for the given date.
/// This is idempotent: running it twice for the same date replaces the existing record.
pub async fn take_daily_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    date: NaiveDate,
) -> Result<(), String> {
    let date_str = date.format("%Y-%m-%d").to_string();

    // 1. Load all holdings from DB (synchronous)
    let holdings = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT h.id, h.account_id, h.symbol, h.name, h.market,
                        h.shares, h.avg_cost, h.currency, c.name as category_name
                 FROM holdings h
                 LEFT JOIN categories c ON h.category_id = c.id
                 WHERE h.shares > 0",
            )
            .map_err(|e| e.to_string())?;

        #[derive(Debug)]
        struct HoldingRow {
            id: String,
            account_id: String,
            symbol: String,
            name: String,
            market: String,
            shares: f64,
            avg_cost: f64,
            currency: String,
            category_name: Option<String>,
        }

        let rows = stmt.query_map([], |row| {
            Ok(HoldingRow {
                id: row.get(0)?,
                account_id: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                shares: row.get(5)?,
                avg_cost: row.get(6)?,
                currency: row.get(7)?,
                category_name: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
        let result = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        result
    };

    if holdings.is_empty() {
        return Ok(());
    }

    // 2. Fetch quotes for all holdings (async)
    let symbols: Vec<(String, String)> = holdings
        .iter()
        .map(|h| (h.symbol.clone(), h.market.clone()))
        .collect();
    let quotes = fetch_quotes_batch(symbols).await?;
    let quote_map: std::collections::HashMap<String, f64> = quotes
        .iter()
        .map(|q| (q.symbol.clone(), q.current_price))
        .collect();

    // 3. Get exchange rates (async)
    let rates = crate::services::exchange_rate_service::get_cached_rates(cache).await?;

    // 4. Calculate per-holding snapshots and aggregate values
    let mut us_cost = 0.0f64;
    let mut us_value = 0.0f64;
    let mut cn_cost = 0.0f64;
    let mut cn_value = 0.0f64;
    let mut hk_cost = 0.0f64;
    let mut hk_value = 0.0f64;

    let mut snapshots: Vec<DailyHoldingSnapshot> = Vec::new();

    for holding in &holdings {
        let close_price = *quote_map.get(&holding.symbol).unwrap_or(&0.0);
        let market_value = holding.shares * close_price;
        let cost = holding.shares * holding.avg_cost;

        match holding.market.as_str() {
            "US" => {
                us_cost += cost;
                us_value += market_value;
            }
            "CN" => {
                cn_cost += cost;
                cn_value += market_value;
            }
            "HK" => {
                hk_cost += cost;
                hk_value += market_value;
            }
            _ => {}
        }

        snapshots.push(DailyHoldingSnapshot {
            id: 0,
            date: date_str.clone(),
            account_id: holding.account_id.clone(),
            symbol: holding.symbol.clone(),
            market: holding.market.clone(),
            category_name: holding.category_name.clone(),
            shares: holding.shares,
            avg_cost: holding.avg_cost,
            close_price,
            market_value,
        });
    }

    // Convert all values to USD for total aggregation
    let total_cost = us_cost
        + crate::services::exchange_rate_service::convert_currency(cn_cost, "CNY", "USD", &rates)
        + crate::services::exchange_rate_service::convert_currency(hk_cost, "HKD", "USD", &rates);
    let total_value = us_value
        + crate::services::exchange_rate_service::convert_currency(cn_value, "CNY", "USD", &rates)
        + crate::services::exchange_rate_service::convert_currency(hk_value, "HKD", "USD", &rates);

    let daily_pnl = total_value - total_cost;

    // 5. Compute cumulative PnL (previous day's total_value as baseline)
    let prev_total_value: f64 = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT COALESCE(SUM(total_value), 0) FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
            rusqlite::params![date_str],
            |row| row.get(0),
        )
        .unwrap_or(0.0)
    };
    let cumulative_pnl = total_value - prev_total_value;

    let rates_json = serde_json::to_string(&rates).unwrap_or_default();

    // 6. Persist to DB (synchronous, upsert)
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT OR REPLACE INTO daily_portfolio_values
             (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value, hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                date_str, total_cost, total_value,
                us_cost, us_value, cn_cost, cn_value, hk_cost, hk_value,
                rates_json, daily_pnl, cumulative_pnl
            ],
        )
        .map_err(|e| e.to_string())?;

        // Delete existing snapshots for this date, then insert new ones
        conn.execute(
            "DELETE FROM daily_holding_snapshots WHERE date = ?1",
            rusqlite::params![date_str],
        )
        .map_err(|e| e.to_string())?;

        for snap in &snapshots {
            conn.execute(
                "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, category_name, shares, avg_cost, close_price, market_value)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    snap.date, snap.account_id, snap.symbol, snap.market,
                    snap.category_name, snap.shares, snap.avg_cost,
                    snap.close_price, snap.market_value
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Check if today's snapshot has already been taken; if not, take it.
pub async fn auto_snapshot_check(
    db: &Database,
    cache: &ExchangeRateCache,
) -> Result<(), String> {
    let today = chrono::Utc::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();

    let already_taken: bool = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM daily_portfolio_values WHERE date = ?1",
                rusqlite::params![today_str],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    };

    if !already_taken {
        take_daily_snapshot(db, cache, today).await?;
    }

    Ok(())
}

/// Query daily portfolio values in a date range.
pub fn get_daily_values(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<DailyPortfolioValue>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start = start_date.format("%Y-%m-%d").to_string();
    let end = end_date.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT id, date, total_cost, total_value, us_cost, us_value,
                    cn_cost, cn_value, hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl
             FROM daily_portfolio_values
             WHERE date BETWEEN ?1 AND ?2
             ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let values = stmt
        .query_map(rusqlite::params![start, end], |row| {
            Ok(DailyPortfolioValue {
                id: row.get(0)?,
                date: row.get(1)?,
                total_cost: row.get(2)?,
                total_value: row.get(3)?,
                us_cost: row.get(4)?,
                us_value: row.get(5)?,
                cn_cost: row.get(6)?,
                cn_value: row.get(7)?,
                hk_cost: row.get(8)?,
                hk_value: row.get(9)?,
                exchange_rates: row.get(10)?,
                daily_pnl: row.get(11)?,
                cumulative_pnl: row.get(12)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(values)
}
