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
