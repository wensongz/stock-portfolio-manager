use crate::db::Database;
#[cfg(test)]
use crate::models::DailyPortfolioValue;
use crate::models::{DailyHoldingSnapshot, ExchangeRates};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_provider_service;
#[cfg(test)]
use crate::services::quote_service::{
    fetch_quotes_batch_cached_with_providers, quote_key, QuoteCache,
};
use crate::services::quote_service::{fetch_stock_history, QuoteServiceState};
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// A market's daily bar is dated in that market's timezone. Allow time after
/// the close for the daily bar to be published.
fn last_closed_market_date_at(market: &str, now: DateTime<Utc>) -> NaiveDate {
    let (timezone, cutoff_hour, cutoff_minute) = match market {
        "US" => (chrono_tz::America::New_York, 16, 30),
        "HK" => (chrono_tz::Asia::Hong_Kong, 16, 30),
        _ => (chrono_tz::Asia::Shanghai, 16, 0),
    };
    let local = now.with_timezone(&timezone);
    let today = local.date_naive();
    if (local.hour(), local.minute()) < (cutoff_hour, cutoff_minute) {
        today.pred_opt().unwrap_or(today)
    } else {
        today
    }
}

pub(crate) fn last_closed_market_date_for(market: &str) -> NaiveDate {
    last_closed_market_date_at(market, Utc::now())
}

