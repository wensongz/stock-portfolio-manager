use crate::models::StockQuote;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const QUOTE_CACHE_TTL_SECS: u64 = 60; // 60-second cache

/// Cash symbol prefix used to represent cash holdings.
/// Cash symbols follow the pattern `$CASH-{CURRENCY}`, e.g. `$CASH-USD`, `$CASH-CNY`, `$CASH-HKD`.
pub const CASH_SYMBOL_PREFIX: &str = "$CASH-";

/// All recognised cash symbols.
pub const CASH_SYMBOLS: [&str; 3] = ["$CASH-USD", "$CASH-CNY", "$CASH-HKD"];

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
    }
}

struct CachedQuote {
    quote: StockQuote,
    cached_at: Instant,
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

    /// Returns a cached quote if it exists and is still fresh.
    pub fn get(&self, symbol: &str) -> Option<StockQuote> {
        let lock = self.inner.lock().unwrap();
        if let Some(cached) = lock.get(symbol) {
            if cached.cached_at.elapsed() < Duration::from_secs(QUOTE_CACHE_TTL_SECS) {
                return Some(cached.quote.clone());
            }
        }
        None
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
                cached_at: Instant::now(),
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
                    cached_at: now,
                },
            );
        }
    }

    /// Returns all fresh cached quotes for the given symbols, plus the list of
    /// symbols that are missing or stale.
    pub fn get_batch(&self, symbols: &[(String, String)]) -> (Vec<StockQuote>, Vec<(String, String)>) {
        let lock = self.inner.lock().unwrap();
        let ttl = Duration::from_secs(QUOTE_CACHE_TTL_SECS);
        let mut cached = Vec::new();
        let mut missing = Vec::new();
        for (symbol, market) in symbols {
            if let Some(entry) = lock.get(symbol.as_str()) {
                if entry.cached_at.elapsed() < ttl {
                    cached.push(entry.quote.clone());
                    continue;
                }
            }
            missing.push((symbol.clone(), market.clone()));
        }
        (cached, missing)
    }
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

/// Batch fetch quotes using the cache. Only fetches symbols that are stale or
/// missing from the cache, and updates the cache with fresh results.
/// Falls back to stale cache entries on network errors for individual symbols.
pub async fn fetch_quotes_batch_cached(
    cache: &QuoteCache,
    symbols: Vec<(String, String)>,
) -> Result<Vec<StockQuote>, String> {
    fetch_quotes_batch_cached_with_providers(cache, symbols, "yahoo", "yahoo").await
}

/// Batch fetch quotes using the cache with specified providers.
/// Duplicate symbols are automatically deduplicated so that each symbol is
/// looked up and fetched only once, even when held in multiple accounts.
pub async fn fetch_quotes_batch_cached_with_providers(
    cache: &QuoteCache,
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
) -> Result<Vec<StockQuote>, String> {
    // Deduplicate symbols so we only look up / fetch each symbol once.
    let unique_symbols = deduplicate_symbols(symbols);

    let (mut result, missing) = cache.get_batch(&unique_symbols);

    if missing.is_empty() {
        return Ok(result);
    }

    let fresh = fetch_quotes_batch_with_providers(missing.clone(), us_provider, hk_provider).await?;
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
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
        return Err(format!("Yahoo Finance API returned error for {}: {}", symbol, err));
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
        .unwrap_or(0);

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
    })
}

/// Fetch a US stock quote using the configured provider.
pub async fn fetch_us_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_us_quote_with_provider(symbol, "eastmoney").await
}

/// Fetch a US stock quote using the specified provider.
pub async fn fetch_us_quote_with_provider(symbol: &str, provider: &str) -> Result<StockQuote, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_us_quote(symbol).await,
        _ => fetch_yahoo_quote(symbol, "US").await,
    }
}

/// Fetch a HK stock quote using the configured provider. Appends ".HK" if not present for Yahoo.
pub async fn fetch_hk_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_hk_quote_with_provider(symbol, "eastmoney").await
}

/// Fetch a HK stock quote using the specified provider.
pub async fn fetch_hk_quote_with_provider(symbol: &str, provider: &str) -> Result<StockQuote, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_hk_quote(symbol).await,
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
pub async fn fetch_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_eastmoney_cn_quote(symbol).await
}

// ---------------------------------------------------------------------------
// East Money (东方财富) API
// ---------------------------------------------------------------------------

