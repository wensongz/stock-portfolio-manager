use crate::db::Database;
use crate::models::{
    DashboardSummary, ExchangeRates, HoldingDetail,
};
use crate::services::exchange_rate_service::{convert_currency, get_cached_rates, ExchangeRateCache};
use crate::services::quote_service::fetch_quotes_batch;
use tauri::State;

/// Build HoldingDetail records from raw holdings + quotes + account/category lookups.
/// This is the shared implementation; call `build_holding_details_pub` from other modules.
pub async fn build_holding_details_pub(
    db: &Database,
) -> Result<Vec<HoldingDetail>, String> {
    build_holding_details(db).await
}

async fn build_holding_details(
    db: &Database,
) -> Result<Vec<HoldingDetail>, String> {
    // Load holdings and lookup data in one DB operation
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
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
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
            .map_err(|e| e.to_string())?;
        let result = stmt
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
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        result
    };

    if rows.is_empty() {
        return Ok(vec![]);
    }

    // Fetch quotes
    let symbols: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.symbol.clone(), r.market.clone()))
        .collect();
    let quotes = fetch_quotes_batch(symbols).await?;
    let quote_map: std::collections::HashMap<String, f64> = quotes
        .into_iter()
        .map(|q| (q.symbol.clone(), q.current_price))
        .collect();

    let details = rows
        .into_iter()
        .map(|r| {
            let current_price = *quote_map.get(&r.symbol).unwrap_or(&0.0);
            let market_value = r.shares * current_price;
            let cost_value = r.shares * r.avg_cost;
            let pnl = market_value - cost_value;
            let pnl_percent = if cost_value != 0.0 {
                pnl / cost_value * 100.0
            } else {
                0.0
            };
            HoldingDetail {
                id: r.id,
                account_id: r.account_id,
                account_name: r.account_name,
                symbol: r.symbol,
                name: r.name,
                market: r.market,
                category_name: r.category_name,
                category_color: r.category_color,
                shares: r.shares,
                avg_cost: r.avg_cost,
                current_price,
                market_value,
                cost_value,
                pnl,
                pnl_percent,
                currency: r.currency,
            }
        })
        .collect();

    Ok(details)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_holdings_with_quotes(
    db: State<'_, Database>,
) -> Result<Vec<HoldingDetail>, String> {
    build_holding_details(&db).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_dashboard_summary(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    base_currency: Option<String>,
) -> Result<DashboardSummary, String> {
    let base = base_currency.unwrap_or_else(|| "USD".to_string());

    let rates: ExchangeRates = get_cached_rates(&cache).await.unwrap_or_else(|_| ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: chrono::Utc::now().to_rfc3339(),
    });

    let details = build_holding_details(&db).await?;

    let mut us_market_value = 0.0f64;
    let mut cn_market_value = 0.0f64;
    let mut hk_market_value = 0.0f64;
    let mut total_cost_base = 0.0f64;

    for d in &details {
        let mv_base = convert_currency(d.market_value, &d.currency, &base, &rates);
        let cv_base = convert_currency(d.cost_value, &d.currency, &base, &rates);
        match d.market.as_str() {
            "US" => us_market_value += mv_base,
            "CN" => cn_market_value += mv_base,
            "HK" => hk_market_value += mv_base,
            _ => {}
        }
        total_cost_base += cv_base;
    }

    let total_market_value = us_market_value + cn_market_value + hk_market_value;
    let total_pnl = total_market_value - total_cost_base;
    let total_pnl_percent = if total_cost_base != 0.0 {
        total_pnl / total_cost_base * 100.0
    } else {
        0.0
    };

    // Daily PnL: difference between today's total value and yesterday's snapshot
    let daily_pnl: f64 = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let prev: f64 = conn
            .query_row(
                "SELECT COALESCE(total_value, 0) FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
                rusqlite::params![today],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        // prev is in USD; convert to base currency
        let prev_base = convert_currency(prev, "USD", &base, &rates);
        total_market_value - prev_base
    };

    Ok(DashboardSummary {
        total_market_value,
        total_cost: total_cost_base,
        total_pnl,
        total_pnl_percent,
        daily_pnl,
        us_market_value,
        cn_market_value,
        hk_market_value,
        exchange_rates: rates,
        base_currency: base,
    })
}
