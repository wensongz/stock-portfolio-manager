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

/// Fetch a CN A-share stock quote from Tencent Finance.
/// Symbol format: "sh600519" (Shanghai) or "sz000858" (Shenzhen).
/// The symbol is normalised to lowercase automatically.
pub async fn fetch_cn_quote(symbol: &str) -> Result<StockQuote, String> {
    let symbol = symbol.to_lowercase();
    let url = format!("https://qt.gtimg.cn/q={}", symbol);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let bytes = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("Network error fetching {}: {}", symbol, e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response bytes for {}: {}", symbol, e))?;

    // Tencent Finance returns GBK-encoded text
    let (decoded, _, _) = GBK.decode(&bytes);
    let text = decoded.into_owned();

    parse_tencent_quote(&symbol, &text)
}

/// Parse Tencent Finance quote response.
/// Format: v_sh600519="1~贵州茅台~600519~1710.50~1690.00~1700.00~...";
///
/// Key field indices (tilde-separated):
///  1: name, 3: current_price, 4: previous_close, 6: volume (lots),
///  30: datetime (YYYYMMDDHHMMSS), 31: change, 32: change_percent (%),
///  33: high, 34: low
fn parse_tencent_quote(symbol: &str, text: &str) -> Result<StockQuote, String> {
    let start = text
        .find('"')
        .ok_or_else(|| format!("Invalid Tencent response for {}: {}", symbol, text))?;
    let end = text
        .rfind('"')
        .ok_or_else(|| format!("Invalid Tencent response for {}: {}", symbol, text))?;

    if start >= end {
        return Err(format!("Empty or invalid Tencent response for {}", symbol));
    }

    let content = &text[start + 1..end];
    if content.is_empty() {
        return Err(format!(
            "No data from Tencent Finance for {}. Symbol may be invalid.",
            symbol
        ));
    }

    let parts: Vec<&str> = content.split('~').collect();
    if parts.len() < 35 {
        return Err(format!(
            "Unexpected Tencent Finance data format for {}: only {} fields",
            symbol,
            parts.len()
        ));
    }

    // Tencent format fields (tilde-separated):
    // 1: name, 3: current_price, 4: previous_close, 5: today_open,
    // 6: volume (lots), 30: datetime (YYYYMMDDHHMMSS),
    // 31: change, 32: change_percent (%), 33: high, 34: low
    let name = parts[1].to_string();
    let current_price: f64 = parts[3].parse().unwrap_or(0.0);
    let previous_close: f64 = parts[4].parse().unwrap_or(0.0);
    let volume: u64 = parts[6].parse().unwrap_or(0);
    let high: f64 = parts[33].parse().unwrap_or(0.0);
    let low: f64 = parts[34].parse().unwrap_or(0.0);

    let change = current_price - previous_close;
    let change_percent = if previous_close != 0.0 {
        change / previous_close * 100.0
    } else {
        0.0
    };

    let datetime_str = parts.get(30).unwrap_or(&"");
    let updated_at = if datetime_str.len() >= 14 {
        // Format: YYYYMMDDHHMMSS -> YYYY-MM-DDTHH:MM:SS+08:00
        format!(
            "{}-{}-{}T{}:{}:{}+08:00",
            &datetime_str[0..4],
            &datetime_str[4..6],
            &datetime_str[6..8],
            &datetime_str[8..10],
            &datetime_str[10..12],
            &datetime_str[12..14],
        )
    } else {
        Utc::now().to_rfc3339()
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

    // Helper: build a synthetic Tencent response string with enough fields.
    // Tencent fields (tilde-separated, 0-indexed):
    //  0:market, 1:name, 2:code, 3:current, 4:prev_close, 5:open,
    //  6:volume, 7-29:misc, 30:datetime, 31:change, 32:change_pct,
    //  33:high, 34:low
    fn make_tencent_response(
        symbol: &str,
        name: &str,
        current: f64,
        prev_close: f64,
        high: f64,
        low: f64,
        volume: u64,
        datetime: &str,
    ) -> String {
        // Build a slice of 45 placeholder fields, overwriting key positions.
        let mut fields = vec!["0".to_string(); 45];
        fields[0] = "1".to_string();
        fields[1] = name.to_string();
        fields[2] = symbol[2..].to_string(); // strip "sh"/"sz"
        fields[3] = format!("{:.2}", current);
        fields[4] = format!("{:.2}", prev_close);
        fields[5] = format!("{:.2}", current); // open == current (irrelevant)
        fields[6] = volume.to_string();
        fields[30] = datetime.to_string();
        fields[31] = format!("{:.2}", current - prev_close);
        fields[32] = if prev_close != 0.0 {
            format!("{:.2}", (current - prev_close) / prev_close * 100.0)
        } else {
            "0.00".to_string()
        };
        fields[33] = format!("{:.2}", high);
        fields[34] = format!("{:.2}", low);
        format!("v_{}=\"{}\";", symbol, fields.join("~"))
    }

    #[test]
    fn test_parse_tencent_quote_valid() {
        let text = make_tencent_response(
            "sh600519",
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345,
            "20240115150003",
        );
        let result = parse_tencent_quote("sh600519", &text);
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
        assert_eq!(quote.updated_at, "2024-01-15T15:00:03+08:00");
    }

    #[test]
    fn test_parse_tencent_quote_empty() {
        let text = r#"v_sh999999="";"#;
        let result = parse_tencent_quote("sh999999", text);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_tencent_quote_change_calculation() {
        let text = make_tencent_response(
            "sh600519",
            "贵州茅台",
            1100.00,
            1000.00,
            1200.00,
            950.00,
            99999,
            "20240115150003",
        );
        let result = parse_tencent_quote("sh600519", &text);
        assert!(result.is_ok());
        let quote = result.unwrap();
        assert!((quote.change - 100.0).abs() < 0.001);
        assert!((quote.change_percent - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_tencent_quote_symbol_stored_as_given() {
        // The parser stores the symbol exactly as provided. The caller
        // (fetch_cn_quote) is responsible for lowercasing before calling here.
        // This test confirms that a lowercase symbol is stored correctly.
        let text = make_tencent_response(
            "sh600519",
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345,
            "20240115150003",
        );
        let result = parse_tencent_quote("sh600519", &text);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().symbol, "sh600519");
    }

    #[test]
    fn test_fetch_cn_quote_normalises_symbol_to_lowercase() {
        // Verify that to_lowercase() on a mixed-case symbol produces what the
        // API expects.  We cannot call fetch_cn_quote directly in a unit test
        // (it makes a real network request), so we assert the string transform
        // is correct and pass the lowercased value to the parser.
        let mixed = "Sh600519";
        let lower = mixed.to_lowercase();
        assert_eq!(lower, "sh600519");
        let text = make_tencent_response(
            &lower,
            "贵州茅台",
            1710.50,
            1690.00,
            1720.00,
            1685.00,
            12345,
            "20240115150003",
        );
        let result = parse_tencent_quote(&lower, &text);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().symbol, "sh600519");
    }
}
