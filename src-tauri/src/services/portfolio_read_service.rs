use crate::db::Database;
use crate::models::{DashboardReport, DashboardSummary, ExchangeRates, HoldingDetail};
use crate::services::exchange_rate_service::convert_currency;
use crate::services::quote_provider_service;
use crate::services::quote_service::{
    fetch_quotes_batch_cached_with_providers, QuoteCache, QuoteServiceState,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteReadMode {
    CacheOnly,
    RefreshMissing,
}

#[derive(Debug)]
pub struct PortfolioReadModel {
    holdings: Vec<HoldingDetail>,
    missing_quote_keys: HashSet<(String, String)>,
    category_ids_by_holding: HashMap<String, Option<String>>,
    quote_warning: Option<String>,
    quotes_refreshed: bool,
}

fn normalized_quote_key(market: &str, symbol: &str) -> (String, String) {
    (
        market.trim().to_ascii_uppercase(),
        symbol.trim().to_ascii_uppercase(),
    )
}

fn quote_values_by_market_and_symbol(
    quotes: &[crate::models::StockQuote],
) -> HashMap<(String, String), (f64, f64)> {
    quotes
        .iter()
        .map(|quote| {
            (
                normalized_quote_key(&quote.market, &quote.symbol),
                (quote.current_price, quote.change),
            )
        })
        .collect()
}

impl PortfolioReadModel {
    pub async fn load(
        db: &Database,
        quote_cache: &QuoteCache,
        quote_state: Option<&QuoteServiceState>,
        mode: QuoteReadMode,
    ) -> Result<Self, String> {
        struct Row {
            id: String,
            account_id: String,
            account_name: String,
            symbol: String,
            name: String,
            market: String,
            category_name: String,
            category_color: String,
            category_id: Option<String>,
            shares: f64,
            avg_cost: f64,
            currency: String,
        }

        let rows: Vec<Row> = {
            let conn = db.conn.lock().map_err(|error| error.to_string())?;
            let mut statement = conn
                .prepare(
                    "SELECT h.id, h.account_id, a.name AS account_name,
                            h.symbol, h.name, h.market,
                            COALESCE(c.name, '未分类') AS category_name,
                            COALESCE(c.color, '#8B8B8B') AS category_color,
                            h.category_id, h.shares, h.avg_cost, h.currency
                     FROM holdings h
                     LEFT JOIN accounts a ON h.account_id = a.id
                     LEFT JOIN categories c ON h.category_id = c.id
                     WHERE h.shares > 0
                     ORDER BY h.market, h.symbol",
                )
                .map_err(|error| error.to_string())?;
            let result = statement
                .query_map([], |row| {
                    Ok(Row {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        account_name: row.get(2)?,
                        symbol: row.get(3)?,
                        name: row.get(4)?,
                        market: row.get(5)?,
                        category_name: row.get(6)?,
                        category_color: row.get(7)?,
                        category_id: row.get(8)?,
                        shares: row.get(9)?,
                        avg_cost: row.get(10)?,
                        currency: row.get(11)?,
                    })
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            result
        };

        if rows.is_empty() {
            return Ok(Self {
                holdings: vec![],
                missing_quote_keys: HashSet::new(),
                category_ids_by_holding: HashMap::new(),
                quote_warning: None,
                quotes_refreshed: false,
            });
        }

        let symbols: Vec<(String, String)> = rows
            .iter()
            .map(|row| (row.symbol.clone(), row.market.clone()))
            .collect();
        let quote_result = match mode {
            QuoteReadMode::CacheOnly => {
                let (cached, _missing) = quote_cache.get_batch(&symbols);
                crate::services::quote_service::QuoteFetchResult {
                    data: cached,
                    warning: None,
                    did_refresh: false,
                }
            }
            QuoteReadMode::RefreshMissing => {
                let config = quote_provider_service::get_quote_provider_config(db)?;
                let state = quote_state.ok_or_else(|| {
                    "quote service state is required when refreshing holding details".to_string()
                })?;
                fetch_quotes_batch_cached_with_providers(
                    state,
                    quote_cache,
                    symbols.clone(),
                    &config.us_provider,
                    &config.hk_provider,
                    &config.cn_provider,
                    false,
                )
                .await?
            }
        };
        let available_quote_keys = quote_result
            .data
            .iter()
            .map(|quote| normalized_quote_key(&quote.market, &quote.symbol))
            .collect::<HashSet<_>>();
        let missing_quote_keys = symbols
            .iter()
            .map(|(symbol, market)| normalized_quote_key(market, symbol))
            .filter(|key| !available_quote_keys.contains(key))
            .collect();
        let quote_map = quote_values_by_market_and_symbol(&quote_result.data);
        let category_ids_by_holding = rows
            .iter()
            .map(|row| (row.id.clone(), row.category_id.clone()))
            .collect();

        let holdings = rows
            .into_iter()
            .map(|row| {
                let quote_key = normalized_quote_key(&row.market, &row.symbol);
                let (current_price, change) = *quote_map.get(&quote_key).unwrap_or(&(0.0, 0.0));
                let market_value = row.shares * current_price;
                let cost_value = row.shares * row.avg_cost;
                let pnl = market_value - cost_value;
                let pnl_percent = if cost_value > 0.0 {
                    Some(pnl / cost_value * 100.0)
                } else {
                    None
                };
                HoldingDetail {
                    id: row.id,
                    account_id: row.account_id,
                    account_name: row.account_name,
                    symbol: row.symbol,
                    name: row.name,
                    market: row.market,
                    category_name: row.category_name,
                    category_color: row.category_color,
                    shares: row.shares,
                    avg_cost: row.avg_cost,
                    current_price,
                    market_value,
                    cost_value,
                    pnl,
                    pnl_percent,
                    daily_pnl: row.shares * change,
                    currency: row.currency,
                    market_value_usd: market_value,
                }
            })
            .collect();

        Ok(Self {
            holdings,
            missing_quote_keys,
            category_ids_by_holding,
            quote_warning: quote_result.warning,
            quotes_refreshed: quote_result.did_refresh,
        })
    }

    pub fn holdings(&self) -> &[HoldingDetail] {
        &self.holdings
    }

    pub fn missing_quote_keys(&self) -> &HashSet<(String, String)> {
        &self.missing_quote_keys
    }

    pub fn category_id_for_holding(&self, holding_id: &str) -> Option<&str> {
        self.category_ids_by_holding
            .get(holding_id)
            .and_then(Option::as_deref)
    }

    pub fn holdings_with_usd(&self, rates: &ExchangeRates) -> Vec<HoldingDetail> {
        self.holdings
            .iter()
            .cloned()
            .map(|mut holding| {
                holding.market_value_usd =
                    convert_currency(holding.market_value, &holding.currency, "USD", rates);
                holding
            })
            .collect()
    }

    pub fn dashboard_report(&self, rates: ExchangeRates, base_currency: String) -> DashboardReport {
        let holdings = self.holdings_with_usd(&rates);
        let mut us_market_value = 0.0;
        let mut cn_market_value = 0.0;
        let mut hk_market_value = 0.0;
        let mut total_cost = 0.0;

        for holding in &self.holdings {
            let market_value = convert_currency(
                holding.market_value,
                &holding.currency,
                &base_currency,
                &rates,
            );
            let cost_value = convert_currency(
                holding.cost_value,
                &holding.currency,
                &base_currency,
                &rates,
            );
            match holding.market.as_str() {
                "US" => us_market_value += market_value,
                "CN" => cn_market_value += market_value,
                "HK" => hk_market_value += market_value,
                _ => {}
            }
            total_cost += cost_value;
        }

        let total_market_value = us_market_value + cn_market_value + hk_market_value;
        let total_pnl = total_market_value - total_cost;
        let total_pnl_percent = if total_cost != 0.0 {
            total_pnl / total_cost * 100.0
        } else {
            0.0
        };
        let daily_pnl = self
            .holdings
            .iter()
            .map(|holding| {
                convert_currency(holding.daily_pnl, &holding.currency, &base_currency, &rates)
            })
            .sum();

        DashboardReport {
            summary: DashboardSummary {
                total_market_value,
                total_cost,
                total_pnl,
                total_pnl_percent,
                daily_pnl,
                us_market_value,
                cn_market_value,
                hk_market_value,
                exchange_rates: rates,
                base_currency,
            },
            holdings,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_holdings_for_test(holdings: Vec<HoldingDetail>) -> Self {
        Self {
            category_ids_by_holding: holdings
                .iter()
                .map(|holding| (holding.id.clone(), None))
                .collect(),
            holdings,
            missing_quote_keys: HashSet::new(),
            quote_warning: None,
            quotes_refreshed: false,
        }
    }

    pub fn quote_warning(&self) -> Option<&str> {
        self.quote_warning.as_deref()
    }

    pub fn quotes_refreshed(&self) -> bool {
        self.quotes_refreshed
    }
}

#[cfg(test)]
mod tests {
    use super::{PortfolioReadModel, QuoteReadMode};
    use crate::db::Database;
    use crate::models::{ExchangeRates, StockQuote};
    use crate::services::quote_service::{QuoteCache, QuoteServiceState};

    fn seeded_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
             VALUES ('acct-us', 'US Broker', 'US', '', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at)
             VALUES ('growth', '成长', '#1677ff', '', 0, 0, '2026-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO holdings
             (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
             VALUES ('holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', 'growth', 10, 10, 'USD', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        drop(conn);
        db
    }

    fn cached_aapl() -> StockQuote {
        StockQuote {
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            current_price: 12.0,
            previous_close: 11.0,
            change: 1.0,
            change_percent: 100.0 / 11.0,
            updated_at: "2026-09-02T09:30:00Z".to_string(),
            ..StockQuote::default()
        }
    }

    #[tokio::test]
    async fn cache_only_builds_holding_details_without_quote_state() {
        let db = seeded_db();
        let cache = QuoteCache::new();
        cache.set(cached_aapl());

        let model = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
            .await
            .unwrap();

        assert_eq!(model.holdings().len(), 1);
        let holding = &model.holdings()[0];
        assert_eq!(holding.account_name, "US Broker");
        assert_eq!(holding.category_name, "成长");
        assert_eq!(holding.current_price, 12.0);
        assert_eq!(holding.market_value, 120.0);
        assert_eq!(holding.cost_value, 100.0);
        assert_eq!(holding.pnl, 20.0);
        assert_eq!(holding.daily_pnl, 10.0);
    }

    #[tokio::test]
    async fn refresh_missing_requires_quote_state() {
        let db = seeded_db();
        let cache = QuoteCache::new();
        let error = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::RefreshMissing)
            .await
            .unwrap_err();
        assert!(error.contains("quote service state is required"));
    }

    #[tokio::test]
    async fn refresh_missing_succeeds_without_network_when_every_quote_is_cached() {
        let db = seeded_db();
        let cache = QuoteCache::new();
        cache.set(cached_aapl());
        let quote_state = QuoteServiceState::new();

        let model = PortfolioReadModel::load(
            &db,
            &cache,
            Some(&quote_state),
            QuoteReadMode::RefreshMissing,
        )
        .await
        .unwrap();

        assert_eq!(model.holdings().len(), 1);
        assert_eq!(model.holdings()[0].current_price, 12.0);
        assert_eq!(model.holdings()[0].daily_pnl, 10.0);
    }

    #[tokio::test]
    async fn refresh_missing_fetches_only_uncached_symbols_and_writes_fresh_quotes_to_cache() {
        let db = seeded_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-cash-usd', 'acct-us', '$CASH-USD', 'USD Cash', 'US', NULL, 2, 1, 'USD', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-cash-cny', 'acct-us', '$CASH-CNY', 'CNY Cash', 'CN', NULL, 3, 1, 'CNY', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
        }
        let cache = QuoteCache::new();
        cache.set(cached_aapl());
        cache.set(StockQuote {
            symbol: "$CASH-USD".to_string(),
            name: "cached USD cash".to_string(),
            market: "US".to_string(),
            current_price: 7.0,
            ..StockQuote::default()
        });
        let quote_state = QuoteServiceState::new();

        let model = PortfolioReadModel::load(
            &db,
            &cache,
            Some(&quote_state),
            QuoteReadMode::RefreshMissing,
        )
        .await
        .unwrap();

        let cached_cash = model
            .holdings()
            .iter()
            .find(|holding| holding.symbol == "$CASH-USD")
            .unwrap();
        let fetched_cash = model
            .holdings()
            .iter()
            .find(|holding| holding.symbol == "$CASH-CNY")
            .unwrap();
        assert_eq!(cached_cash.current_price, 7.0);
        assert_eq!(cached_cash.market_value, 14.0);
        assert_eq!(fetched_cash.current_price, 1.0);
        assert_eq!(fetched_cash.market_value, 3.0);
        assert_eq!(cache.get("$CASH-USD").unwrap().current_price, 7.0);
        assert_eq!(cache.get("$CASH-CNY").unwrap().current_price, 1.0);
        assert_eq!(cache.get("AAPL").unwrap().current_price, 12.0);
    }

    #[tokio::test]
    async fn loader_preserves_missing_quote_category_order_and_zero_cost_semantics() {
        let db = seeded_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-cn', 'CN Broker', 'CN', '', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-cn-missing', 'acct-cn', '600000', 'Pudong Bank', 'CN', NULL, 10, 8, 'CNY', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-free', 'acct-us', 'FREE', 'Free Lot', 'US', 'growth', 10, 0, 'USD', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
        }
        let cache = QuoteCache::new();
        cache.set(cached_aapl());
        cache.set(StockQuote {
            symbol: "FREE".to_string(),
            name: "Free Lot".to_string(),
            market: "US".to_string(),
            current_price: 5.0,
            change: 0.5,
            ..StockQuote::default()
        });

        let model = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
            .await
            .unwrap();
        let holdings = model.holdings();

        assert_eq!(
            holdings
                .iter()
                .map(|holding| holding.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["600000", "AAPL", "FREE"]
        );
        let missing = &holdings[0];
        assert_eq!(missing.category_name, "未分类");
        assert_eq!(missing.category_color, "#8B8B8B");
        assert_eq!(missing.current_price, 0.0);
        assert_eq!(missing.market_value, 0.0);
        assert_eq!(missing.pnl, -80.0);
        assert_eq!(missing.pnl_percent, Some(-100.0));
        assert_eq!(missing.daily_pnl, 0.0);

        let free = &holdings[2];
        assert_eq!(free.cost_value, 0.0);
        assert_eq!(free.market_value, 50.0);
        assert_eq!(free.pnl, 50.0);
        assert_eq!(free.pnl_percent, None);
        assert_eq!(free.daily_pnl, 5.0);
    }

    #[tokio::test]
    async fn holdings_with_usd_normalizes_value_without_overwriting_native_value() {
        let db = seeded_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-cn', 'CN Broker', 'CN', '', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-cn', 'acct-cn', '600519', 'Kweichow Moutai', 'CN', 'growth', 100, 8, 'CNY', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
        }
        let cache = QuoteCache::new();
        cache.set(StockQuote {
            symbol: "600519".to_string(),
            name: "Kweichow Moutai".to_string(),
            market: "CN".to_string(),
            current_price: 10.0,
            ..StockQuote::default()
        });
        let model = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
            .await
            .unwrap();
        let rates = ExchangeRates {
            usd_cny: 5.0,
            usd_hkd: 7.8,
            cny_hkd: 1.56,
            updated_at: "2026-09-02T09:30:00Z".to_string(),
        };

        let holdings = model.holdings_with_usd(&rates);
        let holding = holdings
            .iter()
            .find(|holding| holding.id == "holding-cn")
            .unwrap();
        assert_eq!(holding.market_value, 1_000.0);
        assert_eq!(holding.market_value_usd, 200.0);
    }

    #[tokio::test]
    async fn dashboard_report_uses_one_model_for_summary_and_holdings() {
        let db = seeded_db();
        let cache = QuoteCache::new();
        cache.set(cached_aapl());
        let model = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
            .await
            .unwrap();
        let rates = ExchangeRates {
            usd_cny: 5.0,
            usd_hkd: 7.8,
            cny_hkd: 1.56,
            updated_at: "2026-09-02T09:30:00Z".to_string(),
        };

        let report = model.dashboard_report(rates, "USD".to_string());

        assert_eq!(report.summary.total_market_value, 120.0);
        assert_eq!(report.summary.total_cost, 100.0);
        assert_eq!(report.summary.total_pnl, 20.0);
        assert_eq!(report.summary.daily_pnl, 10.0);
        assert_eq!(report.holdings.len(), 1);
        assert_eq!(report.holdings[0].market_value_usd, 120.0);
    }

    #[test]
    fn quote_lookup_keeps_identical_symbols_in_different_markets_separate() {
        let quotes = vec![
            StockQuote {
                symbol: "SAME".to_string(),
                market: "US".to_string(),
                current_price: 10.0,
                change: 1.0,
                ..StockQuote::default()
            },
            StockQuote {
                symbol: " same ".to_string(),
                market: "CN".to_string(),
                current_price: 20.0,
                change: 2.0,
                ..StockQuote::default()
            },
        ];

        let lookup = super::quote_values_by_market_and_symbol(&quotes);

        assert_eq!(
            lookup.get(&("US".to_string(), "SAME".to_string())),
            Some(&(10.0, 1.0))
        );
        assert_eq!(
            lookup.get(&("CN".to_string(), "SAME".to_string())),
            Some(&(20.0, 2.0))
        );
    }
}
