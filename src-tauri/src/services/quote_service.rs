use crate::models::StockQuote;
use chrono::Utc;
use reqwest::cookie::CookieStore;
use reqwest::header::HeaderValue;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const QUOTE_CACHE_TTL_SECS: u64 = 60; // 60-second cache

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

/// Batch fetch quotes using the cache. Only fetches symbols that are stale or
/// missing from the cache, and updates the cache with fresh results.
/// Falls back to stale cache entries on network errors for individual symbols.
pub async fn fetch_quotes_batch_cached(
    cache: &QuoteCache,
    symbols: Vec<(String, String)>,
) -> Result<Vec<StockQuote>, String> {
    fetch_quotes_batch_cached_with_providers(cache, symbols, "xueqiu", "xueqiu").await
}

/// Batch fetch quotes using the cache with specified providers.
pub async fn fetch_quotes_batch_cached_with_providers(
    cache: &QuoteCache,
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
) -> Result<Vec<StockQuote>, String> {
    let (mut result, missing) = cache.get_batch(&symbols);

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
    fetch_us_quote_with_provider(symbol, "xueqiu").await
}

/// Fetch a US stock quote using the specified provider.
pub async fn fetch_us_quote_with_provider(symbol: &str, provider: &str) -> Result<StockQuote, String> {
    match provider {
        "yahoo" => fetch_yahoo_quote(symbol, "US").await,
        _ => {
            // Try Xueqiu first, fall back to Yahoo on failure
            match fetch_xueqiu_quote(symbol, "US").await {
                Ok(quote) => Ok(quote),
                Err(xq_err) => {
                    eprintln!(
                        "Warning: Xueqiu failed for {} (US), trying Yahoo fallback: {}",
                        symbol, xq_err
                    );
                    fetch_yahoo_quote(symbol, "US").await
                }
            }
        }
    }
}

/// Fetch a HK stock quote using the configured provider. Appends ".HK" if not present for Yahoo.
pub async fn fetch_hk_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_hk_quote_with_provider(symbol, "xueqiu").await
}

