use crate::db::Database;
use crate::models::{
    AccountStatistics, CategoryStatistics, ExchangeRates, MarketStatistics, StatisticsOverview,
};
use crate::services::exchange_rate_service::{get_cached_rates, ExchangeRateCache};
use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
use crate::services::quote_service::QuoteCache;
use crate::services::statistics_service;
use rusqlite::OptionalExtension;
use tauri::State;

struct CategoryRow {
    id: String,
    name: String,
    color: String,
    is_system: bool,
}

fn is_cash_category(category: &CategoryRow) -> bool {
    category.is_system && category.name == "现金类"
}

fn load_category(db: &Database, category_id: &str) -> Result<Option<CategoryRow>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    conn.query_row(
        "SELECT id, name, color, is_system FROM categories WHERE id = ?1",
        rusqlite::params![category_id],
        |row| {
            Ok(CategoryRow {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                is_system: row.get::<_, i32>(3)? != 0,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_overview(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    base_currency: Option<String>,
) -> Result<StatisticsOverview, String> {
    let base = base_currency.unwrap_or_else(|| "USD".to_string());
    let rates = get_cached_rates(&cache, &db).await?;
    let model = PortfolioReadModel::load(&db, &quote_cache, None, QuoteReadMode::CacheOnly).await?;
    Ok(statistics_service::overview(&model, &rates, &base))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_by_market(
    db: State<'_, Database>,
    _cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    market: String,
) -> Result<MarketStatistics, String> {
    let model = PortfolioReadModel::load(&db, &quote_cache, None, QuoteReadMode::CacheOnly).await?;
    Ok(statistics_service::by_market(&model, &market))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_by_account(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    account_id: String,
) -> Result<AccountStatistics, String> {
    load_account_statistics(
        db.inner(),
        quote_cache.inner(),
        &account_id,
        get_cached_rates(cache.inner(), db.inner()),
    )
    .await
}

async fn load_account_statistics(
    db: &Database,
    quote_cache: &QuoteCache,
    account_id: &str,
    rates: impl std::future::Future<Output = Result<ExchangeRates, String>>,
) -> Result<AccountStatistics, String> {
    let market: String = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        conn.query_row(
            "SELECT market FROM accounts WHERE id = ?1",
            [&account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?
    };
    let model = PortfolioReadModel::load(db, quote_cache, None, QuoteReadMode::CacheOnly).await?;
    let currency = statistics_service::account_currency(&market);
    let needs_conversion = model
        .holdings()
        .iter()
        .any(|detail| detail.account_id == account_id && detail.currency != currency);
    let rates = if needs_conversion {
        Some(rates.await?)
    } else {
        None
    };
    statistics_service::by_account(&model, rates.as_ref(), account_id, &market)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_by_category(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    category_id: String,
    base_currency: Option<String>,
) -> Result<CategoryStatistics, String> {
    let base = base_currency.unwrap_or_else(|| "USD".to_string());
    let rates = get_cached_rates(&cache, &db).await?;
    let category = load_category(&db, &category_id)?;
    let (category_id, category_name, category_color, is_cash_category) = match category {
        Some(category) => {
            let is_cash = is_cash_category(&category);
            (category.id, category.name, category.color, is_cash)
        }
        None => (
            category_id,
            "未分类".to_string(),
            "#8B8B8B".to_string(),
            false,
        ),
    };
    let model = PortfolioReadModel::load(&db, &quote_cache, None, QuoteReadMode::CacheOnly).await?;
    Ok(statistics_service::by_category(
        &model,
        &rates,
        &base,
        &category_id,
        &category_name,
        &category_color,
        is_cash_category,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_currency_and_empty_accounts_do_not_request_exchange_rates() {
        for (market, currency) in [("US", "USD"), ("HK", "HKD"), ("CN", "CNY")] {
            for balance in [None, Some(1000.0)] {
                let db = Database::new(":memory:").unwrap();
                {
                    let conn = db.conn.lock().unwrap();
                    conn.execute("INSERT INTO accounts(id,name,market,created_at,updated_at) VALUES('native','Native',?1,'2026-01-01','2026-01-01')", [market]).unwrap();
                    if let Some(amount) = balance {
                        conn.execute("INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('cash','native',?1,'Cash',?2,?3,1,?4,'2026-01-01','2026-01-01')", rusqlite::params![format!("$CASH-{currency}"),market,amount,currency]).unwrap();
                    }
                    // An unrelated foreign-currency account must not force an FX request.
                    conn.execute_batch("INSERT INTO accounts(id,name,market,created_at,updated_at) VALUES('other','Other','HK','2026-01-01','2026-01-01'); INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('other-cash','other','$CASH-HKD','Cash','HK',7800,1,'HKD','2026-01-01','2026-01-01');").unwrap();
                }
                let requested = std::cell::Cell::new(false);
                let result = load_account_statistics(&db, &QuoteCache::new(), "native", async {
                    requested.set(true);
                    Err("offline: no exchange-rate cache".to_string())
                })
                .await
                .expect("native amounts must work offline without cached rates");
                assert!(!requested.get(), "native statistics must not request FX");
                assert_eq!(result.total_market_value, balance.unwrap_or(0.0));
                assert_eq!(result.total_cost, balance.unwrap_or(0.0));
                assert_eq!(result.total_pnl, 0.0);
                assert_eq!(result.market, market);
            }
        }
    }

    #[tokio::test]
    async fn native_stock_and_cash_weights_remain_correct_without_fx() {
        for (market, currency) in [("US", "USD"), ("HK", "HKD"), ("CN", "CNY")] {
            let db = Database::new(":memory:").unwrap();
            {
                let conn = db.conn.lock().unwrap();
                conn.execute("INSERT INTO accounts(id,name,market,created_at,updated_at) VALUES('native','Native',?1,'2026-01-01','2026-01-01')", [market]).unwrap();
                conn.execute("INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('stock','native','STOCK','Stock',?1,10,10,?2,'2026-01-01','2026-01-01')",rusqlite::params![market,currency]).unwrap();
                conn.execute("INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('cash','native',?1,'Cash',?2,1000,1,?3,'2026-01-01','2026-01-01')",rusqlite::params![format!("$CASH-{currency}"),market,currency]).unwrap();
            }
            let quotes = QuoteCache::new();
            quotes.set(crate::models::StockQuote {
                symbol: "STOCK".into(),
                market: market.into(),
                current_price: 20.0,
                ..Default::default()
            });
            let result = load_account_statistics(&db, &quotes, "native", async {
                Err("offline".to_string())
            })
            .await
            .unwrap();
            assert_eq!(result.total_market_value, 1200.0);
            assert_eq!(result.total_cost, 1100.0);
            assert_eq!(result.total_pnl, 100.0);
            let total = result
                .holdings
                .iter()
                .map(|h| h.market_value_usd)
                .sum::<f64>();
            let stock = result.holdings.iter().find(|h| h.id == "stock").unwrap();
            assert!((stock.market_value_usd / total - 1.0 / 6.0).abs() < 1e-9);
        }
    }

    #[tokio::test]
    async fn foreign_currency_account_requires_rates_instead_of_guessing_offline() {
        let db = Database::new(":memory:").unwrap();
        db.conn.lock().unwrap().execute_batch("INSERT INTO accounts(id,name,market,created_at,updated_at) VALUES('mixed','Mixed','US','2026-01-01','2026-01-01'); INSERT INTO holdings(id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES('hkd','mixed','$CASH-HKD','HKD Cash','HK',7800,1,'HKD','2026-01-01','2026-01-01');").unwrap();
        let error = load_account_statistics(&db, &QuoteCache::new(), "mixed", async {
            Err("offline".to_string())
        })
        .await
        .unwrap_err();
        assert_eq!(error, "offline");
        let cache = ExchangeRateCache::new();
        cache.set(ExchangeRates {
            usd_cny: 7.0,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / 7.0,
            updated_at: "2026-11-03".into(),
        });
        let result = load_account_statistics(
            &db,
            &QuoteCache::new(),
            "mixed",
            get_cached_rates(&cache, &db),
        )
        .await
        .unwrap();
        assert!((result.total_market_value - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn category_lookup_distinguishes_missing_from_malformed_rows() {
        let db = Database::new(":memory:").unwrap();
        assert!(load_category(&db, "missing").unwrap().is_none());

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO categories
             (id, name, color, icon, is_system, sort_order, created_at)
             VALUES ('broken', 'Broken', X'FF', '', 0, 0, '2025-01-01')",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(load_category(&db, "broken").is_err());
    }

    #[test]
    fn only_the_system_cash_category_gets_cash_semantics() {
        let db = Database::new(":memory:").unwrap();
        let system_cash_id: String = {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO categories
                 (id, name, color, icon, is_system, sort_order, created_at)
                 VALUES ('custom-cash', '现金类', '#000000', '', 0, 100, '2025-01-01')",
                [],
            )
            .unwrap();
            conn.query_row(
                "SELECT id FROM categories WHERE name = '现金类' AND is_system = 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };

        assert!(is_cash_category(
            &load_category(&db, &system_cash_id).unwrap().unwrap()
        ));
        assert!(!is_cash_category(
            &load_category(&db, "custom-cash").unwrap().unwrap()
        ));
    }
}
