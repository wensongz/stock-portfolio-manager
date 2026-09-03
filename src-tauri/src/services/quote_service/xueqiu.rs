use super::timestamp_to_market_date;
use crate::models::{PriceCandle, StockQuote};
use crate::services::http_client;
use chrono::Utc;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Xueqiu (雪球) API
// ---------------------------------------------------------------------------

/// Runtime-owned Xueqiu credentials and session token.
///
/// Tauri manages one instance for the application. Tests can create isolated
/// instances, and quote requests no longer open a second SQLite connection or
/// rely on process-wide mutable statics.
pub struct QuoteServiceState {
    token_initialized: AtomicBool,
    user_cookie: Mutex<Option<String>>,
    user_u: Mutex<Option<String>>,
    auto_cookie: Mutex<Option<String>>,
}

impl QuoteServiceState {
    pub fn new() -> Self {
        Self {
            token_initialized: AtomicBool::new(false),
            user_cookie: Mutex::new(None),
            user_u: Mutex::new(None),
            auto_cookie: Mutex::new(None),
        }
    }
}

impl Default for QuoteServiceState {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) const XUEQIU_COOKIE_EXPIRED_HINT: &str =
    "雪球 Cookie 可能已经过期，请到设置页面更新雪球 Cookie。";
pub(super) const XUEQIU_API_FAILED_HINT: &str = "访问雪球行情服务失败，请检查网络连接或稍后重试。";

pub(super) fn is_xueqiu_cookie_expired_error(err: &str) -> bool {
    err.contains("Xueqiu API error")
        && (err.contains("400016")
            || err.contains("重新登录帐号后再试")
            || err.contains("刷新页面或者重新登录帐号后再试"))
}

pub(super) fn is_xueqiu_request_error(err: &str) -> bool {
    err.contains("Xueqiu") || err.contains("xueqiu.com") || err.contains("stock.xueqiu.com")
}

pub(super) fn quote_warning_for_error(err: &str) -> Option<String> {
    if is_xueqiu_cookie_expired_error(err) {
        Some(XUEQIU_COOKIE_EXPIRED_HINT.to_string())
    } else if is_xueqiu_request_error(err) {
        Some(XUEQIU_API_FAILED_HINT.to_string())
    } else {
        None
    }
}

/// Set (or clear) the user-provided Xueqiu cookie string.
pub fn set_xueqiu_user_cookie(state: &QuoteServiceState, cookie: Option<String>) {
    let cookie = cookie
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    *state.user_cookie.lock().unwrap() = cookie;
}

/// Return a clone of the current user-provided Xueqiu cookie, if any.
fn get_xueqiu_user_cookie(state: &QuoteServiceState) -> Option<String> {
    state.user_cookie.lock().unwrap().clone()
}

/// Set (or clear) the user-provided Xueqiu `u` cookie value.
pub fn set_xueqiu_user_u(state: &QuoteServiceState, u_value: Option<String>) {
    let u_value = u_value
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    *state.user_u.lock().unwrap() = u_value;
}

/// Return a clone of the current user-provided Xueqiu `u` cookie value, if any.
fn get_xueqiu_user_u(state: &QuoteServiceState) -> Option<String> {
    state.user_u.lock().unwrap().clone()
}

