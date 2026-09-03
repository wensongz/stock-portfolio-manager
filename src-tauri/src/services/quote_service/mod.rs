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
    build_xueqiu_cookie_header, build_xueqiu_realtime_url, parse_xueqiu_quote,
    parse_xueqiu_realtime_body, to_xueqiu_cn_symbol, to_xueqiu_hk_symbol, to_xueqiu_us_symbol,
    xueqiu_history_request_count, XueqiuData, XueqiuKlineResponse, XueqiuQuote, XueqiuResponse,
    XUEQIU_API_FAILED_HINT, XUEQIU_COOKIE_EXPIRED_HINT,
};
pub use xueqiu::{
    fetch_candles_xueqiu, reset_xueqiu_token, set_xueqiu_user_cookie, set_xueqiu_user_u,
    xueqiu_fetch, QuoteServiceState,
};
#[allow(unused_imports)]
pub(crate) use xueqiu::{
    fetch_index_history_xueqiu, parse_xueqiu_history_response, resolve_xueqiu_history_outcome,
    XueqiuHistoryOutcome,
};
use xueqiu::{
    fetch_stock_history_xueqiu_outcome, fetch_xueqiu_cn_quote, fetch_xueqiu_hk_quote,
    fetch_xueqiu_realtime_batch, fetch_xueqiu_us_quote, plan_xueqiu_realtime_batches,
    quote_warning_for_error,
};
pub use yahoo::{fetch_stock_history_yahoo, fetch_yahoo_quote, to_yahoo_symbol};

/// Cash symbol prefix used to represent cash holdings.
/// Cash symbols follow the pattern `$CASH-{CURRENCY}`, e.g. `$CASH-USD`, `$CASH-CNY`, `$CASH-HKD`.
pub const CASH_SYMBOL_PREFIX: &str = "$CASH-";

#[derive(Debug, Clone)]
pub struct QuoteFetchResult<T> {
    pub data: T,
    pub warning: Option<String>,
    pub did_refresh: bool,
}