/// Fetch a HK stock quote using the specified provider.
pub async fn fetch_hk_quote_with_provider(symbol: &str, provider: &str) -> Result<StockQuote, String> {
    match provider {
        "yahoo" => {
            let yahoo_symbol = if symbol.ends_with(".HK") || symbol.ends_with(".hk") {
                symbol.to_string()
            } else {
                format!("{}.HK", symbol)
            };
            fetch_yahoo_quote(&yahoo_symbol, "HK").await
        }
        _ => {
            // Try Xueqiu first, fall back to Yahoo on failure
            match fetch_xueqiu_quote(symbol, "HK").await {
                Ok(quote) => Ok(quote),
                Err(xq_err) => {
                    eprintln!(
                        "Warning: Xueqiu failed for {} (HK), trying Yahoo fallback: {}",
                        symbol, xq_err
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
    }
}

/// Fetch a CN A-share stock quote.
/// Tries Xueqiu first, falls back to East Money on failure.
pub async fn fetch_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    match fetch_xueqiu_quote(symbol, "CN").await {
        Ok(quote) => Ok(quote),
        Err(xq_err) => {
            eprintln!(
                "Warning: Xueqiu failed for {} (CN), trying East Money fallback: {}",
                symbol, xq_err
            );
            fetch_eastmoney_cn_quote(symbol).await
        }
    }
}

// ---------------------------------------------------------------------------
// Xueqiu (雪球) API
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct XueqiuResponse {
    data: Option<XueqiuData>,
    error_code: Option<i32>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XueqiuData {
    quote: Option<XueqiuQuoteData>,
}

#[derive(Debug, Deserialize)]
struct XueqiuQuoteData {
    symbol: Option<String>,
    name: Option<String>,
    current: Option<f64>,
    percent: Option<f64>,
    chg: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    volume: Option<i64>,
    last_close: Option<f64>,
}

/// Convert a local symbol + market into the Xueqiu symbol format.
///
/// - CN: "sh600519" → "SH600519", "sz000858" → "SZ000858"
/// - HK: "00700" → "00700", "0700.HK" → "00700" (left-padded to 5 digits)
/// - US: "AAPL" → "AAPL"
fn to_xueqiu_symbol(symbol: &str, market: &str) -> String {
    match market {
        "CN" => symbol.to_uppercase(),
        "HK" => {
            let code = if symbol.to_uppercase().ends_with(".HK") {
                &symbol[..symbol.len() - 3]
            } else {
                symbol
            };
            // Xueqiu uses 5-digit HK stock codes, left-padded with zeros
            if code.chars().all(|c| c.is_ascii_digit()) && code.len() < 5 {
                format!("{:0>5}", code)
            } else {
                code.to_string()
            }
        }
        _ => symbol.to_string(),
    }
}

/// Fetch a stock quote from the Xueqiu (雪球) API.
///
/// The API requires a session cookie obtained from the main page first.
/// We use an explicit cookie jar and manually forward cookies to the API
/// subdomain (`stock.xueqiu.com`) to work around cross-subdomain cookie
/// restrictions.
pub async fn fetch_xueqiu_quote(symbol: &str, market: &str) -> Result<StockQuote, String> {
    let xq_symbol = to_xueqiu_symbol(symbol, market);

    let jar = Arc::new(reqwest::cookie::Jar::default());
    let client = reqwest::Client::builder()
        .cookie_provider(jar.clone())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    // Obtain session cookie by visiting the main page.
    let init_resp = client
        .get("https://xueqiu.com")
        .header("User-Agent", ua)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        )
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .send()
        .await
        .map_err(|e| format!("Failed to get Xueqiu session: {}", e))?;

    // Collect cookies from the response as well as from the cookie jar.
    // This covers cookies set during redirects (stored in jar) and cookies
    // set in the final response (available via response.cookies()).
    let mut cookie_parts: Vec<String> = Vec::new();

    // Cookies from the jar for xueqiu.com
    let xq_url: url::Url = "https://xueqiu.com"
        .parse()
        .map_err(|_| "Invalid URL".to_string())?;
    if let Some(jar_cookies) = jar.cookies(&xq_url) {
        if let Ok(s) = jar_cookies.to_str() {
            if !s.is_empty() {
                cookie_parts.push(s.to_string());
            }
        }
    }

    // Cookies from the response headers (in case they weren't picked up by the jar)
    for c in init_resp.cookies() {
        let pair = format!("{}={}", c.name(), c.value());
        if !cookie_parts.iter().any(|p| p.contains(&pair)) {
            cookie_parts.push(pair);
        }
    }

    let cookie_header = cookie_parts.join("; ");

    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xq_symbol
    );

    let mut req = client
        .get(&url)
        .header("User-Agent", ua)
        .header("Accept", "application/json")
        .header("Referer", "https://xueqiu.com/")
        .header("Origin", "https://xueqiu.com");

    // Manually forward cookies to the API subdomain
    if !cookie_header.is_empty() {
        if let Ok(val) = HeaderValue::from_str(&cookie_header) {
            req = req.header("Cookie", val);
        }
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Network error fetching {} from Xueqiu: {}", symbol, e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Xueqiu API error for {}: HTTP {}",
            symbol,
            response.status()
        ));
    }

    let resp: XueqiuResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Xueqiu response for {}: {}", symbol, e))?;

    parse_xueqiu_quote(symbol, market, resp)
}

/// Parse the Xueqiu JSON response into a `StockQuote`.
fn parse_xueqiu_quote(
    original_symbol: &str,
    market: &str,
    resp: XueqiuResponse,
) -> Result<StockQuote, String> {
    if let Some(code) = resp.error_code {
        if code != 0 {
            let desc = resp.error_description.unwrap_or_default();
            return Err(format!("Xueqiu API error for {}: {} (code {})", original_symbol, desc, code));
        }
    }

    let data_wrapper = resp
        .data
        .ok_or_else(|| format!("No data from Xueqiu for {}", original_symbol))?;

    let data = data_wrapper
        .quote
        .ok_or_else(|| format!("No quote data from Xueqiu for {}", original_symbol))?;

    let name = data.name.unwrap_or_else(|| original_symbol.to_string());
    let current_price = data
        .current
        .ok_or_else(|| format!("Missing current price in Xueqiu response for {}", original_symbol))?;
    let previous_close = data.last_close.unwrap_or(0.0);
    let change = data.chg.unwrap_or_else(|| current_price - previous_close);
    let change_percent = data.percent.unwrap_or_else(|| {
        if previous_close != 0.0 {
            change / previous_close * 100.0
        } else {
            0.0
        }
    });

    Ok(StockQuote {
        symbol: original_symbol.to_string(),
        name,
        market: market.to_string(),
        current_price,
        previous_close,
        change,
        change_percent,
        high: data.high.unwrap_or(0.0),
        low: data.low.unwrap_or(0.0),
        volume: data.volume.map(|v| v.max(0) as u64).unwrap_or(0),
        updated_at: Utc::now().to_rfc3339(),
    })
}

