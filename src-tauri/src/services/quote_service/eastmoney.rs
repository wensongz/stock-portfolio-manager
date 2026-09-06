use crate::models::{PriceCandle, StockQuote};
use crate::services::http_client;
use chrono::Utc;
use serde::Deserialize;
use std::time::Duration;

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
pub(super) fn parse_eastmoney_body(body: &str, symbol: &str) -> Result<EastMoneyResponse, String> {
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
pub(super) struct EastMoneyResponse {
    pub(super) data: Option<EastMoneyData>,
}

/// One portfolio symbol and the EastMoney `secid` candidates used by the
/// batch quote endpoint. US symbols are probed across all supported US
/// exchange namespaces because the application stores only `market = US`,
/// not the individual listing exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EastMoneyBatchRequestSymbol {
    pub(super) original_symbol: String,
    pub(super) market: String,
    pub(super) api_secids: Vec<String>,
    pub(super) aliases: Vec<(String, String)>,
}

/// Keep expanded `secid` lists below common proxy URL-size limits. A US
/// symbol consumes four candidates, while CN and HK symbols consume one.
const EASTMONEY_QUOTE_BATCH_MAX_SECIDS: usize = 200;

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

#[derive(Debug, Deserialize)]
struct EastMoneyBatchResponse {
    rc: Option<i32>,
    data: Option<EastMoneyBatchData>,
}