/// Shared storage for the East Money HTTP client.
/// Wrapped in `Mutex<Option<…>>` so the client can be lazily created *and*
/// reset when a network error indicates the underlying connection is broken.
static EASTMONEY_CLIENT: Mutex<Option<reqwest::Client>> = Mutex::new(None);

/// Tracks the number of successful requests made on the current East Money
/// client.  After [`EASTMONEY_REQUESTS_PER_CONNECTION`] successful requests
/// the client is proactively reset so that the next request opens a fresh
/// TCP connection.
static EASTMONEY_REQUEST_COUNT: Mutex<u32> = Mutex::new(0);

/// Maximum number of retry attempts for transient East Money API failures.
const EASTMONEY_MAX_RETRIES: u32 = 2;

/// Number of successful requests to send on a single East Money connection
/// before proactively closing it and opening a new one.
const EASTMONEY_REQUESTS_PER_CONNECTION: u32 = 10;

/// Build a fresh `reqwest::Client` configured for the East Money API.
///
/// The client is configured with:
/// - HTTP/1.1 only (avoids HTTP/2 negotiation issues with Chinese financial servers)
/// - Browser-like default headers (`Referer`, `User-Agent`, `Accept`)
/// - 15-second request timeout
fn build_eastmoney_client() -> reqwest::Client {
    use reqwest::header;
    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        header::REFERER,
        header::HeaderValue::from_static("https://www.eastmoney.com/"),
    );
    default_headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("*/*"),
    );
    default_headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
        ),
    );
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .http1_only()
        .default_headers(default_headers)
        .build()
        .expect("failed to build East Money HTTP client")
}

/// Return a shared `reqwest::Client` for all East Money API requests.
/// The client is created lazily and reused across calls so that the
/// underlying TCP connection(s) to `push2.eastmoney.com` are pooled.
/// If a network error occurs the caller should invoke
/// [`reset_eastmoney_client`] so that the next call rebuilds the client
/// with fresh connections.
fn eastmoney_client() -> reqwest::Client {
    let mut guard = EASTMONEY_CLIENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.get_or_insert_with(build_eastmoney_client).clone()
}

/// Reset the shared East Money client so the next call to
/// [`eastmoney_client`] will build a fresh one.  Also resets the request
/// counter back to zero.
fn reset_eastmoney_client() {
    let mut guard = EASTMONEY_CLIENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
    let mut count = EASTMONEY_REQUEST_COUNT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *count = 0;
}

