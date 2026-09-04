use super::timestamp_to_market_date;
use crate::models::StockQuote;
use crate::services::http_client;
use chrono::Utc;
use serde::Deserialize;

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
    regular_market_change_percent: Option<f64>,
    regular_market_volume: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct YahooIndicators {
    quote: Option<Vec<YahooQuoteIndicator>>,
}

#[derive(Debug, Deserialize)]
struct YahooQuoteIndicator {
    volume: Option<Vec<Option<u64>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct YahooBatchRequestSymbol {
    pub(super) original_symbol: String,
    pub(super) market: String,
    pub(super) api_symbol: String,
    pub(super) aliases: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
struct YahooSparkResponse {
    spark: YahooSpark,
}

#[derive(Debug, Deserialize)]
struct YahooSpark {
    result: Option<Vec<YahooSparkResult>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct YahooSparkResult {
    symbol: String,
    response: Option<Vec<YahooSparkChart>>,
}

#[derive(Debug, Deserialize)]
struct YahooSparkChart {
    meta: YahooMeta,
}

const YAHOO_QUOTE_BATCH_SIZE: usize = 20;
const YAHOO_US_SYMBOL_MAX_LEN: usize = 32;

fn is_safe_yahoo_us_symbol(symbol: &str) -> bool {
    if symbol.is_empty() || symbol.len() > YAHOO_US_SYMBOL_MAX_LEN {
        return false;
    }
    let body = symbol.strip_prefix('^').unwrap_or(symbol);
    if body.is_empty()
        || !body
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
        || !body
            .chars()
            .last()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
    {
        return false;
    }
    let mut previous_was_separator = false;
    for ch in body.chars() {
        if ch.is_ascii_alphanumeric() {
            previous_was_separator = false;
        } else if matches!(ch, '-' | '=') && !previous_was_separator {
            previous_was_separator = true;
        } else {
            return false;
        }
    }
    true
}

fn to_yahoo_batch_symbol(symbol: &str, market: &str) -> Result<String, String> {
    if !matches!(market, "US" | "HK") {
        return Err(format!("Unsupported Yahoo quote market: {}", market));
    }
    if market == "HK" {
        let normalized = symbol.trim().to_ascii_uppercase();
        let code = normalized
            .strip_suffix(".HK")
            .unwrap_or(normalized.as_str());
        if code.is_empty() || code.len() > 5 || !code.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(format!("Invalid Yahoo HK quote symbol: {}", symbol));
        }
    }
    let api_symbol = to_yahoo_symbol(symbol, market);
    if market == "US" && !is_safe_yahoo_us_symbol(&api_symbol) {
        return Err(format!("Invalid Yahoo quote symbol: {}", symbol));
    }
    Ok(api_symbol)
}

pub(super) fn plan_yahoo_quote_batches(
    symbols: &[(String, String)],
) -> (Vec<Vec<YahooBatchRequestSymbol>>, Vec<(String, String)>) {
    let mut planned: Vec<YahooBatchRequestSymbol> = Vec::new();
    let mut index_by_api_symbol: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut invalid = Vec::new();

    for (symbol, market) in symbols {
        let api_symbol = match to_yahoo_batch_symbol(symbol, market) {
            Ok(api_symbol) => api_symbol,
            Err(_) => {
                invalid.push((symbol.clone(), market.clone()));
                continue;
            }
        };
        let key = api_symbol.to_ascii_uppercase();
        if let Some(existing_index) = index_by_api_symbol.get(&key).copied() {
            planned[existing_index]
                .aliases
                .push((symbol.clone(), market.clone()));
        } else {
            index_by_api_symbol.insert(key, planned.len());
            planned.push(YahooBatchRequestSymbol {
                original_symbol: symbol.clone(),
                market: market.clone(),
                api_symbol,
                aliases: Vec::new(),
            });
        }
    }

    let batches = planned
        .chunks(YAHOO_QUOTE_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect();
    (batches, invalid)
}

pub(super) fn build_yahoo_spark_url(request_symbols: &[YahooBatchRequestSymbol]) -> String {
    let symbols = request_symbols
        .iter()
        .map(|symbol| symbol.api_symbol.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut url = url::Url::parse("https://query1.finance.yahoo.com/v7/finance/spark")
        .expect("Yahoo spark URL is valid");
    url.query_pairs_mut()
        .append_pair("symbols", &symbols)
        .append_pair("range", "1d")
        .append_pair("interval", "1d");
    url.into()
}

pub(super) fn parse_yahoo_spark_body(
    body: &str,
    request_symbols: &[YahooBatchRequestSymbol],
) -> Result<Vec<StockQuote>, String> {
    let response: YahooSparkResponse = serde_json::from_str(body).map_err(|error| {
        let preview: String = body.chars().take(200).collect();
        format!(
            "Failed to parse Yahoo spark response: {}. Response preview: {}",
            error, preview
        )
    })?;
    if let Some(error) = response.spark.error {
        let message = error
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        return Err(format!("Yahoo spark API returned error: {}", message));
    }

    let request_by_api_symbol: std::collections::HashMap<String, &YahooBatchRequestSymbol> =
        request_symbols
            .iter()
            .map(|symbol| (symbol.api_symbol.to_ascii_uppercase(), symbol))
            .collect();
    let mut seen = std::collections::HashSet::new();
    let mut quotes = Vec::new();

    for item in response.spark.result.unwrap_or_default() {
        let Some(request) = request_by_api_symbol.get(&item.symbol.to_ascii_uppercase()) else {
            continue;
        };
        let Some(meta) = item
            .response
            .and_then(|items| items.into_iter().next())
            .map(|item| item.meta)
        else {
            continue;
        };
        let Some(current_price) = meta.regular_market_price else {
            continue;
        };
        let previous_close = meta
            .previous_close
            .or(meta.chart_previous_close)
            .unwrap_or(0.0);
        let change = current_price - previous_close;
        let change_percent = meta.regular_market_change_percent.unwrap_or_else(|| {
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
            if !seen.insert(original_symbol.clone()) {
                continue;
            }
            quotes.push(StockQuote {
                symbol: original_symbol.clone(),
                name: meta
                    .short_name
                    .as_deref()
                    .or(meta.long_name.as_deref())
                    .filter(|name| !name.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| original_symbol.clone()),
                market: market.clone(),
                current_price,
                previous_close,
                change,
                change_percent,
                high: meta.regular_market_day_high.unwrap_or(0.0),
                low: meta.regular_market_day_low.unwrap_or(0.0),
                volume: meta.regular_market_volume.unwrap_or(0.0) as i64,
                updated_at: Utc::now().to_rfc3339(),
                ..Default::default()
            });
        }
    }
    Ok(quotes)
}

/// Fetch one batch of at most 20 US or HK quotes from Yahoo's spark endpoint.
pub(super) async fn fetch_yahoo_quotes_batch(
    request_symbols: &[YahooBatchRequestSymbol],
) -> Result<Vec<StockQuote>, String> {
    if request_symbols.is_empty() {
        return Ok(Vec::new());
    }
    let url = build_yahoo_spark_url(request_symbols);
    let response = http_client::general_client()
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Network error fetching Yahoo quote batch: {}", error))?;
    if !response.status().is_success() {
        let status = response.status();
        let preview: String = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        return Err(format!(
            "Yahoo spark API error: HTTP {}. Response: {}",
            status, preview
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read Yahoo spark response body: {}", error))?;
    parse_yahoo_spark_body(&body, request_symbols)
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

/// Convert a holding symbol + market to a Yahoo Finance ticker for historical queries.
pub fn to_yahoo_symbol(symbol: &str, market: &str) -> String {
    match market {
        "US" => {
            // Yahoo Finance uses hyphens in US symbols (e.g., "BRK-B"), convert dots to hyphens.
            symbol.trim().to_ascii_uppercase().replace('.', "-")
        }
        "HK" => {
            let normalized = symbol.trim().to_ascii_uppercase();
            let code = normalized
                .strip_suffix(".HK")
                .unwrap_or(normalized.as_str());
            let normalized = code
                .parse::<u32>()
                .map(|number| format!("{:04}", number))
                .unwrap_or_else(|_| code.to_string());
            format!("{}.HK", normalized)
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
