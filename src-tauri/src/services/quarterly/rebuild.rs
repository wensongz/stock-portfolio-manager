use crate::db::Database;
use crate::models::quarterly::QuarterlySnapshot;
use crate::models::quote::ExchangeRates;
use crate::services::exchange_rate_service::{
    convert_currency, get_cached_rates, ExchangeRateCache,
};
use crate::services::quote_provider_service;
use crate::services::quote_service::{
    fetch_quotes_batch_cached_with_providers, fetch_stock_history, QuoteCache, QuoteServiceState,
};
use chrono::{NaiveDate, Utc};
use rusqlite::OptionalExtension;
use std::collections::{BTreeMap, BTreeSet};

use super::{date_to_quarter, parse_quarter, quarter_end_date};

#[path = "cash.rs"]
mod cash;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct PositionKey {
    account_id: String,
    symbol: String,
    market: String,
}

impl PositionKey {
    fn new(account_id: &str, symbol: &str, market: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            symbol: symbol.to_uppercase(),
            market: market.to_string(),
        }
    }

    fn from_holding(holding: &WorkingHolding) -> Self {
        Self::new(&holding.account_id, &holding.symbol, &holding.market)
    }
}

#[derive(Debug, Clone)]
pub(super) struct WorkingHolding {
    pub(super) account_id: String,
    pub(super) account_name: String,
    pub(super) symbol: String,
    pub(super) name: String,
    pub(super) market: String,
    pub(super) currency: String,
    pub(super) category_name: String,
    pub(super) category_color: String,
    pub(super) shares: f64,
    pub(super) avg_cost: f64,
    pub(super) notes: Option<String>,
    pub(super) decision_quality: Option<String>,
}

#[derive(Debug)]
struct ReplayState {
    holding: WorkingHolding,
}

pub(super) fn load_historical_holdings(
    db: &Database,
    end_date: NaiveDate,
) -> Result<Vec<WorkingHolding>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let adjustments = BTreeMap::from([
        (
            "CN".to_string(),
            quote_provider_service::market_adjusts_sell_pay_cost(&conn, "CN"),
        ),
        (
            "HK".to_string(),
            quote_provider_service::market_adjusts_sell_pay_cost(&conn, "HK"),
        ),
        (
            "US".to_string(),
            quote_provider_service::market_adjusts_sell_pay_cost(&conn, "US"),
        ),
    ]);
    let mut stmt = conn
        .prepare(
            "SELECT t.account_id, COALESCE(a.name, ''), t.symbol, t.name, t.market,
                    COALESCE((
                        SELECT c.name
                        FROM holdings h
                        LEFT JOIN categories c ON c.id = h.category_id
                        WHERE h.account_id = t.account_id
                          AND UPPER(h.symbol) = UPPER(t.symbol)
                          AND h.market = t.market
                        LIMIT 1
                    ), '未分类'),
                    COALESCE((
                        SELECT c.color
                        FROM holdings h
                        LEFT JOIN categories c ON c.id = h.category_id
                        WHERE h.account_id = t.account_id
                          AND UPPER(h.symbol) = UPPER(t.symbol)
                          AND h.market = t.market
                        LIMIT 1
                    ), '#8B8B8B'),
                    t.transaction_type, t.shares, t.price, t.total_amount, t.commission,
                    t.currency
             FROM transactions t
             LEFT JOIN accounts a ON a.id = t.account_id
             WHERE DATE(t.traded_at) <= ?1
               AND UPPER(t.symbol) NOT LIKE '$CASH-%'
             ORDER BY JULIANDAY(t.traded_at) ASC, JULIANDAY(t.created_at) ASC, t.id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![end_date.format("%Y-%m-%d").to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, f64>(8)?,
                    row.get::<_, f64>(9)?,
                    row.get::<_, f64>(10)?,
                    row.get::<_, f64>(11)?,
                    row.get::<_, String>(12)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;

    let mut states: BTreeMap<PositionKey, ReplayState> = BTreeMap::new();
    for row in rows {
        let (
            account_id,
            account_name,
            symbol,
            name,
            market,
            category_name,
            category_color,
            transaction_type,
            shares,
            price,
            total_amount,
            commission,
            currency,
        ) = row.map_err(|error| error.to_string())?;
        if [shares, price, total_amount, commission]
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(format!(
                "non-finite historical transaction for {account_id}/{market}/{symbol}"
            ));
        }
        let key = PositionKey {
            account_id: account_id.clone(),
            symbol: symbol.to_uppercase(),
            market: market.clone(),
        };
        let state = states.entry(key).or_insert_with(|| ReplayState {
            holding: WorkingHolding {
                account_id,
                account_name,
                symbol,
                name: name.clone(),
                market: market.clone(),
                currency,
                category_name,
                category_color,
                shares: 0.0,
                avg_cost: 0.0,
                notes: None,
                decision_quality: None,
            },
        });
        state.holding.name = name;
        let adjust_cost = adjustments.get(&market).copied().unwrap_or(true);
        match transaction_type.as_str() {
            "OPEN" => {
                state.holding.shares = shares;
                state.holding.avg_cost = price;
            }
            "BUY" | "STOCK_IN" => {
                let new_shares = state.holding.shares + shares;
                if new_shares > 0.0 {
                    state.holding.avg_cost = (state.holding.shares * state.holding.avg_cost
                        + shares * price
                        + commission)
                        / new_shares;
                }
                state.holding.shares = new_shares;
            }
            "SELL" | "STOCK_OUT" => {
                let remaining = state.holding.shares - shares;
                if remaining < -1e-9 {
                    return Err(format!(
                        "negative historical position for {}/{}/{}",
                        state.holding.account_id, state.holding.market, state.holding.symbol
                    ));
                }
                if adjust_cost && transaction_type == "SELL" {
                    state.holding.avg_cost = if remaining > 1e-9 {
                        (state.holding.shares * state.holding.avg_cost - total_amount + commission)
                            / remaining
                    } else {
                        0.0
                    };
                }
                if transaction_type == "STOCK_OUT" && remaining <= 1e-9 {
                    state.holding.avg_cost = 0.0;
                }
                state.holding.shares = remaining.max(0.0);
            }
            "PAY" if adjust_cost && state.holding.shares > 0.0 => {
                state.holding.avg_cost = (state.holding.shares * state.holding.avg_cost
                    - (total_amount - commission))
                    / state.holding.shares;
            }
            "PAY" => {}
            other => {
                return Err(format!(
                    "unsupported historical transaction type {other} for {}/{}/{}",
                    state.holding.account_id, state.holding.market, state.holding.symbol
                ));
            }
        }
    }

    let mut holdings: Vec<_> = states
        .into_values()
        .map(|state| state.holding)
        .filter(|holding| holding.shares > 1e-9)
        .collect();
    holdings.extend(cash::load_cash_holdings(&conn, end_date)?);
    Ok(holdings)
}

