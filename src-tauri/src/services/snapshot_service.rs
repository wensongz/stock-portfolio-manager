use crate::db::Database;
use crate::models::{DailyHoldingSnapshot, DailyPortfolioValue};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::{fetch_quotes_batch_cached_with_providers, fetch_stock_history, QuoteCache};
use crate::services::quote_provider_service;
use chrono::{Datelike, NaiveDate};

/// Take a daily portfolio snapshot for the given date.
/// This is idempotent: running it twice for the same date replaces the existing record.
pub async fn take_daily_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
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
    let quotes = {
        let config = quote_provider_service::get_quote_provider_config(db)?;
        fetch_quotes_batch_cached_with_providers(quote_cache, symbols, &config.us_provider, &config.hk_provider, &config.cn_provider, true).await?
    };
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

    // cumulative_pnl: total unrealized P&L since positions were opened
    let cumulative_pnl = total_value - total_cost;

    // daily_pnl: change in portfolio value compared to previous day's snapshot
    let prev_total_value: f64 = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT COALESCE(total_value, 0) FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
            rusqlite::params![date_str],
            |row| row.get(0),
        )
        .unwrap_or(0.0)
    };
    let daily_pnl = total_value - prev_total_value;

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
    quote_cache: &QuoteCache,
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
        take_daily_snapshot(db, cache, quote_cache, today).await?;
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

