use super::{
    fetch_candles_eastmoney, fetch_candles_xueqiu, fetch_stock_history_eastmoney,
    fetch_stock_history_xueqiu_outcome, fetch_stock_history_yahoo, resolve_xueqiu_history_outcome,
};
use crate::models::PriceCandle;
use tracing::warn;

/// Fetch historical daily OHLCV candles for a stock across providers.
///
/// Dispatches by provider with a resilient fallback chain mirroring
/// [`fetch_stock_history`]: xueqiu → eastmoney → yahoo (yahoo path returns
/// close-only candles when OHLCV is unavailable).
pub async fn fetch_stock_candles(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
    provider: &str,
) -> Result<Vec<PriceCandle>, String> {
    match provider {
        "xueqiu" => match fetch_candles_xueqiu(symbol, market, start_date, end_date).await {
            Ok(c) if !c.is_empty() => Ok(c),
            Ok(_) | Err(_) => {
                match fetch_candles_eastmoney(symbol, market, start_date, end_date).await {
                    Ok(c) if !c.is_empty() => Ok(c),
                    Ok(_) => Ok(Vec::new()),
                    Err(e) => Err(e),
                }
            }
        },
        "eastmoney" => fetch_candles_eastmoney(symbol, market, start_date, end_date).await,
        _ => fetch_candles_eastmoney(symbol, market, start_date, end_date).await,
    }
}

/// Fetch historical daily closing prices using the appropriate provider
/// based on the market and the configured provider name.
/// Falls back to Yahoo Finance for unknown providers.
/// When Xueqiu is selected but returns an error or a genuinely empty result,
/// East Money is used as an automatic fallback. A first trading date after
/// the requested range is treated as a valid pre-listing response.
pub async fn fetch_stock_history(
    symbol: &str,
    market: &str,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
    provider: &str,
) -> Result<Vec<(chrono::NaiveDate, f64)>, String> {
    match provider {
        "xueqiu" => {
            let outcome =
                fetch_stock_history_xueqiu_outcome(symbol, market, start_date, end_date).await;
            resolve_xueqiu_history_outcome(
                symbol,
                market,
                outcome,
                || fetch_stock_history_eastmoney(symbol, market, start_date, end_date),
                || fetch_stock_history_yahoo(symbol, market, start_date, end_date),
            )
            .await
        }
        "eastmoney" => {
            match fetch_stock_history_eastmoney(symbol, market, start_date, end_date).await {
                Ok(prices) if !prices.is_empty() => Ok(prices),
                Ok(_empty) => {
                    warn!(
                        "fetch_stock_history: EastMoney returned empty history for {} ({}), falling back to yahoo",
                        symbol, market
                    );
                    fetch_stock_history_yahoo(symbol, market, start_date, end_date).await
                }
                Err(e) => {
                    warn!(
                        "fetch_stock_history: EastMoney history failed for {} ({}): {}, falling back to yahoo",
                        symbol, market, e
                    );
                    fetch_stock_history_yahoo(symbol, market, start_date, end_date).await
                }
            }
        }
        _ => fetch_stock_history_yahoo(symbol, market, start_date, end_date).await,
    }
}