#[derive(Debug, Deserialize)]
struct EastMoneyBatchData {
    diff: Option<EastMoneyBatchDiff>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EastMoneyBatchDiff {
    List(Vec<EastMoneyBatchQuote>),
    Map(std::collections::HashMap<String, EastMoneyBatchQuote>),
}

impl EastMoneyBatchDiff {
    fn into_quotes(self) -> Vec<EastMoneyBatchQuote> {
        match self {
            Self::List(quotes) => quotes,
            Self::Map(quotes) => quotes.into_values().collect(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct EastMoneyBatchQuote {
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f2: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f3: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f4: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f5: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f8: Option<f64>,
    f12: Option<String>,
    f13: Option<i32>,
    f14: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f15: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f16: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f18: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f20: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f23: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    f115: Option<f64>,
}

/// Parse a batch quote response and restore the exact symbols supplied by the
/// caller. Items omitted by EastMoney, or present without a current price,
/// are intentionally left out so the orchestration layer can fall back to the
/// existing single-symbol request path.
pub(super) fn parse_eastmoney_batch_body(
    body: &str,
    request_symbols: &[EastMoneyBatchRequestSymbol],
) -> Result<Vec<StockQuote>, String> {
    let response: EastMoneyBatchResponse = serde_json::from_str(body).map_err(|e| {
        let preview: String = body.chars().take(EASTMONEY_RESPONSE_PREVIEW_LEN).collect();
        format!(
            "Failed to parse East Money batch response: {}. Response preview: {}",
            e, preview
        )
    })?;
    if response.rc.is_some_and(|rc| rc != 0) {
        return Err(format!(
            "East Money batch API returned rc={}",
            response.rc.unwrap_or_default()
        ));
    }
    let diff = response
        .data
        .and_then(|data| data.diff)
        .ok_or_else(|| "No data from East Money batch quotes".to_string())?;

    let request_by_secid: std::collections::HashMap<String, &EastMoneyBatchRequestSymbol> =
        request_symbols
            .iter()
            .flat_map(|request| {
                request
                    .api_secids
                    .iter()
                    .map(move |secid| (secid.to_ascii_uppercase(), request))
            })
            .collect();
    let mut seen = std::collections::HashSet::new();
    let mut quotes = Vec::new();

    for item in diff.into_quotes() {
        let (Some(code), Some(market_id), Some(current_price)) =
            (item.f12.as_deref(), item.f13, item.f2)
        else {
            continue;
        };
        let secid = format!("{}.{}", market_id, code).to_ascii_uppercase();
        let Some(request) = request_by_secid.get(&secid) else {
            continue;
        };
        let previous_close = item.f18.unwrap_or(0.0);
        let change = item.f4.unwrap_or(current_price - previous_close);
        let change_percent = item.f3.unwrap_or_else(|| {
            if previous_close == 0.0 {
                0.0
            } else {
                change / previous_close * 100.0
            }
        });
        let original_symbols = std::iter::once((&request.original_symbol, &request.market)).chain(
            request
                .aliases
                .iter()
                .map(|(symbol, market)| (symbol, market)),
        );
        for (original_symbol, market) in original_symbols {
            if !seen.insert((market.clone(), original_symbol.clone())) {
                continue;
            }
            quotes.push(StockQuote {
                symbol: original_symbol.clone(),
                name: item
                    .f14
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| original_symbol.clone()),
                market: market.clone(),
                current_price,
                previous_close,
                change,
                change_percent,
                high: item.f15.unwrap_or(0.0),
                low: item.f16.unwrap_or(0.0),
                volume: item.f5.unwrap_or(0.0) as i64,
                updated_at: Utc::now().to_rfc3339(),
                pe_ttm: item.f115,
                pb: item.f23,
                market_cap: item.f20,
                dividend_yield: None,
                eps: None,
                roe: None,
                turnover_rate: item.f8,
            });
        }
    }
    Ok(quotes)
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
pub(super) struct EastMoneyData {
    /// Current price
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f43: Option<f64>,
    /// Day high
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f44: Option<f64>,
    /// Day low
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f45: Option<f64>,
    /// Volume (lots / 手) — stored as f64 because the API may return
    /// the value with a decimal point (e.g. `30279.0`).
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f47: Option<f64>,
    /// Stock name (e.g. "贵州茅台")
    pub(super) f58: Option<String>,
    /// Previous close
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f60: Option<f64>,
    /// Change amount
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f169: Option<f64>,
    /// Change percentage
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f170: Option<f64>,
    // ── Fundamentals (added for investment analysis) ──
    /// P/E ratio (TTM)
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f163: Option<f64>,
    /// P/B ratio
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f167: Option<f64>,
    /// Total market capitalisation (元)
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f116: Option<f64>,
    /// Turnover rate (percent)
    #[serde(default, deserialize_with = "deserialize_lenient_f64")]
    pub(super) f168: Option<f64>,
}

/// Fetch a CN A-share stock quote from East Money (东方财富).
/// Symbol format: "sh600519" (Shanghai) or "sz000858" (Shenzhen).
/// The symbol is normalised to lowercase automatically.
pub(super) async fn fetch_eastmoney_cn_quote(symbol: &str) -> Result<StockQuote, String> {
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
pub(super) async fn fetch_eastmoney_us_quote(symbol: &str) -> Result<StockQuote, String> {
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
pub(super) async fn fetch_eastmoney_hk_quote(symbol: &str) -> Result<StockQuote, String> {
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
        "HSCE" | "HSCEI" => Some(("100.HSCEI", "恒生中国企业指数")),
        "000300" | "CSI300" | "HS300" => Some(("1.000300", "沪深300")),
        "000001" | "SSE" | "SSEC" | "SHCOMP" => Some(("1.000001", "上证综指")),
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
pub(super) fn to_eastmoney_secid(symbol: &str) -> Result<String, String> {
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
pub(super) fn to_eastmoney_us_secid(symbol: &str) -> String {
    let upper = symbol.to_uppercase();
    if upper.contains('-') {
        format!("106.{}", upper.replace('-', "_"))
    } else {
        format!("105.{}", upper)
    }
}

/// Convert a HK stock symbol to East Money secid format: "116.{5-digit code}".
/// Strips the ".HK" suffix if present and zero-pads to 5 digits.
pub(super) fn to_eastmoney_hk_secid(symbol: &str) -> Result<String, String> {
    let code = symbol.trim_end_matches(".HK").trim_end_matches(".hk");
    // Zero-pad to 5 digits if the code is purely numeric
    if code.chars().all(|c| c.is_ascii_digit()) {
        let padded = format!("{:0>5}", code);
        Ok(format!("116.{}", padded))
    } else {
        Err(format!("Invalid HK symbol: {}", symbol))
    }
}

fn is_safe_eastmoney_us_code(code: &str) -> bool {
    !code.is_empty()
        && code
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn to_eastmoney_batch_secids(symbol: &str, market: &str) -> Result<Vec<String>, String> {
    match market {
        "CN" => {
            let normalized = symbol.trim().to_lowercase();
            let code = normalized
                .get(2..)
                .ok_or_else(|| format!("Invalid CN symbol: {}", symbol))?;
            if code.len() != 6 || !code.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(format!("Invalid CN symbol: {}", symbol));
            }
            Ok(vec![to_eastmoney_secid(&normalized)?])
        }
        "HK" => Ok(vec![to_eastmoney_hk_secid(symbol)?]),
        "US" => {
            let code = symbol.trim().to_uppercase().replace(['-', '.'], "_");
            if !is_safe_eastmoney_us_code(&code) {
                return Err(format!("Invalid US symbol: {}", symbol));
            }
            Ok([105, 106, 107, 153]
                .into_iter()
                .map(|market_id| format!("{}.{}", market_id, code))
                .collect())
        }
        _ => Err(format!("Unknown market: {}", market)),
    }
}

/// Normalize symbols for EastMoney's multi-symbol quote endpoint and split
/// them by expanded `secid` count. Symbols which cannot be represented are
/// returned for the existing per-symbol fallback path.
pub(super) fn plan_eastmoney_quote_batches(
    symbols: &[(String, String)],
) -> (Vec<Vec<EastMoneyBatchRequestSymbol>>, Vec<(String, String)>) {
    let mut planned: Vec<EastMoneyBatchRequestSymbol> = Vec::new();
    let mut index_by_secids: std::collections::HashMap<(String, Vec<String>), usize> =
        std::collections::HashMap::new();
    let mut invalid = Vec::new();

    for (symbol, market) in symbols {
        let api_secids = match to_eastmoney_batch_secids(symbol, market) {
            Ok(secids) => secids,
            Err(_) => {
                invalid.push((symbol.clone(), market.clone()));
                continue;
            }
        };
        let key = (market.trim().to_ascii_uppercase(), api_secids.clone());
        if let Some(existing_index) = index_by_secids.get(&key).copied() {
            planned[existing_index]
                .aliases
                .push((symbol.clone(), market.clone()));
            continue;
        }
        index_by_secids.insert(key, planned.len());
        planned.push(EastMoneyBatchRequestSymbol {
            original_symbol: symbol.clone(),
            market: market.clone(),
            api_secids,
            aliases: Vec::new(),
        });
    }

    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_secid_count = 0;
    for request_symbol in planned {
        if !current_batch.is_empty()
            && current_secid_count + request_symbol.api_secids.len()
                > EASTMONEY_QUOTE_BATCH_MAX_SECIDS
        {
            batches.push(std::mem::take(&mut current_batch));
            current_secid_count = 0;
        }
        current_secid_count += request_symbol.api_secids.len();
        current_batch.push(request_symbol);
    }
    if !current_batch.is_empty() {
        batches.push(current_batch);
    }
    (batches, invalid)
}

/// Build a one-shot JSON batch quote URL. `fltt=2` asks EastMoney to return
/// decoded decimal values, so prices and percentages require no field-specific
/// scaling in the parser.
pub(super) fn build_eastmoney_batch_url(request_symbols: &[EastMoneyBatchRequestSymbol]) -> String {
    let secids = request_symbols
        .iter()
        .flat_map(|symbol| symbol.api_secids.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "https://push2delay.eastmoney.com/api/qt/ulist.np/get?\
         secids={secids}&\
         fields=f2,f3,f4,f5,f8,f12,f13,f14,f15,f16,f18,f20,f23,f115&\
         fltt=2&invt=3&ut=fa5fd1943c7b386f172d6893dbfba10b"
    )
}

/// Fetch one planned batch from EastMoney's one-shot multi-symbol endpoint.
pub(super) async fn fetch_eastmoney_quotes_batch(
    request_symbols: &[EastMoneyBatchRequestSymbol],
) -> Result<Vec<StockQuote>, String> {
    if request_symbols.is_empty() {
        return Ok(Vec::new());
    }
    let url = build_eastmoney_batch_url(request_symbols);
    let response = send_eastmoney_request(&url, "realtime batch").await?;
    if !response.status().is_success() {
        let status = response.status();
        let body_preview = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(EASTMONEY_RESPONSE_PREVIEW_LEN)
            .collect::<String>();
        return Err(format!(
            "East Money batch API error: HTTP {}. Response: {}",
            status, body_preview
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read East Money batch response body: {}", e))?;
    parse_eastmoney_batch_body(&body, request_symbols)
}

/// Parse the East Money JSON response into a `StockQuote`.
pub(super) fn parse_eastmoney_quote(
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
// Historical price fetching
// ---------------------------------------------------------------------------

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