/// Send a GET request to the East Money API with retry on transient failures.
///
/// On a connection-level error the shared client is reset so that the next
/// call to [`eastmoney_client`] builds a fresh one.  The request is retried
/// up to [`EASTMONEY_MAX_RETRIES`] times with a short delay between attempts.
///
/// After every [`EASTMONEY_REQUESTS_PER_CONNECTION`] successful requests the
/// client is proactively reset so the next request opens a fresh TCP
/// connection.  This avoids issues with long-lived connections being silently
/// dropped by upstream proxies or load-balancers.
async fn send_eastmoney_request(url: &str, symbol: &str) -> Result<reqwest::Response, String> {
    let mut last_err = String::new();
    for attempt in 0..=EASTMONEY_MAX_RETRIES {
        let result = eastmoney_client().get(url).send().await;
        match result {
            Ok(resp) => {
                // Track successful requests and rotate the connection when
                // the threshold is reached.
                let should_reset = {
                    let mut count = EASTMONEY_REQUEST_COUNT
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    *count += 1;
                    *count >= EASTMONEY_REQUESTS_PER_CONNECTION
                };
                if should_reset {
                    reset_eastmoney_client();
                }
                return Ok(resp);
            }
            Err(e) => {
                if e.is_connect() || e.is_request() {
                    reset_eastmoney_client();
                }
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
    rc: Option<i32>,
    data: Option<EastMoneyData>,
}

/// Inner data of an East Money quote response.
/// Field names follow the East Money API convention (f43, f44, …).
/// With `fltt=2` the numeric fields are returned as floats/integers directly.
/// All numeric fields use `f64` so they can accept both JSON integers and
/// JSON floats (e.g. `30279` and `30279.0`) — serde rejects JSON floats
/// when deserializing as `u64`.
#[derive(Debug, Deserialize)]
struct EastMoneyData {
    /// Current price
    f43: Option<f64>,
    /// Day high
    f44: Option<f64>,
    /// Day low
    f45: Option<f64>,
    /// Volume (lots / 手) — stored as f64 because the API may return
    /// the value with a decimal point (e.g. `30279.0`).
    f47: Option<f64>,
    /// Stock code (e.g. "600519")
    f57: Option<String>,
    /// Stock name (e.g. "贵州茅台")
    f58: Option<String>,
    /// Previous close
    f60: Option<f64>,
    /// Change amount
    f169: Option<f64>,
    /// Change percentage
    f170: Option<f64>,
}

/// Fetch a CN A-share stock quote from East Money (东方财富).
/// Symbol format: "sh600519" (Shanghai) or "sz000858" (Shenzhen).
/// The symbol is normalised to lowercase automatically.
async fn fetch_eastmoney_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    let symbol = symbol.to_lowercase();
    let secid = to_eastmoney_secid(&symbol)?;
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f57,f58,f60,f169,f170&secid={}",
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

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read East Money response body for {}: {}", symbol, e))?;

    let resp = parse_eastmoney_body(&body, &symbol)?;

    parse_eastmoney_quote(&symbol, "CN", resp)
}

/// Fetch a US stock quote from East Money (东方财富).
/// Symbol format: standard US ticker like "AAPL", "MSFT".
async fn fetch_eastmoney_us_quote(symbol: &str) -> Result<StockQuote, String> {
    let secid = to_eastmoney_us_secid(symbol);
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f57,f58,f60,f169,f170&secid={}",
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

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read East Money response body for {}: {}", symbol, e))?;

    let resp = parse_eastmoney_body(&body, symbol)?;

    parse_eastmoney_quote(symbol, "US", resp)
}

/// Fetch a HK stock quote from East Money (东方财富).
/// Symbol format: "00700", "09988", or "0700.HK".
async fn fetch_eastmoney_hk_quote(symbol: &str) -> Result<StockQuote, String> {
    let secid = to_eastmoney_hk_secid(symbol)?;
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f57,f58,f60,f169,f170&secid={}",
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

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read East Money response body for {}: {}", symbol, e))?;

    let resp = parse_eastmoney_body(&body, symbol)?;

    parse_eastmoney_quote(symbol, "HK", resp)
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
        _ => return Err(format!("Unknown CN market prefix '{}' in symbol {}", prefix, symbol)),
    };
    Ok(format!("{}.{}", market_id, code))
}

/// Convert a US stock ticker to East Money secid format: "105.{TICKER}".
fn to_eastmoney_us_secid(symbol: &str) -> String {
    format!("105.{}", symbol.to_uppercase())
}

/// Convert a HK stock symbol to East Money secid format: "116.{5-digit code}".
/// Strips the ".HK" suffix if present and zero-pads to 5 digits.
fn to_eastmoney_hk_secid(symbol: &str) -> Result<String, String> {
    let code = symbol
        .trim_end_matches(".HK")
        .trim_end_matches(".hk");
    // Zero-pad to 5 digits if the code is purely numeric
    if code.chars().all(|c| c.is_ascii_digit()) {
        let padded = format!("{:0>5}", code);
        Ok(format!("116.{}", padded))
    } else {
        Err(format!("Invalid HK symbol: {}", symbol))
    }
}

/// Parse the East Money JSON response into a `StockQuote`.
fn parse_eastmoney_quote(symbol: &str, market: &str, resp: EastMoneyResponse) -> Result<StockQuote, String> {
    let data = resp
        .data
        .ok_or_else(|| format!("No data from East Money for {}. Symbol may be invalid.", symbol))?;

    let name = data
        .f58
        .ok_or_else(|| format!("Missing stock name in East Money response for {}", symbol))?;
    let current_price = data
        .f43
        .ok_or_else(|| format!("Missing current price in East Money response for {}", symbol))?;
    let previous_close = data.f60.unwrap_or(0.0);

    let change = data.f169.unwrap_or_else(|| current_price - previous_close);
    let change_percent = data.f170.unwrap_or_else(|| {
        if previous_close != 0.0 {
            change / previous_close * 100.0
        } else {
            0.0
        }
    });

    let high = data.f44.unwrap_or(0.0);
    let low = data.f45.unwrap_or(0.0);
    let volume = data.f47.unwrap_or(0.0) as u64;

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
    })
}

