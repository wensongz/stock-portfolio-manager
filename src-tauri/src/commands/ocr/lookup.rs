use super::*;

// ---------------------------------------------------------------------------
// Xueqiu stock-name → A-share code lookup
// ---------------------------------------------------------------------------

/// Response structure returned by Xueqiu `query/v1/search/stock.json`.
/// Example: `{"stocks": [{"code": "SH600036", "name": "招商银行", ...}]}`
#[derive(Debug, Deserialize)]
struct XueqiuSearchResponse {
    stocks: Option<Vec<XueqiuSearchItem>>,
}

#[derive(Debug, Deserialize)]
struct XueqiuSearchItem {
    /// e.g. "SH600036", "SZ000001", "NYSE:CHA"
    code: Option<String>,
    /// Display name, e.g. "招商银行", "中国电信"
    name: Option<String>,
}

// ---------------------------------------------------------------------------
// East Money (东方财富) stock search — no authentication required
// ---------------------------------------------------------------------------

/// Public search token bundled with East Money's web search API. It is a
/// well-known static value used across East Money's own front-ends and does
/// not require any per-user authentication.
const EASTMONEY_SEARCH_TOKEN: &str = "D43BF722C8E33BDC906FB84D85E326E8";

/// Response returned by East Money `searchapi.eastmoney.com/api/suggest/get`.
///
/// All fields use PascalCase (matching the raw JSON), normalised to snake_case
/// via `#[serde(rename_all = "PascalCase")]`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EastmoneySearchResponse {
    quotation_code_table: Option<EastmoneySearchTable>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EastmoneySearchTable {
    data: Option<Vec<EastmoneySearchItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EastmoneySearchItem {
    /// Raw 6-digit code, e.g. "600519", "000001"
    code: Option<String>,
    /// Display name, e.g. "贵州茅台"
    name: Option<String>,
    /// Market number: "1" = Shanghai (SH), "0" = Shenzhen (SZ)
    mkt_num: Option<String>,
    /// Asset class, e.g. "AStock" for A-shares
    classify: Option<String>,
}

/// Build the East Money suggest API URL for the given query.
fn eastmoney_search_url(query: &str) -> String {
    format!(
        "https://searchapi.eastmoney.com/api/suggest/get?input={}&type=14&token={}",
        urlencoding::encode(query),
        EASTMONEY_SEARCH_TOKEN
    )
}

/// Map an East Money `MktNum` ("1"/"0") to the lowercase exchange prefix
/// used throughout the app ("sh"/"sz"). Returns `None` for non-A-share markets.
fn eastmoney_market_prefix(mkt_num: &str) -> Option<&'static str> {
    match mkt_num {
        "1" => Some("sh"),
        "0" => Some("sz"),
        _ => None,
    }
}

/// Query East Money to resolve a Chinese stock name to its A-share code.
///
/// Returns `Ok(Some("sh600519"))` on success, `Ok(None)` when no A-share
/// result is found, and `Err(…)` for network / API failures. The returned
/// code is lower-cased to match the format produced by the Xueqiu lookup.
async fn lookup_via_eastmoney(name: &str) -> Result<Option<String>, String> {
    let url = eastmoney_search_url(name);
    let resp = http_client::eastmoney_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("查询东方财富失败: {}", e))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: EastmoneySearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析东方财富响应失败: {}", e))?;

    let items = body
        .quotation_code_table
        .and_then(|t| t.data)
        .unwrap_or_default();

    for item in &items {
        // Only accept A-shares.
        if item.classify.as_deref() != Some("AStock") {
            continue;
        }
        let (Some(code), Some(mkt_num)) = (item.code.as_deref(), item.mkt_num.as_deref()) else {
            continue;
        };
        if code.is_empty() {
            continue;
        }
        if let Some(prefix) = eastmoney_market_prefix(mkt_num) {
            return Ok(Some(format!("{}{}", prefix, code)));
        }
    }

    Ok(None)
}

/// Query East Money to resolve a ticker symbol (e.g. "600519", "000001") to
/// its display name, preferring the first A-share result.
async fn lookup_name_via_eastmoney(symbol: &str) -> Result<Option<String>, String> {
    let url = eastmoney_search_url(symbol);
    let resp = http_client::eastmoney_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("查询东方财富失败: {}", e))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: EastmoneySearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析东方财富响应失败: {}", e))?;

    let items = body
        .quotation_code_table
        .and_then(|t| t.data)
        .unwrap_or_default();

    // Prefer an A-share result whose code exactly matches the symbol.
    for item in &items {
        if item.classify.as_deref() != Some("AStock") {
            continue;
        }
        if item.code.as_deref() == Some(symbol) {
            if let Some(name) = item.name.as_deref().filter(|n| !n.is_empty()) {
                return Ok(Some(name.to_string()));
            }
        }
    }

    // Fallback: first A-share result with any name.
    for item in &items {
        if item.classify.as_deref() != Some("AStock") {
            continue;
        }
        if let Some(name) = item.name.as_deref().filter(|n| !n.is_empty()) {
            return Ok(Some(name.to_string()));
        }
    }

    Ok(None)
}

