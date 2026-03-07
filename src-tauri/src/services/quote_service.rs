use crate::models::StockQuote;
use chrono::Utc;
use encoding_rs::GBK;
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

/// Fetch a CN A-share stock quote from Sina Finance.
/// Symbol format: "sh600519" (Shanghai) or "sz000858" (Shenzhen).
pub async fn fetch_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    let url = format!("https://hq.sinajs.cn/list={}", symbol);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let bytes = client
        .get(&url)
        .header("Referer", "https://finance.sina.com.cn")
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("Network error fetching {}: {}", symbol, e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response bytes for {}: {}", symbol, e))?;

    // Sina Finance returns GBK-encoded text
    let (decoded, _, _) = GBK.decode(&bytes);
    let text = decoded.into_owned();

    parse_sina_quote(symbol, &text)
}

/// Parse Sina Finance quote response.
/// Format: var hq_str_sh600519="贵州茅台,1700.00,1690.00,...,2024-01-15,15:00:00,...";
fn parse_sina_quote(symbol: &str, text: &str) -> Result<StockQuote, String> {
    let start = text
        .find('"')
        .ok_or_else(|| format!("Invalid Sina response for {}: {}", symbol, text))?;
    let end = text
        .rfind('"')
        .ok_or_else(|| format!("Invalid Sina response for {}: {}", symbol, text))?;

    if start >= end {
        return Err(format!("Empty or invalid Sina response for {}", symbol));
    }

    let content = &text[start + 1..end];
    if content.is_empty() {
        return Err(format!(
            "No data from Sina Finance for {}. Symbol may be invalid.",
            symbol
        ));
    }

    let parts: Vec<&str> = content.split(',').collect();
    if parts.len() < 32 {
        return Err(format!(
            "Unexpected Sina Finance data format for {}: only {} fields",
            symbol,
            parts.len()
        ));
    }

    // Sina format fields:
    // 0: name, 1: today_open, 2: yesterday_close, 3: current_price,
    // 4: high, 5: low, 6: bid, 7: ask, 8: volume (lots), 9: amount,
    // 10-19: bid/ask levels, ...
    // 31: date, 32: time
    let name = parts[0].to_string();
    let previous_close: f64 = parts[2].parse().unwrap_or(0.0);
    let current_price: f64 = parts[3].parse().unwrap_or(0.0);
    let high: f64 = parts[4].parse().unwrap_or(0.0);
    let low: f64 = parts[5].parse().unwrap_or(0.0);
    // Volume in Sina is in lots (手, 100 shares each)
    let volume: u64 = parts[8].parse().unwrap_or(0);

    let change = current_price - previous_close;
    let change_percent = if previous_close != 0.0 {
        change / previous_close * 100.0
    } else {
        0.0
    };

    let date_str = parts.get(30).unwrap_or(&"");
    let time_str = parts.get(31).unwrap_or(&"");
    let updated_at = if date_str.is_empty() || *date_str == "0000-00-00" {
        Utc::now().to_rfc3339()
    } else {
        format!("{}T{}+08:00", date_str, time_str)
    };

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
        updated_at,
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

    #[test]
    fn test_parse_sina_quote_valid() {
        // Simulated Sina response for sh600519 (Kweichow Moutai)
        let text = r#"var hq_str_sh600519="贵州茅台,1700.00,1690.00,1710.50,1720.00,1685.00,1710.00,1711.00,12345678,21000000000.00,100,1710.00,200,1710.50,300,1711.00,400,1711.50,500,1712.00,600,1709.50,700,1709.00,800,1708.50,900,1708.00,1000,1707.50,2024-01-15,15:00:00,00,贵州茅台,D,D";"#;
        let result = parse_sina_quote("sh600519", text);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
        let quote = result.unwrap();
        assert_eq!(quote.symbol, "sh600519");
        assert_eq!(quote.name, "贵州茅台");
        assert_eq!(quote.market, "CN");
        assert!((quote.current_price - 1710.50).abs() < 0.001);
        assert!((quote.previous_close - 1690.00).abs() < 0.001);
        assert!((quote.high - 1720.00).abs() < 0.001);
        assert!((quote.low - 1685.00).abs() < 0.001);
    }

    #[test]
    fn test_parse_sina_quote_empty() {
        let text = r#"var hq_str_sh999999="";"#;
        let result = parse_sina_quote("sh999999", text);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_sina_quote_change_calculation() {
        let text = r#"var hq_str_sh600519="贵州茅台,1700.00,1000.00,1100.00,1200.00,950.00,1100.00,1101.00,12345678,21000000000.00,100,1100.00,200,1100.50,300,1101.00,400,1101.50,500,1102.00,600,1099.50,700,1099.00,800,1098.50,900,1098.00,1000,1097.50,2024-01-15,15:00:00,00,贵州茅台,D,D";"#;
        let result = parse_sina_quote("sh600519", text);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
    }
}