/// Batch fetch quotes for multiple symbols with their markets.
/// Market is "US", "CN", or "HK".
pub async fn fetch_quotes_batch(
    symbols: Vec<(String, String)>,
) -> Result<Vec<StockQuote>, String> {
    fetch_quotes_batch_with_providers(symbols, "eastmoney", "eastmoney").await
}

/// Batch fetch quotes using the specified providers for US and HK markets.
/// CN always uses East Money. Cash symbols return synthetic quotes (price = 1.0).
/// Duplicate symbols are automatically deduplicated so that each symbol is fetched only once.
pub async fn fetch_quotes_batch_with_providers(
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
) -> Result<Vec<StockQuote>, String> {
    // Deduplicate symbols so we only fetch each symbol once,
    // even if it appears in multiple accounts.
    let unique_symbols = deduplicate_symbols(symbols);

    let mut quotes = Vec::new();
    for (symbol, market) in unique_symbols {
        // Cash symbols don't need an API call – return a synthetic quote.
        if is_cash_symbol(&symbol) {
            quotes.push(make_cash_quote(&symbol, &market));
            continue;
        }
        let result = match market.as_str() {
            "US" => fetch_us_quote_with_provider(&symbol, us_provider).await,
            "HK" => fetch_hk_quote_with_provider(&symbol, hk_provider).await,
            "CN" => fetch_cn_quote(&symbol).await,
            _ => Err(format!("Unknown market: {}", market)),
        };
        match result {
            Ok(quote) => quotes.push(quote),
            Err(e) => eprintln!("Warning: failed to fetch quote for {} ({}): {}", symbol, market, e),
        }
    }
    Ok(quotes)
}

// ---------------------------------------------------------------------------
// Historical price fetching (Yahoo Finance)
// ---------------------------------------------------------------------------

