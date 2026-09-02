use crate::db::Database;
use crate::models::{DashboardReport, DashboardSummary, ExchangeRates, HoldingDetail};
use crate::services::exchange_rate_service::convert_currency;
use crate::services::quote_provider_service;
use crate::services::quote_service::{
    fetch_quotes_batch_cached_with_providers, QuoteCache, QuoteServiceState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteReadMode {
    CacheOnly,
    RefreshMissing,
}

#[derive(Debug)]
pub struct PortfolioReadModel {
    holdings: Vec<HoldingDetail>,
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
                            h.shares, h.avg_cost, h.currency
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
                        shares: row.get(8)?,
                        avg_cost: row.get(9)?,
                        currency: row.get(10)?,
                    })
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            result
        };

        if rows.is_empty() {
            return Ok(Self { holdings: vec![] });
        }

        let symbols: Vec<(String, String)> = rows
            .iter()
            .map(|row| (row.symbol.clone(), row.market.clone()))
            .collect();
        let quotes = match mode {
            QuoteReadMode::CacheOnly => {
                let (cached, _missing) = quote_cache.get_batch(&symbols);
                cached
            }
            QuoteReadMode::RefreshMissing => {
                let config = quote_provider_service::get_quote_provider_config(db)?;
                let state = quote_state.ok_or_else(|| {
                    "quote service state is required when refreshing holding details".to_string()
                })?;
                fetch_quotes_batch_cached_with_providers(
                    state,
                    quote_cache,
                    symbols,
                    &config.us_provider,
                    &config.hk_provider,
                    &config.cn_provider,
                    false,
                )
                .await?
            }
        };
        let quote_map: std::collections::HashMap<String, (f64, f64)> = quotes
            .into_iter()
            .map(|quote| (quote.symbol.clone(), (quote.current_price, quote.change)))
            .collect();

        let holdings = rows
            .into_iter()
            .map(|row| {
                let (current_price, change) = *quote_map.get(&row.symbol).unwrap_or(&(0.0, 0.0));
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

        Ok(Self { holdings })
    }

    pub fn holdings(&self) -> &[HoldingDetail] {
        &self.holdings
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
        Self { holdings }
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
}