/// Ensure the Xueqiu HTTP client has a valid session token.
///
/// Xueqiu requires an `xq_a_token` cookie which is set when visiting the
/// homepage.  This function visits `https://xueqiu.com` once to acquire the
/// cookie, and remembers the result in [`QuoteServiceState`].
///
/// The homepage request uses browser page-load headers (`Accept: text/html`)
/// rather than API-style headers to ensure the server returns a full page
/// response that sets the session cookie.
///
/// If a user-provided cookie is configured, the homepage visit is skipped
/// entirely because authentication is handled via the explicit `Cookie` header
/// added in [`send_xueqiu_request`].
async fn ensure_xueqiu_token(state: &QuoteServiceState) -> Result<(), String> {
    if state.token_initialized.load(Ordering::SeqCst) {
        return Ok(());
    }

    // If a user-provided cookie is configured, skip the homepage visit
    // entirely – authentication is handled via the explicit Cookie header
    // built in build_xueqiu_cookie_header().
    if get_xueqiu_user_cookie(state).is_some() {
        state.token_initialized.store(true, Ordering::SeqCst);
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
    // explicitly to API requests through the managed runtime state.
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
        *state.auto_cookie.lock().unwrap() = Some(token.clone());
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
        state.token_initialized.store(true, Ordering::SeqCst);
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
pub fn reset_xueqiu_token(state: &QuoteServiceState) {
    state.token_initialized.store(false, Ordering::SeqCst);
    *state.auto_cookie.lock().unwrap() = None;
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
pub(super) fn build_xueqiu_cookie_header(state: &QuoteServiceState) -> Option<String> {
    let user_cookie = get_xueqiu_user_cookie(state);
    let auto_token = state.auto_cookie.lock().unwrap().clone();
    let u_value = get_xueqiu_user_u(state);

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
async fn send_xueqiu_request(
    state: &QuoteServiceState,
    url: &str,
    symbol: &str,
) -> Result<reqwest::Response, String> {
    ensure_xueqiu_token(state).await?;

    let client = http_client::xueqiu_client();
    let mut last_err = String::new();

    for attempt in 0..=XUEQIU_MAX_RETRIES {
        let mut req = client.get(url);

        if let Some(cookie) = build_xueqiu_cookie_header(state) {
            req = req.header(reqwest::header::COOKIE, cookie);
        }

        let result = req.send().await;
        match result {
            Ok(resp)
                if resp.status() == reqwest::StatusCode::BAD_REQUEST
                    && attempt < XUEQIU_MAX_RETRIES =>
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
                reset_xueqiu_token(state);
                ensure_xueqiu_token(state).await?;
            }
            Ok(resp) => return Ok(resp),
            Err(e) => {
                last_err = format!("Network error fetching {} from Xueqiu: {}", symbol, e);
                if attempt < XUEQIU_MAX_RETRIES {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    reset_xueqiu_token(state);
                    ensure_xueqiu_token(state).await?;
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
pub async fn xueqiu_fetch(
    state: &QuoteServiceState,
    url: &str,
) -> Result<reqwest::Response, String> {
    send_xueqiu_request(state, url, "lookup").await
}

/// Maximum number of characters to include in error messages as a response
/// body preview for debugging failed Xueqiu API responses.
const XUEQIU_RESPONSE_PREVIEW_LEN: usize = 200;

/// Xueqiu API response wrapper.
#[derive(Debug, Deserialize)]
pub(super) struct XueqiuResponse {
    pub(super) data: Option<XueqiuData>,
    pub(super) error_code: Option<i32>,
    pub(super) error_description: Option<String>,
}

/// Inner data of a Xueqiu quote response.
#[derive(Debug, Deserialize)]
pub(super) struct XueqiuData {
    pub(super) quote: Option<XueqiuQuote>,
}

/// Xueqiu quote fields.
#[derive(Debug, Deserialize, Default)]
pub(super) struct XueqiuQuote {
    /// Stock name (e.g. "贵州茅台", "Apple Inc.")
    pub(super) name: Option<String>,
    /// Current price
    pub(super) current: Option<f64>,
    /// Previous close
    pub(super) last_close: Option<f64>,
    /// Price change
    pub(super) chg: Option<f64>,
    /// Change percentage
    pub(super) percent: Option<f64>,
    /// Day high
    pub(super) high: Option<f64>,
    /// Day low
    pub(super) low: Option<f64>,
    /// Volume
    pub(super) volume: Option<f64>,
    // ── Fundamentals (Xueqiu returns these in the same quote payload) ──
    /// P/E ratio (TTM)
    pub(super) pe_ttm: Option<f64>,
    /// P/B ratio
    pub(super) pb: Option<f64>,
    /// Total market capitalisation
    pub(super) market_capital: Option<f64>,
    /// Dividend yield (fraction, e.g. 0.025 = 2.5%)
    pub(super) dividend_yield: Option<f64>,
    /// Earnings per share
    pub(super) eps: Option<f64>,
    /// Turnover rate (percent)
    pub(super) turnover_rate: Option<f64>,
}

/// Xueqiu kline (historical candlestick) API response wrapper.
#[derive(Debug, Deserialize)]
pub(super) struct XueqiuKlineResponse {
    pub(super) data: Option<XueqiuKlineData>,
    pub(super) error_code: Option<i32>,
    pub(super) error_description: Option<String>,
}

/// Inner data of a Xueqiu kline response.
#[derive(Debug, Deserialize)]
pub(super) struct XueqiuKlineData {
    /// Column names, e.g. ["timestamp", "volume", "open", "high", "low", "close", ...]
    pub(super) column: Option<Vec<String>>,
    /// Each item is one trading day: values in the same order as `column`.
    pub(super) item: Option<Vec<Vec<serde_json::Value>>>,
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
    let explicit_successful_empty = data.column.as_ref().is_some_and(Vec::is_empty)
        && data.item.as_ref().is_some_and(Vec::is_empty);
    if explicit_successful_empty {
        return Ok(XueqiuHistoryOutcome::Empty);
    }
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
pub(super) fn parse_xueqiu_body(body: &str, symbol: &str) -> Result<XueqiuResponse, String> {
    serde_json::from_str(body).map_err(|e| {
        let preview: String = body.chars().take(XUEQIU_RESPONSE_PREVIEW_LEN).collect();
        format!(
            "Failed to parse Xueqiu response for {}: {}. Response preview: {}",
            symbol, e, preview
        )
    })
}

/// Parse the Xueqiu API response into a `StockQuote`.
pub(super) fn parse_xueqiu_quote(
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
pub(super) fn to_xueqiu_cn_symbol(symbol: &str) -> Result<String, String> {
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
pub(super) fn to_xueqiu_us_symbol(symbol: &str) -> String {
    symbol.to_uppercase().replace('-', ".")
}

/// Convert a HK stock symbol to Xueqiu format.
/// Strips the ".HK" suffix if present and zero-pads to 5 digits.
pub(super) fn to_xueqiu_hk_symbol(symbol: &str) -> Result<String, String> {
    let code = symbol.trim_end_matches(".HK").trim_end_matches(".hk");
    if code.chars().all(|c| c.is_ascii_digit()) {
        let padded = format!("{:0>5}", code);
        Ok(padded)
    } else {
        Err(format!("Invalid HK symbol for Xueqiu: {}", symbol))
    }
}

/// Fetch a CN A-share stock quote from Xueqiu (雪球).
pub(super) async fn fetch_xueqiu_cn_quote(
    state: &QuoteServiceState,
    symbol: &str,
) -> Result<StockQuote, String> {
    let xueqiu_symbol = to_xueqiu_cn_symbol(symbol)?;
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xueqiu_symbol
    );

    let response = send_xueqiu_request(state, &url, symbol).await?;

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
pub(super) async fn fetch_xueqiu_us_quote(
    state: &QuoteServiceState,
    symbol: &str,
) -> Result<StockQuote, String> {
    let xueqiu_symbol = to_xueqiu_us_symbol(symbol);
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xueqiu_symbol
    );

    let response = send_xueqiu_request(state, &url, symbol).await?;

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
pub(super) async fn fetch_xueqiu_hk_quote(
    state: &QuoteServiceState,
    symbol: &str,
) -> Result<StockQuote, String> {
    let xueqiu_symbol = to_xueqiu_hk_symbol(symbol)?;
    let url = format!(
        "https://stock.xueqiu.com/v5/stock/quote.json?symbol={}&extend=detail",
        xueqiu_symbol
    );

    let response = send_xueqiu_request(state, &url, symbol).await?;

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

/// Fetch historical daily OHLCV candles from Xueqiu.
///
/// Mirrors [`fetch_stock_history_xueqiu`] but retains open/high/low/close/volume
/// for technical-analysis indicators.
pub async fn fetch_candles_xueqiu(
    state: &QuoteServiceState,
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

    let resp = send_xueqiu_request(state, &url, symbol).await?;
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

/// Uses the Xueqiu kline API (`/v5/stock/chart/kline.json`).
/// Returns a list of (date, close_price) pairs sorted by date ascending.
#[allow(dead_code)] // Retained as the provider-specific API for callers that do not want fallback.
pub async fn fetch_stock_history_xueqiu(
    state: &QuoteServiceState,
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    match fetch_stock_history_xueqiu_outcome(state, symbol, market, start_date, end_date).await? {
        XueqiuHistoryOutcome::Prices(prices) => Ok(prices),
        XueqiuHistoryOutcome::StartsAfterRange { .. } | XueqiuHistoryOutcome::Empty => {
            Ok(Vec::new())
        }
    }
}

pub(super) async fn fetch_stock_history_xueqiu_outcome(
    state: &QuoteServiceState,
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

    fetch_history_xueqiu_api_symbol(state, &xueqiu_symbol, symbol, market, start_date, end_date)
        .await
}

#[allow(dead_code)] // Called by the Task 2 live calendar adapter, wired by Task 3.
pub(crate) async fn fetch_index_history_xueqiu(
    state: &QuoteServiceState,
    api_symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<XueqiuHistoryOutcome, String> {
    fetch_history_xueqiu_api_symbol(state, api_symbol, api_symbol, market, start_date, end_date)
        .await
}

pub(super) fn xueqiu_history_request_count(
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> i64 {
    (end_date - start_date).num_days().saturating_add(1).max(2)
}

async fn fetch_history_xueqiu_api_symbol(
    state: &QuoteServiceState,
    api_symbol: &str,
    display_symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<XueqiuHistoryOutcome, String> {
    // Xueqiu returns trading days going backwards from begin.
    // Calendar days in range is a safe upper bound (there are fewer
    // trading days than calendar days), with a minimum of 2.
    let count = xueqiu_history_request_count(start_date, end_date);

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
        api_symbol, begin_ts, count
    );

    let response = send_xueqiu_request(state, &url, display_symbol).await?;

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
            status, display_symbol, body_preview
        ));
    }

    let body = response.text().await.map_err(|e| {
        format!(
            "fetch_stock_history_xueqiu: read error for {}: {}",
            display_symbol, e
        )
    })?;

    parse_xueqiu_history_response(&body, display_symbol, market, start_date, end_date, &url)
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
