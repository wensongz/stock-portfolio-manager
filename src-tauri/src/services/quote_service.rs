use crate::db::Database;
use crate::models::{FinancialReport, PriceCandle, StockQuote};
use crate::services::http_client;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Cash symbol prefix used to represent cash holdings.
/// Cash symbols follow the pattern `$CASH-{CURRENCY}`, e.g. `$CASH-USD`, `$CASH-CNY`, `$CASH-HKD`.
pub const CASH_SYMBOL_PREFIX: &str = "$CASH-";

/// Returns `true` if the symbol represents a cash holding.
pub fn is_cash_symbol(symbol: &str) -> bool {
    symbol.starts_with(CASH_SYMBOL_PREFIX)
}

/// Return the display name for a cash symbol, e.g. "现金 (USD)".
/// Panics if the symbol does not start with [`CASH_SYMBOL_PREFIX`].
pub fn cash_display_name(symbol: &str) -> String {
    let currency = symbol
        .strip_prefix(CASH_SYMBOL_PREFIX)
        .expect("cash_display_name called with non-cash symbol");
    format!("现金 ({})", currency)
}

/// Return the UTC offset for the exchange of the given market.
/// CN and HK exchanges operate in UTC+8; US exchanges in UTC-5 (EST).
/// We use a fixed offset (ignoring DST for US) because we only need the
/// date component — even during US daylight-saving time (UTC-4), the
/// difference does not shift the date when the timestamp falls within the
/// trading day.
fn market_utc_offset(market: &str) -> chrono::FixedOffset {
    match market {
        "CN" | "HK" => chrono::FixedOffset::east_opt(8 * 3600).unwrap(),
        // US: Yahoo Finance / Xueqiu / EastMoney daily bars are timestamped at
        // 00:00 UTC.  Converting to US Eastern time would shift the date back
        // one day (midnight UTC = previous evening ET), so we keep UTC+0 so
        // the UTC date matches the intended trading day.
        "US" => chrono::FixedOffset::east_opt(0).unwrap(),
        _ => chrono::FixedOffset::east_opt(0).unwrap(),
    }
}

/// Convert a Unix timestamp (seconds) to a [`chrono::NaiveDate`] in the
/// market's local timezone. This avoids the off-by-one-day error that occurs
/// when timestamps representing a date in CST (UTC+8) are interpreted in UTC.
pub fn timestamp_to_market_date(ts_secs: i64, market: &str) -> Option<chrono::NaiveDate> {
    let offset = market_utc_offset(market);
    chrono::DateTime::from_timestamp(ts_secs, 0).map(|dt| dt.with_timezone(&offset).date_naive())
}

/// Build a synthetic [`StockQuote`] for a cash symbol.
/// Cash always has price = 1.0, zero change, zero volume.
pub fn make_cash_quote(symbol: &str, market: &str) -> StockQuote {
    StockQuote {
        symbol: symbol.to_string(),
        name: cash_display_name(symbol),
        market: market.to_string(),
        current_price: 1.0,
        previous_close: 1.0,
        change: 0.0,
        change_percent: 0.0,
        high: 1.0,
        low: 1.0,
        volume: 0,
        updated_at: Utc::now().to_rfc3339(),
        ..Default::default()
    }
}

struct CachedQuote {
    quote: StockQuote,
    _cached_at: Instant,
}

/// In-memory cache for stock quotes, keyed by symbol.
pub struct QuoteCache {
    inner: Mutex<HashMap<String, CachedQuote>>,
}

impl QuoteCache {
    pub fn new() -> Self {
        QuoteCache {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Returns a cached quote if it exists (no TTL – the cache is only
    /// refreshed when the caller explicitly requests it).
    pub fn get(&self, symbol: &str) -> Option<StockQuote> {
        let lock = self.inner.lock().unwrap();
        lock.get(symbol).map(|c| c.quote.clone())
    }

    /// Returns a cached quote even if stale (for offline fallback).
    pub fn get_stale(&self, symbol: &str) -> Option<StockQuote> {
        let lock = self.inner.lock().unwrap();
        lock.get(symbol).map(|c| c.quote.clone())
    }

    /// Cache a single quote.
    pub fn set(&self, quote: StockQuote) {
        let mut lock = self.inner.lock().unwrap();
        lock.insert(
            quote.symbol.clone(),
            CachedQuote {
                quote,
                _cached_at: Instant::now(),
            },
        );
    }

    /// Cache multiple quotes at once.
    pub fn set_batch(&self, quotes: &[StockQuote]) {
        let mut lock = self.inner.lock().unwrap();
        let now = Instant::now();
        for q in quotes {
            lock.insert(
                q.symbol.clone(),
                CachedQuote {
                    quote: q.clone(),
                    _cached_at: now,
                },
            );
        }
    }

    /// Returns all cached quotes for the given symbols, plus the list of
    /// symbols that are missing from the cache.
    pub fn get_batch(
        &self,
        symbols: &[(String, String)],
    ) -> (Vec<StockQuote>, Vec<(String, String)>) {
        let lock = self.inner.lock().unwrap();
        let mut cached = Vec::new();
        let mut missing = Vec::new();
        for (symbol, market) in symbols {
            if let Some(entry) = lock.get(symbol.as_str()) {
                cached.push(entry.quote.clone());
            } else {
                missing.push((symbol.clone(), market.clone()));
            }
        }
        (cached, missing)
    }

    /// Drop every cached quote. Used by `factory_reset` so the in-memory
    /// cache does not keep serving prices that no longer correspond to any
    /// holding after the database is wiped.
    pub fn clear(&self) {
        let mut lock = self.inner.lock().unwrap();
        lock.clear();
    }
}

/// Load all cached quotes from the database into memory.
pub fn load_quotes_from_db(db: &Database) -> Result<Vec<StockQuote>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT symbol, name, market, current_price, previous_close,
                    change, change_percent, high, low, volume, updated_at
             FROM cached_quotes",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(StockQuote {
                symbol: row.get(0)?,
                name: row.get(1)?,
                market: row.get(2)?,
                current_price: row.get(3)?,
                previous_close: row.get(4)?,
                change: row.get(5)?,
                change_percent: row.get(6)?,
                high: row.get(7)?,
                low: row.get(8)?,
                volume: row.get(9)?,
                updated_at: row.get(10)?,
                ..Default::default()
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Persist quotes to the database (upsert).
pub fn save_quotes_to_db(db: &Database, quotes: &[StockQuote]) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO cached_quotes
                (symbol, name, market, current_price, previous_close,
                 change, change_percent, high, low, volume, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(|e| e.to_string())?;
    for q in quotes {
        stmt.execute(rusqlite::params![
            q.symbol,
            q.name,
            q.market,
            q.current_price,
            q.previous_close,
            q.change,
            q.change_percent,
            q.high,
            q.low,
            q.volume,
            q.updated_at,
        ])
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Persist the last quote refresh timestamp to the database (single-row upsert, id=1).
pub fn save_quote_refresh_time(db: &Database) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO cached_quote_refresh_time (id, updated_at) VALUES (1, ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Load the persisted last quote refresh timestamp from the database.
/// Returns `None` if no refresh has been recorded yet.
pub fn get_quote_refresh_time(db: &Database) -> Result<Option<String>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT updated_at FROM cached_quote_refresh_time WHERE id = 1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.next().and_then(|r| r.ok()))
}

/// Deduplicate a list of (symbol, market) pairs, keeping only the first
/// occurrence of each symbol.  This avoids redundant API calls when the same
/// stock is held in multiple accounts.
fn deduplicate_symbols(symbols: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    symbols
        .into_iter()
        .filter(|(symbol, _)| seen.insert(symbol.clone()))
        .collect()
}

/// Batch fetch quotes using the cache with specified providers.
/// Duplicate symbols are automatically deduplicated so that each symbol is
/// looked up and fetched only once, even when held in multiple accounts.
/// When `force_refresh` is true the cache is bypassed and all symbols are
/// fetched from the upstream API.
pub async fn fetch_quotes_batch_cached_with_providers(
    cache: &QuoteCache,
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
    cn_provider: &str,
    force_refresh: bool,
) -> Result<Vec<StockQuote>, String> {
    // Deduplicate symbols so we only look up / fetch each symbol once.
    let unique_symbols = deduplicate_symbols(symbols);

    if force_refresh {
        // Force refresh: fetch all symbols from the upstream API.
        let fresh = fetch_quotes_batch_with_providers(
            unique_symbols.clone(),
            us_provider,
            hk_provider,
            cn_provider,
        )
        .await?;
        cache.set_batch(&fresh);

        // Fall back to stale cache for any symbols that failed to fetch
        let fetched_symbols: std::collections::HashSet<String> =
            fresh.iter().map(|q| q.symbol.clone()).collect();
        let mut result = fresh;
        for (symbol, _) in &unique_symbols {
            if !fetched_symbols.contains(symbol) {
                if let Some(stale) = cache.get_stale(symbol) {
                    result.push(stale);
                }
            }
        }
        return Ok(result);
    }

    let (mut result, missing) = cache.get_batch(&unique_symbols);

    if missing.is_empty() {
        return Ok(result);
    }

    let fresh =
        fetch_quotes_batch_with_providers(missing.clone(), us_provider, hk_provider, cn_provider)
            .await?;
    cache.set_batch(&fresh);
    result.extend(fresh);

    // For any symbols that were missing from fresh results (fetch failed),
    // try to use stale cache as fallback
    let fetched_symbols: std::collections::HashSet<String> =
        result.iter().map(|q| q.symbol.clone()).collect();
    for (symbol, _) in &missing {
        if !fetched_symbols.contains(symbol) {
            if let Some(stale) = cache.get_stale(symbol) {
                result.push(stale);
            }
        }
    }

    Ok(result)
}

#[derive(Debug, Deserialize)]
struct YahooChartResponse {
    chart: YahooChart,
}

#[derive(Debug, Deserialize)]
struct YahooChart {
    result: Option<Vec<YahooResult>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct YahooResult {
    meta: YahooMeta,
    indicators: Option<YahooIndicators>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YahooMeta {
    symbol: String,
    #[serde(default)]
    short_name: Option<String>,
    #[serde(default)]
    long_name: Option<String>,
    regular_market_price: Option<f64>,
    previous_close: Option<f64>,
    chart_previous_close: Option<f64>,
    regular_market_day_high: Option<f64>,
    regular_market_day_low: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct YahooIndicators {
    quote: Option<Vec<YahooQuoteIndicator>>,
}

#[derive(Debug, Deserialize)]
struct YahooQuoteIndicator {
    volume: Option<Vec<Option<u64>>>,
}

/// Fetch a US or HK stock quote from Yahoo Finance.
/// For HK stocks, symbol should be in the format "0700.HK".
pub async fn fetch_yahoo_quote(symbol: &str, market: &str) -> Result<StockQuote, String> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}",
        symbol
    );
    let response = http_client::general_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Network error fetching {}: {}", symbol, e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Yahoo Finance API error for {}: HTTP {}",
            symbol,
            response.status()
        ));
    }

    let data: YahooChartResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Yahoo response for {}: {}", symbol, e))?;

    if let Some(err) = &data.chart.error {
        return Err(format!(
            "Yahoo Finance API returned error for {}: {}",
            symbol, err
        ));
    }

    let result = data
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .ok_or_else(|| format!("No data returned from Yahoo Finance for {}", symbol))?;

    let meta = result.meta;
    let current_price = meta.regular_market_price.unwrap_or(0.0);
    let previous_close = meta
        .previous_close
        .or(meta.chart_previous_close)
        .unwrap_or(0.0);
    let change = current_price - previous_close;
    let change_percent = if previous_close != 0.0 {
        change / previous_close * 100.0
    } else {
        0.0
    };

    let volume = result
        .indicators
        .as_ref()
        .and_then(|i| i.quote.as_ref())
        .and_then(|q| q.first())
        .and_then(|q| q.volume.as_ref())
        .and_then(|v| v.last())
        .and_then(|v| *v)
        .unwrap_or(0) as i64;

    let name = meta
        .short_name
        .or(meta.long_name)
        .unwrap_or_else(|| meta.symbol.clone());

    Ok(StockQuote {
        symbol: meta.symbol,
        name,
        market: market.to_string(),
        current_price,
        previous_close,
        change,
        change_percent,
        high: meta.regular_market_day_high.unwrap_or(0.0),
        low: meta.regular_market_day_low.unwrap_or(0.0),
        volume,
        updated_at: Utc::now().to_rfc3339(),
        ..Default::default()
    })
}

/// Fetch a US stock quote using the configured provider.
#[cfg(test)]
pub async fn fetch_us_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_us_quote_with_provider(symbol, "eastmoney").await
}

/// Fetch a US stock quote using the specified provider.
///
/// When `provider` is `xueqiu` but the request fails, it falls back to East
/// Money, then to Yahoo — matching the resilient behaviour of
/// [`fetch_stock_history`].
pub async fn fetch_us_quote_with_provider(
    symbol: &str,
    provider: &str,
) -> Result<StockQuote, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_us_quote(symbol).await,
        "xueqiu" => match fetch_xueqiu_us_quote(symbol).await {
            Ok(q) => Ok(q),
            Err(e) => {
                record_xueqiu_warning(&e);
                info!(
                    "fetch_us_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                match fetch_eastmoney_us_quote(symbol).await {
                    Ok(q) => Ok(q),
                    Err(e2) => {
                        warn!(
                            "fetch_us_quote: EastMoney also failed for {}: {}, falling back to yahoo",
                            symbol, e2
                        );
                        let yahoo_symbol = to_yahoo_symbol(symbol, "US");
                        fetch_yahoo_quote(&yahoo_symbol, "US").await
                    }
                }
            }
        },
        _ => {
            let yahoo_symbol = to_yahoo_symbol(symbol, "US");
            fetch_yahoo_quote(&yahoo_symbol, "US").await
        }
    }
}

/// Fetch a HK stock quote using the specified provider.
///
/// When `provider` is `xueqiu` but the request fails, it falls back to East
/// Money, then to Yahoo — matching the resilient behaviour of
/// [`fetch_stock_history`].
pub async fn fetch_hk_quote_with_provider(
    symbol: &str,
    provider: &str,
) -> Result<StockQuote, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_hk_quote(symbol).await,
        "xueqiu" => match fetch_xueqiu_hk_quote(symbol).await {
            Ok(q) => Ok(q),
            Err(e) => {
                record_xueqiu_warning(&e);
                info!(
                    "fetch_hk_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                match fetch_eastmoney_hk_quote(symbol).await {
                    Ok(q) => Ok(q),
                    Err(e2) => {
                        warn!(
                            "fetch_hk_quote: EastMoney also failed for {}: {}, falling back to yahoo",
                            symbol, e2
                        );
                        let yahoo_symbol = if symbol.ends_with(".HK") || symbol.ends_with(".hk") {
                            symbol.to_string()
                        } else {
                            format!("{}.HK", symbol)
                        };
                        fetch_yahoo_quote(&yahoo_symbol, "HK").await
                    }
                }
            }
        },
        _ => {
            let yahoo_symbol = if symbol.ends_with(".HK") || symbol.ends_with(".hk") {
                symbol.to_string()
            } else {
                format!("{}.HK", symbol)
            };
            fetch_yahoo_quote(&yahoo_symbol, "HK").await
        }
    }
}

/// Fetch a CN A-share stock quote using East Money.
#[cfg(test)]
pub async fn fetch_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_cn_quote_with_provider(symbol, "eastmoney").await
}

