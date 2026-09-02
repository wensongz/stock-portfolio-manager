use crate::db::Database;
use crate::models::{AccountStatistics, CategoryStatistics, MarketStatistics, StatisticsOverview};
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
}

fn load_category(db: &Database, category_id: &str) -> Result<Option<CategoryRow>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    conn.query_row(
        "SELECT id, name, color FROM categories WHERE id = ?1",
        rusqlite::params![category_id],
        |row| {
            Ok(CategoryRow {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
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
    _cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    account_id: String,
) -> Result<AccountStatistics, String> {
    let model = PortfolioReadModel::load(&db, &quote_cache, None, QuoteReadMode::CacheOnly).await?;
    Ok(statistics_service::by_account(&model, &account_id))
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
    let (category_id, category_name, category_color) = match category {
        Some(category) => (category.id, category.name, category.color),
        None => (category_id, "未分类".to_string(), "#8B8B8B".to_string()),
    };
    let model = PortfolioReadModel::load(&db, &quote_cache, None, QuoteReadMode::CacheOnly).await?;
    Ok(statistics_service::by_category(
        &model,
        &rates,
        &base,
        &category_id,
        &category_name,
        &category_color,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
