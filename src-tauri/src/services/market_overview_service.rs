//! Market overview service: assembles a "today's market" snapshot for the AI
//! assistant's `get_market_overview` tool.
//!
//! This fills a gap the rest of the app never needed: a single, concise view
//! of the major indices plus the user's own holdings' daily performance. It is
//! read-only and best-effort — every index is fetched independently, and a
//! failure on one index (e.g. Yahoo rate-limiting a CN index) degrades to a
//! null entry rather than failing the whole call, so the model still gets
//! *most* of the picture.

use crate::db::Database;
use crate::models::quote::StockQuote;
use crate::services::exchange_rate_service::{
    convert_currency, get_cached_rates, ExchangeRateCache,
};
use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
use crate::services::quote_service::{self, QuoteCache};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::warn;

/// A single index row in the overview.
#[derive(Debug, Clone, Serialize)]
pub struct IndexQuote {
    pub name: &'static str,
    pub symbol: &'static str,
    /// `null` when the fetch failed (the model should treat this as "data
    /// unavailable" rather than "price is zero").
    pub quote: Option<StockQuote>,
}

/// The full snapshot handed back to the model.
#[derive(Debug, Clone, Serialize)]
pub struct MarketOverview {
    pub generated_at: String,
    pub indices: Vec<IndexQuote>,
    /// Aggregate daily P&L of the user's open positions, in USD. `null` when
    /// the user has no holdings or the holding build failed.
    pub holdings_daily_pnl_usd: Option<f64>,
    /// Number of open positions used to compute the aggregate above.
    pub holdings_count: usize,
    /// Per-holding daily P&L (top 10 by absolute P&L) so the model can point at
    /// specific movers. Kept small to stay within the tool-result budget.
    pub top_movers: Option<Vec<MoverRow>>,
    /// Explains why holding-derived USD fields are unavailable.
    pub holdings_data_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoverRow {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub daily_pnl_usd: f64,
}

struct HoldingsSummary {
    holdings_daily_pnl_usd: Option<f64>,
    holdings_count: usize,
    top_movers: Option<Vec<MoverRow>>,
    error: Option<String>,
}

fn summarize_holdings(
    details: &[crate::models::dashboard::HoldingDetail],
    rates: Result<crate::models::quote::ExchangeRates, String>,
) -> HoldingsSummary {
    if details.is_empty() {
        return HoldingsSummary {
            holdings_daily_pnl_usd: None,
            holdings_count: 0,
            top_movers: Some(Vec::new()),
            error: None,
        };
    }
    let rates = match rates {
        Ok(rates) => rates,
        Err(error) => {
            return HoldingsSummary {
                holdings_daily_pnl_usd: None,
                holdings_count: details.len(),
                top_movers: None,
                error: Some(error),
            };
        }
    };
    let total = details
        .iter()
        .map(|detail| convert_currency(detail.daily_pnl, &detail.currency, "USD", &rates))
        .sum();
    let mut movers = details
        .iter()
        .filter(|detail| detail.shares != 0.0)
        .map(|detail| MoverRow {
            symbol: detail.symbol.clone(),
            name: detail.name.clone(),
            market: detail.market.clone(),
            daily_pnl_usd: convert_currency(detail.daily_pnl, &detail.currency, "USD", &rates),
        })
        .collect::<Vec<_>>();
    movers.sort_by(|a, b| {
        b.daily_pnl_usd
            .abs()
            .partial_cmp(&a.daily_pnl_usd.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    movers.truncate(10);
    HoldingsSummary {
        holdings_daily_pnl_usd: Some(total),
        holdings_count: details.len(),
        top_movers: Some(movers),
        error: None,
    }
}

/// Per-provider mapping for a single market index.
///
/// Yahoo Finance now returns 403 for index symbols, so we fetch indices
/// exclusively from EastMoney (东方财富), which needs no authentication and
/// carries every index we report.
struct IndexSpec {
    /// Canonical symbol shown to the user / used as the cache key.
    symbol: &'static str,
    name: &'static str,
    market: &'static str,
    /// EastMoney secid (e.g. `100.SPX`, `1.000300`).
    eastmoney: &'static str,
}

/// The indices we report. All fetched from EastMoney (no auth, reliable).
const INDICES: &[IndexSpec] = &[
    IndexSpec {
        symbol: "^GSPC",
        name: "标普500",
        market: "US",
        eastmoney: "100.SPX",
    },
    IndexSpec {
        symbol: "^IXIC",
        name: "纳斯达克",
        market: "US",
        eastmoney: "100.NDX",
    },
    IndexSpec {
        symbol: "^DJI",
        name: "道琼斯",
        market: "US",
        eastmoney: "100.DJIA",
    },
    IndexSpec {
        symbol: "^HSI",
        name: "恒生指数",
        market: "HK",
        eastmoney: "100.HSI",
    },
    IndexSpec {
        symbol: "000300.SS",
        name: "沪深300",
        market: "CN",
        eastmoney: "1.000300",
    },
    IndexSpec {
        symbol: "000001.SS",
        name: "上证综指",
        market: "CN",
        eastmoney: "1.000001",
    },
];

/// Entry point for the `get_market_overview` tool.
///
/// `rate_cache` + `db` are passed in directly so we share the live in-process
/// caches with the rest of the app (the chat loop already holds both handles).
pub async fn get_market_overview(
    db: &Database,
    rate_cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
) -> Result<MarketOverview, String> {
    let mut indices: Vec<IndexQuote> = Vec::new();

    // Fetch each index via the multi-provider fallback. A failure on one index
    // degrades to a null entry rather than failing the whole overview, so the
    // model still gets *most* of the picture.
    for spec in INDICES {
        let quote = fetch_index_quote(quote_cache, spec).await;
        indices.push(IndexQuote {
            name: spec.name,
            symbol: spec.symbol,
            quote,
        });
    }

    // User holdings daily P&L, normalised to USD.
    let holdings = match PortfolioReadModel::load(db, quote_cache, None, QuoteReadMode::CacheOnly)
        .await
    {
        Ok(model) => summarize_holdings(model.holdings(), get_cached_rates(rate_cache, db).await),
        Err(error) => {
            warn!(target: "market_overview", "failed to build holdings: {error}");
            HoldingsSummary {
                holdings_daily_pnl_usd: None,
                holdings_count: 0,
                top_movers: None,
                error: Some(error),
            }
        }
    };

    Ok(MarketOverview {
        generated_at: Utc::now().to_rfc3339(),
        indices,
        holdings_daily_pnl_usd: holdings.holdings_daily_pnl_usd,
        holdings_count: holdings.holdings_count,
        top_movers: holdings.top_movers,
        holdings_data_error: holdings.error,
    })
}

/// Fetch one index quote from EastMoney (东方财富).
///
/// Indices are fetched exclusively from EastMoney: it needs no authentication
/// and carries every index we report. (Yahoo Finance now 403s index symbols.)
/// A failure returns `None` rather than propagating, so one bad index can't
/// blank the whole overview.
async fn fetch_index_quote(quote_cache: &QuoteCache, spec: &IndexSpec) -> Option<StockQuote> {
    if let Some(cached) = quote_cache.get(spec.market, spec.symbol) {
        return Some(cached);
    }
    match quote_service::fetch_index_quote_eastmoney(spec.eastmoney, spec.symbol, spec.market).await
    {
        Ok(q) => {
            quote_cache.set(q.clone());
            Some(q)
        }
        Err(e) => {
            warn!(
                target: "market_overview",
                name = spec.name,
                secid = spec.eastmoney,
                "eastmoney index fetch failed: {e}"
            );
            None
        }
    }
}

/// Convenience: render the overview as a compact JSON `Value` for the tool
/// result. Exposed for tests; production uses the serde-derive path via
/// `serde_json::to_value`.
#[allow(dead_code)]
pub fn to_json_value(o: &MarketOverview) -> Value {
    json!({
        "generated_at": o.generated_at,
        "indices": o.indices.iter().map(|i| json!({
            "name": i.name,
            "symbol": i.symbol,
            "quote": i.quote,
        })).collect::<Vec<_>>(),
        "holdings_daily_pnl_usd": o.holdings_daily_pnl_usd,
        "holdings_count": o.holdings_count,
        "top_movers": o.top_movers,
        "holdings_data_error": o.holdings_data_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_table_is_nonempty() {
        assert!(!INDICES.is_empty());
    }

    #[test]
    fn index_symbols_are_unique() {
        let mut all: Vec<&str> = INDICES.iter().map(|s| s.symbol).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), INDICES.len(), "duplicate canonical index symbol");
    }

    #[test]
    fn every_index_has_eastmoney_secid() {
        // EastMoney is the guaranteed fallback (no auth, carries all indices),
        // so every entry must have a secid or the fallback chain is broken.
        for s in INDICES {
            assert!(
                !s.eastmoney.is_empty(),
                "{} missing eastmoney secid",
                s.name
            );
        }
    }

    #[test]
    fn index_table_covers_expected_indices() {
        let names: Vec<&str> = INDICES.iter().map(|s| s.name).collect();
        for expected in [
            "标普500",
            "纳斯达克",
            "道琼斯",
            "恒生指数",
            "沪深300",
            "上证综指",
        ] {
            assert!(names.contains(&expected), "missing index {expected}");
        }
    }

    #[test]
    fn unavailable_rates_keep_holdings_count_but_remove_usd_values() {
        let details = vec![crate::models::dashboard::HoldingDetail {
            id: "holding".to_string(),
            account_id: "acct".to_string(),
            account_name: "账户".to_string(),
            symbol: "600000".to_string(),
            name: "浦发银行".to_string(),
            market: "CN".to_string(),
            category_name: "分红股".to_string(),
            category_color: "#fff".to_string(),
            shares: 100.0,
            avg_cost: 9.0,
            current_price: 10.0,
            market_value: 1000.0,
            cost_value: 900.0,
            pnl: 100.0,
            pnl_percent: Some(11.11),
            daily_pnl: 20.0,
            currency: "CNY".to_string(),
            market_value_usd: 0.0,
        }];

        let summary = summarize_holdings(&details, Err("offline".to_string()));

        assert_eq!(summary.holdings_count, 1);
        assert_eq!(summary.holdings_daily_pnl_usd, None);
        assert!(summary.top_movers.is_none());
        assert_eq!(summary.error.as_deref(), Some("offline"));
    }
}