fn load_historical_prices_from_db(
    db: &Database,
    holdings: &[WorkingHolding],
    end_date: NaiveDate,
) -> Result<BTreeMap<PositionKey, f64>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let end_date = end_date.format("%Y-%m-%d").to_string();
    let mut prices = BTreeMap::new();
    for holding in holdings {
        let key = PositionKey::from_holding(holding);
        if crate::services::quote_service::is_cash_symbol(&holding.symbol) {
            prices.insert(key, 1.0);
            continue;
        }
        let price = conn
            .query_row(
                "SELECT close_price
                 FROM daily_holding_snapshots
                 WHERE UPPER(symbol) = UPPER(?1)
                   AND market = ?2
                   AND date <= ?3
                   AND close_price > 0
                 ORDER BY date DESC
                 LIMIT 1",
                rusqlite::params![holding.symbol, holding.market, end_date],
                |row| row.get::<_, f64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(price) = price.filter(|price| price.is_finite() && *price > 0.0) {
            prices.insert(key, price);
        }
    }
    Ok(prices)
}

fn ensure_complete_prices(
    holdings: &[WorkingHolding],
    prices: &BTreeMap<PositionKey, f64>,
) -> Result<(), String> {
    let missing = holdings
        .iter()
        .filter(|holding| {
            prices
                .get(&PositionKey::from_holding(holding))
                .is_none_or(|price| !price.is_finite() || *price <= 0.0)
        })
        .map(|holding| {
            format!(
                "{}/{}/{}",
                holding.account_id, holding.market, holding.symbol
            )
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing closing price for {}", missing.join(", ")))
    }
}

fn validate_rates(rates: ExchangeRates, label: &str) -> Result<ExchangeRates, String> {
    if [rates.usd_cny, rates.usd_hkd, rates.cny_hkd]
        .iter()
        .all(|rate| rate.is_finite() && *rate > 0.0)
    {
        Ok(rates)
    } else {
        Err(format!(
            "invalid {label} exchange rates: expected positive finite values"
        ))
    }
}

#[cfg(test)]
fn load_historical_rates(db: &Database, end_date: NaiveDate) -> Result<ExchangeRates, String> {
    crate::services::historical_exchange_rate_service::load_for_snapshot(db, end_date, None)
}

type SnapshotNotes = BTreeMap<PositionKey, (Option<String>, Option<String>)>;

#[derive(Debug)]
struct ExistingSnapshot {
    id: String,
    created_at: String,
    overall_notes: Option<String>,
    exchange_rates: String,
    notes: SnapshotNotes,
}

fn load_existing_snapshot(
    db: &Database,
    quarter: &str,
    requested_id: Option<&str>,
) -> Result<Option<ExistingSnapshot>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let header = if let Some(snapshot_id) = requested_id {
        conn.query_row(
            "SELECT id, created_at, overall_notes, exchange_rates
             FROM quarterly_snapshots
             WHERE id = ?1 AND quarter = ?2",
            rusqlite::params![snapshot_id, quarter],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Snapshot not found for {quarter}: {snapshot_id}"))?
        .into()
    } else {
        conn.query_row(
            "SELECT id, created_at, overall_notes, exchange_rates
             FROM quarterly_snapshots
             WHERE quarter = ?1",
            rusqlite::params![quarter],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
    };
    let Some((id, created_at, overall_notes, exchange_rates)) = header else {
        return Ok(None);
    };
    let notes = load_snapshot_notes(&conn, &id)?;
    Ok(Some(ExistingSnapshot {
        id,
        created_at,
        overall_notes,
        exchange_rates,
        notes,
    }))
}

fn load_snapshot_notes(conn: &rusqlite::Connection, id: &str) -> Result<SnapshotNotes, String> {
    let mut stmt = conn
        .prepare(
            "SELECT account_id, symbol, market, notes, decision_quality
             FROM quarterly_holding_snapshots
             WHERE quarterly_snapshot_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let notes = stmt
        .query_map(rusqlite::params![id], |row| {
            let account_id = row.get::<_, String>(0)?;
            let symbol = row.get::<_, String>(1)?;
            let market = row.get::<_, String>(2)?;
            Ok((
                PositionKey::new(&account_id, &symbol, &market),
                (
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ),
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(notes)
}

fn load_current_holdings(db: &Database) -> Result<Vec<WorkingHolding>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT h.account_id, COALESCE(a.name, ''), h.symbol, h.name, h.market,
                    COALESCE(c.name, '未分类'), COALESCE(c.color, '#8B8B8B'),
                    h.shares, CASE WHEN UPPER(h.symbol) LIKE '$CASH-%' THEN 1.0 ELSE h.avg_cost END,
                    h.currency
             FROM holdings h
             LEFT JOIN accounts a ON a.id = h.account_id
             LEFT JOIN categories c ON c.id = h.category_id
             WHERE h.shares > 0 OR UPPER(h.symbol) LIKE '$CASH-%'
             ORDER BY h.market, UPPER(h.symbol), h.account_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(WorkingHolding {
                account_id: row.get(0)?,
                account_name: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                category_name: row.get(5)?,
                category_color: row.get(6)?,
                shares: row.get(7)?,
                avg_cost: row.get(8)?,
                currency: row.get(9)?,
                notes: None,
                decision_quality: None,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

async fn resolve_current_prices(
    db: &Database,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    holdings: &[WorkingHolding],
) -> Result<BTreeMap<PositionKey, f64>, String> {
    let mut prices = BTreeMap::new();
    let mut requested = BTreeSet::new();
    for holding in holdings {
        if crate::services::quote_service::is_cash_symbol(&holding.symbol) {
            prices.insert(PositionKey::from_holding(holding), 1.0);
        } else {
            requested.insert((holding.symbol.clone(), holding.market.clone()));
        }
    }
    if !requested.is_empty() {
        let config = quote_provider_service::get_quote_provider_config(db)?;
        let quotes = fetch_quotes_batch_cached_with_providers(
            quote_state,
            quote_cache,
            requested.into_iter().collect(),
            &config.us_provider,
            &config.hk_provider,
            &config.cn_provider,
            true,
        )
        .await?;
        for quote in quotes.data {
            if quote.current_price.is_finite() && quote.current_price > 0.0 {
                for holding in holdings.iter().filter(|holding| {
                    holding.symbol.eq_ignore_ascii_case(&quote.symbol)
                        && holding.market == quote.market
                }) {
                    prices.insert(PositionKey::from_holding(holding), quote.current_price);
                }
            }
        }
    }
    ensure_complete_prices(holdings, &prices)?;
    Ok(prices)
}

async fn resolve_historical_prices<Fetch, FetchFuture>(
    db: &Database,
    holdings: &[WorkingHolding],
    end_date: NaiveDate,
    fetch_history: &Fetch,
) -> Result<BTreeMap<PositionKey, f64>, String>
where
    Fetch: Fn(String, String, NaiveDate, NaiveDate, String) -> FetchFuture,
    FetchFuture: std::future::Future<Output = Result<Vec<(NaiveDate, f64)>, String>>,
{
    let mut prices = load_historical_prices_from_db(db, holdings, end_date)?;
    let missing = holdings
        .iter()
        .filter(|holding| !prices.contains_key(&PositionKey::from_holding(holding)))
        .map(|holding| (holding.symbol.clone(), holding.market.clone()))
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        let config = quote_provider_service::get_quote_provider_config(db)?;
        for (symbol, market) in missing {
            let provider = match market.as_str() {
                "US" => config.us_provider.clone(),
                "HK" => config.hk_provider.clone(),
                "CN" => config.cn_provider.clone(),
                _ => continue,
            };
            let history = fetch_history(
                symbol.clone(),
                market.clone(),
                end_date - chrono::Duration::days(10),
                end_date + chrono::Duration::days(2),
                provider,
            )
            .await;
            if let Ok(history) = history {
                if let Some((_, price)) = history
                    .into_iter()
                    .filter(|(date, price)| *date <= end_date && price.is_finite() && *price > 0.0)
                    .max_by_key(|(date, _)| *date)
                {
                    for holding in holdings.iter().filter(|holding| {
                        holding.symbol.eq_ignore_ascii_case(&symbol) && holding.market == market
                    }) {
                        prices.insert(PositionKey::from_holding(holding), price);
                    }
                }
            }
        }
    }
    ensure_complete_prices(holdings, &prices)?;
    Ok(prices)
}

struct ComputedHolding {
    holding: WorkingHolding,
    close_price: f64,
    market_value: f64,
    cost_value: f64,
    pnl: f64,
    pnl_percent: f64,
    weight: f64,
}

struct ComputedSnapshot {
    total_value: f64,
    total_cost: f64,
    total_pnl: f64,
    us_value: f64,
    us_cost: f64,
    cn_value: f64,
    cn_cost: f64,
    hk_value: f64,
    hk_cost: f64,
    holdings: Vec<ComputedHolding>,
}

fn compute_snapshot(
    holdings: Vec<WorkingHolding>,
    prices: &BTreeMap<PositionKey, f64>,
    rates: &ExchangeRates,
) -> ComputedSnapshot {
    let mut market_totals: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut rows = Vec::with_capacity(holdings.len());
    for holding in holdings {
        let close_price = prices[&PositionKey::from_holding(&holding)];
        let market_value = holding.shares * close_price;
        let cost_value = holding.shares * holding.avg_cost;
        let totals = market_totals.entry(holding.market.clone()).or_default();
        let market_currency = match holding.market.as_str() {
            "CN" => "CNY",
            "HK" => "HKD",
            _ => "USD",
        };
        totals.0 += convert_currency(market_value, &holding.currency, market_currency, rates);
        totals.1 += convert_currency(cost_value, &holding.currency, market_currency, rates);
        let pnl = market_value - cost_value;
        rows.push(ComputedHolding {
            holding,
            close_price,
            market_value,
            cost_value,
            pnl,
            pnl_percent: if cost_value > 0.0 {
                pnl / cost_value * 100.0
            } else {
                0.0
            },
            weight: 0.0,
        });
    }
    let (us_value, us_cost) = market_totals.get("US").copied().unwrap_or_default();
    let (cn_value, cn_cost) = market_totals.get("CN").copied().unwrap_or_default();
    let (hk_value, hk_cost) = market_totals.get("HK").copied().unwrap_or_default();
    let total_value = us_value
        + convert_currency(cn_value, "CNY", "USD", rates)
        + convert_currency(hk_value, "HKD", "USD", rates);
    let total_cost = us_cost
        + convert_currency(cn_cost, "CNY", "USD", rates)
        + convert_currency(hk_cost, "HKD", "USD", rates);
    for row in &mut rows {
        let value_usd = convert_currency(row.market_value, &row.holding.currency, "USD", rates);
        row.weight = if total_value != 0.0 {
            value_usd / total_value * 100.0
        } else {
            0.0
        };
    }
    ComputedSnapshot {
        total_value,
        total_cost,
        total_pnl: total_value - total_cost,
        us_value,
        us_cost,
        cn_value,
        cn_cost,
        hk_value,
        hk_cost,
        holdings: rows,
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_snapshot(
    db: &Database,
    id: &str,
    quarter: &str,
    snapshot_date: NaiveDate,
    created_at: &str,
    overall_notes: Option<&str>,
    rates: &ExchangeRates,
    computed: &ComputedSnapshot,
    exists: bool,
    input_revision: i64,
) -> Result<Option<String>, String> {
    let rates_json = serde_json::to_string(rates).map_err(|error| error.to_string())?;
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    if crate::services::snapshot_cache_service::current_revision(&tx)? != input_revision {
        return Err("ledger changed while rebuilding quarterly snapshot; please retry".into());
    }
    // Notes can autosave while quotes are being fetched without changing the
    // ledger revision. Read their latest values under the final write lock,
    // including explicit clears, before replacing any snapshot rows.
    let (overall_notes, latest_notes) = if exists {
        let overall_notes = tx
            .query_row(
                "SELECT overall_notes FROM quarterly_snapshots WHERE id = ?1",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| error.to_string())?;
        (overall_notes, load_snapshot_notes(&tx, id)?)
    } else {
        (overall_notes.map(str::to_owned), SnapshotNotes::new())
    };
    crate::services::historical_exchange_rate_service::record_snapshot_rates_in(
        &tx,
        &snapshot_date.format("%Y-%m-%d").to_string(),
        rates,
    )?;
    if exists {
        tx.execute(
            "DELETE FROM quarterly_holding_snapshots WHERE quarterly_snapshot_id = ?1",
            rusqlite::params![id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "UPDATE quarterly_snapshots
             SET snapshot_date = ?1, total_value = ?2, total_cost = ?3, total_pnl = ?4,
                 us_value = ?5, us_cost = ?6, cn_value = ?7, cn_cost = ?8,
                 hk_value = ?9, hk_cost = ?10, exchange_rates = ?11,
                 overall_notes = ?12
             WHERE id = ?13",
            rusqlite::params![
                snapshot_date.format("%Y-%m-%d").to_string(),
                computed.total_value,
                computed.total_cost,
                computed.total_pnl,
                computed.us_value,
                computed.us_cost,
                computed.cn_value,
                computed.cn_cost,
                computed.hk_value,
                computed.hk_cost,
                rates_json,
                overall_notes,
                id,
            ],
        )
        .map_err(|error| error.to_string())?;
    } else {
        tx.execute(
            "INSERT INTO quarterly_snapshots
             (id, quarter, snapshot_date, total_value, total_cost, total_pnl,
              us_value, us_cost, cn_value, cn_cost, hk_value, hk_cost,
              exchange_rates, overall_notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                id,
                quarter,
                snapshot_date.format("%Y-%m-%d").to_string(),
                computed.total_value,
                computed.total_cost,
                computed.total_pnl,
                computed.us_value,
                computed.us_cost,
                computed.cn_value,
                computed.cn_cost,
                computed.hk_value,
                computed.hk_cost,
                rates_json,
                overall_notes,
                created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    for row in &computed.holdings {
        let (notes, decision_quality) = latest_notes
            .get(&PositionKey::from_holding(&row.holding))
            .map(|(notes, quality)| (notes.as_deref(), quality.as_deref()))
            .unwrap_or((
                row.holding.notes.as_deref(),
                row.holding.decision_quality.as_deref(),
            ));
        tx.execute(
            "INSERT INTO quarterly_holding_snapshots
             (id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
              category_name, category_color, shares, avg_cost, close_price, market_value,
              cost_value, pnl, pnl_percent, weight, notes, decision_quality, currency)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, ?16, ?17, ?18, ?19, ?20)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                id,
                row.holding.account_id,
                row.holding.account_name,
                row.holding.symbol,
                row.holding.name,
                row.holding.market,
                row.holding.category_name,
                row.holding.category_color,
                row.holding.shares,
                row.holding.avg_cost,
                row.close_price,
                row.market_value,
                row.cost_value,
                row.pnl,
                row.pnl_percent,
                row.weight,
                notes,
                decision_quality,
                row.holding.currency,
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(overall_notes)
}

pub(super) async fn rebuild_quarterly_snapshot(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    quarter: &str,
    existing_id: Option<&str>,
) -> Result<QuarterlySnapshot, String> {
    rebuild_quarterly_snapshot_with_history_fetcher(
        db,
        cache,
        quote_cache,
        quote_state,
        quarter,
        existing_id,
        |symbol, market, start, end, provider| async move {
            fetch_stock_history(quote_state, &symbol, &market, start, end, &provider).await
        },
    )
    .await
}

async fn rebuild_quarterly_snapshot_with_history_fetcher<Fetch, FetchFuture>(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    quarter: &str,
    existing_id: Option<&str>,
    fetch_history: Fetch,
) -> Result<QuarterlySnapshot, String>
where
    Fetch: Fn(String, String, NaiveDate, NaiveDate, String) -> FetchFuture,
    FetchFuture: std::future::Future<Output = Result<Vec<(NaiveDate, f64)>, String>>,
{
    let today = Utc::now().date_naive();
    let input_revision = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        crate::services::snapshot_cache_service::current_revision(&conn)?
    };
    let (year, quarter_number) = parse_quarter(quarter)?;
    let end_date = quarter_end_date(year, quarter_number);
    let is_current = date_to_quarter(today) == quarter;
    let snapshot_date = if is_current {
        today.min(end_date)
    } else {
        end_date
    };
    let existing = load_existing_snapshot(db, quarter, existing_id)?;
    let mut holdings = if is_current {
        load_current_holdings(db)?
    } else {
        load_historical_holdings(db, end_date)?
    };
    if holdings.is_empty() {
        return Err(format!("No holdings found to snapshot for {quarter}"));
    }
    if let Some(existing) = &existing {
        for holding in &mut holdings {
            if let Some((notes, decision_quality)) =
                existing.notes.get(&PositionKey::from_holding(holding))
            {
                holding.notes = notes.clone();
                holding.decision_quality = decision_quality.clone();
            }
        }
    }
    let prices = if is_current {
        resolve_current_prices(db, quote_cache, quote_state, &holdings).await?
    } else {
        resolve_historical_prices(db, &holdings, end_date, &fetch_history).await?
    };
    let rates = if is_current {
        validate_rates(get_cached_rates(cache, db).await?, "current")?
    } else {
        crate::services::historical_exchange_rate_service::load_for_snapshot(
            db,
            end_date,
            existing
                .as_ref()
                .map(|snapshot| snapshot.exchange_rates.as_str()),
        )?
    };
    let computed = compute_snapshot(holdings, &prices, &rates);
    let id = existing
        .as_ref()
        .map(|snapshot| snapshot.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let created_at = existing
        .as_ref()
        .map(|snapshot| snapshot.created_at.clone())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let overall_notes = existing
        .as_ref()
        .and_then(|snapshot| snapshot.overall_notes.clone());
    let overall_notes = persist_snapshot(
        db,
        &id,
        quarter,
        snapshot_date,
        &created_at,
        overall_notes.as_deref(),
        &rates,
        &computed,
        existing.is_some(),
        input_revision,
    )?;
    let holding_count = computed
        .holdings
        .iter()
        .filter(|row| !crate::services::quote_service::is_cash_symbol(&row.holding.symbol))
        .map(|row| {
            (
                row.holding.market.clone(),
                row.holding.symbol.to_uppercase(),
            )
        })
        .collect::<BTreeSet<_>>()
        .len();
    Ok(QuarterlySnapshot {
        id,
        quarter: quarter.to_string(),
        snapshot_date: snapshot_date.format("%Y-%m-%d").to_string(),
        total_value: computed.total_value,
        total_cost: computed.total_cost,
        total_pnl: computed.total_pnl,
        us_value: computed.us_value,
        us_cost: computed.us_cost,
        cn_value: computed.cn_value,
        cn_cost: computed.cn_cost,
        hk_value: computed.hk_value,
        hk_cost: computed.hk_cost,
        exchange_rates: serde_json::to_string(&rates).map_err(|error| error.to_string())?,
        overall_notes,
        created_at,
        holding_count,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_complete_prices, load_historical_holdings, load_historical_prices_from_db,
        load_historical_rates, rebuild_quarterly_snapshot_with_history_fetcher, PositionKey,
        WorkingHolding,
    };
    use crate::db::Database;
    use crate::services::exchange_rate_service::ExchangeRateCache;
    use crate::services::quote_service::{QuoteCache, QuoteServiceState};
    use chrono::NaiveDate;

    fn insert_account(db: &Database, id: &str, name: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES (?1, ?2, 'US', '2025-01-01', '2025-01-01')",
            rusqlite::params![id, name],
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_transaction(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        name: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        total_amount: f64,
        commission: f64,
        traded_at: &str,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transactions
             (id, holding_id, account_id, symbol, name, market, transaction_type,
              shares, price, total_amount, commission, currency, traded_at, notes, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, 'US', ?5, ?6, ?7, ?8, ?9,
                     'USD', ?10, NULL, ?10)",
            rusqlite::params![
                id,
                account_id,
                symbol,
                name,
                transaction_type,
                shares,
                price,
                total_amount,
                commission,
                traded_at,
            ],
        )
        .unwrap();
    }

    fn holding(account_id: &str, symbol: &str, market: &str) -> WorkingHolding {
        WorkingHolding {
            account_id: account_id.to_string(),
            account_name: account_id.to_string(),
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            market: market.to_string(),
            currency: match market {
                "CN" => "CNY",
                "HK" => "HKD",
                _ => "USD",
            }
            .to_string(),
            category_name: "未分类".to_string(),
            category_color: "#8B8B8B".to_string(),
            shares: 1.0,
            avg_cost: 1.0,
            notes: None,
            decision_quality: None,
        }
    }

    fn insert_cash(db: &Database, account: &str, balance: f64, created_at: &str) {
        db.conn.lock().unwrap().execute(
            "INSERT INTO holdings
             (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?1, '$CASH-USD', 'USD Cash', 'US', ?2, 1, 'USD', ?3, ?3)",
            rusqlite::params![account, balance, created_at],
        ).unwrap();
    }

    #[test]
    fn stock_transfers_reconstruct_historical_holdings_and_cash() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_transaction(
            &db,
            "cash",
            "acct-a",
            "$CASH-USD",
            "Cash",
            "OPEN",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "2025-01-01",
        );
        insert_transaction(
            &db,
            "in",
            "acct-a",
            "AAPL",
            "Apple",
            "STOCK_IN",
            10.0,
            20.0,
            200.0,
            0.0,
            "2025-01-02",
        );
        insert_transaction(
            &db,
            "out",
            "acct-a",
            "AAPL",
            "Apple",
            "STOCK_OUT",
            4.0,
            0.0,
            0.0,
            0.0,
            "2025-02-02",
        );
        for (month, shares) in [(1, 10.0), (3, 6.0)] {
            let holdings =
                load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, month, 28).unwrap())
                    .unwrap();
            let stock = holdings.iter().find(|h| h.symbol == "AAPL").unwrap();
            assert_eq!((stock.shares, stock.avg_cost), (shares, 20.0));
            assert_eq!(
                holdings
                    .iter()
                    .find(|h| h.symbol == "$CASH-USD")
                    .unwrap()
                    .shares,
                1000.0
            );
        }
    }

    #[test]
    fn quarterly_cash_replays_opening_and_every_cash_flow_through_cutoff() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        for (id, symbol, kind, shares, price, amount, fee, date) in [
            (
                "cash-open",
                "$CASH-USD",
                "OPEN",
                1000.0,
                1.0,
                1000.0,
                0.0,
                "2025-01-01",
            ),
            (
                "stock-open",
                "AAPL",
                "OPEN",
                10.0,
                10.0,
                100.0,
                0.0,
                "2025-01-02",
            ),
            ("buy", "AAPL", "BUY", 10.0, 10.0, 100.0, 5.0, "2025-01-10"),
            ("sell", "AAPL", "SELL", 4.0, 10.0, 40.0, 2.0, "2025-01-20"),
            ("pay", "AAPL", "PAY", 0.0, 0.0, 10.0, 1.0, "2025-02-01"),
            (
                "deposit",
                "$CASH-USD",
                "BUY",
                50.0,
                1.0,
                50.0,
                2.0,
                "2025-02-02",
            ),
            (
                "withdraw",
                "$CASH-USD",
                "SELL",
                30.0,
                1.0,
                30.0,
                1.0,
                "2025-03-31T23:59:59Z",
            ),
            (
                "later-buy",
                "AAPL",
                "BUY",
                30.0,
                10.0,
                300.0,
                0.0,
                "2025-04-01",
            ),
        ] {
            insert_transaction(
                &db, id, "acct-a", symbol, symbol, kind, shares, price, amount, fee, date,
            );
        }
        let positions =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();
        let cash = positions
            .iter()
            .find(|h| h.symbol == "$CASH-USD")
            .expect("cash must be included");
        assert_eq!(cash.shares, 963.0);
        assert_eq!(cash.avg_cost, 1.0);
    }

    #[test]
    fn quarterly_cash_infers_baseline_from_all_flows_despite_late_cash_row() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_cash(&db, "acct-a", 50.0, "2025-04-01");
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "Apple",
            "BUY",
            10.0,
            10.0,
            100.0,
            2.0,
            "2025-01-01",
        );
        insert_transaction(
            &db,
            "deposit",
            "acct-a",
            "$CASH-USD",
            "Cash",
            "BUY",
            200.0,
            1.0,
            200.0,
            1.0,
            "2025-04-01",
        );
        let positions =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();
        let cash = positions
            .iter()
            .find(|h| h.symbol == "$CASH-USD")
            .expect("cash must be included");
        assert_eq!(cash.shares, -151.0);
    }

    #[test]
    fn quarterly_cash_only_and_current_negative_and_zero_balances_are_retained() {
        let db = Database::new(":memory:").unwrap();
        for (id, balance) in [("positive", 50.0), ("negative", -25.0), ("zero", 0.0)] {
            insert_account(&db, id, id);
            insert_cash(&db, id, balance, "2025-01-01");
        }
        let historical =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();
        assert_eq!(historical.len(), 3);
        let current = super::load_current_holdings(&db).unwrap();
        assert_eq!(current.len(), 3);
        assert_eq!(current.iter().map(|h| h.shares).sum::<f64>(), 25.0);
    }

    #[test]
    fn quarterly_cash_current_cost_is_one_even_for_legacy_nonunit_cost() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_cash(&db, "acct-a", 100.0, "2025-01-01");
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE holdings SET avg_cost = 2", [])
            .unwrap();
        let current = super::load_current_holdings(&db).unwrap();
        let historical =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();
        let prices =
            std::collections::BTreeMap::from([(PositionKey::from_holding(&current[0]), 1.0)]);
        let rates = crate::models::quote::ExchangeRates {
            usd_cny: 7.1,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.1,
            updated_at: "2025-03-31".into(),
        };
        let computed = super::compute_snapshot(current, &prices, &rates);
        assert_eq!(computed.total_cost, 100.0);
        assert_eq!(computed.total_pnl, 0.0);
        assert_eq!(historical[0].avg_cost, 1.0);
    }

    #[test]
    fn quarterly_cash_does_not_project_later_account_balances_into_old_quarters() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "future", "Future");
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE accounts SET created_at = '2025-04-01'", [])
            .unwrap();
        insert_cash(&db, "future", 500.0, "2025-04-01");
        assert!(
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn quarterly_cash_missing_baseline_fails_instead_of_assuming_zero() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "Apple",
            "BUY",
            10.0,
            10.0,
            100.0,
            0.0,
            "2025-01-01",
        );
        let error = load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap())
            .unwrap_err();
        assert!(error.contains("missing historical cash baseline"));
    }

    #[test]
    fn quarterly_valuation_converts_actual_currency_to_market_and_overall_totals() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_cash(&db, "acct-a", 70.0, "2025-01-01");
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE holdings SET market = 'CN'", [])
            .unwrap();
        let holdings = super::load_current_holdings(&db).unwrap();
        let prices =
            std::collections::BTreeMap::from([(PositionKey::from_holding(&holdings[0]), 1.0)]);
        let rates = crate::models::quote::ExchangeRates {
            usd_cny: 7.0,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.0,
            updated_at: "2025-03-31".into(),
        };
        let computed = super::compute_snapshot(holdings, &prices, &rates);
        assert_eq!(computed.cn_value, 490.0);
        assert_eq!(computed.cn_cost, 490.0);
        assert_eq!(computed.total_value, 70.0);
        assert_eq!(computed.holdings[0].market_value, 70.0);
        assert_eq!(computed.holdings[0].weight, 100.0);
    }

    #[test]
    fn quarterly_valuation_uses_one_usd_basis_when_cross_rate_was_rounded() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_cash(&db, "acct-a", 71.0, "2025-01-01");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE holdings SET symbol = '$CASH-CNY', currency = 'CNY', market = 'HK'",
                [],
            )
            .unwrap();
        let holdings = super::load_current_holdings(&db).unwrap();
        let prices =
            std::collections::BTreeMap::from([(PositionKey::from_holding(&holdings[0]), 1.0)]);
        let rates = crate::models::quote::ExchangeRates {
            usd_cny: 7.1,
            usd_hkd: 7.8,
            cny_hkd: 1.1,
            updated_at: "2025-03-31".into(),
        };
        let computed = super::compute_snapshot(holdings, &prices, &rates);
        assert_eq!(computed.hk_value, 78.0);
        assert_eq!(computed.total_value, 10.0);
        assert_eq!(computed.holdings[0].weight, 100.0);
    }

    #[test]
    fn quarterly_cash_and_stock_share_utc_cutoff_for_offset_timestamps() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_transaction(
            &db,
            "cash-open",
            "acct-a",
            "$CASH-USD",
            "Cash",
            "OPEN",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "2025-03-30",
        );
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "Apple",
            "BUY",
            10.0,
            10.0,
            100.0,
            0.0,
            "2025-04-01T00:30:00+08:00",
        );
        let positions =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();
        assert_eq!(
            positions
                .iter()
                .find(|h| h.symbol == "AAPL")
                .unwrap()
                .shares,
            10.0
        );
        assert_eq!(
            positions
                .iter()
                .find(|h| h.symbol == "$CASH-USD")
                .unwrap()
                .shares,
            900.0
        );
    }

    #[test]
    fn quarterly_cash_creation_uses_utc_cutoff_and_openings_use_instant_order() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "created", "Created");
        insert_cash(&db, "created", 5.0, "2025-04-01T00:30:00+08:00");
        insert_account(&db, "openings", "Openings");
        insert_transaction(
            &db,
            "first",
            "openings",
            "$CASH-USD",
            "Cash",
            "OPEN",
            100.0,
            1.0,
            100.0,
            0.0,
            "2025-03-31T23:30:00+08:00",
        );
        insert_transaction(
            &db,
            "last",
            "openings",
            "$CASH-USD",
            "Cash",
            "OPEN",
            200.0,
            1.0,
            200.0,
            0.0,
            "2025-03-31T20:00:00Z",
        );
        let positions =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();
        assert_eq!(
            positions
                .iter()
                .find(|h| h.account_id == "openings")
                .unwrap()
                .shares,
            200.0
        );
        assert_eq!(
            positions
                .iter()
                .find(|h| h.account_id == "created")
                .expect("cash creation was in Q1 UTC")
                .shares,
            5.0
        );
    }

    #[test]
    fn historical_replay_uses_only_transactions_through_quarter_end() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_cash(&db, "acct-a", 470.0, "2025-01-01");
        insert_transaction(
            &db,
            "open",
            "acct-a",
            "AAPL",
            "Apple",
            "OPEN",
            10.0,
            10.0,
            100.0,
            0.0,
            "2025-01-02T09:30:00Z",
        );
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "Apple",
            "BUY",
            5.0,
            20.0,
            100.0,
            0.0,
            "2025-02-01T09:30:00Z",
        );
        insert_transaction(
            &db,
            "sell-q1",
            "acct-a",
            "AAPL",
            "Apple",
            "SELL",
            3.0,
            30.0,
            90.0,
            0.0,
            "2025-03-01T09:30:00Z",
        );
        insert_transaction(
            &db,
            "sell-q2",
            "acct-a",
            "AAPL",
            "Apple",
            "SELL",
            12.0,
            40.0,
            480.0,
            0.0,
            "2025-04-15T09:30:00Z",
        );

        let positions =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();

        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].account_id, "acct-a");
        assert_eq!(positions[0].symbol, "AAPL");
        assert_eq!(positions[0].shares, 12.0);
        assert!((positions[0].avg_cost - 13.333333333333334).abs() < 1e-9);
    }

    #[test]
    fn historical_replay_keeps_same_symbol_separate_by_account() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_account(&db, "acct-b", "账户 B");
        insert_transaction(
            &db,
            "a-open",
            "acct-a",
            "MSFT",
            "Microsoft",
            "OPEN",
            2.0,
            100.0,
            200.0,
            0.0,
            "2025-01-02T09:30:00Z",
        );
        insert_transaction(
            &db,
            "b-open",
            "acct-b",
            "MSFT",
            "Microsoft",
            "OPEN",
            7.0,
            110.0,
            770.0,
            0.0,
            "2025-01-03T09:30:00Z",
        );

        let positions =
            load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();

        assert_eq!(positions.len(), 2);
        assert_eq!(positions[0].account_id, "acct-a");
        assert_eq!(positions[0].shares, 2.0);
        assert_eq!(positions[1].account_id, "acct-b");
        assert_eq!(positions[1].shares, 7.0);
    }

    #[test]
    fn historical_replay_rejects_unexplained_negative_position() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_transaction(
            &db,
            "sell",
            "acct-a",
            "NVDA",
            "NVIDIA",
            "SELL",
            1.0,
            100.0,
            100.0,
            0.0,
            "2025-01-02T09:30:00Z",
        );

        let error = load_historical_holdings(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap())
            .unwrap_err();

        assert!(error.contains("negative historical position"));
        assert!(error.contains("acct-a"));
        assert!(error.contains("NVDA"));
    }

    #[test]
    fn historical_prices_are_keyed_by_symbol_and_market_and_stop_at_cutoff() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        for (date, market, price) in [
            ("2025-03-28", "US", 101.0),
            ("2025-03-28", "HK", 88.0),
            ("2025-04-01", "US", 999.0),
        ] {
            conn.execute(
                "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, category_name, shares, avg_cost,
                  close_price, market_value)
                 VALUES (?1, 'acct', '700', ?2, '未分类', 1, 1, ?3, ?3)",
                rusqlite::params![date, market, price],
            )
            .unwrap();
        }
        drop(conn);
        let holdings = vec![holding("acct", "700", "US"), holding("acct", "700", "HK")];

        let prices = load_historical_prices_from_db(
            &db,
            &holdings,
            NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
        )
        .unwrap();

        assert_eq!(prices[&PositionKey::new("acct", "700", "US")], 101.0);
        assert_eq!(prices[&PositionKey::new("acct", "700", "HK")], 88.0);
    }

    #[test]
    fn complete_price_check_rejects_missing_security_without_using_zero() {
        let holdings = vec![holding("acct", "AAPL", "US")];
        let error = ensure_complete_prices(&holdings, &Default::default()).unwrap_err();

        assert!(error.contains("missing closing price"));
        assert!(error.contains("acct/US/AAPL"));
    }

    #[test]
    fn historical_rates_require_saved_positive_finite_values() {
        let db = Database::new(":memory:").unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2025, 3, 31).unwrap();

        let missing = load_historical_rates(&db, cutoff).unwrap_err();
        assert!(missing.contains("missing historical exchange rates"));

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO daily_portfolio_values
             (date, exchange_rates) VALUES ('2025-03-28', ?1)",
            rusqlite::params![
                r#"{"usd_cny":0,"usd_hkd":7.8,"cny_hkd":1.08,"updated_at":"2025-03-28"}"#
            ],
        )
        .unwrap();
        drop(conn);

        let invalid = load_historical_rates(&db, cutoff).unwrap_err();
        assert!(invalid.contains("invalid historical exchange rates"));
    }

    #[test]
    fn historical_rates_use_latest_record_on_or_before_cutoff() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        for (date, usd_cny) in [("2025-03-28", 7.1), ("2025-04-01", 9.9)] {
            let rates = format!(
                r#"{{"usd_cny":{usd_cny},"usd_hkd":7.8,"cny_hkd":1.0985915493,"updated_at":"{date}"}}"#
            );
            conn.execute(
                "INSERT INTO daily_portfolio_values (date, exchange_rates) VALUES (?1, ?2)",
                rusqlite::params![date, rates],
            )
            .unwrap();
        }
        drop(conn);

        let rates =
            load_historical_rates(&db, NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()).unwrap();

        assert_eq!(rates.usd_cny, 7.1);
        assert_eq!(rates.updated_at, "2025-03-28");
    }

    fn insert_historical_price_and_rates(db: &Database, symbol: &str, price: f64) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO daily_holding_snapshots
             (date, account_id, symbol, market, category_name, shares, avg_cost,
              close_price, market_value)
             VALUES ('2025-03-28', 'acct-a', ?1, 'US', '未分类', 1, 1, ?2, ?2)",
            rusqlite::params![symbol, price],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO daily_portfolio_values (date, exchange_rates)
             VALUES ('2025-03-28', ?1)",
            rusqlite::params![
                r#"{"usd_cny":7.1,"usd_hkd":7.8,"cny_hkd":1.0985915493,"updated_at":"2025-03-28"}"#
            ],
        )
        .unwrap();
    }

    fn insert_existing_snapshot(db: &Database, id: &str, total_value: f64) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_snapshots
             (id, quarter, snapshot_date, total_value, total_cost, total_pnl,
              exchange_rates, overall_notes, created_at)
             VALUES (?1, '2025-Q1', '2025-03-31', ?2, 5, ?2 - 5,
                     '{}', '季度总评', '2025-04-01')",
            rusqlite::params![id, total_value],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO quarterly_holding_snapshots
             (id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
              category_name, category_color, shares, avg_cost, close_price, market_value,
              cost_value, pnl, pnl_percent, weight, notes)
             VALUES ('old-row', ?1, 'acct-a', '账户 A', 'AAPL', 'Apple', 'US',
                     '未分类', '#8B8B8B', 1, 5, ?2, ?2, 5, ?2 - 5, 0, 100, '账户 A 笔记')",
            rusqlite::params![id, total_value],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn canonical_rebuild_preserves_ids_and_account_scoped_notes() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_account(&db, "acct-b", "账户 B");
        insert_transaction(
            &db,
            "a-open",
            "acct-a",
            "AAPL",
            "Apple",
            "OPEN",
            2.0,
            10.0,
            20.0,
            0.0,
            "2025-01-02T09:30:00Z",
        );
        insert_transaction(
            &db,
            "b-open",
            "acct-b",
            "AAPL",
            "Apple",
            "OPEN",
            3.0,
            20.0,
            60.0,
            0.0,
            "2025-01-03T09:30:00Z",
        );
        insert_historical_price_and_rates(&db, "AAPL", 100.0);
        insert_existing_snapshot(&db, "snapshot-q1", 10.0);
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("UPDATE quarterly_holding_snapshots SET decision_quality = 'good' WHERE id = 'old-row'", []).unwrap();
            conn.execute(
                "INSERT INTO quarterly_holding_snapshots
                 (id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                  category_name, category_color, shares, avg_cost, close_price, market_value,
                  cost_value, pnl, pnl_percent, weight, notes)
                 VALUES ('old-row-b', 'snapshot-q1', 'acct-b', '账户 B', 'AAPL', 'Apple',
                         'US', '未分类', '#8B8B8B', 1, 5, 10, 10, 5, 5, 100, 50,
                         '账户 B 笔记')",
                [],
            )
            .unwrap();
        }
        let cache = ExchangeRateCache::new();
        let quote_cache = QuoteCache::new();
        let quote_state = QuoteServiceState::new();

        let rebuilt = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &cache,
            &quote_cache,
            &quote_state,
            "2025-Q1",
            Some("snapshot-q1"),
            |_, _, _, _, _| async { Err("history fetch must not be needed".to_string()) },
        )
        .await
        .unwrap();

        assert_eq!(rebuilt.id, "snapshot-q1");
        assert_eq!(rebuilt.overall_notes.as_deref(), Some("季度总评"));
        assert_eq!(rebuilt.total_value, 500.0);
        let conn = db.conn.lock().unwrap();
        assert_eq!(conn.query_row("SELECT decision_quality FROM quarterly_holding_snapshots WHERE account_id = 'acct-a'", [], |row| row.get::<_, Option<String>>(0)).unwrap().as_deref(), Some("good"));
        let rows = conn
            .prepare(
                "SELECT account_id, shares, notes
                 FROM quarterly_holding_snapshots
                 WHERE quarterly_snapshot_id = 'snapshot-q1'
                 ORDER BY account_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("acct-a".to_string(), 2.0, Some("账户 A 笔记".to_string())),
                ("acct-b".to_string(), 3.0, Some("账户 B 笔记".to_string())),
            ]
        );
    }

    #[tokio::test]
    async fn failed_rebuild_leaves_existing_snapshot_unchanged() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_transaction(
            &db,
            "open",
            "acct-a",
            "AAPL",
            "Apple",
            "OPEN",
            2.0,
            10.0,
            20.0,
            0.0,
            "2025-01-02T09:30:00Z",
        );
        insert_existing_snapshot(&db, "snapshot-q1", 777.0);
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO daily_portfolio_values (date, exchange_rates)
                 VALUES ('2025-03-28', ?1)",
                rusqlite::params![
                    r#"{"usd_cny":7.1,"usd_hkd":7.8,"cny_hkd":1.0985915493,"updated_at":"2025-03-28"}"#
                ],
            )
            .unwrap();
        }
        let error = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &ExchangeRateCache::new(),
            &QuoteCache::new(),
            &QuoteServiceState::new(),
            "2025-Q1",
            Some("snapshot-q1"),
            |_, _, _, _, _| async { Err("offline".to_string()) },
        )
        .await
        .unwrap_err();

        assert!(error.contains("missing closing price"));
        let conn = db.conn.lock().unwrap();
        let state = conn
            .query_row(
                "SELECT qs.total_value, qhs.notes
                 FROM quarterly_snapshots qs
                 JOIN quarterly_holding_snapshots qhs ON qhs.quarterly_snapshot_id = qs.id
                 WHERE qs.id = 'snapshot-q1'",
                [],
                |row| Ok((row.get::<_, f64>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(state, (777.0, "账户 A 笔记".to_string()));
    }

    #[tokio::test]
    async fn quarterly_rebuild_preserves_notes_saved_or_cleared_during_price_fetch() {
        for (overall_notes, notes, quality) in [
            (
                Some("new quarter note"),
                Some("new holding note"),
                Some("good"),
            ),
            (None, None, None),
        ] {
            let db = Database::new(":memory:").unwrap();
            insert_account(&db, "acct-a", "账户 A");
            insert_transaction(
                &db,
                "open",
                "acct-a",
                "AAPL",
                "Apple",
                "OPEN",
                2.0,
                10.0,
                20.0,
                0.0,
                "2025-01-02",
            );
            insert_historical_price_and_rates(&db, "OTHER", 1.0);
            insert_existing_snapshot(&db, "snapshot-q1", 777.0);
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE quarterly_holding_snapshots SET decision_quality = 'old'",
                    [],
                )
                .unwrap();
            let rebuilt = rebuild_quarterly_snapshot_with_history_fetcher(
                &db, &ExchangeRateCache::new(), &QuoteCache::new(), &QuoteServiceState::new(),
                "2025-Q1", Some("snapshot-q1"), |_, _, _, _, _| {
                    let conn = db.conn.lock().unwrap();
                    conn.execute("UPDATE quarterly_snapshots SET overall_notes = ?1 WHERE id = 'snapshot-q1'", rusqlite::params![overall_notes]).unwrap();
                    conn.execute("UPDATE quarterly_holding_snapshots SET notes = ?1, decision_quality = ?2 WHERE quarterly_snapshot_id = 'snapshot-q1'", rusqlite::params![notes, quality]).unwrap();
                    async { Ok(vec![(NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(), 100.0)]) }
                },
            ).await.unwrap();
            assert_eq!(rebuilt.overall_notes.as_deref(), overall_notes);
            assert_eq!(rebuilt.total_value, 200.0);
            let saved = db.conn.lock().unwrap().query_row(
                "SELECT qs.overall_notes, h.notes, h.decision_quality FROM quarterly_snapshots qs
                 JOIN quarterly_holding_snapshots h ON h.quarterly_snapshot_id = qs.id
                 WHERE qs.id = 'snapshot-q1' AND h.account_id = 'acct-a'",
                [], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)),
            ).unwrap();
            assert_eq!(
                (saved.0.as_deref(), saved.1.as_deref(), saved.2.as_deref()),
                (overall_notes, notes, quality)
            );
        }
    }

    #[tokio::test]
    async fn quarterly_rebuild_rejects_ledger_changes_during_price_fetch() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_transaction(
            &db,
            "open",
            "acct-a",
            "AAPL",
            "Apple",
            "OPEN",
            2.0,
            10.0,
            20.0,
            0.0,
            "2025-01-02",
        );
        insert_historical_price_and_rates(&db, "OTHER", 1.0);
        insert_existing_snapshot(&db, "snapshot-q1", 777.0);
        let error = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &ExchangeRateCache::new(),
            &QuoteCache::new(),
            &QuoteServiceState::new(),
            "2025-Q1",
            Some("snapshot-q1"),
            |_, _, _, _, _| {
                db.conn
                    .lock()
                    .unwrap()
                    .execute("UPDATE transactions SET shares = 3 WHERE id = 'open'", [])
                    .unwrap();
                async { Ok(vec![(NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(), 100.0)]) }
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("ledger changed"));
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT total_value FROM quarterly_snapshots WHERE id = 'snapshot-q1'",
                    [],
                    |row| row.get::<_, f64>(0)
                )
                .unwrap(),
            777.0
        );
    }

    #[tokio::test]
    async fn quarterly_created_snapshot_count_distinguishes_same_symbol_across_markets() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        for (id, shares) in [("us-open", 2.0), ("hk-open", 3.0)] {
            insert_transaction(
                &db,
                id,
                "acct-a",
                "700",
                "700",
                "OPEN",
                shares,
                10.0,
                shares * 10.0,
                0.0,
                "2025-01-01",
            );
        }
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions SET market = 'HK', currency = 'HKD' WHERE id = 'hk-open'",
                [],
            )
            .unwrap();
        insert_historical_price_and_rates(&db, "700", 100.0);
        let result = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &ExchangeRateCache::new(),
            &QuoteCache::new(),
            &QuoteServiceState::new(),
            "2025-Q1",
            None,
            |_, _, _, _, _| async {
                Ok(vec![(NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(), 78.0)])
            },
        )
        .await
        .unwrap();
        assert_eq!(result.holding_count, 2);
        assert_eq!(result.total_value, 230.0);
    }

    #[tokio::test]
    async fn quarterly_cash_only_create_refresh_and_current_share_currency_and_notes() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "账户 A");
        insert_cash(&db, "acct-a", -70.0, "2025-01-01");
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE holdings SET market = 'CN'", [])
            .unwrap();
        insert_historical_price_and_rates(&db, "unused", 1.0);
        let cache = ExchangeRateCache::new();
        let quotes = QuoteCache::new();
        let quote_state = QuoteServiceState::new();
        let original = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &cache,
            &quotes,
            &quote_state,
            "2025-Q1",
            None,
            |_, _, _, _, _| async { Err("cash needs no network quote".into()) },
        )
        .await
        .unwrap();
        assert_eq!(original.total_value, -70.0);
        assert_eq!(original.cn_value, -497.0);
        assert_eq!(original.total_pnl, 0.0);
        assert_eq!(original.holding_count, 0);
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("UPDATE quarterly_holding_snapshots SET notes = 'keep cash note', decision_quality = 'good'", []).unwrap();
            conn.execute("DELETE FROM daily_portfolio_values", [])
                .unwrap();
            conn.execute("DELETE FROM daily_holding_snapshots", [])
                .unwrap();
        }
        let refreshed = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &cache,
            &quotes,
            &quote_state,
            "2025-Q1",
            Some(&original.id),
            |_, _, _, _, _| async { Err("cash needs no network quote".into()) },
        )
        .await
        .unwrap();
        assert_eq!(refreshed.id, original.id);
        assert_eq!(refreshed.exchange_rates, original.exchange_rates);
        assert_eq!(refreshed.total_value, original.total_value);
        let today = chrono::Utc::now().date_naive();
        cache.set(crate::models::quote::ExchangeRates {
            usd_cny: 7.1,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.1,
            updated_at: today.to_string(),
        });
        let current = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &cache,
            &quotes,
            &quote_state,
            &super::date_to_quarter(today),
            None,
            |_, _, _, _, _| async { Err("cash needs no network quote".into()) },
        )
        .await
        .unwrap();
        assert_eq!(current.total_value, refreshed.total_value);
        assert_eq!(current.cn_value, refreshed.cn_value);
        assert_eq!(current.holding_count, 0);
        let row = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT currency, shares, avg_cost, notes, decision_quality
             FROM quarterly_holding_snapshots WHERE quarterly_snapshot_id = ?1",
                rusqlite::params![original.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "USD".into(),
                -70.0,
                1.0,
                "keep cash note".into(),
                "good".into()
            )
        );
    }
}