/// Backfill missing daily portfolio snapshots for the given date range.
/// Fetches historical closing prices from Yahoo Finance, calculates portfolio
/// values for every missing weekday, and stores them in the database.
/// Returns the number of snapshots created.
///
/// **Note:** This uses *current* exchange rates for all historical dates and
/// *current* holdings composition.  For portfolios with significant
/// multi-currency exposure or frequently changing compositions, the
/// back-filled values are approximate.
pub async fn backfill_snapshots(
    db: &Database,
    cache: &ExchangeRateCache,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<i32, String> {
    let today = chrono::Utc::now().date_naive();
    // Clamp end_date to today
    let end_date = if end_date > today { today } else { end_date };

    if start_date > end_date {
        return Ok(0);
    }

    // 1. Load all current holdings
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

        #[derive(Debug, Clone)]
        struct HoldingRow {
            _id: String,
            account_id: String,
            symbol: String,
            _name: String,
            market: String,
            shares: f64,
            avg_cost: f64,
            _currency: String,
            category_name: Option<String>,
        }

        let rows = stmt.query_map([], |row| {
            Ok(HoldingRow {
                _id: row.get(0)?,
                account_id: row.get(1)?,
                symbol: row.get(2)?,
                _name: row.get(3)?,
                market: row.get(4)?,
                shares: row.get(5)?,
                avg_cost: row.get(6)?,
                _currency: row.get(7)?,
                category_name: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    if holdings.is_empty() {
        return Ok(0);
    }

    // 2. Find all weekdays in range that are missing snapshots
    let existing_dates: std::collections::HashSet<String> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let end_str = end_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT date FROM daily_portfolio_values WHERE date BETWEEN ?1 AND ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![start_str, end_str], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|e| e.to_string())?
    };

    let mut missing_dates: Vec<NaiveDate> = Vec::new();
    let mut d = start_date;
    while d <= end_date {
        let wd = d.weekday();
        if wd != chrono::Weekday::Sat && wd != chrono::Weekday::Sun {
            let ds = d.format("%Y-%m-%d").to_string();
            if !existing_dates.contains(&ds) {
                missing_dates.push(d);
            }
        }
        d = d.succ_opt().unwrap_or(d);
    }

    if missing_dates.is_empty() {
        return Ok(0);
    }

    // 3. Fetch historical prices for each holding using the configured provider
    // Build a map: symbol -> { date -> close_price }
    // Cash symbols skip API calls – their price is always 1.0.
    let mut history_map: std::collections::HashMap<
        String,
        std::collections::HashMap<NaiveDate, f64>,
    > = std::collections::HashMap::new();

    let config = quote_provider_service::get_quote_provider_config(db)?;

    for holding in &holdings {
        // Cash holdings have a constant price of 1.0 – no history fetch needed.
        if crate::services::quote_service::is_cash_symbol(&holding.symbol) {
            // Populate every missing date with price = 1.0 so forward-fill works
            let mut cash_prices =
                std::collections::HashMap::with_capacity(missing_dates.len());
            for d in &missing_dates {
                cash_prices.insert(*d, 1.0);
            }
            history_map.insert(holding.symbol.clone(), cash_prices);
            continue;
        }

        // Select the configured provider for the holding's market.
        let provider = match holding.market.as_str() {
            "US" => config.us_provider.as_str(),
            "HK" => config.hk_provider.as_str(),
            _ => config.cn_provider.as_str(),
        };

        match fetch_stock_history(
            &holding.symbol,
            &holding.market,
            start_date,
            end_date,
            provider,
        )
        .await
        {
            Ok(prices) => {
                let date_price_map: std::collections::HashMap<NaiveDate, f64> =
                    prices.into_iter().collect();
                history_map.insert(holding.symbol.clone(), date_price_map);
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to fetch history for {} ({}): {}",
                    holding.symbol, holding.market, e
                );
            }
        }
    }

    // Build sorted price vectors per symbol for forward-fill on holidays.
    // When a market is closed (e.g. public holidays), the data provider returns
    // no price for that date.  We carry forward the most recent closing price so
    // that the portfolio value is still computed correctly.
    let history_sorted: std::collections::HashMap<String, Vec<(NaiveDate, f64)>> = history_map
        .iter()
        .map(|(symbol, date_map)| {
            let mut sorted: Vec<(NaiveDate, f64)> =
                date_map.iter().map(|(d, p)| (*d, *p)).collect();
            sorted.sort_by_key(|(d, _)| *d);
            (symbol.clone(), sorted)
        })
        .collect();

    // 4. Get exchange rates (use current rates as approximation for all dates)
    let rates = crate::services::exchange_rate_service::get_cached_rates(cache).await?;
    let rates_json = serde_json::to_string(&rates).unwrap_or_default();

    // 5. For each missing date, calculate and store portfolio values
    let mut count = 0i32;

    for date in &missing_dates {
        let date_str = date.format("%Y-%m-%d").to_string();

        let mut us_cost = 0.0f64;
        let mut us_value = 0.0f64;
        let mut cn_cost = 0.0f64;
        let mut cn_value = 0.0f64;
        let mut hk_cost = 0.0f64;
        let mut hk_value = 0.0f64;
        let mut snapshots: Vec<DailyHoldingSnapshot> = Vec::new();
        let mut has_any_price = false;

        for holding in &holdings {
            // Look up the closing price for this stock on this date.
            // If the exact date is missing (market holiday), forward-fill
            // from the most recent prior trading day's closing price.
            let close_price = history_map
                .get(&holding.symbol)
                .and_then(|date_map| {
                    history_sorted
                        .get(&holding.symbol)
                        .and_then(|sorted| forward_fill_price(date_map, sorted, date))
                })
                .unwrap_or_else(|| {
                    eprintln!(
                        "Warning: no historical price for {} ({}) on {}",
                        holding.symbol, holding.market, date_str
                    );
                    0.0
                });

            if close_price > 0.0 {
                has_any_price = true;
            }

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

        // Skip dates where no price data is available at all (e.g. date
        // is before the earliest trading data for every holding).
        if !has_any_price {
            continue;
        }

        let total_cost = us_cost
            + crate::services::exchange_rate_service::convert_currency(
                cn_cost, "CNY", "USD", &rates,
            )
            + crate::services::exchange_rate_service::convert_currency(
                hk_cost, "HKD", "USD", &rates,
            );
        let total_value = us_value
            + crate::services::exchange_rate_service::convert_currency(
                cn_value, "CNY", "USD", &rates,
            )
            + crate::services::exchange_rate_service::convert_currency(
                hk_value, "HKD", "USD", &rates,
            );

        let cumulative_pnl = total_value - total_cost;

        let prev_total_value: f64 = {
            let conn = db.conn.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT COALESCE(total_value, 0) FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
                rusqlite::params![date_str],
                |row| row.get(0),
            )
            .unwrap_or(0.0)
        };
        let daily_pnl = total_value - prev_total_value;

        // Persist
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

        count += 1;
    }

    Ok(count)
}

/// Look up a closing price for a stock on a given date, falling back to the
/// most recent earlier trading day when the market was closed (forward-fill).
/// Returns `None` only when there is no price data at or before the date.
fn forward_fill_price(
    history_map: &std::collections::HashMap<NaiveDate, f64>,
    sorted_prices: &[(NaiveDate, f64)],
    date: &NaiveDate,
) -> Option<f64> {
    // Fast path: exact date match
    if let Some(&price) = history_map.get(date) {
        return Some(price);
    }
    // Forward-fill from the most recent prior trading day
    match sorted_prices.binary_search_by_key(date, |(d, _)| *d) {
        Ok(idx) => Some(sorted_prices[idx].1),
        Err(0) => None,
        Err(idx) => Some(sorted_prices[idx - 1].1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forward_fill_price_exact_match() {
        let mut map = std::collections::HashMap::new();
        map.insert(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(), 10.0);
        map.insert(NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(), 11.0);
        let sorted = vec![
            (NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(), 10.0),
            (NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(), 11.0),
        ];
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(forward_fill_price(&map, &sorted, &d), Some(10.0));
    }

    #[test]
    fn test_forward_fill_price_holiday_uses_previous() {
        // 2026-01-01 is a holiday; nearest prior trading day is 2025-12-31
        let mut map = std::collections::HashMap::new();
        map.insert(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(), 50.0);
        map.insert(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(), 51.0);
        let sorted = vec![
            (NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(), 50.0),
            (NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(), 51.0),
        ];
        // Query 2026-01-01 (holiday) — should forward-fill with 2025-12-31 price
        let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(forward_fill_price(&map, &sorted, &d), Some(50.0));
    }

    #[test]
    fn test_forward_fill_price_no_earlier_data() {
        let map = std::collections::HashMap::new();
        let sorted: Vec<(NaiveDate, f64)> = vec![
            (NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), 20.0),
        ];
        // Query a date before all available data
        let d = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(forward_fill_price(&map, &sorted, &d), None);
    }

    #[test]
    fn test_forward_fill_price_empty_data() {
        let map = std::collections::HashMap::new();
        let sorted: Vec<(NaiveDate, f64)> = vec![];
        let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(forward_fill_price(&map, &sorted, &d), None);
    }

    #[test]
    fn test_forward_fill_price_multiple_holidays() {
        // Simulate a long holiday: trading days on Dec 30 and Jan 5, gap in between
        let mut map = std::collections::HashMap::new();
        map.insert(NaiveDate::from_ymd_opt(2025, 12, 30).unwrap(), 100.0);
        map.insert(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), 105.0);
        let sorted = vec![
            (NaiveDate::from_ymd_opt(2025, 12, 30).unwrap(), 100.0),
            (NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), 105.0),
        ];
        // All dates in the gap should forward-fill from Dec 30
        for day in [31, 1, 2] {
            let (y, m) = if day == 31 { (2025, 12) } else { (2026, 1) };
            let d = NaiveDate::from_ymd_opt(y, m, day).unwrap();
            assert_eq!(
                forward_fill_price(&map, &sorted, &d),
                Some(100.0),
                "failed for date {}-{:02}-{:02}",
                y,
                m,
                day
            );
        }
    }
}