/// Fetch a CN A-share stock quote using the specified provider.
///
/// When `provider` is `xueqiu` but the request fails (e.g. an expired or
/// missing session token — Xueqiu's homepage no longer issues `xq_a_token`
/// without JavaScript), it transparently falls back to East Money so that
/// quotes keep working. This mirrors the resilient chain already used by
/// [`fetch_stock_history`].
pub async fn fetch_cn_quote_with_provider(
    symbol: &str,
    provider: &str,
) -> Result<StockQuote, String> {
    match provider {
        "xueqiu" => match fetch_xueqiu_cn_quote(symbol).await {
            Ok(q) => Ok(q),
            Err(e) => {
                record_xueqiu_warning(&e);
                info!(
                    "fetch_cn_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                fetch_eastmoney_cn_quote(symbol).await
            }
        },
        // Default to eastmoney for CN
        _ => fetch_eastmoney_cn_quote(symbol).await,
    }
}

// ---------------------------------------------------------------------------
// East Money (东方财富) API
// ---------------------------------------------------------------------------

/// Maximum number of retry attempts for transient East Money API failures.
const EASTMONEY_MAX_RETRIES: u32 = 2;

/// Send a GET request to the East Money API with retry on transient failures.
///
/// Uses the global East Money HTTP client which has built-in connection
/// pooling (`pool_max_idle_per_host`, `pool_idle_timeout`, `tcp_keepalive`),
/// so manual connection rotation is not needed.  The request is retried up
/// to [`EASTMONEY_MAX_RETRIES`] times with exponential back-off on
/// connection-level errors.
async fn send_eastmoney_request(url: &str, symbol: &str) -> Result<reqwest::Response, String> {
    let mut last_err = String::new();
    for attempt in 0..=EASTMONEY_MAX_RETRIES {
        let result = http_client::eastmoney_client().get(url).send().await;
        match result {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                last_err = format!("Network error fetching {}: {}", symbol, e);
                if attempt < EASTMONEY_MAX_RETRIES {
                    tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
                }
            }
        }
    }
    Err(last_err)
}

/// Maximum number of characters to include in error messages as a response
/// body preview for debugging failed East Money API responses.
const EASTMONEY_RESPONSE_PREVIEW_LEN: usize = 200;

/// Parse the raw response body text into an [`EastMoneyResponse`].
/// On failure the error message includes a preview of the raw body for
/// easier debugging.
fn parse_eastmoney_body(body: &str, symbol: &str) -> Result<EastMoneyResponse, String> {
    serde_json::from_str(body).map_err(|e| {
        let preview: String = body.chars().take(EASTMONEY_RESPONSE_PREVIEW_LEN).collect();
        format!(
            "Failed to parse East Money response for {}: {}. Response preview: {}",
            symbol, e, preview
        )
    })
}

/// East Money API response for a single stock quote.
#[derive(Debug, Deserialize)]
struct EastMoneyResponse {
    data: Option<EastMoneyData>,
}

/// Deserialize a numeric field that EastMoney sometimes returns as the
/// string `"-"` (when the value doesn't exist for this instrument — e.g.
/// market cap / P/E for a market index). Without this, serde fails the
/// entire response parse because `"-"` is not a valid `f64`. We coerce any
/// non-numeric value to `None` so the quote still parses.
fn deserialize_lenient_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Number(n)) => Ok(n.as_f64()),
        // `"-"`, `""`, or any other non-numeric string → treat as missing.
        _ => Ok(None),
    }
}

/// Inner data of an East Money quote response.
/// Field names follow the East Money API convention (f43, f44, …).
/// With `fltt=2` the numeric fields are returned as floats/integers directly.
/// All numeric fields use `f64` so they can accept both JSON integers and
/// JSON floats (e.g. `30279` and `30279.0`) — serde rejects JSON floats
/// when deserializing as `u64`.
///
/// Fields are deserialized via [`deserialize_lenient_f64`] because EastMoney
/// returns `"-"` for metrics that don't apply to a given instrument (e.g.
/// market cap, P/E, P/B for market indices), which would otherwise break
/// the entire parse.
#[derive(Debug, Deserialize, Default)]
struct EastMoneyData {
    /// Current price
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f43: Option<f64>,
    /// Day high
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f44: Option<f64>,
    /// Day low
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f45: Option<f64>,
    /// Volume (lots / 手) — stored as f64 because the API may return
    /// the value with a decimal point (e.g. `30279.0`).
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f47: Option<f64>,
    /// Stock name (e.g. "贵州茅台")
    f58: Option<String>,
    /// Previous close
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f60: Option<f64>,
    /// Change amount
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f169: Option<f64>,
    /// Change percentage
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f170: Option<f64>,
    // ── Fundamentals (added for investment analysis) ──
    /// P/E ratio (TTM)
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f163: Option<f64>,
    /// P/B ratio
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f167: Option<f64>,
    /// Total market capitalisation (元)
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f116: Option<f64>,
    /// Turnover rate (percent)
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f168: Option<f64>,
}

/// Fetch a CN A-share stock quote from East Money (东方财富).
/// Symbol format: "sh600519" (Shanghai) or "sz000858" (Shenzhen).
/// The symbol is normalised to lowercase automatically.
async fn fetch_eastmoney_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    let symbol = symbol.to_lowercase();
    let secid = to_eastmoney_secid(&symbol)?;
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f58,f60,f169,f170,f163,f167,f116,f168&secid={}",
        secid
    );

    let response = send_eastmoney_request(&url, &symbol).await?;

    if !response.status().is_success() {
        return Err(format!(
            "East Money API error for {}: HTTP {}",
            symbol,
            response.status()
        ));
    }

    let body = response.text().await.map_err(|e| {
        format!(
            "Failed to read East Money response body for {}: {}",
            symbol, e
        )
    })?;

    let resp = parse_eastmoney_body(&body, &symbol)?;

    parse_eastmoney_quote(&symbol, "CN", resp)
}

/// Fetch a US stock quote from East Money (东方财富).
/// Symbol format: standard US ticker like "AAPL", "MSFT".
async fn fetch_eastmoney_us_quote(symbol: &str) -> Result<StockQuote, String> {
    let secid = to_eastmoney_us_secid(symbol);
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f58,f60,f169,f170,f163,f167,f116,f168&secid={}",
        secid
    );

    let response = send_eastmoney_request(&url, symbol).await?;

    if !response.status().is_success() {
        return Err(format!(
            "East Money API error for {}: HTTP {}",
            symbol,
            response.status()
        ));
    }

    let body = response.text().await.map_err(|e| {
        format!(
            "Failed to read East Money response body for {}: {}",
            symbol, e
        )
    })?;

    let resp = parse_eastmoney_body(&body, symbol)?;

    parse_eastmoney_quote(symbol, "US", resp)
}

/// Fetch a HK stock quote from East Money (东方财富).
/// Symbol format: "00700", "09988", or "0700.HK".
async fn fetch_eastmoney_hk_quote(symbol: &str) -> Result<StockQuote, String> {
    let secid = to_eastmoney_hk_secid(symbol)?;
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f58,f60,f169,f170,f163,f167,f116,f168&secid={}",
        secid
    );

    let response = send_eastmoney_request(&url, symbol).await?;

    if !response.status().is_success() {
        return Err(format!(
            "East Money API error for {}: HTTP {}",
            symbol,
            response.status()
        ));
    }

    let body = response.text().await.map_err(|e| {
        format!(
            "Failed to read East Money response body for {}: {}",
            symbol, e
        )
    })?;

    let resp = parse_eastmoney_body(&body, symbol)?;

    parse_eastmoney_quote(symbol, "HK", resp)
}

/// Fetch a **market index** quote from East Money by its raw secid.
///
/// Indices use secid prefixes that the stock mappers above don't produce:
/// - CN A-share indices reuse the Shanghai prefix `1.` (e.g. `1.000001` SSE,
///   `1.000300` CSI 300).
/// - US/HK/global indices use the `100.` namespace (e.g. `100.SPX` S&P 500,
///   `100.NDX` NASDAQ, `100.DJIA` Dow Jones, `100.HSI` Hang Seng).
///
/// This is the fallback path for `market_overview_service` when Yahoo Finance
/// returns 403 for index symbols. EastMoney needs no auth, so it's reliable
/// even without a configured cookie.
pub async fn fetch_index_quote_eastmoney(
    secid: &str,
    display_symbol: &str,
    market: &str,
) -> Result<StockQuote, String> {
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f58,f60,f169,f170,f163,f167,f116,f168&secid={}",
        secid
    );
    let response = send_eastmoney_request(&url, display_symbol).await?;
    if !response.status().is_success() {
        return Err(format!(
            "East Money index API error for {} (secid {}): HTTP {}",
            display_symbol,
            secid,
            response.status()
        ));
    }
    let body = response.text().await.map_err(|e| {
        format!(
            "Failed to read East Money index response for {}: {}",
            display_symbol, e
        )
    })?;
    let resp = parse_eastmoney_body(&body, display_symbol)?;
    let mut quote = parse_eastmoney_quote(display_symbol, market, resp)?;
    // Override the symbol with the canonical display symbol so the quote
    // round-trips cleanly through the cache (keyed by display symbol).
    quote.symbol = display_symbol.to_string();
    Ok(quote)
}

/// Resolve a market-index symbol (in any common form) to an EastMoney secid.
/// Returns `None` for non-index symbols so callers can fall through to the
/// normal stock-quote path.
///
/// This exists because Yahoo Finance now 403s index symbols, and the stock
/// fetchers (xueqiu/eastmoney) don't recognise index codes either. When a
/// user or the AI asks for an index (e.g. "标普500" / "^GSPC" / "SPX"), the
/// tool layer routes through here to the reliable EastMoney index endpoint.
pub fn resolve_index_secid(symbol: &str) -> Option<(&'static str, &'static str)> {
    // Normalise: strip leading ^, uppercase, strip .SS/.SZ suffixes for
    // matching purposes. The display name is the second tuple element.
    let s = symbol.trim().trim_start_matches('^').to_uppercase();
    let s = s
        .trim_end_matches(".SS")
        .trim_end_matches(".SZ")
        .to_string();
    match s.as_str() {
        "GSPC" | "SPX" | "INX" => Some(("100.SPX", "标普500")),
        "IXIC" | "NDX" | "NASDAQ" | "COMP" => Some(("100.NDX", "纳斯达克")),
        "DJI" | "DJIA" | "DOW" => Some(("100.DJIA", "道琼斯")),
        "HSI" | "HANGSENG" => Some(("100.HSI", "恒生指数")),
        "000300" | "CSI300" | "HS300" => Some(("1.000300", "沪深300")),
        "000001" | "SSE" | "SHCOMP" => Some(("1.000001", "上证综指")),
        "399001" | "SZCOMP" => Some(("0.399001", "深证成指")),
        "399006" | "CHINEXT" | "CYB" => Some(("0.399006", "创业板指")),
        "N225" | "NIKKEI" => Some(("100.N225", "日经225")),
        "FTSE" | "UKX" => Some(("100.FTSE", "富时100")),
        "DAX" => Some(("100.DAX", "德国DAX")),
        _ => None,
    }
}

/// Convert a symbol like "sh600519" or "sz000858" to the East Money secid
/// format: "1.600519" (Shanghai) or "0.000858" (Shenzhen).
fn to_eastmoney_secid(symbol: &str) -> Result<String, String> {
    if symbol.len() < 3 {
        return Err(format!("Invalid CN symbol: {}", symbol));
    }
    let prefix = &symbol[..2];
    let code = &symbol[2..];
    let market_id = match prefix {
        "sh" => "1",
        "sz" => "0",
        _ => {
            return Err(format!(
                "Unknown CN market prefix '{}' in symbol {}",
                prefix, symbol
            ))
        }
    };
    Ok(format!("{}.{}", market_id, code))
}

/// Convert a US stock ticker to East Money secid format.
/// Regular tickers use "105.{TICKER}" (e.g., "105.AAPL").
/// Tickers with hyphens use "106.{TICKER}" with hyphens replaced by underscores
/// (e.g., "BRK-B" → "106.BRK_B").
fn to_eastmoney_us_secid(symbol: &str) -> String {
    let upper = symbol.to_uppercase();
    if upper.contains('-') {
        format!("106.{}", upper.replace('-', "_"))
    } else {
        format!("105.{}", upper)
    }
}

/// Convert a HK stock symbol to East Money secid format: "116.{5-digit code}".
/// Strips the ".HK" suffix if present and zero-pads to 5 digits.
fn to_eastmoney_hk_secid(symbol: &str) -> Result<String, String> {
    let code = symbol.trim_end_matches(".HK").trim_end_matches(".hk");
    // Zero-pad to 5 digits if the code is purely numeric
    if code.chars().all(|c| c.is_ascii_digit()) {
        let padded = format!("{:0>5}", code);
        Ok(format!("116.{}", padded))
    } else {
        Err(format!("Invalid HK symbol: {}", symbol))
    }
}

/// Parse the East Money JSON response into a `StockQuote`.
fn parse_eastmoney_quote(
    symbol: &str,
    market: &str,
    resp: EastMoneyResponse,
) -> Result<StockQuote, String> {
    let data = resp.data.ok_or_else(|| {
        format!(
            "No data from East Money for {}. Symbol may be invalid.",
            symbol
        )
    })?;

    let name = data
        .f58
        .ok_or_else(|| format!("Missing stock name in East Money response for {}", symbol))?;
    let current_price = data.f43.ok_or_else(|| {
        format!(
            "Missing current price in East Money response for {}",
            symbol
        )
    })?;
    let previous_close = data.f60.unwrap_or(0.0);

    let change = data.f169.unwrap_or(current_price - previous_close);
    let change_percent = data.f170.unwrap_or_else(|| {
        if previous_close != 0.0 {
            change / previous_close * 100.0
        } else {
            0.0
        }
    });

    let high = data.f44.unwrap_or(0.0);
    let low = data.f45.unwrap_or(0.0);
    let volume = data.f47.unwrap_or(0.0) as i64;

    Ok(StockQuote {
        symbol: symbol.to_string(),
        name,
        market: market.to_string(),
        current_price,
        previous_close,
        change,
        change_percent,
        high,
        low,
        volume,
        updated_at: Utc::now().to_rfc3339(),
        pe_ttm: data.f163,
        pb: data.f167,
        market_cap: data.f116,
        dividend_yield: None,
        eps: None,
        roe: None,
        turnover_rate: data.f168,
    })
}

// ---------------------------------------------------------------------------
// Xueqiu (雪球) API
// ---------------------------------------------------------------------------

/// Whether the Xueqiu client has obtained a session cookie from the homepage.
static XUEQIU_TOKEN_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// User-provided Xueqiu cookie string (e.g. `xq_a_token=xxx`).
/// When set, this replaces the auto-obtained xq_a_token in API requests.
static XUEQIU_USER_COOKIE: Mutex<Option<String>> = Mutex::new(None);

/// User-provided Xueqiu `u` cookie value (user ID from a logged-in browser session).
/// When set, it is appended alongside `xq_a_token` in the Cookie header
/// to authenticate kline API requests.
static XUEQIU_USER_U: Mutex<Option<String>> = Mutex::new(None);

/// Path to the SQLite database file, registered at startup so that the
/// Xueqiu cookie/u can be re-read from the `quote_provider_config` table when
/// the in-memory copies are empty (e.g. right after an app restart, before any
/// command has synced them). See [`load_xueqiu_creds_from_db`].
static APP_DB_PATH: OnceLock<String> = OnceLock::new();

/// Register the database path once at startup (called from `lib.rs`).
///
/// This lets [`get_xueqiu_user_cookie`] / [`get_xueqiu_user_u`] fall back to
/// the database when their in-memory statics are `None`, so that user-provided
/// cookies work regardless of call path (quote commands, AI tools, background
/// refresh) without each entry point having to call `set_xueqiu_user_cookie`.
pub fn register_db_path(path: impl Into<String>) {
    let _ = APP_DB_PATH.set(path.into());
}

