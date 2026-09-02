use crate::models::StockQuote;
use chrono::Utc;
use tracing::{info, warn};

mod cache;
mod eastmoney;
mod financials;
mod history;
mod persistence;
mod xueqiu;
mod yahoo;

use cache::deduplicate_symbols;
pub use cache::{fetch_quotes_batch_cached_with_providers, QuoteCache};
pub use eastmoney::{
    fetch_candles_eastmoney, fetch_index_quote_eastmoney, fetch_stock_history_eastmoney,
    resolve_index_secid,
};
use eastmoney::{fetch_eastmoney_cn_quote, fetch_eastmoney_hk_quote, fetch_eastmoney_us_quote};
#[cfg(test)]
use eastmoney::{
    parse_eastmoney_body, parse_eastmoney_quote, to_eastmoney_hk_secid, to_eastmoney_secid,
    to_eastmoney_us_secid, EastMoneyData, EastMoneyResponse,
};
pub use financials::fetch_financial_statements;
pub use history::{fetch_stock_candles, fetch_stock_history};
pub use persistence::{
    get_quote_refresh_time, load_quotes_from_db, save_quote_refresh_time, save_quotes_to_db,
};
#[cfg(test)]
use xueqiu::{
    build_xueqiu_cookie_header, parse_xueqiu_quote, to_xueqiu_cn_symbol, to_xueqiu_hk_symbol,
    to_xueqiu_us_symbol, xueqiu_history_request_count, XueqiuData, XueqiuKlineResponse,
    XueqiuQuote, XueqiuResponse, XUEQIU_API_FAILED_HINT,
};
#[allow(unused_imports)]
pub use xueqiu::{
    clear_quote_warning, fetch_candles_xueqiu, fetch_stock_history_xueqiu, peek_quote_warning,
    reset_xueqiu_token, set_xueqiu_user_cookie, set_xueqiu_user_u, take_quote_warning,
    xueqiu_fetch, QuoteServiceState,
};
#[allow(unused_imports)]
pub(crate) use xueqiu::{
    fetch_index_history_xueqiu, parse_xueqiu_history_response, resolve_xueqiu_history_outcome,
    XueqiuHistoryOutcome,
};
use xueqiu::{
    fetch_stock_history_xueqiu_outcome, fetch_xueqiu_cn_quote, fetch_xueqiu_hk_quote,
    fetch_xueqiu_us_quote, is_xueqiu_cookie_expired_error, is_xueqiu_request_error,
    record_batch_warning, record_xueqiu_warning,
};
pub use yahoo::{fetch_stock_history_yahoo, fetch_yahoo_quote, to_yahoo_symbol};

/// Cash symbol prefix used to represent cash holdings.
/// Cash symbols follow the pattern `$CASH-{CURRENCY}`, e.g. `$CASH-USD`, `$CASH-CNY`, `$CASH-HKD`.
pub const CASH_SYMBOL_PREFIX: &str = "$CASH-";

/// Returns `true` if the symbol represents a cash holding.
pub fn is_cash_symbol(symbol: &str) -> bool {
    symbol.starts_with(CASH_SYMBOL_PREFIX)
}

/// Return the display name for a cash symbol, e.g. "现金 (USD)".
/// Panics if the symbol does not start with [`CASH_SYMBOL_PREFIX`].
pub fn cash_display_name(symbol: &str) -> String {
    let currency = symbol
        .strip_prefix(CASH_SYMBOL_PREFIX)
        .expect("cash_display_name called with non-cash symbol");
    format!("现金 ({})", currency)
}

/// Return the UTC offset for the exchange of the given market.
/// CN and HK exchanges operate in UTC+8; US exchanges in UTC-5 (EST).
/// We use a fixed offset (ignoring DST for US) because we only need the
/// date component — even during US daylight-saving time (UTC-4), the
/// difference does not shift the date when the timestamp falls within the
/// trading day.
fn market_utc_offset(market: &str) -> chrono::FixedOffset {
    match market {
        "CN" | "HK" => chrono::FixedOffset::east_opt(8 * 3600).unwrap(),
        // US: Yahoo Finance / Xueqiu / EastMoney daily bars are timestamped at
        // 00:00 UTC.  Converting to US Eastern time would shift the date back
        // one day (midnight UTC = previous evening ET), so we keep UTC+0 so
        // the UTC date matches the intended trading day.
        "US" => chrono::FixedOffset::east_opt(0).unwrap(),
        _ => chrono::FixedOffset::east_opt(0).unwrap(),
    }
}

