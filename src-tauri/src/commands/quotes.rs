use crate::db::Database;
use crate::models::{HoldingWithQuote, StockQuote};
use crate::services::quote_service::{fetch_cn_quote, fetch_hk_quote, fetch_us_quote, fetch_quotes_batch_cached, QuoteCache};
use tauri::State;

#[tauri::command(rename_all = "snake_case")]
pub async fn get_real_time_quotes(
    quote_cache: State<'_, QuoteCache>,
    symbols: Vec<(String, String)>,
) -> Result<Vec<StockQuote>, String> {
    fetch_quotes_batch_cached(&quote_cache, symbols).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_holding_quotes(db: State<'_, Database>, quote_cache: State<'_, QuoteCache>) -> Result<Vec<HoldingWithQuote>, String> {
    // Load holdings from DB (synchronous)
    let holdings = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id, symbol, name, market, category_id,
                        shares, avg_cost, currency, created_at, updated_at
                 FROM holdings ORDER BY market, symbol",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::models::Holding {
                id: row.get(0)?,
                account_id: row.get(1)?,
                symbol: row.get(2)?,
                name: row.get(3)?,
                market: row.get(4)?,
                category_id: row.get(5)?,
                shares: row.get(6)?,
                avg_cost: row.get(7)?,
                currency: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
        let result = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        result
    };

    // Fetch quotes for all holdings
    let symbols: Vec<(String, String)> = holdings
        .iter()
        .map(|h| (h.symbol.clone(), h.market.clone()))
        .collect();
    let quotes = fetch_quotes_batch_cached(&quote_cache, symbols).await?;
    let quote_map: std::collections::HashMap<String, StockQuote> = quotes
        .into_iter()
        .map(|q| (q.symbol.clone(), q))
        .collect();

    let result = holdings
        .into_iter()
        .map(|h| {
            let quote = quote_map.get(&h.symbol).cloned();
            let market_value = quote.as_ref().map(|q| q.current_price * h.shares);
            let total_cost = Some(h.avg_cost * h.shares);
            let unrealized_pnl = market_value.zip(total_cost).map(|(mv, tc)| mv - tc);
            let unrealized_pnl_percent = unrealized_pnl.zip(total_cost).and_then(|(pnl, tc)| {
                if tc != 0.0 {
                    Some(pnl / tc * 100.0)
                } else {
                    None
                }
            });
            HoldingWithQuote {
                id: h.id,
                account_id: h.account_id,
                symbol: h.symbol,
                name: h.name,
                market: h.market,
                category_id: h.category_id,
                shares: h.shares,
                avg_cost: h.avg_cost,
                currency: h.currency,
                created_at: h.created_at,
                updated_at: h.updated_at,
                quote,
                market_value,
                total_cost,
                unrealized_pnl,
                unrealized_pnl_percent,
            }
        })
        .collect();

    Ok(result)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_us_quote(quote_cache: State<'_, QuoteCache>, symbol: String) -> Result<StockQuote, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return Ok(cached);
    }
    let quote = fetch_us_quote(&symbol).await?;
    quote_cache.set(quote.clone());
    Ok(quote)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_hk_quote(quote_cache: State<'_, QuoteCache>, symbol: String) -> Result<StockQuote, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return Ok(cached);
    }
    let quote = fetch_hk_quote(&symbol).await?;
    quote_cache.set(quote.clone());
    Ok(quote)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_cn_quote(quote_cache: State<'_, QuoteCache>, symbol: String) -> Result<StockQuote, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return Ok(cached);
    }
    let quote = fetch_cn_quote(&symbol).await?;
    quote_cache.set(quote.clone());
    Ok(quote)
}