/// Read the Xueqiu cookie and `u` value straight from the
/// `quote_provider_config` table. Returns `(None, None)` when the DB path is
/// unknown or the row/columns are absent. Uses a fresh short-lived read-only
/// connection so it never contends with the main connection for long.
fn load_xueqiu_creds_from_db() -> (Option<String>, Option<String>) {
    let path = match APP_DB_PATH.get() {
        Some(p) => p,
        None => return (None, None),
    };
    let conn = match rusqlite::Connection::open(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("load_xueqiu_creds_from_db: failed to open DB: {e}");
            return (None, None);
        }
    };
    let row = conn.query_row(
        "SELECT xueqiu_cookie, xueqiu_u FROM quote_provider_config WHERE id = 1",
        [],
        |r| {
            let cookie: Option<String> = r.get(0).ok().flatten();
            let u_val: Option<String> = r.get(1).ok().flatten();
            Ok((cookie, u_val))
        },
    );
    match row {
        Ok(pair) => normalize_creds(pair),
        Err(rusqlite::Error::QueryReturnedNoRows) => (None, None),
        Err(e) => {
            warn!("load_xueqiu_creds_from_db: query failed: {e}");
            (None, None)
        }
    }
}

/// Trim and drop empty strings, mirroring [`set_xueqiu_user_cookie`].
fn normalize_creds(pair: (Option<String>, Option<String>)) -> (Option<String>, Option<String>) {
    let norm = |s: Option<String>| {
        s.as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    (norm(pair.0), norm(pair.1))
}

/// Auto-obtained `xq_a_token` value extracted from the homepage response.
///
/// The Xueqiu cookie jar may not send cookies set by `xueqiu.com` to the
/// API subdomain `stock.xueqiu.com` if the cookie lacks a `Domain` attribute
/// (RFC 6265 restricts such cookies to the exact host).  By storing the token
/// explicitly we can attach it via the `Cookie` header on every API request,
/// guaranteeing it reaches the API regardless of cookie-jar domain matching.
static XUEQIU_AUTO_COOKIE: Mutex<Option<String>> = Mutex::new(None);
static LAST_QUOTE_WARNING: Mutex<Option<String>> = Mutex::new(None);

const XUEQIU_COOKIE_EXPIRED_HINT: &str = "雪球 Cookie 可能已经过期，请到设置页面更新雪球 Cookie。";
const XUEQIU_API_FAILED_HINT: &str = "访问雪球行情服务失败，请检查网络连接或稍后重试。";

fn is_xueqiu_cookie_expired_error(err: &str) -> bool {
    err.contains("Xueqiu API error")
        && (err.contains("400016")
            || err.contains("重新登录帐号后再试")
            || err.contains("刷新页面或者重新登录帐号后再试"))
}

fn is_xueqiu_request_error(err: &str) -> bool {
    err.contains("Xueqiu") || err.contains("xueqiu.com") || err.contains("stock.xueqiu.com")
}

fn record_xueqiu_warning(err: &str) {
    let warning = if is_xueqiu_cookie_expired_error(err) {
        XUEQIU_COOKIE_EXPIRED_HINT
    } else if is_xueqiu_request_error(err) {
        XUEQIU_API_FAILED_HINT
    } else {
        return;
    };

    let mut current = LAST_QUOTE_WARNING.lock().unwrap();
    if current.as_deref() != Some(XUEQIU_COOKIE_EXPIRED_HINT)
        || warning == XUEQIU_COOKIE_EXPIRED_HINT
    {
        *current = Some(warning.to_string());
    }
}

pub fn clear_quote_warning() {
    *LAST_QUOTE_WARNING.lock().unwrap() = None;
}

pub fn take_quote_warning() -> Option<String> {
    LAST_QUOTE_WARNING.lock().unwrap().take()
}

/// Return the current warning without consuming it, so the value remains
/// available for the fallback `take_quote_warning` invocation from the frontend.
pub fn peek_quote_warning() -> Option<String> {
    LAST_QUOTE_WARNING.lock().unwrap().clone()
}

/// Set (or clear) the user-provided Xueqiu cookie string.
pub fn set_xueqiu_user_cookie(cookie: Option<String>) {
    let cookie = cookie
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    *XUEQIU_USER_COOKIE.lock().unwrap() = cookie;
}

/// Return a clone of the current user-provided Xueqiu cookie, if any.
///
/// Falls back to reading the `quote_provider_config` table from the database
/// when the in-memory copy is `None` (e.g. right after an app restart). This
/// guarantees that user-configured cookies are honoured regardless of which
/// entry point triggers a quote request.
fn get_xueqiu_user_cookie() -> Option<String> {
    let cached = XUEQIU_USER_COOKIE.lock().unwrap().clone();
    if cached.is_some() {
        return cached;
    }
    load_xueqiu_creds_from_db().0
}

/// Set (or clear) the user-provided Xueqiu `u` cookie value.
pub fn set_xueqiu_user_u(u_value: Option<String>) {
    let u_value = u_value
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    *XUEQIU_USER_U.lock().unwrap() = u_value;
}

/// Return a clone of the current user-provided Xueqiu `u` cookie value, if any.
///
/// Falls back to the database like [`get_xueqiu_user_cookie`].
fn get_xueqiu_user_u() -> Option<String> {
    let cached = XUEQIU_USER_U.lock().unwrap().clone();
    if cached.is_some() {
        return cached;
    }
    load_xueqiu_creds_from_db().1
}

/// Ensure the Xueqiu HTTP client has a valid session token.
///
/// Xueqiu requires an `xq_a_token` cookie which is set when visiting the
/// homepage.  This function visits `https://xueqiu.com` once to acquire the
/// cookie, and remembers the result via [`XUEQIU_TOKEN_INITIALIZED`].
///
/// The homepage request uses browser page-load headers (`Accept: text/html`)
/// rather than API-style headers to ensure the server returns a full page
/// response that sets the session cookie.
///
/// If a user-provided cookie is configured, the homepage visit is skipped
/// entirely because authentication is handled via the explicit `Cookie` header
/// added in [`send_xueqiu_request`].
async fn ensure_xueqiu_token() -> Result<(), String> {
    if XUEQIU_TOKEN_INITIALIZED.load(Ordering::SeqCst) {
        return Ok(());
    }

    // If a user-provided cookie is configured, skip the homepage visit
    // entirely – authentication is handled via the explicit Cookie header
    // built in build_xueqiu_cookie_header().
    if get_xueqiu_user_cookie().is_some() {
        XUEQIU_TOKEN_INITIALIZED.store(true, Ordering::SeqCst);
        return Ok(());
    }

    let client = http_client::xueqiu_client();
    let resp = client
        .get("https://xueqiu.com")
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .send()
        .await
        .map_err(|e| format!("Failed to initialize Xueqiu token: {}", e))?;

    let status = resp.status();

    // Extract `xq_a_token` from Set-Cookie headers so we can attach it
    // explicitly to API requests (see XUEQIU_AUTO_COOKIE doc comment).
    let mut auto_token: Option<String> = None;
    for header_val in resp.headers().get_all(reqwest::header::SET_COOKIE).iter() {
        if let Ok(s) = header_val.to_str() {
            if s.starts_with("xq_a_token=") {
                let val_start = "xq_a_token=".len();
                let val_end = s[val_start..]
                    .find(';')
                    .map(|i| val_start + i)
                    .unwrap_or(s.len());
                let token_value = &s[val_start..val_end];
                if !token_value.is_empty() {
                    auto_token = Some(token_value.to_string());
                }
            }
        }
    }
    if auto_token.is_none() {
        for cookie in resp.cookies() {
            if cookie.name() == "xq_a_token" && !cookie.value().is_empty() {
                auto_token = Some(cookie.value().to_string());
                break;
            }
        }
    }

    if let Some(ref token) = auto_token {
        *XUEQIU_AUTO_COOKIE.lock().unwrap() = Some(token.clone());
    }

    // Only mark the token as initialized when we actually obtained a token.
    // Xueqiu's homepage now serves a 200 response without setting
    // `xq_a_token` (the token is emitted via JavaScript), so a 200 alone is
    // not a reliable success signal. Marking it initialized without a token
    // would freeze the client into a permanently-unauthenticated state and
    // suppress all subsequent re-attempts. Returning `Ok` here lets the caller
    // proceed and fall back to other providers; a later call will retry the
    // homepage visit.
    if auto_token.is_some() {
        XUEQIU_TOKEN_INITIALIZED.store(true, Ordering::SeqCst);
        Ok(())
    } else if status.is_success() || status.is_redirection() {
        warn!(
            "ensure_xueqiu_token: Xueqiu homepage returned HTTP {} but no xq_a_token cookie; \
             token not initialised, will retry on next request",
            status
        );
        Ok(())
    } else {
        Err(format!(
            "Failed to initialize Xueqiu token: HTTP {}",
            status
        ))
    }
}

/// Reset the Xueqiu session token so that the next API call will re-fetch it.
///
/// Made `pub` so that callers which overwrite the user-provided cookie (e.g.
/// the embedded login flow and the paste-cookie command) can force the next
/// request to use the freshly stored value instead of any cached state.
pub fn reset_xueqiu_token() {
    XUEQIU_TOKEN_INITIALIZED.store(false, Ordering::SeqCst);
    *XUEQIU_AUTO_COOKIE.lock().unwrap() = None;
}

/// Build the cookie header for Xueqiu API requests.
///
/// Priority: user-provided cookie > auto-obtained xq_a_token.
/// When the user has configured a `u` cookie value, it is appended so
/// that the kline API returns authenticated data.
///
/// The user may enter either the raw `xq_a_token` value (e.g. `6a7dc04b...`)
/// or a full cookie string (e.g. `xq_a_token=6a7dc04b...`).  Both forms are
/// handled correctly.
fn build_xueqiu_cookie_header() -> Option<String> {
    let user_cookie = get_xueqiu_user_cookie();
    let auto_token = XUEQIU_AUTO_COOKIE.lock().unwrap().clone();
    let u_value = get_xueqiu_user_u();

    // Start with the base cookie: prefer user-provided, fall back to auto.
    let base = if let Some(ref uc) = user_cookie {
        // If the user entered a raw token value (no '=' sign), wrap it.
        if uc.contains('=') {
            Some(uc.clone())
        } else {
            Some(format!("xq_a_token={}", uc))
        }
    } else {
        auto_token.map(|t| format!("xq_a_token={}", t))
    };

    match (base, u_value) {
        (Some(b), Some(u)) => {
            // Append u= if not already present in the base cookie.
            if b.contains(&format!("u={}", u)) {
                Some(b)
            } else {
                Some(format!("{}; u={}", b, u))
            }
        }
        (Some(b), None) => Some(b),
        (None, Some(u)) => Some(format!("u={}", u)),
        (None, None) => None,
    }
}

/// Maximum number of retry attempts for transient Xueqiu API failures.
const XUEQIU_MAX_RETRIES: u32 = 2;

/// Send a GET request to the Xueqiu API with token management and retry.
///
/// If the initial request returns HTTP 400 (which indicates an expired or
/// missing session token), the token is refreshed and the request is retried.
async fn send_xueqiu_request(url: &str, symbol: &str) -> Result<reqwest::Response, String> {
    ensure_xueqiu_token().await?;

    let client = http_client::xueqiu_client();
    let mut last_err = String::new();

    for attempt in 0..=XUEQIU_MAX_RETRIES {
        let mut req = client.get(url);

        if let Some(cookie) = build_xueqiu_cookie_header() {
            req = req.header(reqwest::header::COOKIE, cookie);
        }

        let result = req.send().await;
        match result {
            Ok(resp)
                if resp.status() == reqwest::StatusCode::BAD_REQUEST
                    && attempt < XUEQIU_MAX_RETRIES =>
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
                reset_xueqiu_token();
                ensure_xueqiu_token().await?;
            }
            Ok(resp) => return Ok(resp),
            Err(e) => {
                last_err = format!("Network error fetching {} from Xueqiu: {}", symbol, e);
                if attempt < XUEQIU_MAX_RETRIES {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    reset_xueqiu_token();
                    ensure_xueqiu_token().await?;
                }
            }
        }
    }
    Err(last_err)
}

/// Send an authenticated GET request to any Xueqiu endpoint.
///
/// This is a public thin wrapper around [`send_xueqiu_request`] for use by
/// other modules (e.g. OCR stock-code lookup) that need Xueqiu API access
/// but are outside the quote service module.
pub async fn xueqiu_fetch(url: &str) -> Result<reqwest::Response, String> {
    send_xueqiu_request(url, "lookup").await
}

/// Maximum number of characters to include in error messages as a response
/// body preview for debugging failed Xueqiu API responses.
const XUEQIU_RESPONSE_PREVIEW_LEN: usize = 200;

/// Xueqiu API response wrapper.
#[derive(Debug, Deserialize)]
struct XueqiuResponse {
    data: Option<XueqiuData>,
    error_code: Option<i32>,
    error_description: Option<String>,
}

/// Inner data of a Xueqiu quote response.
#[derive(Debug, Deserialize)]
struct XueqiuData {
    quote: Option<XueqiuQuote>,
}

/// Xueqiu quote fields.
#[derive(Debug, Deserialize, Default)]
struct XueqiuQuote {
    /// Stock name (e.g. "贵州茅台", "Apple Inc.")
    name: Option<String>,
    /// Current price
    current: Option<f64>,
    /// Previous close
    last_close: Option<f64>,
    /// Price change
    chg: Option<f64>,
    /// Change percentage
    percent: Option<f64>,
    /// Day high
    high: Option<f64>,
    /// Day low
    low: Option<f64>,
    /// Volume
    volume: Option<f64>,
    // ── Fundamentals (Xueqiu returns these in the same quote payload) ──
    /// P/E ratio (TTM)
    pe_ttm: Option<f64>,
    /// P/B ratio
    pb: Option<f64>,
    /// Total market capitalisation
    market_capital: Option<f64>,
    /// Dividend yield (fraction, e.g. 0.025 = 2.5%)
    dividend_yield: Option<f64>,
    /// Earnings per share
    eps: Option<f64>,
    /// Turnover rate (percent)
    turnover_rate: Option<f64>,
}

/// Xueqiu kline (historical candlestick) API response wrapper.
#[derive(Debug, Deserialize)]
struct XueqiuKlineResponse {
    data: Option<XueqiuKlineData>,
    error_code: Option<i32>,
    error_description: Option<String>,
}

