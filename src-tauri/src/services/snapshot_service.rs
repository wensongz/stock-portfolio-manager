use crate::db::Database;
use crate::models::DailyHoldingSnapshot;
#[cfg(test)]
use crate::models::DailyPortfolioValue;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_provider_service;
#[cfg(test)]
use crate::services::quote_service::{
    fetch_quotes_batch_cached_with_providers, quote_key, QuoteCache,
};
use crate::services::quote_service::{fetch_stock_history, QuoteServiceState};
use chrono::{Datelike, NaiveDate, Timelike};
use tracing::{info, warn};

/// Number of calendar days to look back before the first missing date when
/// fetching historical prices.  This ensures that stocks suspended (停牌)
/// around the start of the backfill window still have a prior trading-day
/// close available for forward-fill.
const SUSPENSION_LOOKBACK_DAYS: i64 = 30;

#[cfg(test)]
fn quote_prices_by_identity(
    quotes: &[crate::models::StockQuote],
) -> std::collections::HashMap<(String, String), f64> {
    quotes
        .iter()
        .map(|quote| (quote_key(&quote.market, &quote.symbol), quote.current_price))
        .collect()
}

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
#[cfg(test)]
pub async fn take_daily_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    date: NaiveDate,
) -> Result<(), String> {
    let date_str = date.format("%Y-%m-%d").to_string();

    // 1. Load all holdings from DB (synchronous)
    let holdings = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT h.account_id, h.symbol, h.market,
                        h.shares, h.avg_cost, c.name as category_name
                 FROM holdings h
                 LEFT JOIN categories c ON h.category_id = c.id
                 WHERE ABS(h.shares) > 0.000000001
                    OR UPPER(h.symbol) LIKE '$CASH-%'",
            )
            .map_err(|e| e.to_string())?;

        #[derive(Debug)]
        struct HoldingRow {
            account_id: String,
            symbol: String,
            market: String,
            shares: f64,
            avg_cost: f64,
            category_name: Option<String>,
        }

        let rows = stmt
            .query_map([], |row| {
                Ok(HoldingRow {
                    account_id: row.get(0)?,
                    symbol: row.get(1)?,
                    market: row.get(2)?,
                    shares: row.get(3)?,
                    avg_cost: row.get(4)?,
                    category_name: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
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
        fetch_quotes_batch_cached_with_providers(
            quote_state,
            quote_cache,
            symbols,
            &config.us_provider,
            &config.hk_provider,
            &config.cn_provider,
            true,
        )
        .await?
        .data
    };
    let quote_map = quote_prices_by_identity(&quotes);

    // 3. Get exchange rates (async)
    let rates = crate::services::exchange_rate_service::get_cached_rates(cache, db).await?;

    // 4. Calculate per-holding snapshots and aggregate values
    let mut us_cost = 0.0f64;
    let mut us_value = 0.0f64;
    let mut cn_cost = 0.0f64;
    let mut cn_value = 0.0f64;
    let mut hk_cost = 0.0f64;
    let mut hk_value = 0.0f64;

    let mut snapshots: Vec<DailyHoldingSnapshot> = Vec::new();

    for holding in &holdings {
        let close_price = *quote_map
            .get(&quote_key(&holding.market, &holding.symbol))
            .unwrap_or(&0.0);
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

/// Query daily portfolio values in a date range.
#[cfg(test)]
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

type HoldingKey = (String, String);

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

#[derive(Debug, Clone)]
struct TxInfo {
    account_id: String,
    symbol: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    trade_date: NaiveDate,
}

#[derive(Debug, Default)]
struct SymbolPricePlan {
    active_symbols_by_date: std::collections::HashMap<NaiveDate, std::collections::HashSet<String>>,
    fetch_windows: std::collections::HashMap<String, (NaiveDate, NaiveDate)>,
}

impl SymbolPricePlan {
    fn is_active(&self, date: NaiveDate, symbol: &str) -> bool {
        self.active_symbols_by_date
            .get(&date)
            .is_some_and(|symbols| symbols.contains(symbol))
    }
}

/// Add the share/cash delta that reverses one transaction from the current
/// holdings state. Backfill uses these deltas to reconstruct each historical
/// end-of-day position.
#[allow(clippy::too_many_arguments)]
fn accumulate_transaction_unwind(
    unwind: &mut std::collections::HashMap<HoldingKey, f64>,
    account_id: &str,
    symbol: &str,
    transaction_type: &str,
    shares: f64,
    total_amount: f64,
    commission: f64,
    currency: &str,
) {
    let key = (account_id.to_string(), symbol.to_string());
    if crate::services::quote_service::is_cash_symbol(symbol) {
        let cash_delta = match transaction_type {
            "BUY" => -(total_amount + commission),
            "SELL" => total_amount + commission,
            _ => 0.0,
        };
        *unwind.entry(key).or_insert(0.0) += cash_delta;
        return;
    }

    let cash_key = (
        account_id.to_string(),
        format!(
            "{}{}",
            crate::services::quote_service::CASH_SYMBOL_PREFIX,
            currency
        ),
    );
    match transaction_type {
        "OPEN" => {
            *unwind.entry(key).or_insert(0.0) -= shares;
        }
        "BUY" => {
            *unwind.entry(key).or_insert(0.0) -= shares;
            *unwind.entry(cash_key).or_insert(0.0) += total_amount + commission;
        }
        "SELL" => {
            *unwind.entry(key).or_insert(0.0) += shares;
            *unwind.entry(cash_key).or_insert(0.0) -= total_amount - commission;
        }
        "PAY" => {
            *unwind.entry(cash_key).or_insert(0.0) -= total_amount - commission;
        }
        _ => {}
    }
}

fn active_non_cash_symbols(
    holdings: &[HoldingRow],
    shares_by_key: &std::collections::HashMap<HoldingKey, f64>,
) -> std::collections::HashSet<String> {
    holdings
        .iter()
        .filter(|holding| {
            !crate::services::quote_service::is_cash_symbol(&holding.symbol)
                && shares_by_key
                    .get(&(holding.account_id.clone(), holding.symbol.clone()))
                    .copied()
                    .unwrap_or(0.0)
                    .abs()
                    >= 1e-9
        })
        .map(|holding| holding.symbol.clone())
        .collect()
}

/// Replay position changes before any network request so historical prices are
/// fetched and resolved only while a stock was actually held. Positions that
/// predate the requested range keep a lookback window for suspension/holiday
/// forward-fill; positions opened inside the range are capped at that date.
fn build_symbol_price_plan(
    holdings: &[HoldingRow],
    transactions: &[TxInfo],
    total_unwind: &std::collections::HashMap<HoldingKey, f64>,
    missing_dates: &[NaiveDate],
) -> SymbolPricePlan {
    let mut plan = SymbolPricePlan::default();
    if missing_dates.is_empty() {
        return plan;
    }

    // `current + total_unwind` is the position immediately before start_date;
    // each transaction's unwind is then reversed to move forward through time.
    let mut shares_by_key: std::collections::HashMap<HoldingKey, f64> = holdings
        .iter()
        .map(|holding| {
            let key = (holding.account_id.clone(), holding.symbol.clone());
            let shares = holding.shares + total_unwind.get(&key).copied().unwrap_or(0.0);
            (key, shares)
        })
        .collect();

    // None means the current active episode began before the loaded range, so
    // its exact acquisition date is unknown and the lookback must be retained.
    let mut episode_start: std::collections::HashMap<String, Option<NaiveDate>> =
        active_non_cash_symbols(holdings, &shares_by_key)
            .into_iter()
            .map(|symbol| (symbol, None))
            .collect();
    let mut transaction_index = 0usize;

    for date in missing_dates {
        while transaction_index < transactions.len()
            && transactions[transaction_index].trade_date <= *date
        {
            let transaction_date = transactions[transaction_index].trade_date;
            let active_before = active_non_cash_symbols(holdings, &shares_by_key);

            // Apply all same-day transactions before evaluating the end-of-day
            // holding state used by a daily portfolio snapshot.
            while transaction_index < transactions.len()
                && transactions[transaction_index].trade_date == transaction_date
            {
                let tx = &transactions[transaction_index];
                let mut transaction_unwind = std::collections::HashMap::new();
                accumulate_transaction_unwind(
                    &mut transaction_unwind,
                    &tx.account_id,
                    &tx.symbol,
                    &tx.transaction_type,
                    tx.shares,
                    tx.total_amount,
                    tx.commission,
                    &tx.currency,
                );
                for (key, reverse_delta) in transaction_unwind {
                    *shares_by_key.entry(key).or_insert(0.0) -= reverse_delta;
                }
                transaction_index += 1;
            }

            let active_after = active_non_cash_symbols(holdings, &shares_by_key);
            for symbol in active_after.difference(&active_before) {
                episode_start.insert(symbol.clone(), Some(transaction_date));
            }
            for symbol in active_before.difference(&active_after) {
                episode_start.remove(symbol);
            }
        }

        let active_symbols = active_non_cash_symbols(holdings, &shares_by_key);
        for symbol in &active_symbols {
            let lookback_start = *date - chrono::Duration::days(SUSPENSION_LOOKBACK_DAYS);
            let fetch_start = match episode_start.get(symbol) {
                Some(Some(start)) => (*start).max(lookback_start),
                _ => lookback_start,
            };
            plan.fetch_windows
                .entry(symbol.clone())
                .and_modify(|window| {
                    window.0 = window.0.min(fetch_start);
                    window.1 = window.1.max(*date);
                })
                .or_insert((fetch_start, *date));
        }
        plan.active_symbols_by_date.insert(*date, active_symbols);
    }

    plan
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
    quote_state: &QuoteServiceState,
    start_date: NaiveDate,
    end_date: NaiveDate,
    force: bool,
) -> Result<i32, String> {
    backfill_snapshots_with_fetcher(
        db,
        cache,
        start_date,
        end_date,
        force,
        |symbol, market, fetch_start, fetch_end, provider| async move {
            fetch_stock_history(
                quote_state,
                &symbol,
                &market,
                fetch_start,
                fetch_end,
                &provider,
            )
            .await
        },
    )
    .await
}

async fn backfill_snapshots_with_fetcher<Fetch, FetchFuture>(
    db: &Database,
    cache: &ExchangeRateCache,
    start_date: NaiveDate,
    end_date: NaiveDate,
    force: bool,
    fetch_history: Fetch,
) -> Result<i32, String>
where
    Fetch: Fn(String, String, NaiveDate, NaiveDate, String) -> FetchFuture,
    FetchFuture: std::future::Future<Output = Result<Vec<(NaiveDate, f64)>, String>>,
{
    // Clamp end_date to the last date for which closing prices are
    // available.  Before CN/HK market close (≈15:00 UTC+8), today's
    // prices do not exist yet, so we use yesterday.
    let latest_closed = last_closed_market_date();
    let end_date = if end_date > latest_closed {
        latest_closed
    } else {
        end_date
    };

    if start_date > end_date {
        return Ok(0);
    }

    // 1. Load all relevant holdings: current active ones PLUS any that had
    //    transactions in the backfill period (they may be sold now but were
    //    held on historical dates).
    let (holdings, input_revision) = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let revision = crate::services::snapshot_cache_service::current_revision(&conn)?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT h.id, h.account_id, h.symbol, h.name, h.market,
                        h.shares, h.avg_cost, h.currency, c.name as category_name
                 FROM holdings h
                 LEFT JOIN categories c ON h.category_id = c.id
                 WHERE ABS(h.shares) > 0.000000001
                    OR UPPER(h.symbol) LIKE '$CASH-%'
                    OR EXISTS (
                        SELECT 1 FROM transactions t
                        WHERE t.account_id = h.account_id
                          AND t.symbol = h.symbol
                          AND DATE(t.traded_at) >= ?1
                    )",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(rusqlite::params![start_str], |row| {
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
        let holdings = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        (holdings, revision)
    };

    if holdings.is_empty() {
        if force {
            let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
            let tx = conn.transaction().map_err(|error| error.to_string())?;
            if crate::services::snapshot_cache_service::current_revision(&tx)? != input_revision {
                return Err("交易或持仓已变更，请重新刷新业绩数据".to_string());
            }
            let start = start_date.format("%Y-%m-%d").to_string();
            let end = end_date.format("%Y-%m-%d").to_string();
            tx.execute(
                "DELETE FROM daily_holding_snapshots WHERE date BETWEEN ?1 AND ?2",
                rusqlite::params![start, end],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "DELETE FROM daily_portfolio_values WHERE date BETWEEN ?1 AND ?2",
                rusqlite::params![start, end],
            )
            .map_err(|error| error.to_string())?;
            tx.commit().map_err(|error| error.to_string())?;
        }
        return Ok(0);
    }

    // 1b. Load transactions from start_date onwards so we can reconstruct
    //     historical holdings by unwinding future transactions.
    let transactions: Vec<TxInfo> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT account_id, symbol, transaction_type, shares, price,
                        total_amount, commission, currency, DATE(traded_at) as trade_date
                 FROM transactions
                 WHERE DATE(traded_at) >= ?1
                 ORDER BY traded_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![start_str], |row| {
                let td_str: String = row.get(8)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, String>(7)?,
                    td_str,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter()
            .filter_map(|(aid, sym, tt, sh, price, ta, com, cur, ds)| {
                NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
                    .ok()
                    .map(|td| TxInfo {
                        account_id: aid,
                        symbol: sym,
                        transaction_type: tt,
                        shares: sh,
                        price,
                        total_amount: ta,
                        commission: com,
                        currency: cur,
                        trade_date: td,
                    })
            })
            .collect()
    };

    // Seed pre-range acquisition prices so a newly allotted stock remains
    // valued correctly even when the requested performance window begins
    // after its allotment date but before its first market close.
    let initial_transaction_prices: std::collections::HashMap<HoldingKey, f64> = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare(
                "SELECT account_id, symbol, price
                 FROM transactions
                 WHERE DATE(traded_at) < ?1
                   AND UPPER(transaction_type) IN ('BUY', 'OPEN')
                   AND price > 0
                 ORDER BY traded_at ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![start_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut prices = std::collections::HashMap::new();
        for row in rows {
            let (account_id, symbol, price) = row.map_err(|error| error.to_string())?;
            if !crate::services::quote_service::is_cash_symbol(&symbol) {
                prices.insert((account_id, symbol), price);
            }
        }
        prices
    };

    // Pre-compute the TOTAL unwind delta across ALL loaded transactions.
    // For a given date D, the adjustment = total_unwind - running_unwind(up to D)
    // gives the unwind of all transactions AFTER D, yielding the shares at D.
    let mut total_unwind: std::collections::HashMap<HoldingKey, f64> =
        std::collections::HashMap::new();
    for tx in &transactions {
        accumulate_transaction_unwind(
            &mut total_unwind,
            &tx.account_id,
            &tx.symbol,
            &tx.transaction_type,
            tx.shares,
            tx.total_amount,
            tx.commission,
            &tx.currency,
        );
    }

    // 2. Find all weekdays in range that are missing snapshots
    let existing_dates: std::collections::HashSet<String> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let end_str = end_date.format("%Y-%m-%d").to_string();
        let mut stmt = conn
            .prepare("SELECT date FROM daily_portfolio_values WHERE date BETWEEN ?1 AND ?2")
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
    // When `force` is true, re-create all snapshots even when no transactions
    // remain in the period. Existing positions can still need repricing, and
    // deleting the last transaction can leave a stale cached value.
    // When `force` is false, only fill dates that have never been calculated.
    // This lets the UI
    // load quickly from cached data without re-fetching historical prices.
    let mut d = start_date;
    while d <= end_date {
        let wd = d.weekday();
        if wd != chrono::Weekday::Sat && wd != chrono::Weekday::Sun {
            let ds = d.format("%Y-%m-%d").to_string();
            if !existing_dates.contains(&ds) || force {
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

    // Reconstruct symbol activity before hitting the quote provider. This
    // prevents querying or warning about dates before a position was opened,
    // while retaining lookback data for positions held before the range.
    let price_plan =
        build_symbol_price_plan(&holdings, &transactions, &total_unwind, &missing_dates);

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
            let mut cash_prices = std::collections::HashMap::with_capacity(missing_dates.len());
            for d in &missing_dates {
                cash_prices.insert(*d, 1.0);
            }
            history_map.insert(symbol.clone(), cash_prices);
            continue;
        }

        let Some((fetch_start, fetch_end)) = price_plan.fetch_windows.get(symbol).copied() else {
            continue;
        };

        // Select the configured provider for the holding's market.
        let provider = match market.as_str() {
            "US" => config.us_provider.as_str(),
            "HK" => config.hk_provider.as_str(),
            _ => config.cn_provider.as_str(),
        };

        match fetch_history(
            symbol.clone(),
            market.clone(),
            fetch_start,
            fetch_end,
            provider.to_string(),
        )
        .await
        {
            Ok(prices) => {
                let date_price_map: std::collections::HashMap<NaiveDate, f64> =
                    prices.into_iter().collect();
                history_map.insert(symbol.clone(), date_price_map);
            }
            Err(e) => {
                return Err(format!("获取 {} ({}) 历史行情失败: {}", symbol, market, e));
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
    let rates = crate::services::exchange_rate_service::get_cached_rates(cache, db).await?;
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
    let mut running_unwind: std::collections::HashMap<HoldingKey, f64> =
        std::collections::HashMap::new();
    let mut transaction_prices = initial_transaction_prices;
    let mut logged_transaction_price_fallbacks: std::collections::HashSet<HoldingKey> =
        std::collections::HashSet::new();

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
    let mut empty_dates: Vec<String> = Vec::new();

    for date in &missing_dates {
        let date_str = date.format("%Y-%m-%d").to_string();

        // Advance running_unwind past transactions on or before this date.
        while txn_idx < transactions.len() && transactions[txn_idx].trade_date <= *date {
            let tx = &transactions[txn_idx];
            if matches!(tx.transaction_type.as_str(), "BUY" | "OPEN")
                && tx.price > 0.0
                && !crate::services::quote_service::is_cash_symbol(&tx.symbol)
            {
                transaction_prices.insert((tx.account_id.clone(), tx.symbol.clone()), tx.price);
            }
            accumulate_transaction_unwind(
                &mut running_unwind,
                &tx.account_id,
                &tx.symbol,
                &tx.transaction_type,
                tx.shares,
                tx.total_amount,
                tx.commission,
                &tx.currency,
            );
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

        // Pre-resolve the closing price for each unique symbol on this date.
        // This avoids redundant forward_fill_price calls when the same stock
        // is held in multiple accounts, and naturally deduplicates warnings.
        let mut resolved_prices: std::collections::HashMap<String, Option<f64>> =
            std::collections::HashMap::new();
        for (symbol, _market) in &unique_symbols {
            if !crate::services::quote_service::is_cash_symbol(symbol)
                && !price_plan.is_active(*date, symbol)
            {
                continue;
            }
            let price = history_map.get(symbol).and_then(|date_map| {
                history_sorted
                    .get(symbol)
                    .and_then(|sorted| forward_fill_price(date_map, sorted, date))
            });
            resolved_prices.insert(symbol.clone(), price);
        }

        let mut warned_missing_symbols = std::collections::HashSet::new();

        for holding in &holdings {
            // Compute adjusted shares for this holding on this date:
            // current shares + (total_unwind - running_unwind) for this key.
            let key = (holding.account_id.clone(), holding.symbol.clone());
            let total_adj = total_unwind.get(&key).copied().unwrap_or(0.0);
            let running_adj = running_unwind.get(&key).copied().unwrap_or(0.0);
            let adjustment = total_adj - running_adj;
            let adjusted_shares = holding.shares + adjustment;

            // Skip holdings with no shares on this date
            if adjusted_shares.abs() < 1e-9
                && !crate::services::quote_service::is_cash_symbol(&holding.symbol)
            {
                continue;
            }
            // Prefer an actual market close. For a newly allotted stock whose
            // first trading day has not arrived, retain its transaction price
            // so the cash-to-stock conversion is performance-neutral.
            let market_price = resolved_prices.get(&holding.symbol).copied().flatten();
            let transaction_price = transaction_prices.get(&key).copied();
            let close_price = market_price.or(transaction_price).unwrap_or_else(|| {
                if warned_missing_symbols.insert(holding.symbol.clone()) {
                    warn!(
                        "no historical or transaction price for {} ({}) on {}",
                        holding.symbol, holding.market, date_str
                    );
                }
                0.0
            });

            if market_price.is_none()
                && transaction_price.is_some()
                && logged_transaction_price_fallbacks.insert(key.clone())
            {
                info!(
                    "using transaction price {} for {} ({}) from {} until market history becomes available",
                    close_price, holding.symbol, holding.market, date_str
                );
            }

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

        // A date before the portfolio's first position has no holdings to
        // value. Remove its old cache, but preserve cached dates that still
        // have positions and merely lack usable prices.
        if snapshots.is_empty() {
            empty_dates.push(date_str);
            continue;
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
        // Holdings and transactions were read before asynchronous price/rate
        // requests. Reject an obsolete calculation before it can recreate
        // snapshots invalidated by a transaction or holding change.
        if crate::services::snapshot_cache_service::current_revision(&tx)? != input_revision {
            return Err("交易或持仓已变更，请重新刷新业绩数据".to_string());
        }

        for date in &empty_dates {
            tx.execute(
                "DELETE FROM daily_holding_snapshots WHERE date = ?1",
                [date],
            )
            .map_err(|error| error.to_string())?;
            tx.execute("DELETE FROM daily_portfolio_values WHERE date = ?1", [date])
                .map_err(|error| error.to_string())?;
        }

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
#[path = "snapshot_refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestLogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for TestLogWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    const IPO_XUEQIU_HISTORY_BODY: &str = r#"{
      "data": {
        "symbol": "SZ001248",
        "column": [
          "timestamp", "volume", "open", "high", "low", "close",
          "chg", "percent", "turnoverrate", "amount",
          "volume_post", "amount_post"
        ],
        "item": [
          [1782921600000, 721697718, 21.6, 30.16, 21.6, 23.95,
           13.84, 136.89, 67.93, 17692297780.0, null, null]
        ]
      },
      "error_code": 0,
      "error_description": ""
    }"#;

    async fn fetch_ipo_history_fixture(
        symbol: String,
        market: String,
        start_date: NaiveDate,
        end_date: NaiveDate,
        eastmoney_fallbacks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        yahoo_fallbacks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Vec<(NaiveDate, f64)>, String> {
        let outcome = crate::services::quote_service::parse_xueqiu_history_response(
            IPO_XUEQIU_HISTORY_BODY,
            &symbol,
            &market,
            start_date,
            end_date,
            "https://example.test/kline",
        )?;
        crate::services::quote_service::resolve_xueqiu_history_outcome(
            &symbol,
            &market,
            Ok(outcome),
            move || async move {
                eastmoney_fallbacks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(Vec::new())
            },
            move || async move {
                yahoo_fallbacks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(Vec::new())
            },
        )
        .await
    }

    #[test]
    fn test_price_plan_starts_at_first_purchase_and_skips_pre_purchase_dates() {
        let purchase_date = NaiveDate::from_ymd_opt(2026, 6, 24).unwrap();
        let may_22 = NaiveDate::from_ymd_opt(2026, 5, 22).unwrap();
        let may_25 = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        let june_25 = NaiveDate::from_ymd_opt(2026, 6, 25).unwrap();
        let holdings = vec![HoldingRow {
            _id: "holding-cn".to_string(),
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            _name: "华菱线缆".to_string(),
            market: "CN".to_string(),
            shares: 100.0,
            avg_cost: 10.0,
            _currency: "CNY".to_string(),
            category_name: None,
        }];
        let transactions = vec![TxInfo {
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            transaction_type: "BUY".to_string(),
            shares: 100.0,
            price: 10.0,
            total_amount: 1_000.0,
            commission: 0.0,
            currency: "CNY".to_string(),
            trade_date: purchase_date,
        }];
        let mut total_unwind = std::collections::HashMap::new();
        accumulate_transaction_unwind(
            &mut total_unwind,
            "acct-cn",
            "sz001248",
            "BUY",
            100.0,
            1_000.0,
            0.0,
            "CNY",
        );
        let missing_dates = vec![may_22, may_25, purchase_date, june_25];

        let plan = build_symbol_price_plan(&holdings, &transactions, &total_unwind, &missing_dates);

        assert!(!plan.is_active(may_22, "sz001248"));
        assert!(!plan.is_active(may_25, "sz001248"));
        assert!(plan.is_active(purchase_date, "sz001248"));
        assert_eq!(
            plan.fetch_windows.get("sz001248"),
            Some(&(purchase_date, june_25))
        );
    }

    #[test]
    fn test_price_plan_keeps_lookback_for_position_held_before_window() {
        let may_22 = NaiveDate::from_ymd_opt(2026, 5, 22).unwrap();
        let may_25 = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        let holdings = vec![HoldingRow {
            _id: "holding-cn".to_string(),
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            _name: "华菱线缆".to_string(),
            market: "CN".to_string(),
            shares: 100.0,
            avg_cost: 10.0,
            _currency: "CNY".to_string(),
            category_name: None,
        }];

        let plan = build_symbol_price_plan(
            &holdings,
            &[],
            &std::collections::HashMap::new(),
            &[may_22, may_25],
        );

        assert!(plan.is_active(may_22, "sz001248"));
        assert_eq!(
            plan.fetch_windows.get("sz001248"),
            Some(&(
                may_22 - chrono::Duration::days(SUSPENSION_LOOKBACK_DAYS),
                may_25,
            ))
        );
    }

    #[test]
    fn test_price_plan_caps_sparse_missing_date_fetch_to_lookback_window() {
        let purchase_date = NaiveDate::from_ymd_opt(2026, 6, 24).unwrap();
        let missing_date = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
        let holdings = vec![HoldingRow {
            _id: "holding-cn".to_string(),
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            _name: "华菱线缆".to_string(),
            market: "CN".to_string(),
            shares: 100.0,
            avg_cost: 10.0,
            _currency: "CNY".to_string(),
            category_name: None,
        }];
        let transactions = vec![TxInfo {
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            transaction_type: "BUY".to_string(),
            shares: 100.0,
            price: 10.0,
            total_amount: 1_000.0,
            commission: 0.0,
            currency: "CNY".to_string(),
            trade_date: purchase_date,
        }];
        let mut total_unwind = std::collections::HashMap::new();
        accumulate_transaction_unwind(
            &mut total_unwind,
            "acct-cn",
            "sz001248",
            "BUY",
            100.0,
            1_000.0,
            0.0,
            "CNY",
        );

        let plan =
            build_symbol_price_plan(&holdings, &transactions, &total_unwind, &[missing_date]);

        assert_eq!(
            plan.fetch_windows.get("sz001248"),
            Some(&(
                missing_date - chrono::Duration::days(SUSPENSION_LOOKBACK_DAYS),
                missing_date,
            ))
        );
    }

    #[tokio::test]
    async fn test_ipo_is_excluded_before_allotment_and_uses_issue_price_until_listing() {
        let db = Database::new(":memory:").unwrap();
        let rate_cache = ExchangeRateCache::new();
        rate_cache.set(crate::models::ExchangeRates {
            usd_cny: 1.0,
            usd_hkd: 1.0,
            cny_hkd: 1.0,
            updated_at: "2026-05-22".to_string(),
        });
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-cn', 'CN account', 'CN', NULL, '2026-05-22', '2026-06-24')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('stock-cn', 'acct-cn', 'sz001248', '华润新能源', 'CN', NULL,
                         100, 10.11, 'CNY', '2026-06-24', '2026-06-24')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('cash-cny', 'acct-cn', '$CASH-CNY', 'CNY Cash', 'CN', NULL,
                         0, 1, 'CNY', '2026-05-22', '2026-06-24')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('buy-stock', 'stock-cn', 'acct-cn', 'sz001248', '华润新能源', 'CN', 'BUY',
                         100, 10.11, 1011, 0, 'CNY', '2026-06-24T09:00:00Z', NULL,
                         '2026-06-24T09:00:00Z')",
                [],
            )
            .unwrap();
        }

        let start = NaiveDate::from_ymd_opt(2026, 5, 22).unwrap();
        let pre_buy_end = NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();
        let buy_date = NaiveDate::from_ymd_opt(2026, 6, 24).unwrap();
        let pre_listing_end = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let listing_date = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        let fetch_calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_for_fetcher = fetch_calls.clone();
        let eastmoney_fallbacks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let yahoo_fallbacks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let eastmoney_fallbacks_for_fetcher = eastmoney_fallbacks.clone();
        let yahoo_fallbacks_for_fetcher = yahoo_fallbacks.clone();
        let captured_logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let logs_for_writer = captured_logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || TestLogWriter(logs_for_writer.clone()))
            .finish();
        let subscriber_guard = tracing::subscriber::set_default(subscriber);

        backfill_snapshots_with_fetcher(
            &db,
            &rate_cache,
            start,
            pre_listing_end,
            false,
            move |symbol, market, fetch_start, fetch_end, provider| {
                let calls = calls_for_fetcher.clone();
                let eastmoney_fallbacks = eastmoney_fallbacks_for_fetcher.clone();
                let yahoo_fallbacks = yahoo_fallbacks_for_fetcher.clone();
                async move {
                    calls.lock().unwrap().push((
                        symbol.clone(),
                        market.clone(),
                        fetch_start,
                        fetch_end,
                        provider,
                    ));
                    fetch_ipo_history_fixture(
                        symbol,
                        market,
                        fetch_start,
                        fetch_end,
                        eastmoney_fallbacks,
                        yahoo_fallbacks,
                    )
                    .await
                }
            },
        )
        .await
        .unwrap();

        let calls_for_listing_fetcher = fetch_calls.clone();
        let eastmoney_fallbacks_for_listing = eastmoney_fallbacks.clone();
        let yahoo_fallbacks_for_listing = yahoo_fallbacks.clone();
        backfill_snapshots_with_fetcher(
            &db,
            &rate_cache,
            start,
            listing_date,
            false,
            move |symbol, market, fetch_start, fetch_end, provider| {
                let calls = calls_for_listing_fetcher.clone();
                let eastmoney_fallbacks = eastmoney_fallbacks_for_listing.clone();
                let yahoo_fallbacks = yahoo_fallbacks_for_listing.clone();
                async move {
                    calls.lock().unwrap().push((
                        symbol.clone(),
                        market.clone(),
                        fetch_start,
                        fetch_end,
                        provider,
                    ));
                    fetch_ipo_history_fixture(
                        symbol,
                        market,
                        fetch_start,
                        fetch_end,
                        eastmoney_fallbacks,
                        yahoo_fallbacks,
                    )
                    .await
                }
            },
        )
        .await
        .unwrap();
        drop(subscriber_guard);

        {
            let calls = fetch_calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].0, "sz001248");
            assert_eq!(calls[0].1, "CN");
            assert_eq!(calls[0].2, buy_date);
            assert_eq!(calls[0].3, pre_listing_end);
            assert_eq!(calls[1].0, "sz001248");
            assert_eq!(calls[1].1, "CN");
            assert_eq!(calls[1].2, buy_date);
            assert_eq!(calls[1].3, listing_date);
        }
        assert_eq!(
            eastmoney_fallbacks.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(yahoo_fallbacks.load(std::sync::atomic::Ordering::SeqCst), 0);
        let log_output = String::from_utf8(captured_logs.lock().unwrap().clone()).unwrap();
        assert!(!log_output.contains("no historical or transaction price for sz001248"));
        assert!(!log_output.contains("falling back to eastmoney"));
        assert!(log_output.contains("using transaction price 10.11 for sz001248"));

        let values = get_daily_values(&db, start, pre_buy_end).unwrap();
        assert_eq!(values.len(), 2);
        assert!(values
            .iter()
            .all(|value| (value.total_value - 1_011.0).abs() < 1e-9));

        let stock_snapshot_count: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM daily_holding_snapshots
                 WHERE symbol = 'sz001248' AND date < '2026-06-24'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(stock_snapshot_count, 0);

        let pre_listing_prices: Vec<(String, f64)> = {
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT date, close_price FROM daily_holding_snapshots
                     WHERE symbol = 'sz001248'
                       AND date BETWEEN '2026-06-24' AND '2026-07-01'
                     ORDER BY date",
                )
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(pre_listing_prices.len(), 6);
        assert!(pre_listing_prices
            .iter()
            .all(|(_, price)| (*price - 10.11).abs() < 1e-9));

        let listing_price: f64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT close_price FROM daily_holding_snapshots
                 WHERE symbol = 'sz001248' AND date = '2026-07-02'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!((listing_price - 23.95).abs() < 1e-9);

        let filter = crate::services::performance_service::PerformanceFilter::default();
        let summary = crate::services::performance_service::get_performance_summary(
            &db,
            start,
            pre_buy_end,
            &filter,
        )
        .unwrap();
        assert!(summary.total_pnl.abs() < 1e-9);
        assert!(summary.total_return.abs() < 1e-9);

        let attribution = crate::services::performance_service::get_return_attribution(
            &db,
            start,
            pre_buy_end,
            &filter,
        )
        .unwrap();
        assert!(attribution.by_holding.is_empty());
        assert!(attribution.total_pnl.abs() < 1e-9);

        let ranking = crate::services::performance_service::get_holding_performance_ranking(
            &db,
            start,
            pre_buy_end,
            "return_rate",
            10,
            &filter,
        )
        .unwrap();
        assert!(ranking.is_empty());
        let post_buy_summary = crate::services::performance_service::get_performance_summary(
            &db,
            buy_date,
            pre_listing_end,
            &filter,
        )
        .unwrap();
        assert!(post_buy_summary.total_pnl.abs() < 1e-9);
        assert!(post_buy_summary.total_return.abs() < 1e-9);

        let listing_summary = crate::services::performance_service::get_performance_summary(
            &db,
            pre_listing_end,
            listing_date,
            &filter,
        )
        .unwrap();
        assert!((listing_summary.total_pnl - 1_384.0).abs() < 1e-9);
        assert!((listing_summary.total_return - 136.894_164_193_867_46).abs() < 1e-9);

        let post_buy_attribution = crate::services::performance_service::get_return_attribution(
            &db,
            buy_date,
            listing_date,
            &filter,
        )
        .unwrap();
        assert_eq!(post_buy_attribution.by_holding.len(), 1);
        assert_eq!(
            post_buy_attribution.by_holding[0].name,
            "sz001248 华润新能源"
        );
        assert!((post_buy_attribution.by_holding[0].pnl - 1_384.0).abs() < 1e-9);

        let post_buy_ranking =
            crate::services::performance_service::get_holding_performance_ranking(
                &db,
                buy_date,
                listing_date,
                "return_rate",
                10,
                &filter,
            )
            .unwrap();
        assert_eq!(post_buy_ranking.len(), 1);
        assert_eq!(post_buy_ranking[0].symbol, "sz001248");
        assert!(post_buy_ranking[0].start_value.abs() < 1e-9);
        assert!((post_buy_ranking[0].end_value - 2_395.0).abs() < 1e-9);
        assert!((post_buy_ranking[0].pnl - 1_384.0).abs() < 1e-9);
        assert!((post_buy_ranking[0].return_rate - 136.894_164_193_867_46).abs() < 1e-9);

        // Rebuild a range that starts after the allotment date. The issue
        // price must still be available even though the BUY is before the
        // requested snapshot window.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("DELETE FROM daily_holding_snapshots", [])
                .unwrap();
            conn.execute("DELETE FROM daily_portfolio_values", [])
                .unwrap();
        }
        backfill_snapshots_with_fetcher(
            &db,
            &rate_cache,
            pre_listing_end,
            pre_listing_end,
            false,
            |_symbol, _market, _fetch_start, _fetch_end, _provider| async { Ok(Vec::new()) },
        )
        .await
        .unwrap();
        let pre_listing_price_after_late_start: f64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT close_price FROM daily_holding_snapshots
                 WHERE symbol = 'sz001248' AND date = '2026-07-01'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!((pre_listing_price_after_late_start - 10.11).abs() < 1e-9);
    }

    #[test]
    fn test_transaction_unwind_matches_stock_open_buy_sell_and_pay_cash_semantics() {
        let mut unwind = std::collections::HashMap::new();

        accumulate_transaction_unwind(
            &mut unwind,
            "acct-us",
            "AAPL",
            "OPEN",
            2.0,
            200.0,
            1.0,
            "USD",
        );
        accumulate_transaction_unwind(
            &mut unwind,
            "acct-us",
            "AAPL",
            "BUY",
            1.0,
            110.0,
            2.0,
            "USD",
        );
        accumulate_transaction_unwind(
            &mut unwind,
            "acct-us",
            "AAPL",
            "SELL",
            0.5,
            60.0,
            1.0,
            "USD",
        );
        accumulate_transaction_unwind(&mut unwind, "acct-us", "AAPL", "PAY", 0.0, 10.0, 1.0, "USD");

        // Reverse OPEN (-2), BUY (-1), and SELL (+0.5).
        assert!((unwind[&("acct-us".to_string(), "AAPL".to_string())] + 2.5).abs() < 1e-9);
        // Reverse BUY cash (+112), SELL cash (-59), and net dividend cash (-9).
        assert!((unwind[&("acct-us".to_string(), "$CASH-USD".to_string())] - 44.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_backfill_and_performance_neutralize_cash_deposits_and_withdrawals() {
        let db = Database::new(":memory:").unwrap();
        let rate_cache = ExchangeRateCache::new();
        rate_cache.set(crate::models::ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: "2024-01-01".to_string(),
        });
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-03')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('cash-usd', 'acct-us', '$CASH-USD', 'USD Cash', 'US', NULL,
                         130, 1, 'USD', '2024-01-01', '2024-01-03')",
                [],
            )
            .unwrap();
            for (id, transaction_type, amount, commission, traded_at) in [
                ("initial-deposit", "BUY", 100.0, 0.0, "2024-01-01T09:00:00Z"),
                ("deposit", "BUY", 50.0, 1.0, "2024-01-02T09:00:00Z"),
                ("withdrawal", "SELL", 20.0, 1.0, "2024-01-03T09:00:00Z"),
            ] {
                conn.execute(
                    "INSERT INTO transactions
                     (id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, 'cash-usd', 'acct-us', '$CASH-USD', 'USD Cash', 'US', ?2,
                             0, 1, ?3, ?4, 'USD', ?5, NULL, ?5)",
                    rusqlite::params![id, transaction_type, amount, commission, traded_at],
                )
                .unwrap();
            }
        }

        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let quote_state = QuoteServiceState::new();
        backfill_snapshots(&db, &rate_cache, &quote_state, start, end, false)
            .await
            .unwrap();

        let values = get_daily_values(&db, start, end).unwrap();
        assert_eq!(values.len(), 3);
        assert!((values[0].total_value - 100.0).abs() < 1e-9);
        assert!((values[1].total_value - 151.0).abs() < 1e-9);
        assert!((values[2].total_value - 130.0).abs() < 1e-9);

        let summary = crate::services::performance_service::get_performance_summary(
            &db,
            NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
            end,
            &crate::services::performance_service::PerformanceFilter::default(),
        )
        .unwrap();
        assert!(summary.total_return.abs() < 1e-9);
        assert!(summary.total_pnl.abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_backfill_preserves_terminal_zero_after_full_withdrawal() {
        let db = Database::new(":memory:").unwrap();
        let rate_cache = ExchangeRateCache::new();
        rate_cache.set(crate::models::ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: "2024-01-01".to_string(),
        });
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-02')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('cash-usd', 'acct-us', '$CASH-USD', 'USD Cash', 'US', NULL,
                         0, 1, 'USD', '2024-01-01', '2024-01-02')",
                [],
            )
            .unwrap();
            for (id, transaction_type, traded_at) in [
                ("deposit", "BUY", "2024-01-01T09:00:00Z"),
                ("full-withdrawal", "SELL", "2024-01-02T09:00:00Z"),
            ] {
                conn.execute(
                    "INSERT INTO transactions
                     (id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, 'cash-usd', 'acct-us', '$CASH-USD', 'USD Cash', 'US', ?2,
                             0, 1, 100, 0, 'USD', ?3, NULL, ?3)",
                    rusqlite::params![id, transaction_type, traded_at],
                )
                .unwrap();
            }
        }

        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
        let quote_state = QuoteServiceState::new();
        backfill_snapshots(&db, &rate_cache, &quote_state, start, end, false)
            .await
            .unwrap();

        let values = get_daily_values(&db, start, end).unwrap();
        assert_eq!(values.len(), 2);
        assert!((values[0].total_value - 100.0).abs() < 1e-9);
        assert!(values[1].total_value.abs() < 1e-9);

        let summary = crate::services::performance_service::get_performance_summary(
            &db,
            end,
            end,
            &crate::services::performance_service::PerformanceFilter::default(),
        )
        .unwrap();
        assert!(summary.total_return.abs() < 1e-9);
        assert!(summary.total_pnl.abs() < 1e-9);
        assert!(summary.end_value.abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_daily_snapshot_includes_negative_cash_balance() {
        let db = Database::new(":memory:").unwrap();
        let rate_cache = ExchangeRateCache::new();
        rate_cache.set(crate::models::ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: "2024-01-02".to_string(),
        });
        let quote_cache = QuoteCache::new();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-02')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('cash-usd', 'acct-us', '$CASH-USD', 'USD Cash', 'US', NULL,
                         -50, 1, 'USD', '2024-01-01', '2024-01-02')",
                [],
            )
            .unwrap();
        }

        let date = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
        let quote_state = QuoteServiceState::new();
        take_daily_snapshot(&db, &rate_cache, &quote_cache, &quote_state, date)
            .await
            .unwrap();

        let values = get_daily_values(&db, date, date).unwrap();
        assert_eq!(values.len(), 1);
        assert!((values[0].total_value - (-50.0)).abs() < 1e-9);
    }

    #[test]
    fn daily_snapshot_quote_prices_keep_same_literal_symbol_in_each_market() {
        let prices = quote_prices_by_identity(&[
            crate::models::StockQuote {
                symbol: "0700".to_string(),
                market: "US".to_string(),
                current_price: 7.0,
                ..Default::default()
            },
            crate::models::StockQuote {
                symbol: "0700".to_string(),
                market: "HK".to_string(),
                current_price: 700.0,
                ..Default::default()
            },
        ]);

        assert_eq!(prices.get(&quote_key("US", "0700")), Some(&7.0));
        assert_eq!(prices.get(&quote_key("HK", "0700")), Some(&700.0));
    }

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
        let sorted: Vec<(NaiveDate, f64)> =
            vec![(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), 20.0)];
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
