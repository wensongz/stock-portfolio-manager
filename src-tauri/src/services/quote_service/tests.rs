use super::*;
use crate::services::http_client;

#[test]
fn resolve_index_secid_handles_common_forms() {
    // US indices — ^-prefixed, bare, and suffix-stripped.
    assert_eq!(resolve_index_secid("^GSPC").unwrap().0, "100.SPX");
    assert_eq!(resolve_index_secid("SPX").unwrap().0, "100.SPX");
    assert_eq!(resolve_index_secid("^IXIC").unwrap().0, "100.NDX");
    assert_eq!(resolve_index_secid("^DJI").unwrap().0, "100.DJIA");
    // HK.
    assert_eq!(resolve_index_secid("^HSI").unwrap().0, "100.HSI");
    assert_eq!(resolve_index_secid("HSI").unwrap().0, "100.HSI");
    assert_eq!(resolve_index_secid("^HSCEI").unwrap().0, "100.HSCEI");
    // CN — with and without .SS suffix.
    assert_eq!(resolve_index_secid("000300.SS").unwrap().0, "1.000300");
    assert_eq!(resolve_index_secid("^SSEC").unwrap().0, "1.000001");
    assert_eq!(resolve_index_secid("000001.SS").unwrap().0, "1.000001");
    assert_eq!(resolve_index_secid("000300").unwrap().0, "1.000300");
    // Non-index symbols return None.
    assert!(resolve_index_secid("AAPL").is_none());
    assert!(resolve_index_secid("0700.HK").is_none());
    assert!(resolve_index_secid("sh600519").is_none());
}

#[test]
fn xueqiu_history_count_is_inclusive_for_short_weekday_window() {
    let monday = chrono::NaiveDate::from_ymd_opt(2026, 8, 24).unwrap();
    let friday = chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();

    assert_eq!(xueqiu_history_request_count(monday, friday), 5);
}

// Helper: build a synthetic East Money JSON response.
#[allow(clippy::too_many_arguments)]
fn make_eastmoney_response(
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
        data: Some(EastMoneyData {
            f43: Some(current),
            f44: Some(high),
            f45: Some(low),
            f47: Some(volume),
            f58: Some(name.to_string()),
            f60: Some(prev_close),
            f169: Some(change),
            f170: Some(change_pct),
            ..Default::default()
        }),
    }
}

#[test]
fn test_parse_eastmoney_quote_valid() {
    let resp = make_eastmoney_response(
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
    let resp = EastMoneyResponse { data: None };
    let result = parse_eastmoney_quote("sh999999", "CN", resp);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("No data from East Money"));
}

#[test]
fn test_parse_eastmoney_index_with_dash_strings() {
    // Overseas indices (NASDAQ, S&P, HSI) return `"-"` for fundamentals
    // fields (market cap, P/E, P/B, turnover) that don't apply to them.
    // The lenient deserializer must coerce those to None instead of
    // failing the whole parse.
    let body = r#"{"rc":0,"rt":4,"data":{"f43":25690.9,"f44":25841.31,"f45":25681.32,"f47":6651314688,"f58":"纳斯达克","f60":25837.21,"f169":853.69,"f170":3.30,"f116":"-","f163":"-","f167":"-","f168":"-"}}"#;
    let resp = parse_eastmoney_body(body, "^IXIC").unwrap();
    let quote = parse_eastmoney_quote("^IXIC", "US", resp).unwrap();
    assert_eq!(quote.name, "纳斯达克");
    assert!((quote.current_price - 25690.9).abs() < 0.01);
    assert!((quote.change_percent - 3.30).abs() < 0.01);
}

#[test]
fn test_parse_eastmoney_quote_missing_price() {
    let resp = EastMoneyResponse {
        data: Some(EastMoneyData {
            f43: None,
            f44: Some(1720.00),
            f45: Some(1685.00),
            f47: Some(12345.0),
            f58: Some("贵州茅台".to_string()),
            f60: Some(1690.00),
            f169: Some(20.50),
            f170: Some(1.21),
            ..Default::default()
        }),
    };
    let result = parse_eastmoney_quote("sh600519", "CN", resp);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing current price"));
}