/// Convert a Unix timestamp (seconds) to a [`chrono::NaiveDate`] in the
/// market's local timezone. This avoids the off-by-one-day error that occurs
/// when timestamps representing a date in CST (UTC+8) are interpreted in UTC.
pub fn timestamp_to_market_date(ts_secs: i64, market: &str) -> Option<chrono::NaiveDate> {
    let offset = market_utc_offset(market);
    chrono::DateTime::from_timestamp(ts_secs, 0).map(|dt| dt.with_timezone(&offset).date_naive())
}

/// Build a synthetic [`StockQuote`] for a cash symbol.
/// Cash always has price = 1.0, zero change, zero volume.
pub fn make_cash_quote(symbol: &str, market: &str) -> StockQuote {
    StockQuote {
        symbol: symbol.to_string(),
        name: cash_display_name(symbol),
        market: market.to_string(),
        current_price: 1.0,
        previous_close: 1.0,
        change: 0.0,
        change_percent: 0.0,
        high: 1.0,
        low: 1.0,
        volume: 0,
        updated_at: Utc::now().to_rfc3339(),
        ..Default::default()
    }
}

/// Fetch a US stock quote using the configured provider.
#[cfg(test)]
pub async fn fetch_us_quote(state: &QuoteServiceState, symbol: &str) -> Result<StockQuote, String> {
    fetch_us_quote_with_provider(state, symbol, "eastmoney").await
}

/// Fetch a US stock quote using the specified provider.
///
/// When `provider` is `xueqiu` but the request fails, it falls back to East
/// Money, then to Yahoo — matching the resilient behaviour of
/// [`fetch_stock_history`].
pub async fn fetch_us_quote_with_provider(
    state: &QuoteServiceState,
    symbol: &str,
    provider: &str,
) -> Result<StockQuote, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_us_quote(symbol).await,
        "xueqiu" => match fetch_xueqiu_us_quote(state, symbol).await {
            Ok(q) => Ok(q),
            Err(e) => {
                record_xueqiu_warning(state, &e);
                info!(
                    "fetch_us_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                match fetch_eastmoney_us_quote(symbol).await {
                    Ok(q) => Ok(q),
                    Err(e2) => {
                        warn!(
                            "fetch_us_quote: EastMoney also failed for {}: {}, falling back to yahoo",
                            symbol, e2
                        );
                        let yahoo_symbol = to_yahoo_symbol(symbol, "US");
                        fetch_yahoo_quote(&yahoo_symbol, "US").await
                    }
                }
            }
        },
        _ => {
            let yahoo_symbol = to_yahoo_symbol(symbol, "US");
            fetch_yahoo_quote(&yahoo_symbol, "US").await
        }
    }
}

