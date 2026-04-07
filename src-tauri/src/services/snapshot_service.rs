use crate::db::Database;
use crate::models::{DailyHoldingSnapshot, DailyPortfolioValue};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::{fetch_quotes_batch_cached_with_providers, fetch_stock_history, QuoteCache};
use crate::services::quote_provider_service;
use chrono::{Datelike, NaiveDate, Timelike};

/// Return the latest date for which all markets are guaranteed to have
/// closing prices available.
///
/// Historical price APIs only return data **after** market close.  The
/// furthest-ahead market is CN/HK (UTC+8) which closes at 15:00 local time.
/// We use 16:00 UTC+8 as a safe buffer (allowing for settlement/delayed
/// data publication).
///
/// * If the current time in UTC+8 is **before** 16:00 → yesterday's date
///   (in UTC+8) is the latest date with guaranteed closing prices.
/// * If it is 16:00 or later → today's date (in UTC+8) is safe.
///
/// For US markets (EST/EDT), the close is 16:00 US Eastern, which is
/// already past midnight UTC+8 of the **next** day.  So the CN/HK gate
/// is always the binding constraint: if CN/HK has closed, the US close
/// for the previous calendar day has long since passed.
pub fn last_closed_market_date() -> NaiveDate {
    let utc_plus_8 = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    let now_cst = chrono::Utc::now().with_timezone(&utc_plus_8);
    let today_cst = now_cst.date_naive();

    // CN/HK markets close at 15:00 CST; add 1-hour buffer → 16:00.
    if now_cst.hour() < 16 {
        today_cst - chrono::Duration::days(1)
    } else {
        today_cst
    }
}

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

    let rates_json = serde_json::to_string(&rates).unwrap_or_default();

    // 6. Persist to DB inside a transaction for atomicity and performance.
    {
        let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        // daily_pnl: change in portfolio value compared to previous day's snapshot
        let prev_total_value: f64 = tx
            .query_row(
                "SELECT COALESCE(total_value, 0) FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
                rusqlite::params![date_str],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        let daily_pnl = total_value - prev_total_value;

        tx.execute(
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
        tx.execute(
            "DELETE FROM daily_holding_snapshots WHERE date = ?1",
            rusqlite::params![date_str],
        )
        .map_err(|e| e.to_string())?;

        for snap in &snapshots {
            tx.execute(
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

        tx.commit().map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Check if the latest market-closed day's snapshot has already been taken;
/// if not, take it.  Uses `last_closed_market_date()` so we never attempt to
/// snapshot a date whose closing prices are not yet available.
pub async fn auto_snapshot_check(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
) -> Result<(), String> {
    let today = last_closed_market_date();
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
    force: bool,
) -> Result<i32, String> {
    // Clamp end_date to the last date for which closing prices are
    // available.  Before CN/HK market close (≈15:00 UTC+8), today's
    // prices do not exist yet, so we use yesterday.
    let latest_closed = last_closed_market_date();
    let end_date = if end_date > latest_closed { latest_closed } else { end_date };

    if start_date > end_date {
        return Ok(0);
    }

    // 1. Load all relevant holdings: current active ones PLUS any that had
    //    transactions in the backfill period (they may be sold now but were
    //    held on historical dates).
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

    let holdings = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT h.id, h.account_id, h.symbol, h.name, h.market,
                        h.shares, h.avg_cost, h.currency, c.name as category_name
                 FROM holdings h
                 LEFT JOIN categories c ON h.category_id = c.id
                 WHERE h.shares > 0
                    OR EXISTS (
                        SELECT 1 FROM transactions t
                        WHERE t.account_id = h.account_id
                          AND t.symbol = h.symbol
                          AND DATE(t.traded_at) >= ?1
                    )",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt.query_map(rusqlite::params![start_str], |row| {
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

    // 1b. Load transactions from start_date onwards so we can reconstruct
    //     historical holdings by unwinding future transactions.
    struct TxInfo {
        account_id: String,
        symbol: String,
        transaction_type: String,
        shares: f64,
        total_amount: f64,
        commission: f64,
        currency: String,
        trade_date: NaiveDate,
    }

    let transactions: Vec<TxInfo> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT account_id, symbol, transaction_type, shares,
                        total_amount, commission, currency, DATE(traded_at) as trade_date
                 FROM transactions
                 WHERE DATE(traded_at) >= ?1
                 ORDER BY traded_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![start_str], |row| {
                let td_str: String = row.get(7)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, String>(6)?,
                    td_str,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter()
            .filter_map(|(aid, sym, tt, sh, ta, com, cur, ds)| {
                NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
                    .ok()
                    .map(|td| TxInfo {
                        account_id: aid,
                        symbol: sym,
                        transaction_type: tt,
                        shares: sh,
                        total_amount: ta,
                        commission: com,
                        currency: cur,
                        trade_date: td,
                    })
            })
            .collect()
    };

    // Pre-compute the TOTAL unwind delta across ALL loaded transactions.
    // For a given date D, the adjustment = total_unwind - running_unwind(up to D)
    // gives the unwind of all transactions AFTER D, yielding the shares at D.
    let mut total_unwind: std::collections::HashMap<(String, String), f64> =
        std::collections::HashMap::new();
    for tx in &transactions {
        let key = (tx.account_id.clone(), tx.symbol.clone());
        let cash_sym = format!("{}{}", crate::services::quote_service::CASH_SYMBOL_PREFIX, tx.currency);
        let cash_key = (tx.account_id.clone(), cash_sym);
        match tx.transaction_type.as_str() {
            "BUY" => {
                *total_unwind.entry(key).or_insert(0.0) -= tx.shares;
                *total_unwind.entry(cash_key).or_insert(0.0) +=
                    tx.total_amount + tx.commission;
            }
            "SELL" => {
                *total_unwind.entry(key).or_insert(0.0) += tx.shares;
                *total_unwind.entry(cash_key).or_insert(0.0) -=
                    tx.total_amount - tx.commission;
            }
            _ => {}
        }
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
    // When `force` is true, re-create ALL snapshots if any transactions
    // exist in the period (a transaction on date D changes the adjusted
    // holdings for every date around D).  When `force` is false, only
    // fill in dates that have never been calculated – this lets the UI
    // load quickly from cached data without re-fetching historical prices.
    let has_transactions = force && transactions.iter().any(|tx| tx.trade_date <= end_date);
    let mut d = start_date;
    while d <= end_date {
        let wd = d.weekday();
        if wd != chrono::Weekday::Sat && wd != chrono::Weekday::Sun {
            let ds = d.format("%Y-%m-%d").to_string();
            if !existing_dates.contains(&ds) || has_transactions {
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

    // Narrow the fetch window to cover only the missing dates.  When most
    // snapshots are already cached (e.g. a new day just started), this avoids
    // re-fetching weeks of historical prices that are already in the DB.
    // No extra look-back is needed here: we only need prices within the
    // missing date range; forward-fill handles holidays within this window.
    let fetch_start = missing_dates.first().copied().unwrap_or(start_date);
    let fetch_end = missing_dates.last().copied().unwrap_or(end_date);

    // Deduplicate symbols – multiple accounts may hold the same stock;
    // we only need to fetch historical prices once per unique symbol.
    let unique_symbols: Vec<(String, String)> = {
        let mut seen = std::collections::HashSet::new();
        holdings
            .iter()
            .filter(|h| seen.insert(h.symbol.clone()))
            .map(|h| (h.symbol.clone(), h.market.clone()))
            .collect()
    };

    for (symbol, market) in &unique_symbols {
        // Cash holdings have a constant price of 1.0 – no history fetch needed.
        if crate::services::quote_service::is_cash_symbol(symbol) {
            // Populate every missing date with price = 1.0 so forward-fill works
            let mut cash_prices =
                std::collections::HashMap::with_capacity(missing_dates.len());
            for d in &missing_dates {
                cash_prices.insert(*d, 1.0);
            }
            history_map.insert(symbol.clone(), cash_prices);
            continue;
        }

        // Select the configured provider for the holding's market.
        let provider = match market.as_str() {
            "US" => config.us_provider.as_str(),
            "HK" => config.hk_provider.as_str(),
            _ => config.cn_provider.as_str(),
        };

        match fetch_stock_history(
            symbol,
            market,
            fetch_start,
            fetch_end,
            provider,
        )
        .await
        {
            Ok(prices) => {
                let date_price_map: std::collections::HashMap<NaiveDate, f64> =
                    prices.into_iter().collect();
                history_map.insert(symbol.clone(), date_price_map);
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to fetch history for {} ({}): {}",
                    symbol, market, e
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

    // 5. For each missing date, calculate and store portfolio values.
    //    We reconstruct historical holdings by unwinding transactions:
    //    running_unwind accumulates the unwind of transactions up to each
    //    date; the adjustment for date D = total_unwind - running_unwind
    //    gives the unwind of all transactions AFTER D.
    //
    //    All DB writes are wrapped in a single SQLite transaction for
    //    atomicity and significantly better write performance (avoids
    //    per-statement fsync in autocommit mode).
    let mut count = 0i32;
    let mut txn_idx = 0usize;
    let mut running_unwind: std::collections::HashMap<(String, String), f64> =
        std::collections::HashMap::new();

    // Collect all rows to persist, then batch-write inside a transaction.
    struct DateRow {
        date_str: String,
        total_cost: f64,
        total_value: f64,
        us_cost: f64,
        us_value: f64,
        cn_cost: f64,
        cn_value: f64,
        hk_cost: f64,
        hk_value: f64,
        cumulative_pnl: f64,
        snapshots: Vec<DailyHoldingSnapshot>,
    }
    let mut date_rows: Vec<DateRow> = Vec::with_capacity(missing_dates.len());

    for date in &missing_dates {
        let date_str = date.format("%Y-%m-%d").to_string();

        // Advance running_unwind past transactions on or before this date.
        while txn_idx < transactions.len() && transactions[txn_idx].trade_date <= *date {
            let tx = &transactions[txn_idx];
            let key = (tx.account_id.clone(), tx.symbol.clone());
            let cash_sym = format!(
                "{}{}",
                crate::services::quote_service::CASH_SYMBOL_PREFIX,
                tx.currency
            );
            let cash_key = (tx.account_id.clone(), cash_sym);
            match tx.transaction_type.as_str() {
                "BUY" => {
                    *running_unwind.entry(key).or_insert(0.0) -= tx.shares;
                    *running_unwind.entry(cash_key).or_insert(0.0) +=
                        tx.total_amount + tx.commission;
                }
                "SELL" => {
                    *running_unwind.entry(key).or_insert(0.0) += tx.shares;
                    *running_unwind.entry(cash_key).or_insert(0.0) -=
                        tx.total_amount - tx.commission;
                }
                _ => {}
            }
            txn_idx += 1;
        }

        let mut us_cost = 0.0f64;
        let mut us_value = 0.0f64;
        let mut cn_cost = 0.0f64;
        let mut cn_value = 0.0f64;
        let mut hk_cost = 0.0f64;
        let mut hk_value = 0.0f64;
        let mut snapshots: Vec<DailyHoldingSnapshot> = Vec::new();
        let mut has_any_price = false;

        for holding in &holdings {
            // Compute adjusted shares for this holding on this date:
            // current shares + (total_unwind - running_unwind) for this key.
            let key = (holding.account_id.clone(), holding.symbol.clone());
            let total_adj = total_unwind.get(&key).copied().unwrap_or(0.0);
            let running_adj = running_unwind.get(&key).copied().unwrap_or(0.0);
            let adjustment = total_adj - running_adj;
            let adjusted_shares = holding.shares + adjustment;

            // Skip holdings with no shares on this date
            if adjusted_shares.abs() < 1e-9 {
                continue;
            }
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

            let market_value = adjusted_shares * close_price;
            let cost = adjusted_shares * holding.avg_cost;

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
                shares: adjusted_shares,
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

        date_rows.push(DateRow {
            date_str,
            total_cost,
            total_value,
            us_cost,
            us_value,
            cn_cost,
            cn_value,
            hk_cost,
            hk_value,
            cumulative_pnl,
            snapshots,
        });
    }

    // 6. Batch-persist all computed rows inside a single SQLite transaction.
    //    This avoids per-statement fsync overhead and provides atomicity.
    {
        let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        for row in &date_rows {
            let prev_total_value: f64 = tx
                .query_row(
                    "SELECT COALESCE(total_value, 0) FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
                    rusqlite::params![row.date_str],
                    |r| r.get(0),
                )
                .unwrap_or(0.0);
            let daily_pnl = row.total_value - prev_total_value;

            tx.execute(
                "INSERT OR REPLACE INTO daily_portfolio_values
                 (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value, hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    row.date_str, row.total_cost, row.total_value,
                    row.us_cost, row.us_value, row.cn_cost, row.cn_value,
                    row.hk_cost, row.hk_value,
                    rates_json, daily_pnl, row.cumulative_pnl
                ],
            )
            .map_err(|e| e.to_string())?;

            tx.execute(
                "DELETE FROM daily_holding_snapshots WHERE date = ?1",
                rusqlite::params![row.date_str],
            )
            .map_err(|e| e.to_string())?;

            for snap in &row.snapshots {
                tx.execute(
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

            count += 1;
        }

        tx.commit().map_err(|e| e.to_string())?;
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
