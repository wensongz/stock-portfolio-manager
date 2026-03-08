use crate::models::StockQuote;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const QUOTE_CACHE_TTL_SECS: u64 = 60; // 1-minute cache

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
    let (mut result, missing) = cache.get_batch(&symbols);

    if missing.is_empty() {
        return Ok(result);
    }

    let fresh = fetch_quotes_batch(missing.clone()).await?;
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

/// Fetch a US stock quote.
pub async fn fetch_us_quote(symbol: &str) -> Result<StockQuote, String> {
    fetch_yahoo_quote(symbol, "US").await
}

/// Fetch a HK stock quote. Appends ".HK" if not present.
pub async fn fetch_hk_quote(symbol: &str) -> Result<StockQuote, String> {
    let yahoo_symbol = if symbol.ends_with(".HK") || symbol.ends_with(".hk") {
        symbol.to_string()
    } else {
        format!("{}.HK", symbol)
    };
    fetch_yahoo_quote(&yahoo_symbol, "HK").await
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
pub async fn fetch_cn_quote(symbol: &str) -> Result<StockQuote, String> {
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
    let mut quotes = Vec::new();
    for (symbol, market) in symbols {
        let result = match market.as_str() {
            "US" => fetch_us_quote(&symbol).await,
            "HK" => fetch_hk_quote(&symbol).await,
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
}