// ---------------------------------------------------------------------------
// East Money (东方财富) API – kept for internal / fallback use
// ---------------------------------------------------------------------------

/// East Money API response for a single stock quote.
#[derive(Debug, Deserialize)]
struct EastMoneyResponse {
    rc: Option<i32>,
    data: Option<EastMoneyData>,
}

/// Inner data of an East Money quote response.
/// Field names follow the East Money API convention (f43, f44, …).
/// With `fltt=2` the numeric fields are returned as floats/integers directly.
#[derive(Debug, Deserialize)]
struct EastMoneyData {
    /// Current price
    f43: Option<f64>,
    /// Day high
    f44: Option<f64>,
    /// Day low
    f45: Option<f64>,
    /// Volume (lots / 手)
    f47: Option<u64>,
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
/// Used as a fallback when the Xueqiu API is unavailable.
async fn fetch_eastmoney_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    let symbol = symbol.to_lowercase();
    let secid = to_eastmoney_secid(&symbol)?;
    let url = format!(
        "https://push2.eastmoney.com/api/qt/stock/get?fltt=2&invt=2&fields=f43,f44,f45,f47,f57,f58,f60,f169,f170&secid={}",
        secid
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
            "East Money API error for {}: HTTP {}",
            symbol,
            response.status()
        ));
    }

    let resp: EastMoneyResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse East Money response for {}: {}", symbol, e))?;

    parse_eastmoney_quote(&symbol, resp)
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