/// Fetch a HK stock quote using the specified provider.
///
/// When `provider` is `xueqiu` but the request fails, it falls back to East
/// Money, then to Yahoo — matching the resilient behaviour of
/// [`fetch_stock_history`].
pub async fn fetch_hk_quote_with_provider(
    state: &QuoteServiceState,
    symbol: &str,
    provider: &str,
) -> Result<StockQuote, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_hk_quote(symbol).await,
        "xueqiu" => match fetch_xueqiu_hk_quote(state, symbol).await {
            Ok(q) => Ok(q),
            Err(e) => {
                record_xueqiu_warning(state, &e);
                info!(
                    "fetch_hk_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                match fetch_eastmoney_hk_quote(symbol).await {
                    Ok(q) => Ok(q),
                    Err(e2) => {
                        warn!(
                            "fetch_hk_quote: EastMoney also failed for {}: {}, falling back to yahoo",
                            symbol, e2
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
        },
        _ => {
            let yahoo_symbol = if symbol.ends_with(".HK") || symbol.ends_with(".hk") {
                symbol.to_string()
            } else {
                format!("{}.HK", symbol)
            };
            fetch_yahoo_quote(&yahoo_symbol, "HK").await
        }
    }
}

/// Fetch a CN A-share stock quote using East Money.
#[cfg(test)]
pub async fn fetch_cn_quote(state: &QuoteServiceState, symbol: &str) -> Result<StockQuote, String> {
    fetch_cn_quote_with_provider(state, symbol, "eastmoney").await
}

/// Fetch a CN A-share stock quote using the specified provider.
///
/// When `provider` is `xueqiu` but the request fails (e.g. an expired or
/// missing session token — Xueqiu's homepage no longer issues `xq_a_token`
/// without JavaScript), it transparently falls back to East Money so that
/// quotes keep working. This mirrors the resilient chain already used by
/// [`fetch_stock_history`].
pub async fn fetch_cn_quote_with_provider(
    state: &QuoteServiceState,
    symbol: &str,
    provider: &str,
) -> Result<StockQuote, String> {
    match provider {
        "xueqiu" => match fetch_xueqiu_cn_quote(state, symbol).await {
            Ok(q) => Ok(q),
            Err(e) => {
                record_xueqiu_warning(state, &e);
                info!(
                    "fetch_cn_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                fetch_eastmoney_cn_quote(symbol).await
            }
        },
        // Default to eastmoney for CN
        _ => fetch_eastmoney_cn_quote(symbol).await,
    }
}

/// Batch fetch quotes using the specified providers for US, HK and CN markets.
/// Cash symbols return synthetic quotes (price = 1.0).
/// Duplicate symbols are automatically deduplicated so that each symbol is fetched only once.
pub async fn fetch_quotes_batch_with_providers(
    state: &QuoteServiceState,
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
    cn_provider: &str,
) -> Result<Vec<StockQuote>, String> {
    // Deduplicate symbols so we only fetch each symbol once,
    // even if it appears in multiple accounts.
    let unique_symbols = deduplicate_symbols(symbols);

    let mut quotes = Vec::new();
    let mut has_xueqiu_cookie_warning = false;
    let mut has_xueqiu_api_warning = false;
    // Once we know Xueqiu is unreachable, skip remaining Xueqiu symbols so
    // we don't wait for N × 15-second timeouts (one per symbol).  Non-Xueqiu
    // symbols (e.g. US via Yahoo) are still fetched normally.
    let mut xueqiu_failed = false;
    for (symbol, market) in unique_symbols {
        // Cash symbols don't need an API call – return a synthetic quote.
        if is_cash_symbol(&symbol) {
            quotes.push(make_cash_quote(&symbol, &market));
            continue;
        }
        // Determine whether this symbol would use the Xueqiu API.
        let uses_xueqiu = match market.as_str() {
            "CN" => cn_provider == "xueqiu",
            "HK" => hk_provider == "xueqiu",
            "US" => us_provider == "xueqiu",
            _ => false,
        };
        if xueqiu_failed && uses_xueqiu {
            // Skip: Xueqiu is already known to be unreachable for this batch.
            info!(
                "Skipping {} ({}) – Xueqiu already failed for this batch",
                symbol, market
            );
            continue;
        }
        let result = match market.as_str() {
            "US" => fetch_us_quote_with_provider(state, &symbol, us_provider).await,
            "HK" => fetch_hk_quote_with_provider(state, &symbol, hk_provider).await,
            "CN" => fetch_cn_quote_with_provider(state, &symbol, cn_provider).await,
            _ => Err(format!("Unknown market: {}", market)),
        };
        match result {
            Ok(quote) => quotes.push(quote),
            Err(e) => {
                warn!("failed to fetch quote for {} ({}): {}", symbol, market, e);
                let is_cookie_err = is_xueqiu_cookie_expired_error(&e);
                let is_api_err = is_xueqiu_request_error(&e);
                if is_cookie_err {
                    has_xueqiu_cookie_warning = true;
                } else if is_api_err {
                    has_xueqiu_api_warning = true;
                }
                // Mark Xueqiu as failed for either error kind so we can skip
                // remaining Xueqiu symbols without waiting for more timeouts.
                if is_cookie_err || is_api_err {
                    xueqiu_failed = true;
                }
            }
        }
    }
    record_batch_warning(state, has_xueqiu_cookie_warning, has_xueqiu_api_warning);
    Ok(quotes)
}

#[cfg(test)]
mod tests;
