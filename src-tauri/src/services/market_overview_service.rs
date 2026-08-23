//! Market overview service: assembles a "today's market" snapshot for the AI
//! assistant's `get_market_overview` tool.
//!
//! This fills a gap the rest of the app never needed: a single, concise view
//! of the major indices plus the user's own holdings' daily performance. It is
//! read-only and best-effort — every index is fetched independently, and a
//! failure on one index (e.g. Yahoo rate-limiting a CN index) degrades to a
//! null entry rather than failing the whole call, so the model still gets
//! *most* of the picture.

use crate::commands::dashboard::build_holding_details_pub;
use crate::db::Database;
use crate::models::quote::StockQuote;
use crate::services::exchange_rate_service::{
    convert_currency, get_cached_rates, ExchangeRateCache,
};
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
    pub top_movers: Vec<MoverRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoverRow {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub daily_pnl_usd: f64,
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
    let rates = get_cached_rates(rate_cache, db).await.unwrap_or_else(|_| {
        crate::models::quote::ExchangeRates {
            usd_cny: 7.2,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.2,
            updated_at: Utc::now().to_rfc3339(),
        }
    });
    let (holdings_daily_pnl_usd, holdings_count, top_movers) =
        match build_holding_details_pub(db, quote_cache, true, &rates).await {
            Ok(details) => {
                let total: f64 = details
                    .iter()
                    .map(|d| convert_currency(d.daily_pnl, &d.currency, "USD", &rates))
                    .sum();
                let mut movers: Vec<MoverRow> = details
                    .iter()
                    .filter(|d| d.shares != 0.0)
                    .map(|d| MoverRow {
                        symbol: d.symbol.clone(),
                        name: d.name.clone(),
                        market: d.market.clone(),
                        daily_pnl_usd: convert_currency(d.daily_pnl, &d.currency, "USD", &rates),
                    })
                    .collect();
                // Sort by absolute P&L so the most impactful movers surface first.
                movers.sort_by(|a, b| {
                    b.daily_pnl_usd
                        .abs()
                        .partial_cmp(&a.daily_pnl_usd.abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                movers.truncate(10);
                (total, details.len(), movers)
            }
            Err(e) => {
                warn!(target: "market_overview", "failed to build holdings: {e}");
                (0.0, 0, Vec::new())
            }
        };

    let holdings_daily_pnl_usd = if holdings_count == 0 {
        None
    } else {
        Some(holdings_daily_pnl_usd)
    };

    Ok(MarketOverview {
        generated_at: Utc::now().to_rfc3339(),
        indices,
        holdings_daily_pnl_usd,
        holdings_count,
        top_movers,
    })
}

/// Fetch one index quote from EastMoney (东方财富).
///
/// Indices are fetched exclusively from EastMoney: it needs no authentication
/// and carries every index we report. (Yahoo Finance now 403s index symbols.)
/// A failure returns `None` rather than propagating, so one bad index can't
/// blank the whole overview.
async fn fetch_index_quote(quote_cache: &QuoteCache, spec: &IndexSpec) -> Option<StockQuote> {
    if let Some(cached) = quote_cache.get(spec.symbol) {
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
}