pub(crate) fn merge_quote_warning(current: &mut Option<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };
    if current.as_deref() != Some(xueqiu::XUEQIU_COOKIE_EXPIRED_HINT)
        || candidate == xueqiu::XUEQIU_COOKIE_EXPIRED_HINT
    {
        *current = Some(candidate);
    }
}

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
    fetch_us_quote_with_provider(state, symbol, "eastmoney")
        .await
        .map(|result| result.data)
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
) -> Result<QuoteFetchResult<StockQuote>, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_us_quote(symbol)
            .await
            .map(|data| QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
        "xueqiu" => match fetch_xueqiu_us_quote(state, symbol).await {
            Ok(data) => Ok(QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
            Err(e) => {
                let warning = quote_warning_for_error(&e);
                info!(
                    "fetch_us_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                match fetch_eastmoney_us_quote(symbol).await {
                    Ok(data) => Ok(QuoteFetchResult {
                        data,
                        warning,
                        did_refresh: true,
                    }),
                    Err(e2) => {
                        warn!(
                            "fetch_us_quote: EastMoney also failed for {}: {}, falling back to yahoo",
                            symbol, e2
                        );
                        let yahoo_symbol = to_yahoo_symbol(symbol, "US");
                        fetch_yahoo_quote(&yahoo_symbol, "US")
                            .await
                            .map(|data| QuoteFetchResult {
                                data,
                                warning,
                                did_refresh: true,
                            })
                            .map_err(|fallback| format!("{e}; fallback failed: {fallback}"))
                    }
                }
            }
        },
        _ => {
            let yahoo_symbol = to_yahoo_symbol(symbol, "US");
            fetch_yahoo_quote(&yahoo_symbol, "US")
                .await
                .map(|data| QuoteFetchResult {
                    data,
                    warning: None,
                    did_refresh: true,
                })
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
) -> Result<QuoteFetchResult<StockQuote>, String> {
    match provider {
        "eastmoney" => fetch_eastmoney_hk_quote(symbol)
            .await
            .map(|data| QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
        "xueqiu" => match fetch_xueqiu_hk_quote(state, symbol).await {
            Ok(data) => Ok(QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
            Err(e) => {
                let warning = quote_warning_for_error(&e);
                info!(
                    "fetch_hk_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                match fetch_eastmoney_hk_quote(symbol).await {
                    Ok(data) => Ok(QuoteFetchResult {
                        data,
                        warning,
                        did_refresh: true,
                    }),
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
                        fetch_yahoo_quote(&yahoo_symbol, "HK")
                            .await
                            .map(|data| QuoteFetchResult {
                                data,
                                warning,
                                did_refresh: true,
                            })
                            .map_err(|fallback| format!("{e}; fallback failed: {fallback}"))
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
            fetch_yahoo_quote(&yahoo_symbol, "HK")
                .await
                .map(|data| QuoteFetchResult {
                    data,
                    warning: None,
                    did_refresh: true,
                })
        }
    }
}

/// Fetch a CN A-share stock quote using East Money.
#[cfg(test)]
pub async fn fetch_cn_quote(state: &QuoteServiceState, symbol: &str) -> Result<StockQuote, String> {
    fetch_cn_quote_with_provider(state, symbol, "eastmoney")
        .await
        .map(|result| result.data)
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
) -> Result<QuoteFetchResult<StockQuote>, String> {
    match provider {
        "xueqiu" => match fetch_xueqiu_cn_quote(state, symbol).await {
            Ok(data) => Ok(QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
            Err(e) => {
                let warning = quote_warning_for_error(&e);
                info!(
                    "fetch_cn_quote: Xueqiu failed for {}: {}, falling back to eastmoney",
                    symbol, e
                );
                fetch_eastmoney_cn_quote(symbol)
                    .await
                    .map(|data| QuoteFetchResult {
                        data,
                        warning,
                        did_refresh: true,
                    })
                    .map_err(|fallback| format!("{e}; fallback failed: {fallback}"))
            }
        },
        // Default to eastmoney for CN
        _ => fetch_eastmoney_cn_quote(symbol)
            .await
            .map(|data| QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
    }
}

/// Fetch from the providers that follow Xueqiu in the existing fallback chain.
/// This avoids retrying the same failed Xueqiu request once per symbol after a
/// realtime batch has already failed.
fn restore_original_symbol(quote: &mut StockQuote, original_symbol: &str) {
    quote.symbol = original_symbol.to_string();
}

async fn fetch_quote_after_xueqiu_failure(
    symbol: &str,
    market: &str,
) -> Result<StockQuote, String> {
    let result = match market {
        "US" => match fetch_eastmoney_us_quote(symbol).await {
            Ok(quote) => Ok(quote),
            Err(eastmoney_error) => {
                let yahoo_symbol = to_yahoo_symbol(symbol, "US");
                fetch_yahoo_quote(&yahoo_symbol, "US")
                    .await
                    .map_err(|yahoo_error| {
                        format!(
                            "EastMoney failed: {}; Yahoo fallback failed: {}",
                            eastmoney_error, yahoo_error
                        )
                    })
            }
        },
        "HK" => match fetch_eastmoney_hk_quote(symbol).await {
            Ok(quote) => Ok(quote),
            Err(eastmoney_error) => {
                let yahoo_symbol = to_yahoo_symbol(symbol, "HK");
                fetch_yahoo_quote(&yahoo_symbol, "HK")
                    .await
                    .map_err(|yahoo_error| {
                        format!(
                            "EastMoney failed: {}; Yahoo fallback failed: {}",
                            eastmoney_error, yahoo_error
                        )
                    })
            }
        },
        "CN" => fetch_eastmoney_cn_quote(symbol).await,
        _ => Err(format!("Unknown market: {}", market)),
    };
    let mut quote = result?;
    restore_original_symbol(&mut quote, symbol);
    Ok(quote)
}

/// Batch fetch quotes using the specified providers for US, HK and CN markets.
/// All symbols configured for Xueqiu are combined into multi-symbol realtime
/// requests. Cash symbols return synthetic quotes (price = 1.0), and duplicate
/// symbols are fetched only once.
pub async fn fetch_quotes_batch_with_providers(
    state: &QuoteServiceState,
    symbols: Vec<(String, String)>,
    us_provider: &str,
    hk_provider: &str,
    cn_provider: &str,
) -> Result<QuoteFetchResult<Vec<StockQuote>>, String> {
    let unique_symbols = deduplicate_symbols(symbols);
    let mut quotes = Vec::new();
    let mut warning = None;
    let mut did_refresh = false;
    let mut xueqiu_symbols = Vec::new();
    let mut other_symbols = Vec::new();

    for (symbol, market) in &unique_symbols {
        if is_cash_symbol(symbol) {
            quotes.push(make_cash_quote(symbol, market));
            continue;
        }
        let uses_xueqiu = match market.as_str() {
            "CN" => cn_provider == "xueqiu",
            "HK" => hk_provider == "xueqiu",
            "US" => us_provider == "xueqiu",
            _ => false,
        };
        if uses_xueqiu {
            xueqiu_symbols.push((symbol.clone(), market.clone()));
        } else {
            other_symbols.push((symbol.clone(), market.clone()));
        }
    }

    // Fetch non-Xueqiu providers with their existing per-symbol behaviour.
    for (symbol, market) in other_symbols {
        let result = match market.as_str() {
            "US" => fetch_us_quote_with_provider(state, &symbol, us_provider).await,
            "HK" => fetch_hk_quote_with_provider(state, &symbol, hk_provider).await,
            "CN" => fetch_cn_quote_with_provider(state, &symbol, cn_provider).await,
            _ => Err(format!("Unknown market: {}", market)),
        };
        match result {
            Ok(mut result) => {
                restore_original_symbol(&mut result.data, &symbol);
                merge_quote_warning(&mut warning, result.warning);
                did_refresh |= result.did_refresh;
                quotes.push(result.data);
            }
            Err(e) => {
                warn!("failed to fetch quote for {} ({}): {}", symbol, market, e);
                merge_quote_warning(&mut warning, quote_warning_for_error(&e));
            }
        }
    }

    // A normal portfolio fits in one request; larger portfolios are split at
    // the conservative URL-size boundary. Invalid or missing symbols use the
    // same EastMoney/Yahoo fallback chain as the former single-quote path.
    let (batches, mut fallback_symbols) = plan_xueqiu_realtime_batches(&xueqiu_symbols);
    if !fallback_symbols.is_empty() {
        merge_quote_warning(
            &mut warning,
            quote_warning_for_error("Xueqiu realtime symbol normalization failed"),
        );
    }
    for batch in batches {
        match fetch_xueqiu_realtime_batch(state, &batch).await {
            Ok(batch_quotes) => {
                let fetched: std::collections::HashSet<&str> = batch_quotes
                    .iter()
                    .map(|quote| quote.symbol.as_str())
                    .collect();
                let mut response_omitted_symbol = false;
                for request_symbol in &batch {
                    let original_symbols =
                        std::iter::once((&request_symbol.original_symbol, &request_symbol.market))
                            .chain(
                                request_symbol
                                    .aliases
                                    .iter()
                                    .map(|(symbol, market)| (symbol, market)),
                            );
                    for (symbol, market) in original_symbols {
                        if !fetched.contains(symbol.as_str()) {
                            response_omitted_symbol = true;
                            fallback_symbols.push((symbol.clone(), market.clone()));
                        }
                    }
                }
                if response_omitted_symbol {
                    merge_quote_warning(
                        &mut warning,
                        quote_warning_for_error("Xueqiu realtime response omitted a symbol"),
                    );
                }
                did_refresh |= !batch_quotes.is_empty();
                quotes.extend(batch_quotes);
            }
            Err(error) => {
                warn!("Xueqiu realtime batch failed: {}", error);
                merge_quote_warning(&mut warning, quote_warning_for_error(&error));
                for request_symbol in batch {
                    fallback_symbols.push((request_symbol.original_symbol, request_symbol.market));
                    fallback_symbols.extend(request_symbol.aliases);
                }
            }
        }
    }

    for (symbol, market) in fallback_symbols {
        match fetch_quote_after_xueqiu_failure(&symbol, &market).await {
            Ok(quote) => {
                did_refresh = true;
                quotes.push(quote);
            }
            Err(error) => warn!(
                "failed to fetch fallback quote for {} ({}): {}",
                symbol, market, error
            ),
        }
    }

    // Preserve the caller's symbol order even though requests were grouped by
    // provider and market internally.
    let mut quotes_by_symbol: std::collections::HashMap<String, StockQuote> = quotes
        .into_iter()
        .map(|quote| (quote.symbol.clone(), quote))
        .collect();
    let quotes = unique_symbols
        .iter()
        .filter_map(|(symbol, _)| quotes_by_symbol.remove(symbol))
        .collect();

    Ok(QuoteFetchResult {
        data: quotes,
        warning,
        did_refresh,
    })
}

#[cfg(test)]
mod tests;