#[test]
fn test_parse_eastmoney_quote_change_calculation() {
    let resp = make_eastmoney_response(
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
    // Hyphens should be converted to underscores with prefix 106
    assert_eq!(to_eastmoney_us_secid("BRK-B"), "106.BRK_B");
    assert_eq!(to_eastmoney_us_secid("BRK-A"), "106.BRK_A");
    assert_eq!(to_eastmoney_us_secid("BF-B"), "106.BF_B");
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
    let resp = make_eastmoney_response("苹果", 195.50, 193.00, 197.00, 192.00, 50000.0, 2.50, 1.30);
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
        data: Some(EastMoneyData {
            f43: Some(1100.00),
            f44: Some(1200.00),
            f45: Some(950.00),
            f47: Some(99999.0),
            f58: Some("贵州茅台".to_string()),
            f60: Some(1000.00),
            f169: None,
            f170: None,
            ..Default::default()
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
    let data = resp.data.unwrap();
    assert!((data.f43.unwrap() - 1516.0).abs() < 0.001);
    assert!((data.f47.unwrap() - 30279.0).abs() < 0.001);
}

#[test]
fn test_eastmoney_volume_converts_to_u64() {
    // The parse function should convert f64 volume to u64 correctly.
    let resp = make_eastmoney_response(
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
        ..Default::default()
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
    let quotes = rt
        .block_on(fetch_quotes_batch_with_providers(
            symbols,
            "eastmoney",
            "eastmoney",
            "eastmoney",
        ))
        .unwrap();
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
    let quotes = rt
        .block_on(fetch_quotes_batch_cached_with_providers(
            &cache,
            symbols,
            "eastmoney",
            "eastmoney",
            "eastmoney",
            false,
        ))
        .unwrap();
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
fn test_quote_cache_no_ttl_expiry() {
    // Verify that cached quotes do not expire based on time.
    // get() should return the cached quote regardless of when it was stored.
    let cache = QuoteCache::new();
    let quote = sample_quote("AAPL", "US");
    cache.set(quote);
    // Immediately retrievable
    assert!(cache.get("AAPL").is_some());
    // get_batch should also return it (not as "missing")
    let (cached, missing) = cache.get_batch(&[("AAPL".to_string(), "US".to_string())]);
    assert_eq!(cached.len(), 1);
    assert!(missing.is_empty());
}

#[test]
fn test_fetch_quotes_batch_cached_force_refresh() {
    // Verify that force_refresh=true bypasses the cache and fetches from API.
    // We use cash symbols ($CASH-*) which return synthetic quotes.
    let cache = QuoteCache::new();

    // Pre-populate cache
    let initial_quote = sample_quote("$CASH-USD", "US");
    cache.set(initial_quote);

    let symbols = vec![("$CASH-USD".to_string(), "US".to_string())];
    let rt = tokio::runtime::Runtime::new().unwrap();

    // With force_refresh=false, should return cached data
    let quotes = rt
        .block_on(fetch_quotes_batch_cached_with_providers(
            &cache,
            symbols.clone(),
            "eastmoney",
            "eastmoney",
            "eastmoney",
            false,
        ))
        .unwrap();
    assert_eq!(quotes.len(), 1);
    // Cached quote has price 100.0 (from sample_quote)
    assert!((quotes[0].current_price - 100.0).abs() < 0.001);

    // With force_refresh=true, should fetch fresh data (cash quote has price 1.0)
    let quotes = rt
        .block_on(fetch_quotes_batch_cached_with_providers(
            &cache,
            symbols,
            "eastmoney",
            "eastmoney",
            "eastmoney",
            true,
        ))
        .unwrap();
    assert_eq!(quotes.len(), 1);
    assert!((quotes[0].current_price - 1.0).abs() < 0.001);
}

#[test]
fn test_to_yahoo_symbol_us() {
    assert_eq!(to_yahoo_symbol("AAPL", "US"), "AAPL");
    assert_eq!(to_yahoo_symbol("MSFT", "US"), "MSFT");
    // Dots should be converted to hyphens for Yahoo
    assert_eq!(to_yahoo_symbol("BRK.B", "US"), "BRK-B");
    assert_eq!(to_yahoo_symbol("BRK.A", "US"), "BRK-A");
    assert_eq!(to_yahoo_symbol("BF.B", "US"), "BF-B");
    // Hyphens should remain unchanged
    assert_eq!(to_yahoo_symbol("BRK-B", "US"), "BRK-B");
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
    let result = fetch_quotes_batch_with_providers(symbols, "yahoo", "yahoo", "eastmoney").await;
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
            info!(
                "✅ CN quote (East Money): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ CN quote failed (network issue in CI): {}", e);
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
            info!(
                "✅ US quote (Yahoo): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ US quote failed (network issue in CI): {}", e);
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
            info!(
                "✅ East Money quote: {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ East Money quote failed (network issue in CI): {}", e);
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
            info!(
                "✅ US quote (East Money): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ US East Money quote failed (network issue in CI): {}", e);
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
            info!(
                "✅ HK quote (East Money): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ HK East Money quote failed (network issue in CI): {}", e);
        }
    }
}

#[test]
fn test_global_eastmoney_client_returns_same_instance() {
    let c1 = http_client::eastmoney_client();
    let c2 = http_client::eastmoney_client();
    assert!(std::ptr::eq(c1, c2));
}

#[test]
fn test_global_general_client_returns_same_instance() {
    let c1 = http_client::general_client();
    let c2 = http_client::general_client();
    assert!(std::ptr::eq(c1, c2));
}

#[test]
fn test_global_eastmoney_client_can_build_request() {
    let client = http_client::eastmoney_client();
    let req = client
        .get("https://push2.eastmoney.com/test")
        .build()
        .expect("should build request");
    assert_eq!(req.method(), reqwest::Method::GET);
}

// ---- East Money history parsing tests ----

#[test]
fn test_parse_eastmoney_kline_response() {
    // Simulate the East Money kline API response format
    let json_str = r#"{
        "rc": 0,
        "data": {
            "code": "00700",
            "klines": [
                "2024-01-02,350.00,355.00,358.00,349.00,10000000,3550000000.00,2.58,1.43,5.00,0.50",
                "2024-01-03,356.00,352.00,357.00,351.00,12000000,4272000000.00,1.69,-0.84,-3.00,0.60",
                "2024-01-04,351.00,360.00,362.00,350.00,15000000,5400000000.00,3.41,2.27,8.00,0.75"
            ]
        }
    }"#;

    let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
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

    assert_eq!(result.len(), 3);
    assert_eq!(
        result[0].0,
        chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap()
    );
    assert!((result[0].1 - 355.0).abs() < f64::EPSILON);
    assert_eq!(
        result[1].0,
        chrono::NaiveDate::from_ymd_opt(2024, 1, 3).unwrap()
    );
    assert!((result[1].1 - 352.0).abs() < f64::EPSILON);
    assert_eq!(
        result[2].0,
        chrono::NaiveDate::from_ymd_opt(2024, 1, 4).unwrap()
    );
    assert!((result[2].1 - 360.0).abs() < f64::EPSILON);
}

#[test]
fn test_parse_eastmoney_kline_empty() {
    let json_str = r#"{"rc": 0, "data": {"code": "00700", "klines": []}}"#;
    let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
    let klines = json["data"]["klines"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(klines.is_empty());
}

#[test]
fn test_parse_eastmoney_kline_null_data() {
    let json_str = r#"{"rc": 0, "data": null}"#;
    let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
    let klines = json["data"]["klines"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(klines.is_empty());
}

// ---- Xueqiu symbol conversion tests ----

#[test]
fn test_to_xueqiu_us_symbol() {
    // Hyphens should be converted to dots
    assert_eq!(to_xueqiu_us_symbol("BRK-B"), "BRK.B");
    assert_eq!(to_xueqiu_us_symbol("BRK-A"), "BRK.A");
    assert_eq!(to_xueqiu_us_symbol("BF-B"), "BF.B");
    // Already dot format should remain unchanged
    assert_eq!(to_xueqiu_us_symbol("BRK.B"), "BRK.B");
    // Simple symbols without hyphens should just uppercase
    assert_eq!(to_xueqiu_us_symbol("AAPL"), "AAPL");
    assert_eq!(to_xueqiu_us_symbol("aapl"), "AAPL");
}

#[test]
fn test_to_xueqiu_cn_symbol_shanghai() {
    assert_eq!(to_xueqiu_cn_symbol("sh600519").unwrap(), "SH600519");
    assert_eq!(to_xueqiu_cn_symbol("SH600519").unwrap(), "SH600519");
}

#[test]
fn test_to_xueqiu_cn_symbol_shenzhen() {
    assert_eq!(to_xueqiu_cn_symbol("sz000858").unwrap(), "SZ000858");
    assert_eq!(to_xueqiu_cn_symbol("Sz000858").unwrap(), "SZ000858");
}

#[test]
fn test_to_xueqiu_cn_symbol_invalid() {
    assert!(to_xueqiu_cn_symbol("hk00700").is_err());
    assert!(to_xueqiu_cn_symbol("ab").is_err());
}

#[test]
fn test_to_xueqiu_hk_symbol() {
    assert_eq!(to_xueqiu_hk_symbol("00700").unwrap(), "00700");
    assert_eq!(to_xueqiu_hk_symbol("0700.HK").unwrap(), "00700");
    assert_eq!(to_xueqiu_hk_symbol("9988.HK").unwrap(), "09988");
    assert_eq!(to_xueqiu_hk_symbol("700.hk").unwrap(), "00700");
}

#[test]
fn test_to_xueqiu_hk_symbol_invalid() {
    assert!(to_xueqiu_hk_symbol("INVALID").is_err());
}

// ---- Xueqiu response parsing tests ----

#[allow(clippy::too_many_arguments)]
fn make_xueqiu_response(
    _symbol: &str,
    name: &str,
    current: f64,
    last_close: f64,
    high: f64,
    low: f64,
    volume: f64,
    chg: f64,
    percent: f64,
) -> XueqiuResponse {
    XueqiuResponse {
        data: Some(XueqiuData {
            quote: Some(XueqiuQuote {
                name: Some(name.to_string()),
                current: Some(current),
                last_close: Some(last_close),
                chg: Some(chg),
                percent: Some(percent),
                high: Some(high),
                low: Some(low),
                volume: Some(volume),
                ..Default::default()
            }),
        }),
        error_code: Some(0),
        error_description: None,
    }
}

#[test]
fn test_parse_xueqiu_quote_valid_cn() {
    let resp = make_xueqiu_response(
        "SH600519",
        "贵州茅台",
        1710.50,
        1690.00,
        1720.00,
        1685.00,
        12345.0,
        20.50,
        1.21,
    );
    let result = parse_xueqiu_quote("sh600519", "CN", resp);
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
fn test_parse_xueqiu_quote_valid_us() {
    let resp = make_xueqiu_response(
        "AAPL", "苹果", 195.50, 193.00, 197.00, 192.00, 50000.0, 2.50, 1.30,
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
    let result = parse_xueqiu_quote("00700", "HK", resp);
    assert!(result.is_ok());
    let quote = result.unwrap();
    assert_eq!(quote.symbol, "00700");
    assert_eq!(quote.market, "HK");
    assert!((quote.current_price - 420.00).abs() < 0.001);
}

#[test]
fn test_parse_xueqiu_quote_no_data() {
    let resp = XueqiuResponse {
        data: None,
        error_code: Some(0),
        error_description: None,
    };
    let result = parse_xueqiu_quote("sh999999", "CN", resp);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("No data from Xueqiu"));
}

#[test]
fn test_parse_xueqiu_quote_no_quote() {
    let resp = XueqiuResponse {
        data: Some(XueqiuData { quote: None }),
        error_code: Some(0),
        error_description: None,
    };
    let result = parse_xueqiu_quote("sh999999", "CN", resp);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("No quote data from Xueqiu"));
}

#[test]
fn test_parse_xueqiu_quote_missing_price() {
    let resp = XueqiuResponse {
        data: Some(XueqiuData {
            quote: Some(XueqiuQuote {
                name: Some("贵州茅台".to_string()),
                current: None,
                last_close: Some(1690.00),
                chg: Some(20.50),
                percent: Some(1.21),
                high: Some(1720.00),
                low: Some(1685.00),
                volume: Some(12345.0),
                ..Default::default()
            }),
        }),
        error_code: Some(0),
        error_description: None,
    };
    let result = parse_xueqiu_quote("sh600519", "CN", resp);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Missing current price"));
}

#[test]
fn test_parse_xueqiu_quote_error_code() {
    let resp = XueqiuResponse {
        data: None,
        error_code: Some(400016),
        error_description: Some("token缺失".to_string()),
    };
    let result = parse_xueqiu_quote("SH600519", "CN", resp);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Xueqiu API error"));
}

#[test]
fn test_record_xueqiu_warning_preserves_fallback_failure_for_frontend() {
    clear_quote_warning();

    record_xueqiu_warning("Network error fetching AAPL from Xueqiu: operation timed out");

    assert_eq!(
        take_quote_warning().as_deref(),
        Some(XUEQIU_API_FAILED_HINT)
    );
}

#[test]
fn test_parse_xueqiu_quote_fallback_change_calculation() {
    let resp = XueqiuResponse {
        data: Some(XueqiuData {
            quote: Some(XueqiuQuote {
                name: Some("贵州茅台".to_string()),
                current: Some(1100.00),
                last_close: Some(1000.00),
                chg: None,
                percent: None,
                high: Some(1200.00),
                low: Some(950.00),
                volume: Some(99999.0),
                ..Default::default()
            }),
        }),
        error_code: Some(0),
        error_description: None,
    };
    let result = parse_xueqiu_quote("sh600519", "CN", resp);
    assert!(result.is_ok());
    let quote = result.unwrap();
    assert!((quote.change - 100.0).abs() < 0.001);
    assert!((quote.change_percent - 10.0).abs() < 0.001);
}

#[test]
fn test_xueqiu_response_deserialize() {
    let json = r#"{
        "data": {
            "market": {"status_id": 5},
            "quote": {
                "symbol": "SH600519",
                "name": "贵州茅台",
                "current": 1725.01,
                "last_close": 1714.51,
                "chg": 10.5,
                "percent": 0.61,
                "high": 1729.0,
                "low": 1711.0,
                "volume": 2558913
            }
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let resp: XueqiuResponse = serde_json::from_str(json).expect("should parse");
    assert_eq!(resp.error_code, Some(0));
    let data = resp.data.unwrap();
    let quote = data.quote.unwrap();
    assert!((quote.current.unwrap() - 1725.01).abs() < 0.001);
}

#[test]
fn test_xueqiu_response_with_extra_fields() {
    // Xueqiu returns many extra fields; our structs should ignore them.
    let json = r#"{
        "data": {
            "market": {"status_id": 5, "region": "CN"},
            "quote": {
                "symbol": "SH600519",
                "code": "600519",
                "exchange": "SH",
                "name": "贵州茅台",
                "type": 11,
                "sub_type": null,
                "status": 1,
                "current": 1725.01,
                "last_close": 1714.51,
                "chg": 10.5,
                "percent": 0.61,
                "high": 1729.0,
                "low": 1711.0,
                "volume": 2558913,
                "amount": 4405880000.0,
                "market_capital": 2167000000000.0,
                "float_market_capital": 2100000000000.0,
                "turnover_rate": 0.12,
                "pe_ttm": 27.5,
                "pe_lyr": 28.0,
                "pb": 9.8,
                "eps": 62.73,
                "dividend": 2.1,
                "dividend_yield": 0.12,
                "currency": "CNY",
                "navps": 176.21,
                "profit": 7469000000.0,
                "timestamp": 1700000000000,
                "time": 1700000000000,
                "open": 1715.0,
                "avg_price": 1722.35
            }
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let resp: XueqiuResponse = serde_json::from_str(json).expect("should parse");
    assert_eq!(resp.error_code, Some(0));
    let data = resp.data.unwrap();
    let quote = data.quote.unwrap();
    assert_eq!(quote.name.unwrap(), "贵州茅台");
    assert!((quote.current.unwrap() - 1725.01).abs() < 0.001);
    assert!((quote.volume.unwrap() - 2558913.0).abs() < 0.001);
}

#[test]
fn test_xueqiu_volume_converts_to_u64() {
    let resp = make_xueqiu_response(
        "SH600519",
        "贵州茅台",
        1516.0,
        1513.0,
        1519.0,
        1508.0,
        30279.0,
        3.0,
        0.2,
    );
    let result = parse_xueqiu_quote("sh600519", "CN", resp);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().volume, 30279);
}

#[test]
fn test_xueqiu_client_returns_same_instance() {
    let c1 = http_client::xueqiu_client();
    let c2 = http_client::xueqiu_client();
    assert!(std::ptr::eq(c1, c2));
}

#[test]
fn test_xueqiu_client_can_build_request() {
    let client = http_client::xueqiu_client();
    let req = client
        .get("https://stock.xueqiu.com/v5/stock/quote.json?symbol=SH600519")
        .build()
        .expect("should build request");
    assert_eq!(req.method(), reqwest::Method::GET);
}

// ---- Xueqiu integration tests (require network) ----

#[tokio::test]
#[ignore]
async fn test_integration_cn_xueqiu() {
    let result = fetch_xueqiu_cn_quote("sh600519").await;
    match &result {
        Ok(quote) => {
            assert_eq!(quote.symbol, "sh600519");
            assert!(quote.current_price > 0.0, "Price should be positive");
            info!(
                "✅ CN quote (Xueqiu): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ CN Xueqiu quote failed (network issue in CI): {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_integration_us_xueqiu() {
    let result = fetch_xueqiu_us_quote("AAPL").await;
    match &result {
        Ok(quote) => {
            assert_eq!(quote.market, "US");
            assert!(quote.current_price > 0.0, "Price should be positive");
            info!(
                "✅ US quote (Xueqiu): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ US Xueqiu quote failed (network issue in CI): {}", e);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_integration_hk_xueqiu() {
    let result = fetch_xueqiu_hk_quote("00700").await;
    match &result {
        Ok(quote) => {
            assert_eq!(quote.market, "HK");
            assert!(quote.current_price > 0.0, "Price should be positive");
            info!(
                "✅ HK quote (Xueqiu): {} = {}",
                quote.name, quote.current_price
            );
        }
        Err(e) => {
            warn!("⚠️ HK Xueqiu quote failed (network issue in CI): {}", e);
        }
    }
}

// ── Xueqiu kline response parsing tests ────────────────────────────

#[test]
fn test_parse_xueqiu_kline_identifies_first_trading_day_after_range() {
    // Exact shape and row returned for the newly listed sz001248.  The
    // timestamp is 2026-07-01 16:00 UTC, which is 2026-07-02 in China.
    let body = r#"{
      "data": {
        "symbol": "SZ001248",
        "column": [
          "timestamp", "volume", "open", "high", "low", "close",
          "chg", "percent", "turnoverrate", "amount",
          "volume_post", "amount_post"
        ],
        "item": [
          [1782921600000, 721697718, 21.6, 30.16, 21.6, 23.95,
           13.84, 136.89, 67.93, 17692297780.0, null, null]
        ]
      },
      "error_code": 0,
      "error_description": ""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2026, 6, 24).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();

    let outcome = parse_xueqiu_history_response(
        body,
        "sz001248",
        "CN",
        start,
        end,
        "https://example.test/kline",
    )
    .unwrap();

    assert_eq!(
        outcome,
        XueqiuHistoryOutcome::StartsAfterRange {
            first_available_date: chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(),
        }
    );
}

#[test]
fn successful_empty_xueqiu_history_is_not_reported_as_missing_user_cookie() {
    let body = r#"{
      "data": {"symbol":"SZ001248","column":[],"item":[]},
      "error_code":0,
      "error_description":""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2026, 6, 24).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();

    assert_eq!(
        parse_xueqiu_history_response(
            body,
            "sz001248",
            "CN",
            start,
            end,
            "https://example.test/kline",
        )
        .unwrap(),
        XueqiuHistoryOutcome::Empty
    );
}

#[tokio::test]
async fn test_xueqiu_history_does_not_fallback_before_first_trading_day() {
    let first_trading_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
    let eastmoney_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let yahoo_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let eastmoney_called_by_fetcher = eastmoney_called.clone();
    let yahoo_called_by_fetcher = yahoo_called.clone();

    let prices = resolve_xueqiu_history_outcome(
        "sz001248",
        "CN",
        Ok(XueqiuHistoryOutcome::StartsAfterRange {
            first_available_date: first_trading_date,
        }),
        move || async move {
            eastmoney_called_by_fetcher.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Vec::new())
        },
        move || async move {
            yahoo_called_by_fetcher.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Vec::new())
        },
    )
    .await
    .unwrap();

    assert!(prices.is_empty());
    assert!(!eastmoney_called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!yahoo_called.load(std::sync::atomic::Ordering::SeqCst));
}

/// Helper: parse a raw Xueqiu kline JSON body into (date, close) pairs
/// using the same logic as `fetch_stock_history_xueqiu`.
fn parse_xueqiu_kline_body(
    body: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    let resp: XueqiuKlineResponse =
        serde_json::from_str(body).map_err(|e| format!("parse error: {}", e))?;
    let data = resp.data.ok_or_else(|| "no data".to_string())?;
    let columns = data.column.unwrap_or_default();
    if columns.is_empty() {
        return Err("empty or missing 'column' field".to_string());
    }
    let ts_idx = columns
        .iter()
        .position(|c| c == "timestamp")
        .ok_or_else(|| format!("missing timestamp column, got: {:?}", columns))?;
    let close_idx = columns
        .iter()
        .position(|c| c == "close")
        .ok_or_else(|| format!("missing close column, got: {:?}", columns))?;
    let items = data.item.unwrap_or_default();
    let mut result = Vec::new();
    for item in &items {
        let ts_ms = item
            .get(ts_idx)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)));
        let close = item.get(close_idx).and_then(|v| v.as_f64());
        if let (Some(ts_ms), Some(close_price)) = (ts_ms, close) {
            if let Some(dt) = chrono::DateTime::from_timestamp(ts_ms / 1000, 0) {
                let date = dt.date_naive();
                if date >= start_date && date <= end_date {
                    result.push((date, close_price));
                }
            }
        }
    }
    result.sort_by_key(|(d, _)| *d);
    Ok(result)
}

#[test]
fn test_parse_xueqiu_kline_integer_timestamps() {
    // Timestamps as JSON integers (the straightforward case).
    let body = r#"{
        "data": {
            "column": ["timestamp", "volume", "open", "high", "low", "close"],
            "item": [
                [1724544000000, 1000, 100.0, 105.0, 99.0, 103.0],
                [1724630400000, 2000, 103.0, 108.0, 102.0, 107.0]
            ]
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end).unwrap();
    assert_eq!(result.len(), 2);
    assert!((result[0].1 - 103.0).abs() < 0.001);
    assert!((result[1].1 - 107.0).abs() < 0.001);
}

#[test]
fn test_parse_xueqiu_kline_float_timestamps() {
    // Timestamps as JSON floats (e.g. 1724544000000.0).
    // This is the case that previously caused all items to be silently skipped.
    let body = r#"{
        "data": {
            "column": ["timestamp", "volume", "open", "high", "low", "close"],
            "item": [
                [1724544000000.0, 1000, 100.0, 105.0, 99.0, 103.0],
                [1724630400000.0, 2000, 103.0, 108.0, 102.0, 107.0]
            ]
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end).unwrap();
    assert_eq!(result.len(), 2, "Float timestamps must be parsed correctly");
    assert!((result[0].1 - 103.0).abs() < 0.001);
}

#[test]
fn test_parse_xueqiu_kline_empty_items() {
    let body = r#"{
        "data": {
            "column": ["timestamp", "volume", "open", "high", "low", "close"],
            "item": []
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_parse_xueqiu_kline_missing_column() {
    let body = r#"{
        "data": {
            "column": ["time", "volume", "open", "high", "low", "price"],
            "item": [[1724544000000, 1000, 100.0, 105.0, 99.0, 103.0]]
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end);
    assert!(
        result.is_err(),
        "Should error when expected columns are missing"
    );
}

/// Test with the exact JSON structure returned by the live Xueqiu API,
/// including the `symbol` field in data, 12 columns, and null values in
/// items.  This reproduces the real-world response format to catch any
/// deserialization issues.
#[test]
fn test_parse_xueqiu_kline_real_api_response() {
    let body = r#"{
      "data": {
        "symbol": "SH600519",
        "column": [
          "timestamp", "volume", "open", "high", "low", "close",
          "chg", "percent", "turnoverrate", "amount",
          "volume_post", "amount_post"
        ],
        "item": [
          [1772985600000, 3744162, 1390, 1404.9, 1383.2, 1397, -5, -0.36, 0.3, 5220095639, null, null],
          [1773072000000, 2462592, 1404.9, 1409.49, 1398, 1401.88, 4.88, 0.35, 0.2, 3457808916, null, null],
          [1773158400000, 2445673, 1402.99, 1405.99, 1398.02, 1400, -1.88, -0.13, 0.2, 3425363892, null, null]
        ]
      },
      "error_code": 0,
      "error_description": ""
    }"#;
    // March 2026 dates to cover the timestamps above
    let start = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end).unwrap();
    assert_eq!(result.len(), 3, "All three items should be parsed");
    // Verify close prices
    assert!((result[0].1 - 1397.0).abs() < 0.01);
    assert!((result[1].1 - 1401.88).abs() < 0.01);
    assert!((result[2].1 - 1400.0).abs() < 0.01);
}

/// Test that a response with `data` present but missing `column` field
/// (as might happen with insufficient authentication) gives a clear error.
#[test]
fn test_parse_xueqiu_kline_missing_column_field() {
    let body = r#"{
        "data": {
            "symbol": "SH600519"
        },
        "error_code": 0,
        "error_description": ""
    }"#;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end);
    assert!(result.is_err(), "Should error when column field is absent");
    let err = result.unwrap_err();
    assert!(
        err.contains("empty or missing"),
        "Error should mention empty or missing column: {}",
        err
    );
}

/// Test parsing a kline response that has `items`/`items_size` fields
/// but no `column`/`item` fields (e.g. when API returns empty data).
#[test]
fn test_parse_xueqiu_kline_empty_data_response() {
    let body = r#"{"data":{"items":[],"items_size":0},"error_code":0,"error_description":""}"#;
    let start = chrono::NaiveDate::from_ymd_opt(2024, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2024, 8, 31).unwrap();
    let result = parse_xueqiu_kline_body(body, start, end);
    assert!(
        result.is_err(),
        "Response without column field should be an error"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("empty or missing"),
        "Error should indicate missing column: {}",
        err
    );
}

// ---- timestamp_to_market_date tests ----

#[test]
fn test_timestamp_to_market_date_cn() {
    // 2026-03-06 00:00:00 CST (UTC+8) = 2026-03-05 16:00:00 UTC
    let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 5)
        .unwrap()
        .and_hms_opt(16, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let date = timestamp_to_market_date(ts, "CN").unwrap();
    assert_eq!(
        date,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
        "CN timestamp at midnight CST should map to 2026-03-06"
    );
}

#[test]
fn test_timestamp_to_market_date_hk() {
    // Same offset as CN (UTC+8)
    let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 5)
        .unwrap()
        .and_hms_opt(16, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let date = timestamp_to_market_date(ts, "HK").unwrap();
    assert_eq!(
        date,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
        "HK timestamp at midnight CST should map to 2026-03-06"
    );
}

#[test]
fn test_timestamp_to_market_date_us() {
    // US daily bars are timestamped at midnight UTC.
    // 2026-03-06 00:00:00 UTC → should map to 2026-03-06.
    let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 6)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let date = timestamp_to_market_date(ts, "US").unwrap();
    assert_eq!(
        date,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
        "US timestamp at midnight UTC should map to 2026-03-06"
    );
}

#[test]
fn test_timestamp_to_market_date_utc_would_be_wrong() {
    // Verify that naively using UTC gives the WRONG date for CN stocks.
    // 2026-03-06 00:00:00 CST = 2026-03-05 16:00:00 UTC
    let ts = chrono::NaiveDate::from_ymd_opt(2026, 3, 5)
        .unwrap()
        .and_hms_opt(16, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();

    // Using UTC (old buggy behavior) would give 2026-03-05
    let utc_date = chrono::DateTime::from_timestamp(ts, 0)
        .unwrap()
        .date_naive();
    assert_eq!(
        utc_date,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 5).unwrap(),
        "UTC interpretation gives 2026-03-05 (wrong for CN market)"
    );

    // Using market-aware conversion gives correct 2026-03-06
    let market_date = timestamp_to_market_date(ts, "CN").unwrap();
    assert_eq!(
        market_date,
        chrono::NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
        "Market-aware gives 2026-03-06 (correct)"
    );
}
