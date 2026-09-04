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
#[cfg(test)]
use eastmoney::{
    build_eastmoney_batch_url, parse_eastmoney_batch_body, parse_eastmoney_body,
    parse_eastmoney_quote, to_eastmoney_hk_secid, to_eastmoney_secid, to_eastmoney_us_secid,
    EastMoneyData, EastMoneyResponse,
};
pub use eastmoney::{
    fetch_candles_eastmoney, fetch_index_quote_eastmoney, fetch_stock_history_eastmoney,
    resolve_index_secid,
};
use eastmoney::{
    fetch_eastmoney_cn_quote, fetch_eastmoney_hk_quote, fetch_eastmoney_quotes_batch,
    fetch_eastmoney_us_quote, plan_eastmoney_quote_batches,
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
#[cfg(test)]
use yahoo::{build_yahoo_spark_url, parse_yahoo_spark_body};
pub use yahoo::{fetch_stock_history_yahoo, fetch_yahoo_quote, to_yahoo_symbol};
use yahoo::{fetch_yahoo_quotes_batch, plan_yahoo_quote_batches};

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
                        fetch_yahoo_quote_for_stored_symbol(symbol, "US")
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
        _ => fetch_yahoo_quote_for_stored_symbol(symbol, "US")
            .await
            .map(|data| QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
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
                        fetch_yahoo_quote_for_stored_symbol(symbol, "HK")
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
        _ => fetch_yahoo_quote_for_stored_symbol(symbol, "HK")
            .await
            .map(|data| QuoteFetchResult {
                data,
                warning: None,
                did_refresh: true,
            }),
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

async fn fetch_yahoo_quote_for_stored_symbol(
    symbol: &str,
    market: &str,
) -> Result<StockQuote, String> {
    let yahoo_symbol = to_yahoo_symbol(symbol, market);
    let mut quote = fetch_yahoo_quote(&yahoo_symbol, market).await?;
    restore_original_symbol(&mut quote, symbol);
    Ok(quote)
}

async fn fetch_quote_after_xueqiu_failure(
    symbol: &str,
    market: &str,
) -> Result<StockQuote, String> {
    let result = match market {
        "US" => match fetch_eastmoney_us_quote(symbol).await {
            Ok(quote) => Ok(quote),
            Err(eastmoney_error) => fetch_yahoo_quote_for_stored_symbol(symbol, "US")
                .await
                .map_err(|yahoo_error| {
                    format!(
                        "EastMoney failed: {}; Yahoo fallback failed: {}",
                        eastmoney_error, yahoo_error
                    )
                }),
        },
        "HK" => match fetch_eastmoney_hk_quote(symbol).await {
            Ok(quote) => Ok(quote),
            Err(eastmoney_error) => fetch_yahoo_quote_for_stored_symbol(symbol, "HK")
                .await
                .map_err(|yahoo_error| {
                    format!(
                        "EastMoney failed: {}; Yahoo fallback failed: {}",
                        eastmoney_error, yahoo_error
                    )
                }),
        },
        "CN" => fetch_eastmoney_cn_quote(symbol).await,
        _ => Err(format!("Unknown market: {}", market)),
    };
    let mut quote = result?;
    restore_original_symbol(&mut quote, symbol);
    Ok(quote)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct QuoteProviderRequestPlan {
    cash_symbols: Vec<(String, String)>,
    xueqiu_symbols: Vec<(String, String)>,
    eastmoney_symbols: Vec<(String, String)>,
    yahoo_symbols: Vec<(String, String)>,
    other_symbols: Vec<(String, String)>,
}

fn plan_quote_provider_requests(
    symbols: &[(String, String)],
    us_provider: &str,
    hk_provider: &str,
    cn_provider: &str,
) -> QuoteProviderRequestPlan {
    let mut plan = QuoteProviderRequestPlan::default();
    for (symbol, market) in symbols {
        if is_cash_symbol(symbol) {
            plan.cash_symbols.push((symbol.clone(), market.clone()));
            continue;
        }
        let provider = match market.as_str() {
            "US" => us_provider,
            "HK" => hk_provider,
            "CN" => cn_provider,
            _ => "",
        };
        match provider {
            "xueqiu" => plan.xueqiu_symbols.push((symbol.clone(), market.clone())),
            "eastmoney" => plan
                .eastmoney_symbols
                .push((symbol.clone(), market.clone())),
            "yahoo" if matches!(market.as_str(), "US" | "HK") => {
                plan.yahoo_symbols.push((symbol.clone(), market.clone()))
            }
            _ => plan.other_symbols.push((symbol.clone(), market.clone())),
        }
    }
    plan
}

/// Batch fetch quotes using the specified providers for US, HK and CN markets.
/// Symbols configured for Xueqiu, EastMoney, or Yahoo are combined into
/// provider-specific multi-symbol requests. Cash symbols return synthetic
/// quotes (price = 1.0), and duplicate symbols are fetched only once.
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
    let provider_plan =
        plan_quote_provider_requests(&unique_symbols, us_provider, hk_provider, cn_provider);

    for (symbol, market) in &provider_plan.cash_symbols {
        quotes.push(make_cash_quote(symbol, market));
    }

    // Providers without a multi-symbol endpoint keep their existing
    // per-symbol behaviour.
    for (symbol, market) in provider_plan.other_symbols {
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

    // Yahoo's spark endpoint accepts at most 20 symbols. Missing symbols and
    // failed batches retain the existing single-symbol chart fallback.
    let (yahoo_batches, mut yahoo_fallback_symbols) =
        plan_yahoo_quote_batches(&provider_plan.yahoo_symbols);
    for batch in yahoo_batches {
        match fetch_yahoo_quotes_batch(&batch).await {
            Ok(batch_quotes) => {
                let fetched: std::collections::HashSet<&str> = batch_quotes
                    .iter()
                    .map(|quote| quote.symbol.as_str())
                    .collect();
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
                            yahoo_fallback_symbols.push((symbol.clone(), market.clone()));
                        }
                    }
                }
                did_refresh |= !batch_quotes.is_empty();
                quotes.extend(batch_quotes);
            }
            Err(error) => {
                warn!("Yahoo spark batch failed: {}", error);
                for request_symbol in batch {
                    yahoo_fallback_symbols
                        .push((request_symbol.original_symbol, request_symbol.market));
                    yahoo_fallback_symbols.extend(request_symbol.aliases);
                }
            }
        }
    }

    for (symbol, market) in deduplicate_symbols(yahoo_fallback_symbols) {
        match fetch_yahoo_quote_for_stored_symbol(&symbol, &market).await {
            Ok(quote) => {
                did_refresh = true;
                quotes.push(quote);
            }
            Err(error) => warn!(
                "failed to fetch Yahoo fallback quote for {} ({}): {}",
                symbol, market, error
            ),
        }
    }

    // A normal portfolio fits in one request; larger portfolios are split at
    // the conservative URL-size boundary. Invalid or missing symbols use the
    // same EastMoney/Yahoo fallback chain as the former single-quote path.
    let (xueqiu_batches, mut xueqiu_fallback_symbols) =
        plan_xueqiu_realtime_batches(&provider_plan.xueqiu_symbols);
    if !xueqiu_fallback_symbols.is_empty() {
        merge_quote_warning(
            &mut warning,
            quote_warning_for_error("Xueqiu realtime symbol normalization failed"),
        );
    }
    for batch in xueqiu_batches {
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
                            xueqiu_fallback_symbols.push((symbol.clone(), market.clone()));
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
                    xueqiu_fallback_symbols
                        .push((request_symbol.original_symbol, request_symbol.market));
                    xueqiu_fallback_symbols.extend(request_symbol.aliases);
                }
            }
        }
    }

    // EastMoney is both a selectable provider and Xueqiu's first fallback.
    // Combining both queues ensures an unavailable Xueqiu batch does not turn
    // into one EastMoney request per holding.
    let after_xueqiu: std::collections::HashSet<(String, String)> =
        xueqiu_fallback_symbols.iter().cloned().collect();
    let mut eastmoney_symbols = provider_plan.eastmoney_symbols;
    eastmoney_symbols.extend(xueqiu_fallback_symbols);
    let eastmoney_symbols = deduplicate_symbols(eastmoney_symbols);
    let (eastmoney_batches, mut per_symbol_fallback) =
        plan_eastmoney_quote_batches(&eastmoney_symbols);

    for batch in eastmoney_batches {
        match fetch_eastmoney_quotes_batch(&batch).await {
            Ok(batch_quotes) => {
                let fetched: std::collections::HashSet<&str> = batch_quotes
                    .iter()
                    .map(|quote| quote.symbol.as_str())
                    .collect();
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
                            per_symbol_fallback.push((symbol.clone(), market.clone()));
                        }
                    }
                }
                did_refresh |= !batch_quotes.is_empty();
                quotes.extend(batch_quotes);
            }
            Err(error) => {
                warn!("EastMoney realtime batch failed: {}", error);
                for request_symbol in batch {
                    per_symbol_fallback
                        .push((request_symbol.original_symbol, request_symbol.market));
                    per_symbol_fallback.extend(request_symbol.aliases);
                }
            }
        }
    }

    // The batch endpoint can omit a symbol or return `f2 = null` for an
    // otherwise valid instrument. Preserve the former per-symbol behaviour in
    // those cases, including Yahoo after an earlier Xueqiu failure.
    for (symbol, market) in deduplicate_symbols(per_symbol_fallback) {
        if after_xueqiu.contains(&(symbol.clone(), market.clone())) {
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
            continue;
        }

        let result = match market.as_str() {
            "US" => fetch_us_quote_with_provider(state, &symbol, "eastmoney").await,
            "HK" => fetch_hk_quote_with_provider(state, &symbol, "eastmoney").await,
            "CN" => fetch_cn_quote_with_provider(state, &symbol, "eastmoney").await,
            _ => Err(format!("Unknown market: {}", market)),
        };
        match result {
            Ok(mut result) => {
                restore_original_symbol(&mut result.data, &symbol);
                merge_quote_warning(&mut warning, result.warning);
                did_refresh |= result.did_refresh;
                quotes.push(result.data);
            }
            Err(error) => warn!(
                "failed to fetch EastMoney fallback quote for {} ({}): {}",
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
