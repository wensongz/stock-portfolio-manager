use crate::db::Database;
use crate::models::{
    AccountStatistics, CategoryStatistics, MarketStatistics, PieSlice, PnlItem,
    StatisticsOverview,
};
use crate::services::exchange_rate_service::{convert_currency, get_cached_rates, ExchangeRateCache};
use crate::services::quote_service::QuoteCache;
use crate::commands::dashboard::build_holding_details_pub;
use tauri::State;

fn to_usd_value(amount: f64, currency: &str, rates: &crate::models::ExchangeRates) -> f64 {
    convert_currency(amount, currency, "USD", rates)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_overview(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    base_currency: Option<String>,
) -> Result<StatisticsOverview, String> {
    let base = base_currency.unwrap_or_else(|| "USD".to_string());
    let rates = get_cached_rates(&cache, &db).await.unwrap_or_else(|_| crate::models::ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: chrono::Utc::now().to_rfc3339(),
    });

    let details = build_holding_details_pub(&db, &quote_cache, true).await?;

    // Aggregate distribution values in the requested base currency
    let mut market_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> = std::collections::HashMap::new();
    let mut account_map: std::collections::HashMap<(String, String), f64> = std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    for d in &details {
        let mv_base = convert_currency(d.market_value, &d.currency, &base, &rates);
        let cv_base = convert_currency(d.cost_value, &d.currency, &base, &rates);

        *market_map.entry(d.market.clone()).or_insert(0.0) += mv_base;
        *category_map
            .entry((d.category_name.clone(), Some(d.category_color.clone())))
            .or_insert(0.0) += mv_base;
        *account_map
            .entry((d.account_id.clone(), d.account_name.clone()))
            .or_insert(0.0) += mv_base;
        *stock_map
            .entry(format!("{} {}", d.symbol, d.name))
            .or_insert(0.0) += mv_base;

        total_market_value += mv_base;
        total_cost += cv_base;
    }

    let market_label = |m: &str| match m {
        "US" => "🇺🇸 美股".to_string(),
        "CN" => "🇨🇳 A股".to_string(),
        "HK" => "🇭🇰 港股".to_string(),
        _ => m.to_string(),
    };

    let mut market_distribution: Vec<PieSlice> = market_map
        .into_iter()
        .map(|(k, v)| PieSlice {
            name: market_label(&k),
            value: v,
            color: None,
        })
        .collect();
    market_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let mut category_distribution: Vec<PieSlice> = category_map
        .into_iter()
        .map(|((name, color), v)| PieSlice {
            name,
            value: v,
            color,
        })
        .collect();
    category_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let mut account_distribution: Vec<PieSlice> = account_map
        .into_iter()
        .map(|((_, name), v)| PieSlice {
            name,
            value: v,
            color: None,
        })
        .collect();
    account_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let mut stock_distribution: Vec<PieSlice> = stock_map
        .into_iter()
        .map(|(k, v)| PieSlice { name: k, value: v, color: None })
        .collect();
    stock_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    // Top gainers/losers: aggregate by symbol in base currency so a stock held across
    // multiple accounts/currencies counts as a single entry.
    struct SymbolAgg {
        name: String,
        pnl_base: f64,
        cost_base: f64,
        market_value_base: f64,
    }
    let mut symbol_map: std::collections::HashMap<String, SymbolAgg> = std::collections::HashMap::new();
    for d in &details {
        let pnl_base = convert_currency(d.pnl, &d.currency, &base, &rates);
        let cv_base = convert_currency(d.cost_value, &d.currency, &base, &rates);
        let mv_base = convert_currency(d.market_value, &d.currency, &base, &rates);
        let entry = symbol_map.entry(d.symbol.clone()).or_insert_with(|| SymbolAgg {
            name: d.name.clone(),
            pnl_base: 0.0,
            cost_base: 0.0,
            market_value_base: 0.0,
        });
        entry.pnl_base += pnl_base;
        entry.cost_base += cv_base;
        entry.market_value_base += mv_base;
    }
    let mut pnl_items: Vec<PnlItem> = symbol_map
        .into_iter()
        .map(|(symbol, agg)| {
            let pnl_percent = if agg.cost_base != 0.0 {
                agg.pnl_base / agg.cost_base * 100.0
            } else {
                0.0
            };
            PnlItem {
                symbol,
                name: agg.name,
                pnl: agg.pnl_base,
                pnl_percent,
                market_value: agg.market_value_base,
            }
        })
        .collect();
    pnl_items.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));

    let top_gainers: Vec<PnlItem> = pnl_items.iter().filter(|i| i.pnl > 0.0).take(5).cloned().collect();
    let top_losers: Vec<PnlItem> = pnl_items.iter().rev().filter(|i| i.pnl < 0.0).take(5).cloned().collect();

    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    Ok(StatisticsOverview {
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        market_distribution,
        category_distribution,
        account_distribution,
        stock_distribution,
        top_gainers,
        top_losers,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_by_market(
    db: State<'_, Database>,
    _cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    market: String,
) -> Result<MarketStatistics, String> {
    let all_details = build_holding_details_pub(&db, &quote_cache, true).await?;
    let details: Vec<_> = all_details
        .into_iter()
        .filter(|d| d.market == market)
        .collect();

    let mut account_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> = std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    // Use native currency values directly (CN→CNY, HK→HKD, US→USD)
    for d in &details {
        let mv = d.market_value;
        let cv = d.cost_value;

        *account_map.entry(d.account_name.clone()).or_insert(0.0) += mv;
        *category_map
            .entry((d.category_name.clone(), Some(d.category_color.clone())))
            .or_insert(0.0) += mv;
        *stock_map
            .entry(format!("{} {}", d.symbol, d.name))
            .or_insert(0.0) += mv;

        total_market_value += mv;
        total_cost += cv;
    }

    let mut account_distribution: Vec<PieSlice> = account_map
        .into_iter()
        .map(|(k, v)| PieSlice { name: k, value: v, color: None })
        .collect();
    account_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let mut category_distribution: Vec<PieSlice> = category_map
        .into_iter()
        .map(|((name, color), v)| PieSlice { name, value: v, color })
        .collect();
    category_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let mut stock_distribution: Vec<PieSlice> = stock_map
        .into_iter()
        .map(|(k, v)| PieSlice { name: k, value: v, color: None })
        .collect();
    stock_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    Ok(MarketStatistics {
        market,
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        account_distribution,
        category_distribution,
        stock_distribution,
        holdings: details,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_statistics_by_account(
    db: State<'_, Database>,
    _cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    account_id: String,
) -> Result<AccountStatistics, String> {
    let all_details = build_holding_details_pub(&db, &quote_cache, true).await?;
    let details: Vec<_> = all_details
        .into_iter()
        .filter(|d| d.account_id == account_id)
        .collect();

    let account_name = details.first().map(|d| d.account_name.clone()).unwrap_or_default();
    let market = details.first().map(|d| d.market.clone()).unwrap_or_default();

    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> = std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    // Use native currency values directly (same market within one account)
    for d in &details {
        let mv = d.market_value;
        let cv = d.cost_value;

        *category_map
            .entry((d.category_name.clone(), Some(d.category_color.clone())))
            .or_insert(0.0) += mv;
        *stock_map
            .entry(format!("{} {}", d.symbol, d.name))
            .or_insert(0.0) += mv;

        total_market_value += mv;
        total_cost += cv;
    }

    let mut category_distribution: Vec<PieSlice> = category_map
        .into_iter()
        .map(|((name, color), v)| PieSlice { name, value: v, color })
        .collect();
    category_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let mut stock_distribution: Vec<PieSlice> = stock_map
        .into_iter()
        .map(|(k, v)| PieSlice { name: k, value: v, color: None })
        .collect();
    stock_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    Ok(AccountStatistics {
        account_id,
        account_name,
        market,
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        category_distribution,
        stock_distribution,
        holdings: details,
    })
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
    let rates = get_cached_rates(&cache, &db).await.unwrap_or_else(|_| crate::models::ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: chrono::Utc::now().to_rfc3339(),
    });

    // We need to look up holdings by category_id (not name), so query DB directly
    struct CategoryRow {
        id: String,
        name: String,
        color: String,
    }

    let cat: Option<CategoryRow> = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT id, name, color FROM categories WHERE id = ?1",
            rusqlite::params![category_id],
            |row| Ok(CategoryRow { id: row.get(0)?, name: row.get(1)?, color: row.get(2)? }),
        )
        .ok()
    };

    let (cat_id, cat_name, cat_color) = match cat {
        Some(c) => (c.id, c.name, c.color),
        None => (category_id.clone(), "未分类".to_string(), "#8B8B8B".to_string()),
    };

    let all_details = build_holding_details_pub(&db, &quote_cache, true).await?;
    let mut details: Vec<_> = all_details
        .into_iter()
        .filter(|d| d.category_name == cat_name)
        .collect();

    // Normalise market_value_usd so holdings across multiple markets/currencies
    // can be sorted on a common basis.
    for d in &mut details {
        d.market_value_usd = to_usd_value(d.market_value, &d.currency, &rates);
    }

    let mut market_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    for d in &details {
        let mv_base = convert_currency(d.market_value, &d.currency, &base, &rates);
        let cv_base = convert_currency(d.cost_value, &d.currency, &base, &rates);

        let market_label = match d.market.as_str() {
            "US" => "🇺🇸 美股",
            "CN" => "🇨🇳 A股",
            "HK" => "🇭🇰 港股",
            _ => d.market.as_str(),
        };
        *market_map.entry(market_label.to_string()).or_insert(0.0) += mv_base;

        total_market_value += mv_base;
        total_cost += cv_base;
    }

    let mut market_distribution: Vec<PieSlice> = market_map
        .into_iter()
        .map(|(k, v)| PieSlice { name: k, value: v, color: None })
        .collect();
    market_distribution.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    Ok(CategoryStatistics {
        category_id: cat_id,
        category_name: cat_name,
        category_color: cat_color,
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        market_distribution,
        holdings: details,
    })
}