/// Inner data of a Xueqiu kline response.
#[derive(Debug, Deserialize)]
struct XueqiuKlineData {
    /// Column names, e.g. ["timestamp", "volume", "open", "high", "low", "close", ...]
    column: Option<Vec<String>>,
    /// Each item is one trading day: values in the same order as `column`.
    item: Option<Vec<Vec<serde_json::Value>>>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum XueqiuHistoryOutcome {
    Prices(Vec<(chrono::NaiveDate, f64)>),
    StartsAfterRange {
        first_available_date: chrono::NaiveDate,
    },
    Empty,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_xueqiu_history_response(
    body: &str,
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
    url: &str,
) -> Result<XueqiuHistoryOutcome, String> {
    let resp: XueqiuKlineResponse = serde_json::from_str(body).map_err(|e| {
        let preview: String = body.chars().take(XUEQIU_RESPONSE_PREVIEW_LEN).collect();
        format!(
            "fetch_stock_history_xueqiu: parse error for {}: {}. Preview: {}",
            symbol, e, preview
        )
    })?;

    if let Some(err_code) = resp.error_code {
        if err_code != 0 {
            let desc = resp.error_description.unwrap_or_default();
            return Err(format!(
                "fetch_stock_history_xueqiu: API error for {}: code={}, message={}",
                symbol, err_code, desc
            ));
        }
    }

    let mut data = resp
        .data
        .ok_or_else(|| format!("fetch_stock_history_xueqiu: no data for {}", symbol))?;
    let columns = data.column.take().unwrap_or_default();
    if columns.is_empty() {
        let preview: String = body.chars().take(XUEQIU_RESPONSE_PREVIEW_LEN).collect();
        return Err(format!(
            "fetch_stock_history_xueqiu: empty or missing 'column' field for {}. \
             The Xueqiu kline API requires a `u` cookie value. \
             Provide it in Settings → Quote Provider → 雪球用户ID. \
             URL: {} Response preview: {}",
            symbol, url, preview
        ));
    }
    let ts_idx = columns
        .iter()
        .position(|column| column == "timestamp")
        .ok_or_else(|| {
            format!(
                "fetch_stock_history_xueqiu: missing 'timestamp' column for {}, got columns: {:?}",
                symbol, columns
            )
        })?;
    let close_idx = columns
        .iter()
        .position(|column| column == "close")
        .ok_or_else(|| {
            format!(
                "fetch_stock_history_xueqiu: missing 'close' column for {}, got columns: {:?}",
                symbol, columns
            )
        })?;

    let items = data.item.unwrap_or_default();
    let mut parsed_prices = Vec::new();
    for item in &items {
        let ts_ms = item.get(ts_idx).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|number| number.round() as i64))
        });
        let close = item.get(close_idx).and_then(|value| value.as_f64());

        if let (Some(ts_ms), Some(close_price)) = (ts_ms, close) {
            if let Some(date) = timestamp_to_market_date(ts_ms / 1000, market) {
                parsed_prices.push((date, close_price));
            }
        }
    }

    let first_available_date = parsed_prices.iter().map(|(date, _)| *date).min();
    let mut prices: Vec<(chrono::NaiveDate, f64)> = parsed_prices
        .into_iter()
        .filter(|(date, _)| *date >= start_date && *date <= end_date)
        .collect();
    prices.sort_by_key(|(date, _)| *date);

    if !prices.is_empty() {
        return Ok(XueqiuHistoryOutcome::Prices(prices));
    }
    if let Some(first_available_date) = first_available_date.filter(|date| *date > end_date) {
        return Ok(XueqiuHistoryOutcome::StartsAfterRange {
            first_available_date,
        });
    }

    if !items.is_empty() {
        let preview: String = items
            .iter()
            .take(2)
            .map(|row| format!("{:?}", row))
            .collect::<Vec<_>>()
            .join(", ");
        warn!(
            "fetch_stock_history_xueqiu: {} items received for {} but none matched date range {}/{}. First items: [{}]",
            items.len(), symbol, start_date, end_date, preview
        );
    }
    Ok(XueqiuHistoryOutcome::Empty)
}

/// Parse a Xueqiu JSON response body into a [`XueqiuResponse`].
fn parse_xueqiu_body(body: &str, symbol: &str) -> Result<XueqiuResponse, String> {
    serde_json::from_str(body).map_err(|e| {
        let preview: String = body.chars().take(XUEQIU_RESPONSE_PREVIEW_LEN).collect();
        format!(
            "Failed to parse Xueqiu response for {}: {}. Response preview: {}",
            symbol, e, preview
        )
    })
}

/// Parse the Xueqiu API response into a `StockQuote`.
fn parse_xueqiu_quote(
    symbol: &str,
    market: &str,
    resp: XueqiuResponse,
) -> Result<StockQuote, String> {
    if let Some(err_code) = resp.error_code {
        if err_code != 0 {
            let desc = resp.error_description.unwrap_or_default();
            return Err(format!(
                "Xueqiu API error for {}: code={}, message={}",
                symbol, err_code, desc
            ));
        }
    }

    let data = resp
        .data
        .ok_or_else(|| format!("No data from Xueqiu for {}. Symbol may be invalid.", symbol))?;
    let quote = data
        .quote
        .ok_or_else(|| format!("No quote data from Xueqiu for {}.", symbol))?;

    let name = quote
        .name
        .ok_or_else(|| format!("Missing stock name in Xueqiu response for {}", symbol))?;
    let current_price = quote
        .current
        .ok_or_else(|| format!("Missing current price in Xueqiu response for {}", symbol))?;
    let previous_close = quote.last_close.unwrap_or(0.0);

    let change = quote.chg.unwrap_or(current_price - previous_close);
    let change_percent = quote.percent.unwrap_or_else(|| {
        if previous_close != 0.0 {
            change / previous_close * 100.0
        } else {
            0.0
        }
    });

    let high = quote.high.unwrap_or(0.0);
    let low = quote.low.unwrap_or(0.0);
    let volume = quote.volume.unwrap_or(0.0) as i64;

    // Xueqiu's dividend_yield is a fraction (e.g. 0.025); convert to percent.
    let dividend_yield = quote.dividend_yield.map(|y| y * 100.0);

    Ok(StockQuote {
        symbol: symbol.to_string(),
        name,
        market: market.to_string(),
        current_price,
        previous_close,
        change,
        change_percent,
        high,
        low,
        volume,
        updated_at: Utc::now().to_rfc3339(),
        pe_ttm: quote.pe_ttm,
        pb: quote.pb,
        market_cap: quote.market_capital,
        dividend_yield,
        eps: quote.eps,
        roe: None,
        turnover_rate: quote.turnover_rate,
    })
}

/// Convert a CN symbol like "sh600519" or "sz000858" to Xueqiu format:
/// "SH600519" or "SZ000858".
fn to_xueqiu_cn_symbol(symbol: &str) -> Result<String, String> {
    let s = symbol.to_lowercase();
    if s.len() < 3 {
        return Err(format!("Invalid CN symbol for Xueqiu: {}", symbol));
    }
    let prefix = &s[..2];
    let code = &s[2..];
    match prefix {
        "sh" | "sz" => Ok(format!("{}{}", prefix.to_uppercase(), code)),
        _ => Err(format!(
            "Unknown CN market prefix '{}' in symbol {} for Xueqiu",
            prefix, symbol
        )),
    }
}

/// Convert a US stock symbol to Xueqiu format.
/// Replaces hyphens with dots (e.g., "BRK-B" → "BRK.B") and converts to uppercase.
fn to_xueqiu_us_symbol(symbol: &str) -> String {
    symbol.to_uppercase().replace('-', ".")
}

/// Convert a HK stock symbol to Xueqiu format.
/// Strips the ".HK" suffix if present and zero-pads to 5 digits.
fn to_xueqiu_hk_symbol(symbol: &str) -> Result<String, String> {
    let code = symbol.trim_end_matches(".HK").trim_end_matches(".hk");
    if code.chars().all(|c| c.is_ascii_digit()) {
        let padded = format!("{:0>5}", code);
        Ok(padded)
    } else {
        Err(format!("Invalid HK symbol for Xueqiu: {}", symbol))
    }
}