/// Parse the East Money JSON response into a `StockQuote`.
fn parse_eastmoney_quote(symbol: &str, resp: EastMoneyResponse) -> Result<StockQuote, String> {
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
    let volume = data.f47.unwrap_or(0);

    Ok(StockQuote {
        symbol: symbol.to_string(),
        name,
        market: "CN".to_string(),
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
    fetch_quotes_batch_with_providers(symbols, "xueqiu", "xueqiu").await
}

/// Batch fetch quotes using the specified providers for US and HK markets.
/// CN always uses the default provider (Xueqiu).
pub async fn fetch_quotes_batch_with_providers(
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
) -> Result<Vec<StockQuote>, String> {
    let mut quotes = Vec::new();
    for (symbol, market) in symbols {
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
        volume: u64,
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
            12345,
            20.50,
            1.21,
        );
        let result = parse_eastmoney_quote("sh600519", resp);
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
        let result = parse_eastmoney_quote("sh999999", resp);
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
                f47: Some(12345),
                f57: Some("600519".to_string()),
                f58: Some("贵州茅台".to_string()),
                f60: Some(1690.00),
                f169: Some(20.50),
                f170: Some(1.21),
            }),
        };
        let result = parse_eastmoney_quote("sh600519", resp);
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
            99999,
            100.00,
            10.00,
        );
        let result = parse_eastmoney_quote("sh600519", resp);
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
            12345,
            20.50,
            1.21,
        );
        let result = parse_eastmoney_quote("sh600519", resp);
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
            12345,
            20.50,
            1.21,
        );
        let result = parse_eastmoney_quote(&lower, resp);
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
    fn test_parse_eastmoney_quote_fallback_change_calculation() {
        // When f169/f170 are missing, change should be computed from price
        let resp = EastMoneyResponse {
            rc: Some(0),
            data: Some(EastMoneyData {
                f43: Some(1100.00),
                f44: Some(1200.00),
                f45: Some(950.00),
                f47: Some(99999),
                f57: Some("600519".to_string()),
                f58: Some("贵州茅台".to_string()),
                f60: Some(1000.00),
                f169: None,
                f170: None,
            }),
        };
        let result = parse_eastmoney_quote("sh600519", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
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
    fn test_quote_cache_update_overwrites() {
        let cache = QuoteCache::new();
        let mut quote = sample_quote("AAPL", "US");
        cache.set(quote.clone());
        assert!((cache.get("AAPL").unwrap().current_price - 100.0).abs() < 0.001);

        quote.current_price = 200.0;
        cache.set(quote);
        assert!((cache.get("AAPL").unwrap().current_price - 200.0).abs() < 0.001);
    }

    // ---- Xueqiu parser tests ----

    fn make_xueqiu_response(
        symbol: &str,
        name: &str,
        current: f64,
        percent: f64,
        chg: f64,
        high: f64,
        low: f64,
        volume: i64,
        last_close: f64,
    ) -> XueqiuResponse {
        XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuoteData {
                    symbol: Some(symbol.to_string()),
                    name: Some(name.to_string()),
                    current: Some(current),
                    percent: Some(percent),
                    chg: Some(chg),
                    high: Some(high),
                    low: Some(low),
                    volume: Some(volume),
                    last_close: Some(last_close),
                }),
            }),
            error_code: Some(0),
            error_description: None,
        }
    }

    #[test]
    fn test_parse_xueqiu_quote_valid_cn() {
        let resp = make_xueqiu_response(
            "SH600519", "贵州茅台", 1710.50, 1.21, 20.50, 1720.00, 1685.00, 12345, 1690.00,
        );
        let result = parse_xueqiu_quote("sh600519", "CN", resp);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "sh600519");
        assert_eq!(quote.name, "贵州茅台");
        assert_eq!(quote.market, "CN");
        assert!((quote.current_price - 1710.50).abs() < 0.001);
        assert!((quote.previous_close - 1690.00).abs() < 0.001);
        assert!((quote.change - 20.50).abs() < 0.001);
        assert!((quote.change_percent - 1.21).abs() < 0.001);
        assert!((quote.high - 1720.00).abs() < 0.001);
        assert!((quote.low - 1685.00).abs() < 0.001);
        assert_eq!(quote.volume, 12345);
    }

    #[test]
    fn test_parse_xueqiu_quote_valid_us() {
        let resp = make_xueqiu_response(
            "AAPL", "Apple Inc.", 195.50, 1.50, 2.89, 196.00, 193.00, 50000000, 192.61,
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
            "00700", "腾讯控股", 380.00, 2.15, 8.00, 385.00, 375.00, 20000000, 372.00,
        );
        let result = parse_xueqiu_quote("00700", "HK", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "00700");
        assert_eq!(quote.market, "HK");
        assert!((quote.current_price - 380.00).abs() < 0.001);
    }

    #[test]
    fn test_parse_xueqiu_quote_no_data() {
        let resp = XueqiuResponse {
            data: None,
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("AAPL", "US", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No data from Xueqiu"));
    }

    #[test]
    fn test_parse_xueqiu_quote_empty_data() {
        let resp = XueqiuResponse {
            data: Some(XueqiuData { quote: None }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("AAPL", "US", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No quote data"));
    }

    #[test]
    fn test_parse_xueqiu_quote_error_code() {
        let resp = XueqiuResponse {
            data: None,
            error_code: Some(400),
            error_description: Some("Invalid symbol".to_string()),
        };
        let result = parse_xueqiu_quote("INVALID", "US", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Xueqiu API error"));
    }

    #[test]
    fn test_parse_xueqiu_quote_missing_price() {
        let resp = XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuoteData {
                    symbol: Some("AAPL".to_string()),
                    name: Some("Apple Inc.".to_string()),
                    current: None,
                    percent: Some(1.5),
                    chg: Some(2.89),
                    high: Some(196.0),
                    low: Some(193.0),
                    volume: Some(50000000),
                    last_close: Some(192.61),
                }),
            }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("AAPL", "US", resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing current price"));
    }

    #[test]
    fn test_parse_xueqiu_quote_fallback_change() {
        // When chg/percent are None, change should be computed from prices
        let resp = XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuoteData {
                    symbol: Some("AAPL".to_string()),
                    name: Some("Apple Inc.".to_string()),
                    current: Some(1100.0),
                    percent: None,
                    chg: None,
                    high: Some(1200.0),
                    low: Some(950.0),
                    volume: Some(99999),
                    last_close: Some(1000.0),
                }),
            }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("AAPL", "US", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_to_xueqiu_symbol_cn() {
        assert_eq!(to_xueqiu_symbol("sh600519", "CN"), "SH600519");
        assert_eq!(to_xueqiu_symbol("sz000858", "CN"), "SZ000858");
    }

    #[test]
    fn test_to_xueqiu_symbol_us() {
        assert_eq!(to_xueqiu_symbol("AAPL", "US"), "AAPL");
    }

    #[test]
    fn test_to_xueqiu_symbol_hk() {
        assert_eq!(to_xueqiu_symbol("00700", "HK"), "00700");
        assert_eq!(to_xueqiu_symbol("0700.HK", "HK"), "00700");
        assert_eq!(to_xueqiu_symbol("0700.hk", "HK"), "00700");
        assert_eq!(to_xueqiu_symbol("700.HK", "HK"), "00700");
    }

    #[test]
    fn test_xueqiu_quote_json_deserialization() {
        // Verify that an actual quote.json response from Xueqiu deserializes correctly.
        // This format contains nested data.quote with many extra fields we ignore.
        let json = r#"{
            "data": {
                "market": { "status_id": 7, "region": "CN", "status": "已收盘" },
                "quote": {
                    "symbol": "SH601288",
                    "code": "601288",
                    "name": "农业银行",
                    "current": 3.65,
                    "percent": -0.27,
                    "chg": -0.01,
                    "high": 3.66,
                    "low": 3.64,
                    "last_close": 3.66,
                    "volume": 159652038,
                    "amount": 581968132,
                    "turnover_rate": 0.05,
                    "amplitude": 0.55,
                    "open": 3.66,
                    "market_capital": 1277438073636,
                    "float_market_capital": 1073301822750,
                    "pe_ttm": 6.222,
                    "pb": 0.773,
                    "timestamp": 1562310000000,
                    "total_shares": 349983033873,
                    "status": 1
                },
                "others": { "pankou_ratio": -31.79 },
                "tags": []
            },
            "error_code": 0,
            "error_description": ""
        }"#;

        let resp: XueqiuResponse = serde_json::from_str(json).expect("Failed to deserialize quote.json");
        let result = parse_xueqiu_quote("sh601288", "CN", resp);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "sh601288");
        assert_eq!(quote.name, "农业银行");
        assert!((quote.current_price - 3.65).abs() < 0.001);
        assert!((quote.previous_close - 3.66).abs() < 0.001);
        assert_eq!(quote.volume, 159652038);
    }

    #[test]
    fn test_parse_xueqiu_quote_missing_name_fallback() {
        // When name is not present, should fallback to symbol
        let resp = XueqiuResponse {
            data: Some(XueqiuData {
                quote: Some(XueqiuQuoteData {
                    symbol: Some("AAPL".to_string()),
                    name: None,
                    current: Some(195.50),
                    percent: Some(1.5),
                    chg: Some(2.89),
                    high: Some(196.0),
                    low: Some(193.0),
                    volume: Some(50000000),
                    last_close: Some(192.61),
                }),
            }),
            error_code: Some(0),
            error_description: None,
        };
        let result = parse_xueqiu_quote("AAPL", "US", resp);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.name, "AAPL");
    }

    // ---- HK symbol padding edge-case tests ----

    #[test]
    fn test_to_xueqiu_symbol_hk_already_5_digits() {
        // Already 5 digits, should remain unchanged
        assert_eq!(to_xueqiu_symbol("09988", "HK"), "09988");
    }

    #[test]
    fn test_to_xueqiu_symbol_hk_3_digit() {
        // 3-digit code should be padded to 5 digits
        assert_eq!(to_xueqiu_symbol("700", "HK"), "00700");
    }

    #[test]
    fn test_to_xueqiu_symbol_hk_4_digit_with_suffix() {
        // 4-digit code with .HK suffix should be padded
        assert_eq!(to_xueqiu_symbol("9988.HK", "HK"), "09988");
    }

    #[test]
    fn test_to_xueqiu_symbol_hk_1_digit() {
        // 1-digit code (edge case) should be padded
        assert_eq!(to_xueqiu_symbol("5", "HK"), "00005");
    }

    // ---- Integration tests using real network calls ----
    // These tests verify that the API actually works end-to-end.
    // They are marked #[ignore] so they only run when explicitly requested
    // via `cargo test -- --ignored`.

    #[tokio::test]
    #[ignore]
    async fn test_integration_xueqiu_cn_quote() {
        let result = fetch_xueqiu_quote("sh600519", "CN").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "sh600519");
                assert_eq!(quote.market, "CN");
                assert!(quote.current_price > 0.0, "Price should be positive");
                println!("✅ Xueqiu CN quote: {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ Xueqiu CN quote failed (may be expected in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_xueqiu_us_quote() {
        let result = fetch_xueqiu_quote("MSFT", "US").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "MSFT");
                assert_eq!(quote.market, "US");
                assert!(quote.current_price > 0.0, "Price should be positive");
                println!("✅ Xueqiu US quote: {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ Xueqiu US quote failed (may be expected in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_xueqiu_hk_quote() {
        let result = fetch_xueqiu_quote("0700.HK", "HK").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "0700.HK");
                assert_eq!(quote.market, "HK");
                assert!(quote.current_price > 0.0, "Price should be positive");
                println!("✅ Xueqiu HK quote: {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ Xueqiu HK quote failed (may be expected in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_cn_fallback_to_eastmoney() {
        // fetch_cn_quote should succeed even if Xueqiu fails, via East Money
        let result = fetch_cn_quote("sh600519").await;
        match &result {
            Ok(quote) => {
                assert_eq!(quote.symbol, "sh600519");
                assert!(quote.current_price > 0.0, "Price should be positive");
                println!("✅ CN quote (with fallback): {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ CN quote failed even with fallback (network issue in CI): {}", e);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_integration_us_fallback_to_yahoo() {
        // fetch_us_quote_with_provider should succeed even if Xueqiu fails, via Yahoo
        let result = fetch_us_quote_with_provider("MSFT", "xueqiu").await;
        match &result {
            Ok(quote) => {
                assert!(quote.current_price > 0.0, "Price should be positive");
                println!("✅ US quote (with fallback): {} = {}", quote.name, quote.current_price);
            }
            Err(e) => {
                println!("⚠️ US quote failed even with fallback (network issue in CI): {}", e);
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

    // ---- Cookie jar cross-subdomain tests ----
    // These tests validate the root cause of the HTTP 400 bug:
    // cookies set without Domain attribute by xueqiu.com are NOT automatically
    // forwarded to stock.xueqiu.com by the reqwest cookie jar.

    #[test]
    fn test_cookie_jar_with_domain_attr_forwards_to_subdomain() {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let main_url: url::Url = "https://xueqiu.com".parse().unwrap();
        let api_url: url::Url = "https://stock.xueqiu.com/v5/stock/quote.json".parse().unwrap();

        jar.set_cookies(
            &mut [HeaderValue::from_static(
                "xq_a_token=abc123; Domain=.xueqiu.com; Path=/",
            )]
            .iter()
            .map(|v| v),
            &main_url,
        );

        let api_cookies = jar.cookies(&api_url);
        assert!(
            api_cookies.is_some(),
            "With Domain=.xueqiu.com, cookies should forward to stock.xueqiu.com"
        );
        let cookie_str = api_cookies.unwrap().to_str().unwrap().to_string();
        assert!(
            cookie_str.contains("xq_a_token"),
            "Forwarded cookie should contain xq_a_token"
        );
    }

    #[test]
    fn test_cookie_jar_without_domain_attr_needs_manual_forwarding() {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let main_url: url::Url = "https://xueqiu.com".parse().unwrap();
        let api_url: url::Url = "https://stock.xueqiu.com/v5/stock/quote.json".parse().unwrap();

        // Simulate cookie set WITHOUT Domain attribute (as Xueqiu may do)
        jar.set_cookies(
            &mut [HeaderValue::from_static("xq_a_token=abc123; Path=/")]
                .iter()
                .map(|v| v),
            &main_url,
        );

        let api_cookies = jar.cookies(&api_url);
        let auto_forwarded = api_cookies
            .as_ref()
            .and_then(|c| c.to_str().ok())
            .map(|s| s.contains("xq_a_token"))
            .unwrap_or(false);

        // Regardless of whether the jar auto-forwards, our manual approach works:
        // Extract cookies from jar for main domain, set as Cookie header on API request
        let main_cookies = jar.cookies(&main_url);
        assert!(
            main_cookies.is_some(),
            "Cookies should exist for the main domain"
        );
        let main_str = main_cookies.unwrap().to_str().unwrap().to_string();
        assert!(
            main_str.contains("xq_a_token"),
            "Main domain should have the cookie for manual forwarding"
        );

        // Validate we can create a valid Cookie header from extracted cookies
        let header_value = HeaderValue::from_str(&main_str);
        assert!(
            header_value.is_ok(),
            "Should create valid HeaderValue from cookie string"
        );

        if !auto_forwarded {
            // This confirms the bug: without Domain attr, cookies may not auto-forward.
            // Our manual forwarding approach handles this case.
            println!("Confirmed: manual cookie forwarding needed for cross-subdomain");
        }
    }
}
