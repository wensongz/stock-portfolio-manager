use crate::db::Database;
use crate::models::{
    AccountStatistics, CategoryStatistics, MarketStatistics, PieSlice, PnlItem,
    StatisticsOverview,
};
use crate::services::exchange_rate_service::{convert_currency, get_cached_rates, ExchangeRateCache};
use crate::commands::dashboard::build_holding_details_pub;
use tauri::State;

fn to_usd_value(amount: f64, currency: &str, rates: &crate::models::ExchangeRates) -> f64 {
    convert_currency(amount, currency, "USD", rates)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_statistics_overview(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
) -> Result<StatisticsOverview, String> {
    let rates = get_cached_rates(&cache).await.unwrap_or_else(|_| crate::models::ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: chrono::Utc::now().to_rfc3339(),
    });

    let details = build_holding_details_pub(&db).await?;

    // Aggregate market distribution (values in USD for comparison)
    let mut market_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> = std::collections::HashMap::new();
    let mut account_map: std::collections::HashMap<(String, String), f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    for d in &details {
        let mv_usd = to_usd_value(d.market_value, &d.currency, &rates);
        let cv_usd = to_usd_value(d.cost_value, &d.currency, &rates);

        *market_map.entry(d.market.clone()).or_insert(0.0) += mv_usd;
        *category_map
            .entry((d.category_name.clone(), Some(d.category_color.clone())))
            .or_insert(0.0) += mv_usd;
        *account_map
            .entry((d.account_id.clone(), d.account_name.clone()))
            .or_insert(0.0) += mv_usd;

        total_market_value += mv_usd;
        total_cost += cv_usd;
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

    // Top gainers/losers by absolute PnL (in USD)
    let mut pnl_items: Vec<PnlItem> = details
        .iter()
        .map(|d| {
            let pnl_usd = to_usd_value(d.pnl, &d.currency, &rates);
            let mv_usd = to_usd_value(d.market_value, &d.currency, &rates);
            PnlItem {
                symbol: d.symbol.clone(),
                name: d.name.clone(),
                pnl: pnl_usd,
                pnl_percent: d.pnl_percent,
                market_value: mv_usd,
            }
        })
        .collect();
    pnl_items.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));

    let top_gainers: Vec<PnlItem> = pnl_items.iter().take(5).cloned().collect();
    let top_losers: Vec<PnlItem> = {
        let mut losers = pnl_items.clone();
        losers.sort_by(|a, b| a.pnl.partial_cmp(&b.pnl).unwrap_or(std::cmp::Ordering::Equal));
        losers.into_iter().take(5).collect()
    };

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
        top_gainers,
        top_losers,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_statistics_by_market(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    market: String,
) -> Result<MarketStatistics, String> {
    let rates = get_cached_rates(&cache).await.unwrap_or_else(|_| crate::models::ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: chrono::Utc::now().to_rfc3339(),
    });

    let all_details = build_holding_details_pub(&db).await?;
    let details: Vec<_> = all_details
        .into_iter()
        .filter(|d| d.market == market)
        .collect();

    let mut account_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> = std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    for d in &details {
        let mv_usd = to_usd_value(d.market_value, &d.currency, &rates);
        let cv_usd = to_usd_value(d.cost_value, &d.currency, &rates);

        *account_map.entry(d.account_name.clone()).or_insert(0.0) += mv_usd;
        *category_map
            .entry((d.category_name.clone(), Some(d.category_color.clone())))
            .or_insert(0.0) += mv_usd;
        *stock_map
            .entry(format!("{} {}", d.symbol, d.name))
            .or_insert(0.0) += mv_usd;

        total_market_value += mv_usd;
        total_cost += cv_usd;
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

#[tauri::command(rename_all = "snake_case")]
pub async fn get_statistics_by_account(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    account_id: String,
) -> Result<AccountStatistics, String> {
    let rates = get_cached_rates(&cache).await.unwrap_or_else(|_| crate::models::ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: chrono::Utc::now().to_rfc3339(),
    });

    let all_details = build_holding_details_pub(&db).await?;
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

    for d in &details {
        let mv_usd = to_usd_value(d.market_value, &d.currency, &rates);
        let cv_usd = to_usd_value(d.cost_value, &d.currency, &rates);

        *category_map
            .entry((d.category_name.clone(), Some(d.category_color.clone())))
            .or_insert(0.0) += mv_usd;
        *stock_map
            .entry(format!("{} {}", d.symbol, d.name))
            .or_insert(0.0) += mv_usd;

        total_market_value += mv_usd;
        total_cost += cv_usd;
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

#[tauri::command(rename_all = "snake_case")]
pub async fn get_statistics_by_category(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    category_id: String,
) -> Result<CategoryStatistics, String> {
    let rates = get_cached_rates(&cache).await.unwrap_or_else(|_| crate::models::ExchangeRates {
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

    let all_details = build_holding_details_pub(&db).await?;
    let details: Vec<_> = all_details
        .into_iter()
        .filter(|d| d.category_name == cat_name)
        .collect();

    let mut market_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    let mut total_market_value = 0.0f64;
    let mut total_cost = 0.0f64;

    for d in &details {
        let mv_usd = to_usd_value(d.market_value, &d.currency, &rates);
        let cv_usd = to_usd_value(d.cost_value, &d.currency, &rates);

        let market_label = match d.market.as_str() {
            "US" => "🇺🇸 美股",
            "CN" => "🇨🇳 A股",
            "HK" => "🇭🇰 港股",
            _ => d.market.as_str(),
        };
        *market_map.entry(market_label.to_string()).or_insert(0.0) += mv_usd;

        total_market_value += mv_usd;
        total_cost += cv_usd;
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