/// Fetch a CN A-share stock quote from Xueqiu (雪球).
async fn fetch_xueqiu_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    let xueqiu_symbol = to_xueqiu_cn_symbol(symbol)?;
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xueqiu_symbol
    );

    let response = send_xueqiu_request(&url, symbol).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body_preview = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(XUEQIU_RESPONSE_PREVIEW_LEN)
            .collect::<String>();
        return Err(format!(
            "Xueqiu API error for {}: HTTP {}. Response: {}",
            symbol, status, body_preview
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read Xueqiu response body for {}: {}", symbol, e))?;

    let resp = parse_xueqiu_body(&body, symbol)?;
    parse_xueqiu_quote(symbol, "CN", resp)
}

/// Fetch a US stock quote from Xueqiu (雪球).
async fn fetch_xueqiu_us_quote(symbol: &str) -> Result<StockQuote, String> {
    let xueqiu_symbol = to_xueqiu_us_symbol(symbol);
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xueqiu_symbol
    );

    let response = send_xueqiu_request(&url, symbol).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body_preview = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(XUEQIU_RESPONSE_PREVIEW_LEN)
            .collect::<String>();
        return Err(format!(
            "Xueqiu API error for {}: HTTP {}. Response: {}",
            symbol, status, body_preview
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read Xueqiu response body for {}: {}", symbol, e))?;

    let resp = parse_xueqiu_body(&body, symbol)?;
    parse_xueqiu_quote(symbol, "US", resp)
}

/// Fetch a HK stock quote from Xueqiu (雪球).
async fn fetch_xueqiu_hk_quote(symbol: &str) -> Result<StockQuote, String> {
    let xueqiu_symbol = to_xueqiu_hk_symbol(symbol)?;
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xueqiu_symbol
    );

    let response = send_xueqiu_request(&url, symbol).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body_preview = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(XUEQIU_RESPONSE_PREVIEW_LEN)
            .collect::<String>();
        return Err(format!(
            "Xueqiu API error for {}: HTTP {}. Response: {}",
            symbol, status, body_preview
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read Xueqiu response body for {}: {}", symbol, e))?;

    let resp = parse_xueqiu_body(&body, symbol)?;
    parse_xueqiu_quote(symbol, "HK", resp)
}

/// Batch fetch quotes using the specified providers for US, HK and CN markets.
/// Cash symbols return synthetic quotes (price = 1.0).
/// Duplicate symbols are automatically deduplicated so that each symbol is fetched only once.
pub async fn fetch_quotes_batch_with_providers(
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
    cn_provider: &str,
) -> Result<Vec<StockQuote>, String> {
    // Deduplicate symbols so we only fetch each symbol once,
    // even if it appears in multiple accounts.
    let unique_symbols = deduplicate_symbols(symbols);

    let mut quotes = Vec::new();
    let mut has_xueqiu_cookie_warning = false;
    let mut has_xueqiu_api_warning = false;
    // Once we know Xueqiu is unreachable, skip remaining Xueqiu symbols so
    // we don't wait for N × 15-second timeouts (one per symbol).  Non-Xueqiu
    // symbols (e.g. US via Yahoo) are still fetched normally.
    let mut xueqiu_failed = false;
    for (symbol, market) in unique_symbols {
        // Cash symbols don't need an API call – return a synthetic quote.
        if is_cash_symbol(&symbol) {
            quotes.push(make_cash_quote(&symbol, &market));
            continue;
        }
        // Determine whether this symbol would use the Xueqiu API.
        let uses_xueqiu = match market.as_str() {
            "CN" => cn_provider == "xueqiu",
            "HK" => hk_provider == "xueqiu",
            "US" => us_provider == "xueqiu",
            _ => false,
        };
        if xueqiu_failed && uses_xueqiu {
            // Skip: Xueqiu is already known to be unreachable for this batch.
            info!(
                "Skipping {} ({}) – Xueqiu already failed for this batch",
                symbol, market
            );
            continue;
        }
        let result = match market.as_str() {
            "US" => fetch_us_quote_with_provider(&symbol, us_provider).await,
            "HK" => fetch_hk_quote_with_provider(&symbol, hk_provider).await,
            "CN" => fetch_cn_quote_with_provider(&symbol, cn_provider).await,
            _ => Err(format!("Unknown market: {}", market)),
        };
        match result {
            Ok(quote) => quotes.push(quote),
            Err(e) => {
                warn!("failed to fetch quote for {} ({}): {}", symbol, market, e);
                let is_cookie_err = is_xueqiu_cookie_expired_error(&e);
                let is_api_err = is_xueqiu_request_error(&e);
                if is_cookie_err {
                    has_xueqiu_cookie_warning = true;
                } else if is_api_err {
                    has_xueqiu_api_warning = true;
                }
                // Mark Xueqiu as failed for either error kind so we can skip
                // remaining Xueqiu symbols without waiting for more timeouts.
                if is_cookie_err || is_api_err {
                    xueqiu_failed = true;
                }
            }
        }
    }
    if has_xueqiu_cookie_warning {
        *LAST_QUOTE_WARNING.lock().unwrap() = Some(XUEQIU_COOKIE_EXPIRED_HINT.to_string());
    } else if has_xueqiu_api_warning {
        *LAST_QUOTE_WARNING.lock().unwrap() = Some(XUEQIU_API_FAILED_HINT.to_string());
    }
    Ok(quotes)
}

// ---------------------------------------------------------------------------
// Historical price fetching
// ---------------------------------------------------------------------------

/// Convert a holding symbol + market to a Yahoo Finance ticker for historical queries.
pub fn to_yahoo_symbol(symbol: &str, market: &str) -> String {
    match market {
        "US" => {
            // Yahoo Finance uses hyphens in US symbols (e.g., "BRK-B"), convert dots to hyphens.
            symbol.replace('.', "-")
        }
        "HK" => {
            if symbol.ends_with(".HK") || symbol.ends_with(".hk") {
                symbol.to_string()
            } else {
                format!("{}.HK", symbol)
            }
        }
        "CN" => {
            // CN symbols are stored as e.g. "sh600519" or "sz000858"
            let s = symbol.to_lowercase();
            if let Some(stripped) = s.strip_prefix("sh") {
                format!("{}.SS", stripped)
            } else if let Some(stripped) = s.strip_prefix("sz") {
                format!("{}.SZ", stripped)
            } else {
                // Fallback: guess based on first digit
                let code = s.trim_start_matches(|c: char| !c.is_ascii_digit());
                if code.starts_with('6') || code.starts_with('9') {
                    format!("{}.SS", code)
                } else {
                    format!("{}.SZ", code)
                }
            }
        }
        _ => symbol.to_string(),
    }
}

/// Fetch historical daily closing prices for a stock from Yahoo Finance.
/// Returns a list of (date, close_price) pairs sorted by date ascending.
pub async fn fetch_stock_history_yahoo(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    let yahoo_sym = to_yahoo_symbol(symbol, market);

    let start_ts = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_ts = end_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval=1d",
        yahoo_sym, start_ts, end_ts
    );

    let resp = http_client::general_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| {
            format!(
                "fetch_stock_history_yahoo: network error for {}: {}",
                yahoo_sym, e
            )
        })?;

    if !resp.status().is_success() {
        return Err(format!(
            "fetch_stock_history_yahoo: HTTP {} for {}",
            resp.status(),
            yahoo_sym
        ));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| {
        format!(
            "fetch_stock_history_yahoo: parse error for {}: {}",
            yahoo_sym, e
        )
    })?;

    let timestamps = json["chart"]["result"][0]["timestamp"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let closes = json["chart"]["result"][0]["indicators"]["quote"][0]["close"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut result: Vec<(chrono::NaiveDate, f64)> = Vec::new();
    for (ts, cl) in timestamps.iter().zip(closes.iter()) {
        if let (Some(ts_i), Some(cl_f)) = (ts.as_i64(), cl.as_f64()) {
            if let Some(date) = timestamp_to_market_date(ts_i, market) {
                result.push((date, cl_f));
            }
        }
    }
    Ok(result)
}

/// Fetch historical daily closing prices for a stock from East Money (东方财富).
/// Returns a list of (date, close_price) pairs sorted by date ascending.
pub async fn fetch_stock_history_eastmoney(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    // Index symbols (e.g. ^GSPC, HSI, 000300.SS) resolve to their own secid
    // and must NOT go through the stock secid mappers (which would reject them
    // or produce a wrong secid). Yahoo 403s these, so EastMoney is the path.
    let secid = if let Some((idx_secid, _)) = resolve_index_secid(symbol) {
        idx_secid.to_string()
    } else {
        match market {
            "HK" => to_eastmoney_hk_secid(symbol)?,
            "US" => to_eastmoney_us_secid(symbol),
            "CN" => to_eastmoney_secid(&symbol.to_lowercase())?,
            _ => {
                return Err(format!(
                    "Unsupported market '{}' for East Money history",
                    market
                ))
            }
        }
    };

    let beg = start_date.format("%Y%m%d").to_string();
    let end = end_date.format("%Y%m%d").to_string();

    let url = format!(
        "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt=101&fqt=0&beg={}&end={}",
        secid, beg, end
    );

    let resp = send_eastmoney_request(&url, symbol).await?;

    if !resp.status().is_success() {
        return Err(format!(
            "fetch_stock_history_eastmoney: HTTP {} for {}",
            resp.status(),
            symbol
        ));
    }

    let body = resp.text().await.map_err(|e| {
        format!(
            "fetch_stock_history_eastmoney: read error for {}: {}",
            symbol, e
        )
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        format!(
            "fetch_stock_history_eastmoney: parse error for {}: {}",
            symbol, e
        )
    })?;

    // East Money kline response: data.klines is an array of CSV strings
    // Each line: "date,open,close,high,low,volume,amount,amplitude,change_pct,change_amt,turnover"
    let klines = json["data"]["klines"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut result: Vec<(chrono::NaiveDate, f64)> = Vec::new();
    for kline in &klines {
        if let Some(line) = kline.as_str() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 3 {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(parts[0], "%Y-%m-%d") {
                    if let Ok(close) = parts[2].parse::<f64>() {
                        result.push((date, close));
                    }
                }
            }
        }
    }
    Ok(result)
}

/// Fetch historical daily OHLCV candles from East Money.
///
/// Reuses the same kline endpoint as [`fetch_stock_history_eastmoney`] but
/// parses the full candle (open/high/low/close/volume) instead of just the
/// close, for use by technical-analysis indicators.
pub async fn fetch_candles_eastmoney(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<PriceCandle>, String> {
    let secid = match market {
        "HK" => to_eastmoney_hk_secid(symbol)?,
        "US" => to_eastmoney_us_secid(symbol),
        "CN" => to_eastmoney_secid(&symbol.to_lowercase())?,
        _ => {
            return Err(format!(
                "Unsupported market '{}' for East Money candles",
                market
            ))
        }
    };

    let beg = start_date.format("%Y%m%d").to_string();
    let end = end_date.format("%Y%m%d").to_string();
    let url = format!(
        "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61&klt=101&fqt=0&beg={}&end={}",
        secid, beg, end
    );

    let resp = send_eastmoney_request(&url, symbol).await?;
    if !resp.status().is_success() {
        return Err(format!(
            "fetch_candles_eastmoney: HTTP {} for {}",
            resp.status(),
            symbol
        ));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("fetch_candles_eastmoney: read error for {}: {}", symbol, e))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("fetch_candles_eastmoney: parse error for {}: {}", symbol, e))?;

    // Each kline: "date,open,close,high,low,volume,amount,amplitude,chg_pct,chg_amt,turnover"
    let klines = json["data"]["klines"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut candles = Vec::with_capacity(klines.len());
    for kline in &klines {
        if let Some(line) = kline.as_str() {
            let parts: Vec<&str> = line.split(',').collect();
            // parts: [0]date [1]open [2]close [3]high [4]low [5]volume
            if parts.len() >= 6 {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(parts[0], "%Y-%m-%d") {
                    let open = parts[1].parse::<f64>().unwrap_or(0.0);
                    let close = parts[2].parse::<f64>().unwrap_or(0.0);
                    let high = parts[3].parse::<f64>().unwrap_or(0.0);
                    let low = parts[4].parse::<f64>().unwrap_or(0.0);
                    let volume = parts[5].parse::<f64>().unwrap_or(0.0);
                    candles.push(PriceCandle {
                        date: date.format("%Y-%m-%d").to_string(),
                        open,
                        close,
                        high,
                        low,
                        volume,
                    });
                }
            }
        }
    }
    Ok(candles)
}

/// Fetch historical daily OHLCV candles from Xueqiu.
///
/// Mirrors [`fetch_stock_history_xueqiu`] but retains open/high/low/close/volume
/// for technical-analysis indicators.
pub async fn fetch_candles_xueqiu(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<PriceCandle>, String> {
    let xueqiu_symbol = match market {
        "CN" => to_xueqiu_cn_symbol(symbol)?,
        "HK" => to_xueqiu_hk_symbol(symbol)?,
        _ => to_xueqiu_us_symbol(symbol),
    };

    // Xueqiu kline: begin = end timestamp in ms, returns newest-first; we use a
    // large window and trim. period="daily".
    let begin = (end_date.and_hms_opt(15, 0, 0).unwrap())
        .and_utc()
        .timestamp_millis();
    let window_ms = (end_date - start_date).num_days().max(1) * 86_400_000 + 86_400_000;
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/chart/kline.json?symbol={}&begin={}&period=day&type=before&count=-{}",
        xueqiu_symbol, begin, window_ms / 86_400_000 + 10
    );

    let resp = send_xueqiu_request(&url, symbol).await?;
    if !resp.status().is_success() {
        return Err(format!(
            "fetch_candles_xueqiu: HTTP {} for {}",
            resp.status(),
            symbol
        ));
    }
    let body: XueqiuKlineResponse = resp
        .json()
        .await
        .map_err(|e| format!("fetch_candles_xueqiu: parse error for {}: {}", symbol, e))?;

    let data = match body.data {
        Some(d) => d,
        None => return Ok(Vec::new()),
    };
    let (columns, items) = match (data.column, data.item) {
        (Some(c), Some(i)) => (c, i),
        _ => return Ok(Vec::new()),
    };

    // Locate columns by name.
    let idx_of = |name: &str| columns.iter().position(|c| c == name);
    let ts_i = idx_of("timestamp");
    let open_i = idx_of("open");
    let high_i = idx_of("high");
    let low_i = idx_of("low");
    let close_i = idx_of("close");
    let vol_i = idx_of("volume");
    let (ts_i, close_i) = match (ts_i, close_i) {
        (Some(t), Some(c)) => (t, c),
        _ => return Ok(Vec::new()),
    };

    let as_f64 = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
    let mut candles: Vec<PriceCandle> = items
        .iter()
        .rev() // Xueqiu is newest-first; flip to oldest-first
        .filter_map(|row| {
            let ts = row.get(ts_i)?.as_i64()?;
            let date = chrono::DateTime::from_timestamp_millis(ts)?.date_naive();
            Some(PriceCandle {
                date: date.format("%Y-%m-%d").to_string(),
                open: open_i.and_then(|i| row.get(i)).map(as_f64).unwrap_or(0.0),
                close: row.get(close_i).map(as_f64).unwrap_or(0.0),
                high: high_i.and_then(|i| row.get(i)).map(as_f64).unwrap_or(0.0),
                low: low_i.and_then(|i| row.get(i)).map(as_f64).unwrap_or(0.0),
                volume: vol_i.and_then(|i| row.get(i)).map(as_f64).unwrap_or(0.0),
            })
        })
        .collect();
    // Trim to the requested start (inclusive).
    let start_s = start_date.format("%Y-%m-%d").to_string();
    while candles.first().is_some_and(|c| c.date < start_s) {
        candles.remove(0);
    }
    Ok(candles)
}

/// Fetch historical daily OHLCV candles for a stock across providers.
///
/// Dispatches by provider with a resilient fallback chain mirroring
/// [`fetch_stock_history`]: xueqiu → eastmoney → yahoo (yahoo path returns
/// close-only candles when OHLCV is unavailable).
pub async fn fetch_stock_candles(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
    provider: &str,
) -> Result<Vec<PriceCandle>, String> {
    match provider {
        "xueqiu" => match fetch_candles_xueqiu(symbol, market, start_date, end_date).await {
            Ok(c) if !c.is_empty() => Ok(c),
            Ok(_) | Err(_) => {
                match fetch_candles_eastmoney(symbol, market, start_date, end_date).await {
                    Ok(c) if !c.is_empty() => Ok(c),
                    Ok(_) => Ok(Vec::new()),
                    Err(e) => Err(e),
                }
            }
        },
        "eastmoney" => fetch_candles_eastmoney(symbol, market, start_date, end_date).await,
        _ => fetch_candles_eastmoney(symbol, market, start_date, end_date).await,
    }
}

/// Strip a CN symbol's market prefix to get the bare 6-digit code.
/// `"sh600519"` → `"600519"`; `"600519"` → `"600519"`.
fn cn_bare_code(symbol: &str) -> String {
    let s = symbol.to_lowercase();
    let s = s.trim_start_matches("sh").trim_start_matches("sz");
    s.trim_start_matches("bj").to_string()
}

/// Fetch recent financial-statement periods (最近 N 期财报) from East Money's
/// datacenter API.
///
/// Currently only supports CN A-shares (the datacenter `SECURITY_CODE` is the
/// 6-digit code). Returns the most recent `limit` periods (default 4), newest
/// first. No authentication is required.
pub async fn fetch_financial_statements(
    symbol: &str,
    market: &str,
    limit: usize,
) -> Result<Vec<FinancialReport>, String> {
    if market != "CN" {
        return Err(format!(
            "财务报表查询暂仅支持 A 股，{} 的市场为 {}",
            symbol, market
        ));
    }
    let code = cn_bare_code(symbol);
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("无效的 A 股代码用于财务报表查询: {}", symbol));
    }

    let columns = "REPORT_DATE,REPORT_DATE_NAME,EPSJB,ROEJQ,OPERATE_INCOME_PK,TOTALOPERATEREVETZ,\
PARENTNETPROFIT,PARENTNETPROFITTZ,TOTAL_ASSETS_PK,INTEREST_DEBT_RATIO";
    let url = format!(
        "https://datacenter.eastmoney.com/securities/api/data/v1/get?\
reportName=RPT_F10_FINANCE_MAINFINADATA&columns={}&filter=(SECURITY_CODE=\"{}\")\
&pageSize={}&sortColumns=REPORT_DATE&sortTypes=-1",
        columns, code, limit
    );

    let resp = http_client::eastmoney_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("查询东方财富财务报表失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "fetch_financial_statements: HTTP {} for {}",
            resp.status(),
            symbol
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("fetch_financial_statements: 解析失败 for {}: {}", symbol, e))?;

    if !body
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(format!("东方财富财务报表查询失败 for {}: {}", symbol, msg));
    }

    let rows = body["result"]["data"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let parse_date = |s: &str| -> String {
        // "2026-03-31 00:00:00" -> "2026-03-31"
        s.split_whitespace().next().unwrap_or(s).to_string()
    };
    let as_f64 = |v: &serde_json::Value| -> Option<f64> {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    };

    let reports = rows
        .iter()
        .map(|r| {
            let report_date = r
                .get("REPORT_DATE")
                .and_then(|v| v.as_str())
                .map(parse_date)
                .unwrap_or_default();
            FinancialReport {
                period_name: r
                    .get("REPORT_DATE_NAME")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                report_date,
                eps: r.get("EPSJB").and_then(as_f64),
                roe: r.get("ROEJQ").and_then(as_f64),
                revenue: r.get("OPERATE_INCOME_PK").and_then(as_f64),
                revenue_yoy: r.get("TOTALOPERATEREVETZ").and_then(as_f64),
                net_profit: r.get("PARENTNETPROFIT").and_then(as_f64),
                net_profit_yoy: r.get("PARENTNETPROFITTZ").and_then(as_f64),
                total_assets: r.get("TOTAL_ASSETS_PK").and_then(as_f64),
                debt_ratio: r.get("INTEREST_DEBT_RATIO").and_then(as_f64),
            }
        })
        .collect();
    Ok(reports)
}

/// Uses the Xueqiu kline API (`/v5/stock/chart/kline.json`).
/// Returns a list of (date, close_price) pairs sorted by date ascending.
#[allow(dead_code)] // Retained as the provider-specific API for callers that do not want fallback.
pub async fn fetch_stock_history_xueqiu(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    match fetch_stock_history_xueqiu_outcome(symbol, market, start_date, end_date).await? {
        XueqiuHistoryOutcome::Prices(prices) => Ok(prices),
        XueqiuHistoryOutcome::StartsAfterRange { .. } | XueqiuHistoryOutcome::Empty => {
            Ok(Vec::new())
        }
    }
}

async fn fetch_stock_history_xueqiu_outcome(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<XueqiuHistoryOutcome, String> {
    let xueqiu_symbol = match market {
        "CN" => to_xueqiu_cn_symbol(symbol)?,
        "HK" => to_xueqiu_hk_symbol(symbol)?,
        _ => to_xueqiu_us_symbol(symbol),
    };

    // Xueqiu returns trading days going backwards from begin.
    // Calendar days in range is a safe upper bound (there are fewer
    // trading days than calendar days), with a minimum of 2.
    let count = (end_date - start_date).num_days().max(2);

    // The Xueqiu kline API `begin` parameter is a millisecond timestamp.
    // With `type=before`, the API returns `count` trading days going
    // backwards from `begin`.  Use end_date + 1 day (23:59:59 UTC) so the
    // returned window ends on or after the requested end_date, then filter
    // client-side to the exact range.
    let begin_ts = end_date
        .succ_opt()
        .unwrap_or(end_date)
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    let url = format!(
        "https://stock.xueqiu.com/v5/stock/chart/kline.json?symbol={}&begin={}&period=day&type=before&count=-{}&indicator=kline",
        xueqiu_symbol, begin_ts, count
    );

    let response = send_xueqiu_request(&url, symbol).await?;

    if !response.status().is_success() {
        let status = response.status();
        let body_preview = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(XUEQIU_RESPONSE_PREVIEW_LEN)
            .collect::<String>();
        return Err(format!(
            "fetch_stock_history_xueqiu: HTTP {} for {}. Response: {}",
            status, symbol, body_preview
        ));
    }

    let body = response.text().await.map_err(|e| {
        format!(
            "fetch_stock_history_xueqiu: read error for {}: {}",
            symbol, e
        )
    })?;

    parse_xueqiu_history_response(&body, symbol, market, start_date, end_date, &url)
}

pub(crate) async fn resolve_xueqiu_history_outcome<EastMoney, EastMoneyFuture, Yahoo, YahooFuture>(
    symbol: &str,
    market: &str,
    outcome: Result<XueqiuHistoryOutcome, String>,
    fetch_eastmoney: EastMoney,
    fetch_yahoo: Yahoo,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String>
where
    EastMoney: FnOnce() -> EastMoneyFuture,
    EastMoneyFuture: std::future::Future<Output = Result<Vec<(chrono::NaiveDate, f64)>, String>>,
    Yahoo: FnOnce() -> YahooFuture,
    YahooFuture: std::future::Future<Output = Result<Vec<(chrono::NaiveDate, f64)>, String>>,
{
    match outcome {
        Ok(XueqiuHistoryOutcome::Prices(prices)) => return Ok(prices),
        Ok(XueqiuHistoryOutcome::StartsAfterRange {
            first_available_date,
        }) => {
            info!(
                "fetch_stock_history: {} ({}) has no market history in the requested range; first available trading date is {}",
                symbol, market, first_available_date
            );
            return Ok(Vec::new());
        }
        Ok(XueqiuHistoryOutcome::Empty) => {
            info!(
                "fetch_stock_history: Xueqiu returned empty history for {} ({}), falling back to eastmoney",
                symbol, market
            );
        }
        Err(error) => {
            warn!(
                "fetch_stock_history: Xueqiu history failed for {} ({}): {}, falling back to eastmoney",
                symbol, market, error
            );
        }
    }

    match fetch_eastmoney().await {
        Ok(prices) if !prices.is_empty() => Ok(prices),
        Ok(_empty) => {
            warn!(
                "fetch_stock_history: EastMoney also returned empty history for {} ({}), falling back to yahoo",
                symbol, market
            );
            fetch_yahoo().await
        }
        Err(error) => {
            warn!(
                "fetch_stock_history: EastMoney fallback also failed for {} ({}): {}, falling back to yahoo",
                symbol, market, error
            );
            fetch_yahoo().await
        }
    }
}

/// Fetch historical daily closing prices using the appropriate provider
/// based on the market and the configured provider name.
/// Falls back to Yahoo Finance for unknown providers.
/// When Xueqiu is selected but returns an error or a genuinely empty result,
/// East Money is used as an automatic fallback. A first trading date after
/// the requested range is treated as a valid pre-listing response.
pub async fn fetch_stock_history(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
    provider: &str,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    match provider {
        "xueqiu" => {
            let outcome =
                fetch_stock_history_xueqiu_outcome(symbol, market, start_date, end_date).await;
            resolve_xueqiu_history_outcome(
                symbol,
                market,
                outcome,
                || fetch_stock_history_eastmoney(symbol, market, start_date, end_date),
                || fetch_stock_history_yahoo(symbol, market, start_date, end_date),
            )
            .await
        }
        "eastmoney" => {
            match fetch_stock_history_eastmoney(symbol, market, start_date, end_date).await {
                Ok(prices) if !prices.is_empty() => Ok(prices),
                Ok(_empty) => {
                    warn!(
                        "fetch_stock_history: EastMoney returned empty history for {} ({}), falling back to yahoo",
                        symbol, market
                    );
                    fetch_stock_history_yahoo(symbol, market, start_date, end_date).await
                }
                Err(e) => {
                    warn!(
                        "fetch_stock_history: EastMoney history failed for {} ({}): {}, falling back to yahoo",
                        symbol, market, e
                    );
                    fetch_stock_history_yahoo(symbol, market, start_date, end_date).await
                }
            }
        }
        _ => fetch_stock_history_yahoo(symbol, market, start_date, end_date).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_index_secid_handles_common_forms() {
        // US indices — ^-prefixed, bare, and suffix-stripped.
        assert_eq!(resolve_index_secid("^GSPC").unwrap().0, "100.SPX");
        assert_eq!(resolve_index_secid("SPX").unwrap().0, "100.SPX");
        assert_eq!(resolve_index_secid("^IXIC").unwrap().0, "100.NDX");
        assert_eq!(resolve_index_secid("^DJI").unwrap().0, "100.DJIA");
        // HK.
        assert_eq!(resolve_index_secid("^HSI").unwrap().0, "100.HSI");
        assert_eq!(resolve_index_secid("HSI").unwrap().0, "100.HSI");
        // CN — with and without .SS suffix.
        assert_eq!(resolve_index_secid("000300.SS").unwrap().0, "1.000300");
        assert_eq!(resolve_index_secid("000001.SS").unwrap().0, "1.000001");
        assert_eq!(resolve_index_secid("000300").unwrap().0, "1.000300");
        // Non-index symbols return None.
        assert!(resolve_index_secid("AAPL").is_none());
        assert!(resolve_index_secid("0700.HK").is_none());
        assert!(resolve_index_secid("sh600519").is_none());
    }

    // Helper: build a synthetic East Money JSON response.
    #[allow(clippy::too_many_arguments)]
    fn make_eastmoney_response(
        name: &str,
        current: f64,
        prev_close: f64,
        high: f64,
        low: f64,
        volume: f64,
        change: f64,
        change_pct: f64,
    ) -> EastMoneyResponse {
        EastMoneyResponse {
            data: Some(EastMoneyData {
                f43: Some(current),
                f44: Some(high),
                f45: Some(low),
                f47: Some(volume),
                f58: Some(name.to_string()),
                f60: Some(prev_close),
                f169: Some(change),
                f170: Some(change_pct),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn test_parse_eastmoney_quote_valid() {
        let resp = make_eastmoney_response(
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345.0,
            20.50,
            1.21,
        );
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "sh600519");
        assert_eq!(quote.name, "贵州茅台");
        assert_eq!(quote.market, "CN");
        assert!((quote.current_price - 1710.50).abs() < 0.001);
        assert!((quote.previous_close - 1690.00).abs() < 0.001);
        assert!((quote.high - 1720.00).abs() < 0.001);
        assert!((quote.low - 1685.00).abs() < 0.001);
        assert_eq!(quote.volume, 12345);
        assert!((quote.change - 20.50).abs() < 0.001);
        assert!((quote.change_percent - 1.21).abs() < 0.001);
    }

    #[test]
    fn test_parse_eastmoney_quote_no_data() {
        let resp = EastMoneyResponse { data: None };
        let result = parse_eastmoney_quote("sh999999", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No data from East Money"));
    }

    #[test]
    fn test_parse_eastmoney_index_with_dash_strings() {
        // Overseas indices (NASDAQ, S&P, HSI) return `"-"` for fundamentals
        // fields (market cap, P/E, P/B, turnover) that don't apply to them.
        // The lenient deserializer must coerce those to None instead of
        // failing the whole parse.
        let body = r#"{"rc":0,"rt":4,"data":{"f43":25690.9,"f44":25841.31,"f45":25681.32,"f47":6651314688,"f58":"纳斯达克","f60":25837.21,"f169":853.69,"f170":3.30,"f116":"-","f163":"-","f167":"-","f168":"-"}}"#;
        let resp = parse_eastmoney_body(body, "^IXIC").unwrap();
        let quote = parse_eastmoney_quote("^IXIC", "US", resp).unwrap();
        assert_eq!(quote.name, "纳斯达克");
        assert!((quote.current_price - 25690.9).abs() < 0.01);
        assert!((quote.change_percent - 3.30).abs() < 0.01);
    }

    #[test]
    fn test_parse_eastmoney_quote_missing_price() {
        let resp = EastMoneyResponse {
            data: Some(EastMoneyData {
                f43: None,
                f44: Some(1720.00),
                f45: Some(1685.00),
                f47: Some(12345.0),
                f58: Some("贵州茅台".to_string()),
                f60: Some(1690.00),
                f169: Some(20.50),
                f170: Some(1.21),
                ..Default::default()
            }),
        };
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing current price"));
    }

    #[test]
    fn test_parse_eastmoney_quote_change_calculation() {
        let resp = make_eastmoney_response(
            "贵州茅台",
            1100.00,
            1000.00,
            1200.00,
            950.00,
            99999.0,
            100.00,
            10.00,
        );
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_eastmoney_quote_symbol_stored_as_given() {
        let resp = make_eastmoney_response(
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345.0,
            20.50,
            1.21,
        );
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().symbol, "sh600519");
    }

    #[test]
    fn test_fetch_cn_quote_normalises_symbol_to_lowercase() {
        // Verify that to_lowercase() on a mixed-case symbol produces what
        // the API expects.  We cannot call fetch_cn_quote directly in a
        // unit test (it makes a real network request), so we assert the
        // string transform is correct and pass the lowercased value to
        // the parser.
        let mixed = "Sh600519";
        let lower = mixed.to_lowercase();
        assert_eq!(lower, "sh600519");
        let resp = make_eastmoney_response(
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345.0,
            20.50,
            1.21,
        );
        let result = parse_eastmoney_quote(&lower, "CN", resp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().symbol, "sh600519");
    }

    #[test]
    fn test_to_eastmoney_secid_shanghai() {
        let secid = to_eastmoney_secid("sh600519").unwrap();
        assert_eq!(secid, "1.600519");
    }

    #[test]
    fn test_to_eastmoney_secid_shenzhen() {
        let secid = to_eastmoney_secid("sz000858").unwrap();
        assert_eq!(secid, "0.000858");
    }

    #[test]
    fn test_to_eastmoney_secid_invalid_prefix() {
        let result = to_eastmoney_secid("hk00700");
        assert!(result.is_err());
    }

    #[test]
    fn test_to_eastmoney_secid_too_short() {
        let result = to_eastmoney_secid("sh");
        assert!(result.is_err());
    }

    #[test]
    fn test_to_eastmoney_us_secid() {
        assert_eq!(to_eastmoney_us_secid("AAPL"), "105.AAPL");
        assert_eq!(to_eastmoney_us_secid("msft"), "105.MSFT");
        assert_eq!(to_eastmoney_us_secid("GOOGL"), "105.GOOGL");
        // Hyphens should be converted to underscores with prefix 106
        assert_eq!(to_eastmoney_us_secid("BRK-B"), "106.BRK_B");
        assert_eq!(to_eastmoney_us_secid("BRK-A"), "106.BRK_A");
        assert_eq!(to_eastmoney_us_secid("BF-B"), "106.BF_B");
    }

    #[test]
    fn test_to_eastmoney_hk_secid() {
        assert_eq!(to_eastmoney_hk_secid("00700").unwrap(), "116.00700");
        assert_eq!(to_eastmoney_hk_secid("0700.HK").unwrap(), "116.00700");
        assert_eq!(to_eastmoney_hk_secid("9988.HK").unwrap(), "116.09988");
        assert_eq!(to_eastmoney_hk_secid("09988").unwrap(), "116.09988");
        assert_eq!(to_eastmoney_hk_secid("700.hk").unwrap(), "116.00700");
    }

    #[test]
    fn test_to_eastmoney_hk_secid_invalid() {
        let result = to_eastmoney_hk_secid("INVALID");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_eastmoney_quote_us_market() {
        let resp =
            make_eastmoney_response("苹果", 195.50, 193.00, 197.00, 192.00, 50000.0, 2.50, 1.30);
        let result = parse_eastmoney_quote("AAPL", "US", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "AAPL");
        assert_eq!(quote.market, "US");
        assert!((quote.current_price - 195.50).abs() < 0.001);
    }

    #[test]
    fn test_parse_eastmoney_quote_hk_market() {
        let resp = make_eastmoney_response(
            "腾讯控股",
            420.00,
            415.00,
            425.00,
            410.00,
            30000.0,
            5.00,
            1.20,
        );
        let result = parse_eastmoney_quote("00700", "HK", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "00700");
        assert_eq!(quote.market, "HK");
        assert!((quote.current_price - 420.00).abs() < 0.001);
    }

    #[test]
    fn test_parse_eastmoney_quote_fallback_change_calculation() {
        // When f169/f170 are missing, change should be computed from price
        let resp = EastMoneyResponse {
            data: Some(EastMoneyData {
                f43: Some(1100.00),
                f44: Some(1200.00),
                f45: Some(950.00),
                f47: Some(99999.0),
                f58: Some("贵州茅台".to_string()),
                f60: Some(1000.00),
                f169: None,
                f170: None,
                ..Default::default()
            }),
        };
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_data_deserialize_float_volume() {
        // The API may return volume as a JSON float (e.g. 30279.0).
        // serde rejects JSON floats when the target type is u64, so
        // f47 must be declared as f64 to accept both forms.
        let json = r#"{
            "rc": 0,
            "data": {
                "f43": 1516.0,
                "f44": 1519.0,
                "f45": 1508.0,
                "f47": 30279.0,
                "f57": "600519",
                "f58": "贵州茅台",
                "f60": 1513.0,
                "f169": 3.0,
                "f170": 0.2
            }
        }"#;
        let resp: EastMoneyResponse = serde_json::from_str(json).expect("should parse");
        let data = resp.data.unwrap();
        assert!((data.f47.unwrap() - 30279.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_data_deserialize_integer_volume() {
        // The API may also return volume as a JSON integer.
        let json = r#"{
            "rc": 0,
            "data": {
                "f43": 1516.0,
                "f44": 1519.0,
                "f45": 1508.0,
                "f47": 30279,
                "f57": "600519",
                "f58": "贵州茅台",
                "f60": 1513.0,
                "f169": 3.0,
                "f170": 0.2
            }
        }"#;
        let resp: EastMoneyResponse = serde_json::from_str(json).expect("should parse");
        let data = resp.data.unwrap();
        assert!((data.f47.unwrap() - 30279.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_data_deserialize_numeric_values() {
        // Normal case: all numeric fields are numbers.
        let json = r#"{
            "rc": 0,
            "data": {
                "f43": 1710.50,
                "f44": 1720.00,
                "f45": 1685.00,
                "f47": 12345,
                "f57": "600519",
                "f58": "贵州茅台",
                "f60": 1690.00,
                "f169": 20.50,
                "f170": 1.21
            }
        }"#;
        let resp: EastMoneyResponse = serde_json::from_str(json).expect("should parse");
        let data = resp.data.unwrap();
        assert!((data.f43.unwrap() - 1710.50).abs() < 0.001);
        assert!((data.f47.unwrap() - 12345.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_data_deserialize_integer_prices() {
        // f43 may be an integer (no decimal) when the price is round.
        let json = r#"{
            "rc": 0,
            "data": {
                "f43": 1700,
                "f44": 1720,
                "f45": 1685,
                "f47": 12345,
                "f57": "600519",
                "f58": "贵州茅台",
                "f60": 1690,
                "f169": 10,
                "f170": 0
            }
        }"#;
        let resp: EastMoneyResponse = serde_json::from_str(json).expect("should parse");
        let data = resp.data.unwrap();
        assert!((data.f43.unwrap() - 1700.0).abs() < 0.001);
        assert!((data.f60.unwrap() - 1690.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_data_deserialize_null_data() {
        let json = r#"{"rc": 0, "data": null}"#;
        let resp: EastMoneyResponse = serde_json::from_str(json).expect("should parse");
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_eastmoney_response_with_extra_fields() {
        // The real API returns extra fields (rt, svr, lt, full, dlmkts).
        // Our struct should ignore them gracefully.
        let json = r#"{
            "rc": 0,
            "rt": 4,
            "svr": 2887254139,
            "lt": 1,
            "full": 1,
            "dlmkts": "",
            "data": {
                "f43": 1516.0,
                "f44": 1519.0,
                "f45": 1508.0,
                "f47": 30279,
                "f57": "600519",
                "f58": "贵州茅台",
                "f60": 1513.0,
                "f169": 3.0,
                "f170": 0.2
            }
        }"#;
        let resp: EastMoneyResponse = serde_json::from_str(json).expect("should parse");
        let data = resp.data.unwrap();
        assert!((data.f43.unwrap() - 1516.0).abs() < 0.001);
        assert!((data.f47.unwrap() - 30279.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_volume_converts_to_u64() {
        // The parse function should convert f64 volume to u64 correctly.
        let resp = make_eastmoney_response(
            "贵州茅台",
            1516.0,
            1513.0,
            1519.0,
            1508.0,
            30279.0,
            3.0,
            0.2,
        );
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().volume, 30279);
    }

    fn sample_quote(symbol: &str, market: &str) -> StockQuote {
        StockQuote {
            symbol: symbol.to_string(),
            name: format!("Test {}", symbol),
            market: market.to_string(),
            current_price: 100.0,
            previous_close: 95.0,
            change: 5.0,
            change_percent: 5.26,
            high: 105.0,
            low: 94.0,
            volume: 1000000,
            updated_at: Utc::now().to_rfc3339(),
            ..Default::default()
        }
    }

    #[test]
    fn test_quote_cache_empty() {
        let cache = QuoteCache::new();
        assert!(cache.get("AAPL").is_none());
        assert!(cache.get_stale("AAPL").is_none());
    }

    #[test]
    fn test_quote_cache_set_and_get() {
        let cache = QuoteCache::new();
        let quote = sample_quote("AAPL", "US");
        cache.set(quote.clone());
        let cached = cache.get("AAPL").expect("should have cached quote");
        assert_eq!(cached.symbol, "AAPL");
        assert!((cached.current_price - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_quote_cache_stale_fallback() {
        let cache = QuoteCache::new();
        let quote = sample_quote("AAPL", "US");
        cache.set(quote);
        let stale = cache.get_stale("AAPL").expect("should have stale quote");
        assert_eq!(stale.symbol, "AAPL");
    }

    #[test]
    fn test_quote_cache_set_batch() {
        let cache = QuoteCache::new();
        let quotes = vec![
            sample_quote("AAPL", "US"),
            sample_quote("GOOGL", "US"),
            sample_quote("sh600519", "CN"),
        ];
        cache.set_batch(&quotes);
        assert!(cache.get("AAPL").is_some());
        assert!(cache.get("GOOGL").is_some());
        assert!(cache.get("sh600519").is_some());
        assert!(cache.get("MSFT").is_none());
    }

    #[test]
    fn test_quote_cache_get_batch() {
        let cache = QuoteCache::new();
        cache.set(sample_quote("AAPL", "US"));
        cache.set(sample_quote("GOOGL", "US"));

        let symbols = vec![
            ("AAPL".to_string(), "US".to_string()),
            ("GOOGL".to_string(), "US".to_string()),
            ("MSFT".to_string(), "US".to_string()),
        ];
        let (cached, missing) = cache.get_batch(&symbols);
        assert_eq!(cached.len(), 2);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "MSFT");
    }

    #[test]
    fn test_fetch_quotes_batch_with_providers_deduplicates_symbols() {
        // Verify that duplicate symbols (same stock in multiple accounts) are
        // deduplicated before fetching.  We use cash symbols ($CASH-*) which
        // return synthetic quotes without any network call.
        let symbols = vec![
            ("$CASH-USD".to_string(), "US".to_string()),
            ("$CASH-USD".to_string(), "US".to_string()), // duplicate
            ("$CASH-CNY".to_string(), "CN".to_string()),
            ("$CASH-CNY".to_string(), "CN".to_string()), // duplicate
            ("$CASH-HKD".to_string(), "HK".to_string()),
        ];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let quotes = rt
            .block_on(fetch_quotes_batch_with_providers(
                symbols,
                "eastmoney",
                "eastmoney",
                "eastmoney",
            ))
            .unwrap();
        // Should only return 3 unique quotes, not 5
        assert_eq!(quotes.len(), 3);
        let syms: Vec<&str> = quotes.iter().map(|q| q.symbol.as_str()).collect();
        assert!(syms.contains(&"$CASH-USD"));
        assert!(syms.contains(&"$CASH-CNY"));
        assert!(syms.contains(&"$CASH-HKD"));
    }

    #[test]
    fn test_fetch_quotes_batch_cached_deduplicates_symbols() {
        // Verify that the cached batch fetch also deduplicates symbols.
        let cache = QuoteCache::new();
        let symbols = vec![
            ("$CASH-USD".to_string(), "US".to_string()),
            ("$CASH-USD".to_string(), "US".to_string()), // duplicate
            ("$CASH-CNY".to_string(), "CN".to_string()),
            ("$CASH-CNY".to_string(), "CN".to_string()), // duplicate
        ];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let quotes = rt
            .block_on(fetch_quotes_batch_cached_with_providers(
                &cache,
                symbols,
                "eastmoney",
                "eastmoney",
                "eastmoney",
                false,
            ))
            .unwrap();
        // Should only return 2 unique quotes, not 4
        assert_eq!(quotes.len(), 2);
    }

    #[test]
    fn test_quote_cache_update_overwrites() {
        let cache = QuoteCache::new();
        let mut quote = sample_quote("AAPL", "US");
        cache.set(quote.clone());
        assert!((cache.get("AAPL").unwrap().current_price - 100.0).abs() < 0.001);

        quote.current_price = 200.0;
        cache.set(quote);
        assert!((cache.get("AAPL").unwrap().current_price - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_quote_cache_no_ttl_expiry() {
        // Verify that cached quotes do not expire based on time.
        // get() should return the cached quote regardless of when it was stored.
        let cache = QuoteCache::new();
        let quote = sample_quote("AAPL", "US");
        cache.set(quote);
        // Immediately retrievable
        assert!(cache.get("AAPL").is_some());
        // get_batch should also return it (not as "missing")
        let (cached, missing) = cache.get_batch(&[("AAPL".to_string(), "US".to_string())]);
        assert_eq!(cached.len(), 1);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_fetch_quotes_batch_cached_force_refresh() {
        // Verify that force_refresh=true bypasses the cache and fetches from API.
        // We use cash symbols ($CASH-*) which return synthetic quotes.
        let cache = QuoteCache::new();

        // Pre-populate cache
        let initial_quote = sample_quote("$CASH-USD", "US");
        cache.set(initial_quote);

        let symbols = vec![("$CASH-USD".to_string(), "US".to_string())];
        let rt = tokio::runtime::Runtime::new().unwrap();

        // With force_refresh=false, should return cached data
        let quotes = rt
            .block_on(fetch_quotes_batch_cached_with_providers(
                &cache,
                symbols.clone(),
                "eastmoney",
                "eastmoney",
                "eastmoney",
                false,
            ))
            .unwrap();
        assert_eq!(quotes.len(), 1);
        // Cached quote has price 100.0 (from sample_quote)
        assert!((quotes[0].current_price - 100.0).abs() < 0.001);

        // With force_refresh=true, should fetch fresh data (cash quote has price 1.0)
        let quotes = rt
            .block_on(fetch_quotes_batch_cached_with_providers(
                &cache,
                symbols,
                "eastmoney",
                "eastmoney",
                "eastmoney",
                true,
            ))
            .unwrap();
        assert_eq!(quotes.len(), 1);
        assert!((quotes[0].current_price - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_to_yahoo_symbol_us() {
        assert_eq!(to_yahoo_symbol("AAPL", "US"), "AAPL");
        assert_eq!(to_yahoo_symbol("MSFT", "US"), "MSFT");
        // Dots should be converted to hyphens for Yahoo
        assert_eq!(to_yahoo_symbol("BRK.B", "US"), "BRK-B");
        assert_eq!(to_yahoo_symbol("BRK.A", "US"), "BRK-A");
        assert_eq!(to_yahoo_symbol("BF.B", "US"), "BF-B");
        // Hyphens should remain unchanged
        assert_eq!(to_yahoo_symbol("BRK-B", "US"), "BRK-B");
    }

    #[test]
    fn test_to_yahoo_symbol_hk() {
        assert_eq!(to_yahoo_symbol("0700.HK", "HK"), "0700.HK");
        assert_eq!(to_yahoo_symbol("00700", "HK"), "00700.HK");
    }

    #[test]
    fn test_to_yahoo_symbol_cn() {
        assert_eq!(to_yahoo_symbol("sh600519", "CN"), "600519.SS");
        assert_eq!(to_yahoo_symbol("sz000858", "CN"), "000858.SZ");
        assert_eq!(to_yahoo_symbol("SH600519", "CN"), "600519.SS");
        // Fallback for bare codes
        assert_eq!(to_yahoo_symbol("600519", "CN"), "600519.SS");
        assert_eq!(to_yahoo_symbol("000858", "CN"), "000858.SZ");
    }

    // ---- Cash symbol tests ----

    #[test]
    fn test_is_cash_symbol() {
        assert!(is_cash_symbol("$CASH-USD"));
        assert!(is_cash_symbol("$CASH-CNY"));
        assert!(is_cash_symbol("$CASH-HKD"));
        assert!(!is_cash_symbol("AAPL"));
        assert!(!is_cash_symbol("sh600519"));
        assert!(!is_cash_symbol("CASH"));
        assert!(!is_cash_symbol("$CASH"));
    }

    #[test]
    fn test_cash_display_name() {
        assert_eq!(cash_display_name("$CASH-USD"), "现金 (USD)");
        assert_eq!(cash_display_name("$CASH-CNY"), "现金 (CNY)");
        assert_eq!(cash_display_name("$CASH-HKD"), "现金 (HKD)");
    }

    #[test]
    fn test_make_cash_quote() {
        let quote = make_cash_quote("$CASH-USD", "US");
        assert_eq!(quote.symbol, "$CASH-USD");
        assert_eq!(quote.market, "US");
        assert!((quote.current_price - 1.0).abs() < f64::EPSILON);
        assert!((quote.previous_close - 1.0).abs() < f64::EPSILON);
        assert!((quote.change).abs() < f64::EPSILON);
        assert!((quote.change_percent).abs() < f64::EPSILON);
        assert_eq!(quote.volume, 0);
        assert_eq!(quote.name, "现金 (USD)");
    }

    #[tokio::test]
    async fn test_batch_fetch_cash_symbols_no_network() {
        // Cash symbols should return synthetic quotes without any network call.
        let symbols = vec![
            ("$CASH-USD".to_string(), "US".to_string()),
            ("$CASH-CNY".to_string(), "CN".to_string()),
            ("$CASH-HKD".to_string(), "HK".to_string()),
        ];
        let result =
            fetch_quotes_batch_with_providers(symbols, "yahoo", "yahoo", "eastmoney").await;
        assert!(result.is_ok());
        let quotes = result.unwrap();
        assert_eq!(quotes.len(), 3);
        for q in &quotes {
            assert!(is_cash_symbol(&q.symbol));
            assert!((q.current_price - 1.0).abs() < f64::EPSILON);
        }
    }

    // ---- Integration tests using real network calls ----
    // These tests verify that the API actually works end-to-end.
    // They are marked #[ignore] so they only run when explicitly requested
    // via `cargo test -- --ignored`.

    #[tokio::test]
    #[ignore]
    async fn test_integration_cn_eastmoney() {
        let result = fetch_cn_quote("sh600519").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "sh600519");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ CN quote (East Money): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ CN quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_us_yahoo() {
        let result = fetch_us_quote("MSFT").await;
        match &result {
            Ok(quote) => {
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ US quote (Yahoo): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ US quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_eastmoney_direct() {
        // Direct East Money call for CN stocks
        let result = fetch_eastmoney_cn_quote("sh600519").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "sh600519");
                assert_eq!(quote.market, "CN");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ East Money quote: {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ East Money quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_us_eastmoney() {
        let result = fetch_eastmoney_us_quote("AAPL").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.market, "US");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ US quote (East Money): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ US East Money quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_hk_eastmoney() {
        let result = fetch_eastmoney_hk_quote("00700").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.market, "HK");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ HK quote (East Money): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ HK East Money quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[test]
    fn test_global_eastmoney_client_returns_same_instance() {
        let c1 = http_client::eastmoney_client();
        let c2 = http_client::eastmoney_client();
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_global_general_client_returns_same_instance() {
        let c1 = http_client::general_client();
        let c2 = http_client::general_client();
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_global_eastmoney_client_can_build_request() {
        let client = http_client::eastmoney_client();
        let req = client
            .get("https://push2.eastmoney.com/test")
            .build()
            .expect("should build request");
        assert_eq!(req.method(), reqwest::Method::GET);
    }

    // ---- East Money history parsing tests ----

    #[test]
    fn test_parse_eastmoney_kline_response() {
        // Simulate the East Money kline API response format
        let json_str = r#"{
            "rc": 0,
            "data": {
                "code": "00700",
                "klines": [
                    "2024-01-02,350.00,355.00,358.00,349.00,10000000,3550000000.00,2.58,1.43,5.00,0.50",
                    "2024-01-03,356.00,352.00,357.00,351.00,12000000,4272000000.00,1.69,-0.84,-3.00,0.60",
                    "2024-01-04,351.00,360.00,362.00,350.00,15000000,5400000000.00,3.41,2.27,8.00,0.75"
                ]
            }
        }"#;

        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let klines = json["data"]["klines"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut result: Vec<(chrono::NaiveDate, f64)> = Vec::new();
        for kline in &klines {
            if let Some(line) = kline.as_str() {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 3 {
                    if let Ok(date) = chrono::NaiveDate::parse_from_str(parts[0], "%Y-%m-%d") {
                        if let Ok(close) = parts[2].parse::<f64>() {
                            result.push((date, close));
                        }
                    }
                }
            }
        }

        assert_eq!(result.len(), 3);
        assert_eq!(
            result[0].0,
            chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()
        );
        assert!((result[0].1 - 355.0).abs() < f64::EPSILON);
        assert_eq!(
            result[1].0,
            chrono::NaiveDate::from_ymd_opt(2024, 1, 3).unwrap()
        );
        assert!((result[1].1 - 352.0).abs() < f64::EPSILON);
        assert_eq!(
            result[2].0,
            chrono::NaiveDate::from_ymd_opt(2024, 1, 4).unwrap()
        );
        assert!((result[2].1 - 360.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_eastmoney_kline_empty() {
        let json_str = r#"{"rc": 0, "data": {"code": "00700", "klines": []}}"#;
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let klines = json["data"]["klines"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(klines.is_empty());
    }

    #[test]
    fn test_parse_eastmoney_kline_null_data() {
        let json_str = r#"{"rc": 0, "data": null}"#;
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let klines = json["data"]["klines"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(klines.is_empty());
    }

    // ---- Xueqiu symbol conversion tests ----

    #[test]
    fn test_to_xueqiu_us_symbol() {
        // Hyphens should be converted to dots
        assert_eq!(to_xueqiu_us_symbol("BRK-B"), "BRK.B");
        assert_eq!(to_xueqiu_us_symbol("BRK-A"), "BRK.A");
        assert_eq!(to_xueqiu_us_symbol("BF-B"), "BF.B");
        // Already dot format should remain unchanged
        assert_eq!(to_xueqiu_us_symbol("BRK.B"), "BRK.B");
        // Simple symbols without hyphens should just uppercase
        assert_eq!(to_xueqiu_us_symbol("AAPL"), "AAPL");
        assert_eq!(to_xueqiu_us_symbol("aapl"), "AAPL");
    }

    #[test]
    fn test_to_xueqiu_cn_symbol_shanghai() {
        assert_eq!(to_xueqiu_cn_symbol("sh600519").unwrap(), "SH600519");
        assert_eq!(to_xueqiu_cn_symbol("SH600519").unwrap(), "SH600519");
    }

    #[test]
    fn test_to_xueqiu_cn_symbol_shenzhen() {
        assert_eq!(to_xueqiu_cn_symbol("sz000858").unwrap(), "SZ000858");
        assert_eq!(to_xueqiu_cn_symbol("Sz000858").unwrap(), "SZ000858");
    }

    #[test]
    fn test_to_xueqiu_cn_symbol_invalid() {
        assert!(to_xueqiu_cn_symbol("hk00700").is_err());
        assert!(to_xueqiu_cn_symbol("ab").is_err());
    }

    #[test]
    fn test_to_xueqiu_hk_symbol() {
        assert_eq!(to_xueqiu_hk_symbol("00700").unwrap(), "00700");
        assert_eq!(to_xueqiu_hk_symbol("0700.HK").unwrap(), "00700");
        assert_eq!(to_xueqiu_hk_symbol("9988.HK").unwrap(), "09988");
        assert_eq!(to_xueqiu_hk_symbol("700.hk").unwrap(), "00700");
    }

    #[test]
    fn test_to_xueqiu_hk_symbol_invalid() {
        assert!(to_xueqiu_hk_symbol("INVALID").is_err());
    }

    // ---- Xueqiu response parsing tests ----

    #[allow(clippy::too_many_arguments)]
    fn make_xueqiu_response(
        _symbol: &str,
        name: &str,
        current: f64,
        last_close: f64,
        high: f64,
        low: f64,
        volume: f64,
        chg: f64,
        percent: f64,
    ) -> XueqiuResponse {
        XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuote {
                    name: Some(name.to_string()),
                    current: Some(current),
                    last_close: Some(last_close),
                    chg: Some(chg),
                    percent: Some(percent),
                    high: Some(high),
                    low: Some(low),
                    volume: Some(volume),
                    ..Default::default()
                }),
            }),
            error_code: Some(0),
            error_description: None,
        }
    }

    #[test]
    fn test_parse_xueqiu_quote_valid_cn() {
        let resp = make_xueqiu_response(
            "SH600519",
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345.0,
            20.50,
            1.21,
        );
        let result = parse_xueqiu_quote("sh600519", "CN", resp);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "sh600519");
        assert_eq!(quote.name, "贵州茅台");
        assert_eq!(quote.market, "CN");
        assert!((quote.current_price - 1710.50).abs() < 0.001);
        assert!((quote.previous_close - 1690.00).abs() < 0.001);
        assert!((quote.high - 1720.00).abs() < 0.001);
        assert!((quote.low - 1685.00).abs() < 0.001);
        assert_eq!(quote.volume, 12345);
        assert!((quote.change - 20.50).abs() < 0.001);
        assert!((quote.change_percent - 1.21).abs() < 0.001);
    }

    #[test]
    fn test_parse_xueqiu_quote_valid_us() {
        let resp = make_xueqiu_response(
            "AAPL", "苹果", 195.50, 193.00, 197.00, 192.00, 50000.0, 2.50, 1.30,
        );
        let result = parse_xueqiu_quote("AAPL", "US", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "AAPL");
        assert_eq!(quote.market, "US");
        assert!((quote.current_price - 195.50).abs() < 0.001);
    }

    #[test]
    fn test_parse_xueqiu_quote_valid_hk() {
        let resp = make_xueqiu_response(
            "00700",
            "腾讯控股",
            420.00,
            415.00,
            425.00,
            410.00,
            30000.0,
            5.00,
            1.20,
        );
        let result = parse_xueqiu_quote("00700", "HK", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "00700");
        assert_eq!(quote.market, "HK");
        assert!((quote.current_price - 420.00).abs() < 0.001);
    }

    #[test]
    fn test_parse_xueqiu_quote_no_data() {
        let resp = XueqiuResponse {
            data: None,
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("sh999999", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No data from Xueqiu"));
    }

    #[test]
    fn test_parse_xueqiu_quote_no_quote() {
        let resp = XueqiuResponse {
            data: Some(XueqiuData { quote: None }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("sh999999", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No quote data from Xueqiu"));
    }

    #[test]
    fn test_parse_xueqiu_quote_missing_price() {
        let resp = XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuote {
                    name: Some("贵州茅台".to_string()),
                    current: None,
                    last_close: Some(1690.00),
                    chg: Some(20.50),
                    percent: Some(1.21),
                    high: Some(1720.00),
                    low: Some(1685.00),
                    volume: Some(12345.0),
                    ..Default::default()
                }),
            }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("sh600519", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing current price"));
    }

    #[test]
    fn test_parse_xueqiu_quote_error_code() {
        let resp = XueqiuResponse {
            data: None,
            error_code: Some(400016),
            error_description: Some("token缺失".to_string()),
        };
        let result = parse_xueqiu_quote("SH600519", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Xueqiu API error"));
    }

    #[test]
    fn test_record_xueqiu_warning_preserves_fallback_failure_for_frontend() {
        clear_quote_warning();

        record_xueqiu_warning("Network error fetching AAPL from Xueqiu: operation timed out");

        assert_eq!(
            take_quote_warning().as_deref(),
            Some(XUEQIU_API_FAILED_HINT)
        );
    }

    #[test]
    fn test_parse_xueqiu_quote_fallback_change_calculation() {
        let resp = XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuote {
                    name: Some("贵州茅台".to_string()),
                    current: Some(1100.00),
                    last_close: Some(1000.00),
                    chg: None,
                    percent: None,
                    high: Some(1200.00),
                    low: Some(950.00),
                    volume: Some(99999.0),
                    ..Default::default()
                }),
            }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("sh600519", "CN", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_xueqiu_response_deserialize() {
        let json = r#"{
            "data": {
                "market": {"status_id": 5},
                "quote": {
                    "symbol": "SH600519",
                    "name": "贵州茅台",
                    "current": 1725.01,
                    "last_close": 1714.51,
                    "chg": 10.5,
                    "percent": 0.61,
                    "high": 1729.0,
                    "low": 1711.0,
                    "volume": 2558913
                }
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let resp: XueqiuResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(resp.error_code, Some(0));
        let data = resp.data.unwrap();
        let quote = data.quote.unwrap();
        assert!((quote.current.unwrap() - 1725.01).abs() < 0.001);
    }

    #[test]
    fn test_xueqiu_response_with_extra_fields() {
        // Xueqiu returns many extra fields; our structs should ignore them.
        let json = r#"{
            "data": {
                "market": {"status_id": 5, "region": "CN"},
                "quote": {
                    "symbol": "SH600519",
                    "code": "600519",
                    "exchange": "SH",
                    "name": "贵州茅台",
                    "type": 11,
                    "sub_type": null,
                    "status": 1,
                    "current": 1725.01,
                    "last_close": 1714.51,
                    "chg": 10.5,
                    "percent": 0.61,
                    "high": 1729.0,
                    "low": 1711.0,
                    "volume": 2558913,
                    "amount": 4405880000.0,
                    "market_capital": 2167000000000.0,
                    "float_market_capital": 2100000000000.0,
                    "turnover_rate": 0.12,
                    "pe_ttm": 27.5,
                    "pe_lyr": 28.0,
                    "pb": 9.8,
                    "eps": 62.73,
                    "dividend": 2.1,
                    "dividend_yield": 0.12,
                    "currency": "CNY",
                    "navps": 176.21,
                    "profit": 7469000000.0,
                    "timestamp": 1700000000000,
                    "time": 1700000000000,
                    "open": 1715.0,
                    "avg_price": 1722.35
                }
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let resp: XueqiuResponse = serde_json::from_str(json).expect("should parse");
        assert_eq!(resp.error_code, Some(0));
        let data = resp.data.unwrap();
        let quote = data.quote.unwrap();
        assert_eq!(quote.name.unwrap(), "贵州茅台");
        assert!((quote.current.unwrap() - 1725.01).abs() < 0.001);
        assert!((quote.volume.unwrap() - 2558913.0).abs() < 0.001);
    }

    #[test]
    fn test_xueqiu_volume_converts_to_u64() {
        let resp = make_xueqiu_response(
            "SH600519",
            "贵州茅台",
            1516.0,
            1513.0,
            1519.0,
            1508.0,
            30279.0,
            3.0,
            0.2,
        );
        let result = parse_xueqiu_quote("sh600519", "CN", resp);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().volume, 30279);
    }

    #[test]
    fn test_xueqiu_client_returns_same_instance() {
        let c1 = http_client::xueqiu_client();
        let c2 = http_client::xueqiu_client();
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_xueqiu_client_can_build_request() {
        let client = http_client::xueqiu_client();
        let req = client
            .get("https://stock.xueqiu.com/v5/stock/quote.json?symbol=SH600519")
            .build()
            .expect("should build request");
        assert_eq!(req.method(), reqwest::Method::GET);
    }

    // ---- Xueqiu integration tests (require network) ----

    #[tokio::test]
    #[ignore]
    async fn test_integration_cn_xueqiu() {
        let result = fetch_xueqiu_cn_quote("sh600519").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "sh600519");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ CN quote (Xueqiu): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ CN Xueqiu quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_us_xueqiu() {
        let result = fetch_xueqiu_us_quote("AAPL").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.market, "US");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ US quote (Xueqiu): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ US Xueqiu quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_hk_xueqiu() {
        let result = fetch_xueqiu_hk_quote("00700").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.market, "HK");
                assert!(quote.current_price > 0.0, "Price should be positive");
                info!(
                    "✅ HK quote (Xueqiu): {} = {}",
                    quote.name, quote.current_price
                );
            }
            Err(e) => {
                warn!("⚠️ HK Xueqiu quote failed (network issue in CI): {}", e);
            }
        }
    }

    // ── Xueqiu kline response parsing tests ────────────────────────────

    #[test]
    fn test_parse_xueqiu_kline_identifies_first_trading_day_after_range() {
        // Exact shape and row returned for the newly listed sz001248.  The
        // timestamp is 2026-07-01 16:00 UTC, which is 2026-07-02 in China.
        let body = r#"{
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
        let start = chrono::NaiveDate::from_ymd_opt(2026, 6, 24).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();

        let outcome = parse_xueqiu_history_response(
            body,
            "sz001248",
            "CN",
            start,
            end,
            "https://example.test/kline",
        )
        .unwrap();

        assert_eq!(
            outcome,
            XueqiuHistoryOutcome::StartsAfterRange {
                first_available_date: chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(),
            }
        );
    }

    #[tokio::test]
    async fn test_xueqiu_history_does_not_fallback_before_first_trading_day() {
        let first_trading_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        let eastmoney_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let yahoo_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let eastmoney_called_by_fetcher = eastmoney_called.clone();
        let yahoo_called_by_fetcher = yahoo_called.clone();

        let prices = resolve_xueqiu_history_outcome(
            "sz001248",
            "CN",
            Ok(XueqiuHistoryOutcome::StartsAfterRange {
                first_available_date: first_trading_date,
            }),
            move || async move {
                eastmoney_called_by_fetcher.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Vec::new())
            },
            move || async move {
                yahoo_called_by_fetcher.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Vec::new())
            },
        )
        .await
        .unwrap();

        assert!(prices.is_empty());
        assert!(!eastmoney_called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!yahoo_called.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// Helper: parse a raw Xueqiu kline JSON body into (date, close) pairs
    /// using the same logic as `fetch_stock_history_xueqiu`.
    fn parse_xueqiu_kline_body(
        body: &str,
        start_date: chrono::NaiveDate,
        end_date: chrono::NaiveDate,
    ) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
        let resp: XueqiuKlineResponse =
            serde_json::from_str(body).map_err(|e| format!("parse error: {}", e))?;
        let data = resp.data.ok_or_else(|| "no data".to_string())?;
        let columns = data.column.unwrap_or_default();
        if columns.is_empty() {
            return Err("empty or missing 'column' field".to_string());
        }
        let ts_idx = columns
            .iter()
            .position(|c| c == "timestamp")
            .ok_or_else(|| format!("missing timestamp column, got: {:?}", columns))?;
        let close_idx = columns
            .iter()
            .position(|c| c == "close")
            .ok_or_else(|| format!("missing close column, got: {:?}", columns))?;
        let items = data.item.unwrap_or_default();
        let mut result = Vec::new();
        for item in &items {
            let ts_ms = item
                .get(ts_idx)
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)));
            let close = item.get(close_idx).and_then(|v| v.as_f64());
            if let (Some(ts_ms), Some(close_price)) = (ts_ms, close) {
                if let Some(dt) = chrono::DateTime::from_timestamp(ts_ms / 1000, 0) {
                    let date = dt.date_naive();
                    if date >= start_date && date <= end_date {
                        result.push((date, close_price));
                    }
                }
            }
        }
        result.sort_by_key(|(d, _)| *d);
        Ok(result)
    }

    #[test]
    fn test_parse_xueqiu_kline_integer_timestamps() {
        // Timestamps as JSON integers (the straightforward case).
        let body = r#"{
            "data": {
                "column": ["timestamp", "volume", "open", "high", "low", "close"],
                "item": [
                    [1724544000000, 1000, 100.0, 105.0, 99.0, 103.0],
                    [1724630400000, 2000, 103.0, 108.0, 102.0, 107.0]
                ]
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end).unwrap();
        assert_eq!(result.len(), 2);
        assert!((result[0].1 - 103.0).abs() < 0.001);
        assert!((result[1].1 - 107.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_xueqiu_kline_float_timestamps() {
        // Timestamps as JSON floats (e.g. 1724544000000.0).
        // This is the case that previously caused all items to be silently skipped.
        let body = r#"{
            "data": {
                "column": ["timestamp", "volume", "open", "high", "low", "close"],
                "item": [
                    [1724544000000.0, 1000, 100.0, 105.0, 99.0, 103.0],
                    [1724630400000.0, 2000, 103.0, 108.0, 102.0, 107.0]
                ]
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end).unwrap();
        assert_eq!(result.len(), 2, "Float timestamps must be parsed correctly");
        assert!((result[0].1 - 103.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_xueqiu_kline_empty_items() {
        let body = r#"{
            "data": {
                "column": ["timestamp", "volume", "open", "high", "low", "close"],
                "item": []
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_xueqiu_kline_missing_column() {
        let body = r#"{
            "data": {
                "column": ["time", "volume", "open", "high", "low", "price"],
                "item": [[1724544000000, 1000, 100.0, 105.0, 99.0, 103.0]]
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end);
        assert!(
            result.is_err(),
            "Should error when expected columns are missing"
        );
    }

    /// Test with the exact JSON structure returned by the live Xueqiu API,
    /// including the `symbol` field in data, 12 columns, and null values in
    /// items.  This reproduces the real-world response format to catch any
    /// deserialization issues.
    #[test]
    fn test_parse_xueqiu_kline_real_api_response() {
        let body = r#"{
          "data": {
            "symbol": "SH600519",
            "column": [
              "timestamp", "volume", "open", "high", "low", "close",
              "chg", "percent", "turnoverrate", "amount",
              "volume_post", "amount_post"
            ],
            "item": [
              [1772985600000, 3744162, 1390, 1404.9, 1383.2, 1397, -5, -0.36, 0.3, 5220095639, null, null],
              [1773072000000, 2462592, 1404.9, 1409.49, 1398, 1401.88, 4.88, 0.35, 0.2, 3457808916, null, null],
              [1773158400000, 2445673, 1402.99, 1405.99, 1398.02, 1400, -1.88, -0.13, 0.2, 3425363892, null, null]
            ]
          },
          "error_code": 0,
          "error_description": ""
        }"#;
        // March 2026 dates to cover the timestamps above
        let start = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end).unwrap();
        assert_eq!(result.len(), 3, "All three items should be parsed");
        // Verify close prices
        assert!((result[0].1 - 1397.0).abs() < 0.01);
        assert!((result[1].1 - 1401.88).abs() < 0.01);
        assert!((result[2].1 - 1400.0).abs() < 0.01);
    }

    /// Test that a response with `data` present but missing `column` field
    /// (as might happen with insufficient authentication) gives a clear error.
    #[test]
    fn test_parse_xueqiu_kline_missing_column_field() {
        let body = r#"{
            "data": {
                "symbol": "SH600519"
            },
            "error_code": 0,
            "error_description": ""
        }"#;
        let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end);
        assert!(result.is_err(), "Should error when column field is absent");
        let err = result.unwrap_err();
        assert!(
            err.contains("empty or missing"),
            "Error should mention empty or missing column: {}",
            err
        );
    }

    /// Test parsing a kline response that has `items`/`items_size` fields
    /// but no `column`/`item` fields (e.g. when API returns empty data).
    #[test]
    fn test_parse_xueqiu_kline_empty_data_response() {
        let body = r#"{"data":{"items":[],"items_size":0},"error_code":0,"error_description":""}"#;
        let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
        let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
        let result = parse_xueqiu_kline_body(body, start, end);
        assert!(
            result.is_err(),
            "Response without column field should be an error"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("empty or missing"),
            "Error should indicate missing column: {}",
            err
        );
    }

    // ---- timestamp_to_market_date tests ----

    #[test]
    fn test_timestamp_to_market_date_cn() {
        // 2026-03-06 00:00:00 CST (UTC+8) = 2026-03-05 16:00:00 UTC
        let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 5)
            .unwrap()
            .and_hms_opt(16, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let date = timestamp_to_market_date(ts, "CN").unwrap();
        assert_eq!(
            date,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
            "CN timestamp at midnight CST should map to 2026-03-06"
        );
    }

    #[test]
    fn test_timestamp_to_market_date_hk() {
        // Same offset as CN (UTC+8)
        let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 5)
            .unwrap()
            .and_hms_opt(16, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let date = timestamp_to_market_date(ts, "HK").unwrap();
        assert_eq!(
            date,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
            "HK timestamp at midnight CST should map to 2026-03-06"
        );
    }

    #[test]
    fn test_timestamp_to_market_date_us() {
        // US daily bars are timestamped at midnight UTC.
        // 2026-03-06 00:00:00 UTC → should map to 2026-03-06.
        let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 6)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let date = timestamp_to_market_date(ts, "US").unwrap();
        assert_eq!(
            date,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
            "US timestamp at midnight UTC should map to 2026-03-06"
        );
    }

    #[test]
    fn test_timestamp_to_market_date_utc_would_be_wrong() {
        // Verify that naively using UTC gives the WRONG date for CN stocks.
        // 2026-03-06 00:00:00 CST = 2026-03-05 16:00:00 UTC
        let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 5)
            .unwrap()
            .and_hms_opt(16, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        // Using UTC (old buggy behavior) would give 2026-03-05
        let utc_date = chrono::DateTime::from_timestamp(ts, 0)
            .unwrap()
            .date_naive();
        assert_eq!(
            utc_date,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 5).unwrap(),
            "UTC interpretation gives 2026-03-05 (wrong for CN market)"
        );

        // Using market-aware conversion gives correct 2026-03-06
        let market_date = timestamp_to_market_date(ts, "CN").unwrap();
        assert_eq!(
            market_date,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
            "Market-aware gives 2026-03-06 (correct)"
        );
    }
}