/// Resolve a Chinese stock name to its A-share code.
///
/// Tries East Money first (no authentication, reliable), then falls back to
/// Xueqiu. Returns `Ok(Some("sh600519"))` on success, `Ok(None)` when no
/// A-share result is found, and `Err(…)` only when both backends fail with a
/// network / API error.
pub async fn lookup_cn_stock_code_with_state(
    quote_state: &quote_service::QuoteServiceState,
    name: String,
) -> Result<Option<String>, String> {
    // Primary: East Money (auth-free).
    if let Ok(Some(code)) = lookup_via_eastmoney(&name).await {
        return Ok(Some(code));
    }
    // Fallback: Xueqiu (requires session token).
    match lookup_via_xueqiu(quote_state, &name).await {
        Ok(r) => Ok(r),
        Err(e) => Err(format!("股票代码查询失败: {e}")),
    }
}

/// Xueqiu `query/v1/search/stock.json` lookup.
///
/// Uses the shared Xueqiu session (including `xq_a_token` cookie management)
/// from the quote service so that the search API is called with valid
/// authentication and returns real results.
async fn lookup_via_xueqiu(
    quote_state: &quote_service::QuoteServiceState,
    name: &str,
) -> Result<Option<String>, String> {
    let url = format!(
        "https://xueqiu.com/query/v1/search/stock.json?code={}",
        urlencoding::encode(name)
    );

    let resp = quote_service::xueqiu_fetch(quote_state, &url)
        .await
        .map_err(|e| format!("查询雪球失败: {}", e))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: XueqiuSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析雪球响应失败: {}", e))?;

    let items = body.stocks.unwrap_or_default();

    for item in &items {
        let code = match &item.code {
            Some(s) if !s.is_empty() => s.as_str(),
            _ => continue,
        };
        let is_cn = code.starts_with("SH") || code.starts_with("SZ");
        if is_cn && code.len() == 8 {
            return Ok(Some(code.to_lowercase()));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Ticker-symbol → stock-name lookup (US / HK / CN, for CSV import)
// ---------------------------------------------------------------------------

/// Resolve a ticker symbol (e.g. "AAPL", "00700", "600519") to its display
/// name (e.g. "苹果", "腾讯控股", "贵州茅台").
///
/// Tries East Money first (auth-free, covers A-shares well), then falls back
/// to Xueqiu (covers US/HK/CN). Returns `Ok(None)` when neither backend finds
/// a name, and `Err(…)` only when both fail with a network / API error.
pub async fn lookup_stock_name_by_symbol_with_state(
    quote_state: &quote_service::QuoteServiceState,
    symbol: String,
) -> Result<Option<String>, String> {
    // Primary: East Money (auth-free).
    if let Ok(Some(name)) = lookup_name_via_eastmoney(&symbol).await {
        return Ok(Some(name));
    }
    // Fallback: Xueqiu (covers US/HK/CN; requires session token).
    match lookup_name_via_xueqiu(quote_state, &symbol).await {
        Ok(r) => Ok(r),
        Err(e) => Err(format!("股票名称查询失败: {e}")),
    }
}

/// Xueqiu `query/v1/search/stock.json` symbol → name lookup.
///
/// Prefers the result whose code suffix exactly matches the requested symbol
/// (e.g. "NYSE:AAPL" → suffix "AAPL").  If no exact suffix match is found,
/// returns the first result's name as a best-effort fallback.
async fn lookup_name_via_xueqiu(
    quote_state: &quote_service::QuoteServiceState,
    symbol: &str,
) -> Result<Option<String>, String> {
    let url = format!(
        "https://xueqiu.com/query/v1/search/stock.json?code={}",
        urlencoding::encode(symbol)
    );

    let resp = quote_service::xueqiu_fetch(quote_state, &url)
        .await
        .map_err(|e| format!("查询雪球失败: {}", e))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let body: XueqiuSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析雪球响应失败: {}", e))?;

    let items = body.stocks.unwrap_or_default();

    // First pass: prefer an item whose code suffix exactly matches the symbol
    for item in &items {
        let code = match &item.code {
            Some(s) if !s.is_empty() => s.as_str(),
            _ => continue,
        };
        let suffix = if code.contains(':') {
            code.split(':').next_back().unwrap_or(code)
        } else {
            code
        };
        if suffix.eq_ignore_ascii_case(symbol) {
            if let Some(name) = &item.name {
                if !name.is_empty() {
                    return Ok(Some(name.clone()));
                }
            }
        }
    }

    // Second pass: return the first result with any name
    for item in &items {
        if let Some(name) = &item.name {
            if !name.is_empty() {
                return Ok(Some(name.clone()));
            }
        }
    }

    Ok(None)
}