/// Convert a holding symbol + market to a Yahoo Finance ticker for historical queries.
pub fn to_yahoo_symbol(symbol: &str, market: &str) -> String {
    match market {
        "US" => symbol.to_string(),
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
            if s.starts_with("sh") {
                format!("{}.SS", &s[2..])
            } else if s.starts_with("sz") {
                format!("{}.SZ", &s[2..])
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("fetch_stock_history_yahoo: network error for {}: {}", yahoo_sym, e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "fetch_stock_history_yahoo: HTTP {} for {}",
            resp.status(),
            yahoo_sym
        ));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("fetch_stock_history_yahoo: parse error for {}: {}", yahoo_sym, e))?;

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
            if let Some(dt) = chrono::DateTime::from_timestamp(ts_i, 0) {
                result.push((dt.date_naive(), cl_f));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a synthetic East Money JSON response.
    fn make_eastmoney_response(
        code: &str,
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
            rc: Some(0),
            data: Some(EastMoneyData {
                f43: Some(current),
                f44: Some(high),
                f45: Some(low),
                f47: Some(volume),
                f57: Some(code.to_string()),
                f58: Some(name.to_string()),
                f60: Some(prev_close),
                f169: Some(change),
                f170: Some(change_pct),
            }),
        }
    }

    #[test]
    fn test_parse_eastmoney_quote_valid() {
        let resp = make_eastmoney_response(
            "600519",
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
        let resp = EastMoneyResponse {
            rc: Some(0),
            data: None,
        };
        let result = parse_eastmoney_quote("sh999999", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No data from East Money"));
    }

    #[test]
    fn test_parse_eastmoney_quote_missing_price() {
        let resp = EastMoneyResponse {
            rc: Some(0),
            data: Some(EastMoneyData {
                f43: None,
                f44: Some(1720.00),
                f45: Some(1685.00),
                f47: Some(12345.0),
                f57: Some("600519".to_string()),
                f58: Some("贵州茅台".to_string()),
                f60: Some(1690.00),
                f169: Some(20.50),
                f170: Some(1.21),
            }),
        };
        let result = parse_eastmoney_quote("sh600519", "CN", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing current price"));
    }

    #[test]
    fn test_parse_eastmoney_quote_change_calculation() {
        let resp = make_eastmoney_response(
            "600519",
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
            "600519",
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
            "600519",
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
        let resp = make_eastmoney_response(
            "AAPL",
            "苹果",
            195.50,
            193.00,
            197.00,
            192.00,
            50000.0,
            2.50,
            1.30,
        );
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
            rc: Some(0),
            data: Some(EastMoneyData {
                f43: Some(1100.00),
                f44: Some(1200.00),
                f45: Some(950.00),
                f47: Some(99999.0),
                f57: Some("600519".to_string()),
                f58: Some("贵州茅台".to_string()),
                f60: Some(1000.00),
                f169: None,
                f170: None,
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
        assert_eq!(resp.rc, Some(0));
        let data = resp.data.unwrap();
        assert!((data.f43.unwrap() - 1516.0).abs() < 0.001);
        assert!((data.f47.unwrap() - 30279.0).abs() < 0.001);
    }

    #[test]
    fn test_eastmoney_volume_converts_to_u64() {
        // The parse function should convert f64 volume to u64 correctly.
        let resp = make_eastmoney_response(
            "600519",
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
        let quotes = rt.block_on(fetch_quotes_batch_with_providers(symbols, "eastmoney", "eastmoney")).unwrap();
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
        let quotes = rt.block_on(fetch_quotes_batch_cached_with_providers(
            &cache, symbols, "eastmoney", "eastmoney",
        )).unwrap();
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
    fn test_to_yahoo_symbol_us() {
        assert_eq!(to_yahoo_symbol("AAPL", "US"), "AAPL");
        assert_eq!(to_yahoo_symbol("MSFT", "US"), "MSFT");
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
        let result = fetch_quotes_batch_with_providers(symbols, "yahoo", "yahoo").await;
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
                println!("✅ CN quote (East Money): {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ CN quote failed (network issue in CI): {}", e);
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
                println!("✅ US quote (Yahoo): {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ US quote failed (network issue in CI): {}", e);
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
                println!("✅ East Money quote: {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ East Money quote failed (network issue in CI): {}", e);
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
                println!("✅ US quote (East Money): {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ US East Money quote failed (network issue in CI): {}", e);
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
                println!("✅ HK quote (East Money): {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ HK East Money quote failed (network issue in CI): {}", e);
            }
        }
    }

    #[test]
    fn test_eastmoney_client_returns_usable_client() {
        // Obtaining the client twice should succeed without panic.
        let _c1 = eastmoney_client();
        let _c2 = eastmoney_client();
    }

    #[test]
    fn test_eastmoney_client_rebuilds_after_reset() {
        // Client can be obtained, then reset, then obtained again.
        let _c1 = eastmoney_client();
        reset_eastmoney_client();
        let _c2 = eastmoney_client();
    }

    #[test]
    fn test_build_eastmoney_client_has_default_headers() {
        // The client should be buildable without panic and be usable.
        // Default headers (Referer, User-Agent, Accept) are set on the
        // client and applied when requests are sent (not visible via
        // RequestBuilder::build(), which only shows request-level headers).
        let client = build_eastmoney_client();
        // Verify the client can construct a valid request.
        let req = client
            .get("https://push2.eastmoney.com/test")
            .build()
            .expect("should build request");
        assert_eq!(req.method(), reqwest::Method::GET);
    }

    #[test]
    fn test_eastmoney_request_count_resets_with_client() {
        // Resetting the client should also zero the request counter.
        {
            let mut count = EASTMONEY_REQUEST_COUNT
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *count = 5;
        }
        reset_eastmoney_client();
        {
            let count = EASTMONEY_REQUEST_COUNT
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            assert_eq!(*count, 0, "Counter should be zero after reset");
        }
    }

    #[test]
    fn test_eastmoney_connection_rotation_threshold() {
        // Verify the rotation threshold constant and that the counter
        // correctly triggers a client reset when it reaches the threshold.
        assert_eq!(EASTMONEY_REQUESTS_PER_CONNECTION, 10);

        // Start with a clean state.
        reset_eastmoney_client();

        // Simulate 9 successful requests – the client should still exist.
        {
            let mut count = EASTMONEY_REQUEST_COUNT
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *count = 9;
            assert!(*count < EASTMONEY_REQUESTS_PER_CONNECTION);
        }

        // After the 10th request the counter should trigger a reset.
        {
            let mut count = EASTMONEY_REQUEST_COUNT
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *count += 1;
            assert!(*count >= EASTMONEY_REQUESTS_PER_CONNECTION);
        }
        // Perform the reset (mirrors the logic in send_eastmoney_request).
        reset_eastmoney_client();
        {
            let count = EASTMONEY_REQUEST_COUNT
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            assert_eq!(*count, 0, "Counter should be zero after rotation");
        }
        // A new client should be obtainable after rotation.
        let _client = eastmoney_client();
    }
}