pub fn last_closed_market_date() -> NaiveDate {
    last_closed_market_date_for("CN")
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
type PriceKey = (String, String); // market, symbol
type TransferMarks = HashMap<NaiveDate, HashSet<HoldingKey>>;

#[derive(Debug, Clone)]
struct HoldingRow {
    _id: String,
    account_id: String,
    symbol: String,
    _name: String,
    market: String,
    shares: f64,
    avg_cost: f64,
    currency: String,
    category_name: Option<String>,
}

#[derive(Debug, Clone)]
struct TxInfo {
    account_id: String,
    symbol: String,
    market: String,
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
    active_symbols_by_date: HashMap<NaiveDate, HashSet<PriceKey>>,
    fetch_windows: HashMap<PriceKey, (NaiveDate, NaiveDate)>,
}

impl SymbolPricePlan {
    #[cfg(test)]
    fn is_active(&self, date: NaiveDate, key: &PriceKey) -> bool {
        self.active_symbols_by_date
            .get(&date)
            .is_some_and(|symbols| symbols.contains(key))
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
        "OPEN" | "STOCK_IN" => {
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
        "STOCK_OUT" => {
            *unwind.entry(key).or_insert(0.0) += shares;
        }
        "PAY" => {
            *unwind.entry(cash_key).or_insert(0.0) -= total_amount - commission;
        }
        _ => {}
    }
}

fn active_non_cash_symbols(
    holdings: &[HoldingRow],
    shares_by_key: &HashMap<HoldingKey, f64>,
) -> HashSet<PriceKey> {
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
        .map(|holding| (holding.market.clone(), holding.symbol.clone()))
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
    let mut episode_start: HashMap<PriceKey, Option<NaiveDate>> =
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

fn mark_transfer_prices(
    plan: &mut SymbolPricePlan,
    transactions: &[TxInfo],
    missing_dates: &[NaiveDate],
) -> TransferMarks {
    let mut marks = TransferMarks::new();
    for tx in transactions {
        if crate::services::quote_service::is_cash_symbol(&tx.symbol)
            || !matches!(
                tx.transaction_type.as_str(),
                "OPEN" | "STOCK_IN" | "STOCK_OUT"
            )
        {
            continue;
        }
        // A weekend transfer is reflected in the next valuation, but still
        // needs quotes as of its own date for the external cash-flow mark.
        let valuation_index = missing_dates.partition_point(|date| *date < tx.trade_date);
        let Some(&valuation_date) = missing_dates.get(valuation_index) else {
            continue;
        };
        if valuation_date != tx.trade_date
            && !matches!(
                tx.trade_date.weekday(),
                chrono::Weekday::Sat | chrono::Weekday::Sun
            )
        {
            continue;
        }
        let key = (tx.market.clone(), tx.symbol.clone());
        marks
            .entry(valuation_date)
            .or_default()
            .insert((tx.account_id.clone(), tx.symbol.clone()));
        plan.active_symbols_by_date
            .get_mut(&valuation_date)
            .unwrap()
            .insert(key.clone());
        let start = tx.trade_date - chrono::Duration::days(SUSPENSION_LOOKBACK_DAYS);
        plan.fetch_windows
            .entry(key)
            .and_modify(|(first, last)| {
                *first = (*first).min(start);
                *last = (*last).max(valuation_date);
            })
            .or_insert((start, valuation_date));
    }
    marks
}

fn completed_valuation_date(
    end: NaiveDate,
    now: DateTime<Utc>,
    holdings: &[HoldingRow],
    transactions: &[TxInfo],
) -> NaiveDate {
    let close_dates: HashMap<_, _> = ["US", "CN", "HK"]
        .into_iter()
        .map(|market| (market, last_closed_market_date_at(market, now)))
        .collect();
    let earliest_close = *close_dates.values().min().unwrap();
    let mut date = end.min(close_dates["CN"]);
    while date > earliest_close {
        let mut future_unwind = HashMap::new();
        for tx in transactions
            .iter()
            .rev()
            .take_while(|tx| tx.trade_date > date)
        {
            accumulate_transaction_unwind(
                &mut future_unwind,
                &tx.account_id,
                &tx.symbol,
                &tx.transaction_type,
                tx.shares,
                tx.total_amount,
                tx.commission,
                &tx.currency,
            );
        }
        let unclosed_position = holdings.iter().any(|holding| {
            let key = (holding.account_id.clone(), holding.symbol.clone());
            !crate::services::quote_service::is_cash_symbol(&holding.symbol)
                && date > close_dates[holding.market.as_str()]
                && (holding.shares + future_unwind.get(&key).copied().unwrap_or_default()).abs()
                    >= 1e-9
        });
        let unclosed_transfer = transactions
            .iter()
            .rev()
            .skip_while(|tx| tx.trade_date > date)
            .take_while(|tx| tx.trade_date == date)
            .any(|tx| {
                date > close_dates[tx.market.as_str()]
                    && !crate::services::quote_service::is_cash_symbol(&tx.symbol)
                    && matches!(
                        tx.transaction_type.as_str(),
                        "OPEN" | "STOCK_IN" | "STOCK_OUT"
                    )
            });
        if !unclosed_position && !unclosed_transfer {
            break;
        }
        date = date.pred_opt().unwrap();
    }
    date
}

/// Backfill missing daily portfolio snapshots for the given date range.
/// Fetches historical closing prices from the configured providers, calculates portfolio
/// values for every missing weekday, and stores them in the database.
/// Returns the number of snapshots created.
///
/// Historical positions are reconstructed from holdings and transaction history.
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
    backfill_snapshots_as_of(
        db,
        cache,
        start_date,
        end_date,
        force,
        Utc::now(),
        fetch_history,
    )
    .await
}

async fn backfill_snapshots_as_of<Fetch, FetchFuture>(
    db: &Database,
    cache: &ExchangeRateCache,
    start_date: NaiveDate,
    requested_end: NaiveDate,
    force: bool,
    now: DateTime<Utc>,
    fetch_history: Fetch,
) -> Result<i32, String>
where
    Fetch: Fn(String, String, NaiveDate, NaiveDate, String) -> FetchFuture,
    FetchFuture: std::future::Future<Output = Result<Vec<(NaiveDate, f64)>, String>>,
{
    if start_date > requested_end {
        return Ok(0);
    }

    // 1. Load all relevant holdings: current active ones PLUS any that had
    //    transactions in the backfill period (they may be sold now but were
    //    held on historical dates).
    let (holdings, cash_opening_ledgers, account_inception, input_revision) = {
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
                    currency: row.get(7)?,
                    category_name: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let holdings = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        // An explicit cash OPEN is a dated balance reset. Replaying its ledger
        // prevents a corrected opening from leaking into earlier valuations.
        // Cash without an OPEN retains the existing current-balance anchor.
        let mut cash_opening_ledgers = std::collections::HashMap::new();
        for holding in &holdings {
            if !crate::services::quote_service::is_cash_symbol(&holding.symbol) {
                continue;
            }
            let has_opening: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM transactions
                     WHERE account_id = ?1 AND currency = ?2
                       AND UPPER(symbol) LIKE '$CASH-%' AND transaction_type = 'OPEN')",
                    rusqlite::params![holding.account_id, holding.currency],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if has_opening {
                let ledger = crate::services::cash_reconciliation_service::load_cash_ledger(
                    &conn,
                    &holding.account_id,
                    &holding.currency,
                )?;
                cash_opening_ledgers.insert(
                    (holding.account_id.clone(), holding.currency.clone()),
                    ledger,
                );
            }
        }
        // A broker account can be entered after its holdings or trades were
        // imported. Use the earliest known date from those records together.
        let mut stmt = conn
            .prepare(
                "WITH origins(account_id, date) AS (
                   SELECT id, DATE(created_at) FROM accounts
                   UNION ALL SELECT account_id, DATE(created_at) FROM holdings
                   UNION ALL SELECT account_id, DATE(traded_at) FROM transactions
                 ) SELECT account_id, MIN(date) FROM origins WHERE date IS NOT NULL GROUP BY account_id",
            )
            .map_err(|error| error.to_string())?;
        let account_inception = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(|error| error.to_string())?;
        (holdings, cash_opening_ledgers, account_inception, revision)
    };

    if holdings.is_empty() {
        if force {
            let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
            let tx = conn.transaction().map_err(|error| error.to_string())?;
            if crate::services::snapshot_cache_service::current_revision(&tx)? != input_revision {
                return Err("交易或持仓已变更，请重新刷新业绩数据".to_string());
            }
            let start = start_date.format("%Y-%m-%d").to_string();
            let end = requested_end.format("%Y-%m-%d").to_string();
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
                        total_amount, commission, currency, DATE(traded_at) as trade_date, market
                 FROM transactions
                 WHERE DATE(traded_at) >= ?1
                 ORDER BY JULIANDAY(traded_at), JULIANDAY(created_at), id",
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
                    row.get::<_, String>(9)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter()
            .filter_map(|(aid, sym, tt, sh, price, ta, com, cur, ds, market)| {
                NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
                    .ok()
                    .map(|td| TxInfo {
                        account_id: aid,
                        symbol: sym,
                        market,
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

    let end_date = completed_valuation_date(requested_end, now, &holdings, &transactions);
    if end_date < requested_end {
        // Older versions could have written a previous close under today's
        // still-open US session. Do not leave that row visible in the report.
        let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        if crate::services::snapshot_cache_service::current_revision(&tx)? != input_revision {
            return Err("交易或持仓已变更，请重新刷新业绩数据".to_string());
        }
        let first_incomplete = end_date.succ_opt().unwrap().max(start_date).to_string();
        let last_requested = requested_end.to_string();
        for table in ["daily_holding_snapshots", "daily_portfolio_values"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE date BETWEEN ?1 AND ?2"),
                rusqlite::params![first_incomplete, last_requested],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
    }
    if start_date > end_date {
        return Ok(0);
    }

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
                   AND UPPER(transaction_type) IN ('BUY', 'OPEN', 'STOCK_IN')
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

    let transfer_days: HashSet<_> = transactions
        .iter()
        .filter(|tx| {
            !crate::services::quote_service::is_cash_symbol(&tx.symbol)
                && matches!(
                    tx.transaction_type.as_str(),
                    "OPEN" | "STOCK_IN" | "STOCK_OUT"
                )
        })
        .map(|tx| tx.trade_date.to_string())
        .collect();
    // 2. Find all weekdays in range that need a new snapshot.
    let existing_dates: std::collections::HashSet<String> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let start_str = start_date.format("%Y-%m-%d").to_string();
        let end_str = end_date.format("%Y-%m-%d").to_string();
        let tail_start = (now.date_naive() - chrono::Duration::days(4)).to_string();
        let mut stmt = conn
            .prepare(
                "SELECT d.date, EXISTS (SELECT 1 FROM daily_holding_snapshots h
                   WHERE h.date = d.date AND h.has_market_close IS NULL)
                 FROM daily_portfolio_values d WHERE d.date BETWEEN ?1 AND ?2
                 AND NOT (d.date >= ?3 AND EXISTS (
                   SELECT 1 FROM daily_holding_snapshots h
                   WHERE h.date = d.date AND h.market = 'US'
                     AND UPPER(h.symbol) NOT LIKE '$CASH-%' AND h.has_market_close IS NOT 1
                 ))
                 AND NOT EXISTS (
                   SELECT 1 FROM daily_holding_snapshots h WHERE h.date = d.date
                     AND h.market <> CASE UPPER(h.symbol)
                       WHEN '$CASH-USD' THEN 'US' WHEN '$CASH-CNY' THEN 'CN'
                       WHEN '$CASH-HKD' THEN 'HK' ELSE h.market END
                 )
                 AND NOT EXISTS (
                   SELECT 1 FROM daily_holding_snapshots h
                   JOIN daily_holding_snapshots other ON other.date = h.date
                     AND other.symbol = h.symbol AND other.market <> h.market
                   WHERE h.date = d.date AND h.has_market_close IS NULL
                 )
                 AND NOT EXISTS (
                   SELECT 1 FROM historical_exchange_rates fx WHERE fx.rate_date <= d.date
                     AND fx.observed_at > COALESCE(CASE WHEN json_valid(d.exchange_rates) THEN
                       CASE WHEN DATE(json_extract(d.exchange_rates, '$.updated_at')) <= d.date
                         THEN STRFTIME('%Y-%m-%dT%H:%M:%fZ', json_extract(d.exchange_rates, '$.updated_at'))
                       END END, '')
                 )",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![start_str, end_str, tail_start], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(|row| match row {
            Ok((date, true)) if transfer_days.contains(&date) => None,
            Ok((date, _)) => Some(Ok(date)),
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())?
    };

    let mut missing_dates: Vec<NaiveDate> = Vec::new();
    // When `force` is true, re-create all snapshots even when no transactions
    // remain in the period. Existing positions can still need repricing, and
    // deleting the last transaction can leave a stale cached value.
    // Normal page loads reuse cached values that aren't missing or out of date.
    let earliest_inception = holdings
        .iter()
        .map(|holding| account_inception.get(&holding.account_id))
        .min()
        .flatten();
    let mut d = start_date;
    while d <= end_date {
        let wd = d.weekday();
        if wd != chrono::Weekday::Sat && wd != chrono::Weekday::Sun {
            let ds = d.format("%Y-%m-%d").to_string();
            if !existing_dates.contains(&ds)
                || force
                || earliest_inception.is_some_and(|first| &ds < first)
            {
                missing_dates.push(d);
            }
        }
        d = d.succ_opt().unwrap_or(d);
    }

    if missing_dates.is_empty() {
        return Ok(0);
    }

    // 3. Fetch historical prices for each holding using the configured provider
    let mut history_map: HashMap<PriceKey, HashMap<NaiveDate, f64>> = HashMap::new();

    let config = quote_provider_service::get_quote_provider_config(db)?;

    // Reconstruct symbol activity before hitting the quote provider. This
    // prevents querying or warning about dates before a position was opened,
    // while retaining lookback data for positions held before the range.
    let mut price_plan =
        build_symbol_price_plan(&holdings, &transactions, &total_unwind, &missing_dates);
    let transfer_marks = mark_transfer_prices(&mut price_plan, &transactions, &missing_dates);

    for (key, &(fetch_start, fetch_end)) in &price_plan.fetch_windows {
        let (market, symbol) = key;
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
                let prices: Vec<_> = prices
                    .into_iter()
                    .filter(|(date, close)| {
                        (fetch_start..=fetch_end).contains(date)
                            && close.is_finite()
                            && *close > 0.0
                    })
                    .collect();
                history_map.insert(key.clone(), prices.into_iter().collect());
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
    let history_sorted: HashMap<PriceKey, Vec<(NaiveDate, f64)>> = history_map
        .iter()
        .map(|(symbol, date_map)| {
            let mut sorted: Vec<(NaiveDate, f64)> =
                date_map.iter().map(|(d, p)| (*d, *p)).collect();
            sorted.sort_by_key(|(d, _)| *d);
            (symbol.clone(), sorted)
        })
        .collect();

    // Preserve the observed bar used for each transfer. Its session can be
    // outside the snapshot range, for example before a holiday or weekend.
    let mut transfer_closes: HashMap<PriceKey, BTreeMap<NaiveDate, f64>> = HashMap::new();
    for transfer in transactions.iter().filter(|tx| {
        (start_date..=end_date).contains(&tx.trade_date)
            && matches!(
                tx.transaction_type.as_str(),
                "OPEN" | "STOCK_IN" | "STOCK_OUT"
            )
    }) {
        let key = (transfer.market.clone(), transfer.symbol.clone());
        if let Some(prices) = history_sorted.get(&key) {
            let idx = prices.partition_point(|(date, _)| *date <= transfer.trade_date);
            if let Some(&(date, close)) = idx.checked_sub(1).and_then(|idx| prices.get(idx)) {
                transfer_closes.entry(key).or_default().insert(date, close);
            }
        }
    }
    for ((market, symbol), closes) in transfer_closes {
        crate::services::stock_operation_market_data::upsert_stock_closes(
            db,
            &symbol,
            &market,
            "portfolio_snapshot",
            &closes.into_iter().collect::<Vec<_>>(),
        )?;
    }

    let daily_rates = crate::services::historical_exchange_rate_service::load_daily_snapshot_rates(
        db, start_date, end_date,
    )?;
    let fallback_rates = if daily_rates.for_date(&end_date.to_string()).is_some() {
        None
    } else {
        let cached_rates = cache
            .get_stale()
            .or(crate::services::exchange_rate_service::load_exchange_rates_from_db(db)?);
        let uses_foreign_currency = holdings.iter().any(|holding| {
            holding.currency != "USD"
                || (!crate::services::quote_service::is_cash_symbol(&holding.symbol)
                    && holding.market != "US")
        });
        Some(match cached_rates {
            Some(rates) => rates,
            None if uses_foreign_currency => {
                crate::services::exchange_rate_service::get_cached_rates(cache, db).await?
            }
            // Only USD values will be converted when the portfolio has no
            // foreign assets or flows. Blank provenance excludes these unit
            // rates from the historical exchange rate archive.
            None => ExchangeRates {
                usd_cny: 1.0,
                usd_hkd: 1.0,
                cny_hkd: 1.0,
                updated_at: String::new(),
            },
        })
    };

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
        rates_json: String,
        snapshots: Vec<(DailyHoldingSnapshot, bool)>,
    }
    let mut date_rows: Vec<DateRow> = Vec::with_capacity(missing_dates.len());
    let mut empty_dates: Vec<String> = Vec::new();

    for date in &missing_dates {
        let date_str = date.format("%Y-%m-%d").to_string();

        // Advance running_unwind past transactions on or before this date.
        while txn_idx < transactions.len() && transactions[txn_idx].trade_date <= *date {
            let tx = &transactions[txn_idx];
            if matches!(tx.transaction_type.as_str(), "BUY" | "OPEN" | "STOCK_IN")
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
        let mut snapshots = Vec::new();
        let mut has_any_price = false;

        // Resolve each shared security once, with the market as part of the key.
        let resolved_prices: HashMap<_, _> = price_plan.active_symbols_by_date[date]
            .iter()
            .filter_map(|key| {
                let prices = history_map.get(key)?;
                let price = forward_fill_price(prices, &history_sorted[key], date)?;
                Some((key, (price, prices.contains_key(date))))
            })
            .collect();

        let mut warned_missing_symbols = std::collections::HashSet::new();

        for holding in &holdings {
            if account_inception
                .get(&holding.account_id)
                .is_some_and(|first| &date_str < first)
            {
                continue;
            }
            // Compute adjusted shares for this holding on this date:
            // current shares + (total_unwind - running_unwind) for this key.
            let key = (holding.account_id.clone(), holding.symbol.clone());
            let price_key = (holding.market.clone(), holding.symbol.clone());
            let is_cash = crate::services::quote_service::is_cash_symbol(&holding.symbol);
            let total_adj = total_unwind.get(&key).copied().unwrap_or(0.0);
            let running_adj = running_unwind.get(&key).copied().unwrap_or(0.0);
            let adjustment = total_adj - running_adj;
            let adjusted_shares = if is_cash {
                cash_opening_ledgers
                    .get(&(holding.account_id.clone(), holding.currency.clone()))
                    .and_then(|ledger| ledger.balance_at_date(*date))
                    .unwrap_or(holding.shares + adjustment)
            } else {
                holding.shares + adjustment
            };

            // Skip holdings with no shares on this date
            let has_transfer = transfer_marks
                .get(date)
                .is_some_and(|marks| marks.contains(&key));
            if adjusted_shares.abs() < 1e-9 && !is_cash && !has_transfer {
                continue;
            }
            // Prefer an actual market close. For a newly allotted stock whose
            // first trading day has not arrived, retain its transaction price
            // so the cash-to-stock conversion is performance-neutral.
            let observed = resolved_prices.get(&price_key).copied();
            let has_market_close = observed.is_some_and(|(_, exact)| exact);
            let market_price = if is_cash {
                Some(1.0)
            } else {
                observed.map(|(price, _)| price)
            };
            let transaction_price = transaction_prices.get(&key).copied();
            let close_price = market_price.or(transaction_price).unwrap_or_else(|| {
                if warned_missing_symbols.insert(price_key) {
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

            let market = crate::services::quote_service::holding_market(
                &holding.symbol,
                &holding.currency,
                &holding.market,
            );
            match market {
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

            snapshots.push((
                DailyHoldingSnapshot {
                    id: 0,
                    date: date_str.clone(),
                    account_id: holding.account_id.clone(),
                    symbol: holding.symbol.clone(),
                    market: market.to_string(),
                    category_name: holding.category_name.clone(),
                    shares: adjusted_shares,
                    avg_cost: holding.avg_cost,
                    close_price,
                    market_value,
                },
                has_market_close,
            ));
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

        let rates = daily_rates
            .for_date(&date_str)
            .or(fallback_rates.as_ref())
            .unwrap();
        let total_cost = us_cost
            + crate::services::exchange_rate_service::convert_currency(
                cn_cost, "CNY", "USD", rates,
            )
            + crate::services::exchange_rate_service::convert_currency(
                hk_cost, "HKD", "USD", rates,
            );
        let total_value = us_value
            + crate::services::exchange_rate_service::convert_currency(
                cn_value, "CNY", "USD", rates,
            )
            + crate::services::exchange_rate_service::convert_currency(
                hk_value, "HKD", "USD", rates,
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
            rates_json: serde_json::to_string(rates).map_err(|error| error.to_string())?,
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
                    row.rates_json, daily_pnl, row.cumulative_pnl
                ],
            )
            .map_err(|e| e.to_string())?;

            tx.execute(
                "DELETE FROM daily_holding_snapshots WHERE date = ?1",
                rusqlite::params![row.date_str],
            )
            .map_err(|e| e.to_string())?;

            for (snap, has_market_close) in &row.snapshots {
                tx.execute(
                    "INSERT INTO daily_holding_snapshots
                     (date, account_id, symbol, market, category_name, shares, avg_cost, close_price, market_value, has_market_close)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        snap.date, snap.account_id, snap.symbol, snap.market,
                        snap.category_name, snap.shares, snap.avg_cost,
                        snap.close_price, snap.market_value, has_market_close,
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
            currency: "CNY".to_string(),
            category_name: None,
        }];
        let transactions = vec![TxInfo {
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            market: "CN".to_string(),
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

        let key = ("CN".into(), "sz001248".into());
        assert!(!plan.is_active(may_22, &key));
        assert!(!plan.is_active(may_25, &key));
        assert!(plan.is_active(purchase_date, &key));
        assert_eq!(
            plan.fetch_windows.get(&key),
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
            currency: "CNY".to_string(),
            category_name: None,
        }];

        let plan = build_symbol_price_plan(
            &holdings,
            &[],
            &std::collections::HashMap::new(),
            &[may_22, may_25],
        );

        let key = ("CN".into(), "sz001248".into());
        assert!(plan.is_active(may_22, &key));
        assert_eq!(
            plan.fetch_windows.get(&key),
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
            currency: "CNY".to_string(),
            category_name: None,
        }];
        let transactions = vec![TxInfo {
            account_id: "acct-cn".to_string(),
            symbol: "sz001248".to_string(),
            market: "CN".to_string(),
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
            plan.fetch_windows.get(&("CN".into(), "sz001248".into())),
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
    fn stock_transfers_unwind_shares_without_cash() {
        let mut unwind = std::collections::HashMap::new();
        accumulate_transaction_unwind(
            &mut unwind,
            "a",
            "AAPL",
            "STOCK_IN",
            10.0,
            200.0,
            0.0,
            "USD",
        );
        accumulate_transaction_unwind(&mut unwind, "a", "AAPL", "STOCK_OUT", 4.0, 0.0, 0.0, "USD");
        assert_eq!(unwind[&("a".into(), "AAPL".into())], -6.0);
        assert!(!unwind.contains_key(&("a".into(), "$CASH-USD".into())));
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
