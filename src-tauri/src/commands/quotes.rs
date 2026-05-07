use crate::db::Database;
use crate::models::{HoldingWithQuote, StockQuote};
use crate::services::quote_service::{fetch_cn_quote_with_provider, fetch_hk_quote_with_provider, fetch_us_quote_with_provider, fetch_quotes_batch_cached_with_providers, save_quotes_to_db, QuoteCache, CASH_SYMBOL_PREFIX};
use crate::services::quote_provider_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn get_real_time_quotes(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    symbols: Vec<(String, String)>,
    force_refresh: Option<bool>,
) -> Result<Vec<StockQuote>, String> {
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    crate::services::quote_service::clear_quote_warning();
    let quotes = fetch_quotes_batch_cached_with_providers(&quote_cache, symbols, &config.us_provider, &config.hk_provider, &config.cn_provider, force_refresh.unwrap_or(false)).await?;
    // Persist freshly fetched quotes to the database
    if let Err(e) = save_quotes_to_db(&db, &quotes) {
        eprintln!("Failed to persist quotes to DB: {}", e);
    }
    Ok(quotes)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_holding_quotes(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    refresh_symbols: Option<Vec<(String, String)>>,
) -> Result<Vec<HoldingWithQuote>, String> {
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let should_refresh_from_api = match refresh_symbols.as_ref() {
        Some(symbols) => !symbols.is_empty(),
        None => true,
    };
    if should_refresh_from_api {
        crate::services::quote_service::clear_quote_warning();
    }
    // Load holdings from DB (synchronous) and pre-compute realized PnL for cleared positions.
    // realized_pnl_map: holding_id -> (realized_pnl, total_buy_cost)
    let (holdings, realized_pnl_map) = {
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
        let holdings = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        // For cleared (shares == 0) non-cash holdings, compute realized PnL from transactions:
        //   realized_pnl = SUM(SELL total_amount - commission) - SUM(BUY total_amount + commission)
        //   total_buy_cost = SUM(BUY total_amount + commission)  [used for % calculation]
        // OPEN transactions are excluded (no cash impact).
        let mut realized_pnl_map: std::collections::HashMap<String, (f64, f64)> =
            std::collections::HashMap::new();
        for h in &holdings {
            if h.shares == 0.0 && !h.symbol.starts_with(CASH_SYMBOL_PREFIX) {
                let pnl_data: (f64, f64) = conn
                    .query_row(
                        "SELECT
                            COALESCE(SUM(CASE
                                WHEN transaction_type = 'SELL' THEN total_amount - commission
                                WHEN transaction_type = 'BUY'  THEN -(total_amount + commission)
                                ELSE 0
                            END), 0.0),
                            COALESCE(SUM(CASE
                                WHEN transaction_type = 'BUY' THEN total_amount + commission
                                ELSE 0
                            END), 0.0)
                         FROM transactions WHERE holding_id = ?1",
                        rusqlite::params![h.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap_or((0.0, 0.0));
                realized_pnl_map.insert(h.id.clone(), pnl_data);
            }
        }

        (holdings, realized_pnl_map)
    };

    // Fetch quotes for all holdings.
    // When refresh_symbols is provided, only those symbols are force-refreshed
    // from the upstream API; all other quotes come from cache.
    // When refresh_symbols is None, ALL symbols are force-refreshed.
    let all_symbols: Vec<(String, String)> = holdings
        .iter()
        .map(|h| (h.symbol.clone(), h.market.clone()))
        .collect();

    let quotes = match refresh_symbols {
        Some(ref symbols) if !symbols.is_empty() => {
            // Targeted refresh: force-refresh only the specified symbols
            fetch_quotes_batch_cached_with_providers(
                &quote_cache, symbols.clone(),
                &config.us_provider, &config.hk_provider, &config.cn_provider, true,
            ).await?;
            // Then load all quotes from cache (the refreshed ones are now fresh)
            fetch_quotes_batch_cached_with_providers(
                &quote_cache, all_symbols,
                &config.us_provider, &config.hk_provider, &config.cn_provider, false,
            ).await?
        }
        Some(_) => {
            // Empty list: no refresh needed, just use cache
            fetch_quotes_batch_cached_with_providers(
                &quote_cache, all_symbols,
                &config.us_provider, &config.hk_provider, &config.cn_provider, false,
            ).await?
        }
        None => {
            // No list provided: full refresh of all symbols
            fetch_quotes_batch_cached_with_providers(
                &quote_cache, all_symbols,
                &config.us_provider, &config.hk_provider, &config.cn_provider, true,
            ).await?
        }
    };
    // Persist freshly fetched quotes to the database
    if let Err(e) = save_quotes_to_db(&db, &quotes) {
        eprintln!("Failed to persist quotes to DB: {}", e);
    }
    let quote_map: std::collections::HashMap<String, StockQuote> = quotes
        .into_iter()
        .map(|q| (q.symbol.clone(), q))
        .collect();

    let result = holdings
        .into_iter()
        .map(|h| {
            let quote = quote_map.get(&h.symbol).cloned();
            let is_cleared = h.shares == 0.0 && !h.symbol.starts_with(CASH_SYMBOL_PREFIX);
            let (market_value, total_cost, unrealized_pnl, unrealized_pnl_percent) = if is_cleared {
                // Cleared position: report realized PnL from transaction history.
                let (realized_pnl, total_buy_cost) =
                    realized_pnl_map.get(&h.id).copied().unwrap_or((0.0, 0.0));
                let pnl_pct = if total_buy_cost != 0.0 {
                    Some(realized_pnl / total_buy_cost * 100.0)
                } else {
                    None
                };
                (Some(0.0), Some(total_buy_cost), Some(realized_pnl), pnl_pct)
            } else {
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
                (market_value, total_cost, unrealized_pnl, unrealized_pnl_percent)
            };
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

#[tauri::command]
pub fn take_quote_warning() -> Option<String> {
    crate::services::quote_service::take_quote_warning()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_us_quote(db: State<'_, Database>, quote_cache: State<'_, QuoteCache>, symbol: String) -> Result<StockQuote, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return Ok(cached);
    }
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let quote = fetch_us_quote_with_provider(&symbol, &config.us_provider).await?;
    quote_cache.set(quote.clone());
    if let Err(e) = save_quotes_to_db(&db, &[quote.clone()]) {
        eprintln!("Failed to persist quote to DB: {}", e);
    }
    Ok(quote)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_hk_quote(db: State<'_, Database>, quote_cache: State<'_, QuoteCache>, symbol: String) -> Result<StockQuote, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return Ok(cached);
    }
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let quote = fetch_hk_quote_with_provider(&symbol, &config.hk_provider).await?;
    quote_cache.set(quote.clone());
    if let Err(e) = save_quotes_to_db(&db, &[quote.clone()]) {
        eprintln!("Failed to persist quote to DB: {}", e);
    }
    Ok(quote)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_cn_quote(db: State<'_, Database>, quote_cache: State<'_, QuoteCache>, symbol: String) -> Result<StockQuote, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return Ok(cached);
    }
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let quote = fetch_cn_quote_with_provider(&symbol, &config.cn_provider).await?;
    quote_cache.set(quote.clone());
    if let Err(e) = save_quotes_to_db(&db, &[quote.clone()]) {
        eprintln!("Failed to persist quote to DB: {}", e);
    }
    Ok(quote)
}
