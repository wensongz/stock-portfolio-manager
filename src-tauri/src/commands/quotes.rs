use crate::db::Database;
use crate::models::{HoldingWithQuote, StockQuote};
use crate::services::quote_provider_service;
use crate::services::quote_service::{
    fetch_cn_quote_with_provider, fetch_hk_quote_with_provider,
    fetch_quotes_batch_cached_with_providers, fetch_us_quote_with_provider, get_quote_refresh_time,
    merge_quote_warning, save_quote_refresh_time, save_quotes_to_db, QuoteCache, QuoteFetchResult,
    QuoteServiceState, CASH_SYMBOL_PREFIX,
};
use serde::Serialize;
use tauri::State;
use tracing::warn;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteCommandResult<T> {
    pub data: T,
    pub warning: Option<String>,
    pub refreshed_at: Option<String>,
}

fn finish_quote_command<T>(
    db: &Database,
    fetch: QuoteFetchResult<T>,
) -> Result<QuoteCommandResult<T>, String> {
    let refreshed_at = if fetch.did_refresh {
        Some(save_quote_refresh_time(db)?)
    } else {
        get_quote_refresh_time(db)?
    };
    Ok(QuoteCommandResult {
        data: fetch.data,
        warning: fetch.warning,
        refreshed_at,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_real_time_quotes(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    symbols: Vec<(String, String)>,
    force_refresh: Option<bool>,
) -> Result<QuoteCommandResult<Vec<StockQuote>>, String> {
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let fetch = fetch_quotes_batch_cached_with_providers(
        &quote_state,
        &quote_cache,
        symbols,
        &config.us_provider,
        &config.hk_provider,
        &config.cn_provider,
        force_refresh.unwrap_or(false),
    )
    .await?;
    // Persist freshly fetched quotes to the database
    if fetch.did_refresh {
        if let Err(e) = save_quotes_to_db(&db, &fetch.data) {
            warn!("Failed to persist quotes to DB: {}", e);
        }
    }
    finish_quote_command(&db, fetch)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_holding_quotes(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    refresh_symbols: Option<Vec<(String, String)>>,
) -> Result<QuoteCommandResult<Vec<HoldingWithQuote>>, String> {
    get_holding_quotes_inner(&db, &quote_cache, &quote_state, refresh_symbols).await
}

pub async fn get_holding_quotes_inner(
    db: &Database,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    refresh_symbols: Option<Vec<(String, String)>>,
) -> Result<QuoteCommandResult<Vec<HoldingWithQuote>>, String> {
    let config = quote_provider_service::get_quote_provider_config(db)?;
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
        let rows = stmt
            .query_map([], |row| {
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

        let realized_pnl_map = load_realized_pnl_by_holding(&conn).unwrap_or_else(|e| {
            warn!("Failed to compute realized PnL for cleared holdings: {}", e);
            std::collections::HashMap::new()
        });

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

    let mut fetch = match refresh_symbols {
        Some(ref symbols) if !symbols.is_empty() => {
            // Targeted refresh: force-refresh only the specified symbols
            let targeted = fetch_quotes_batch_cached_with_providers(
                quote_state,
                quote_cache,
                symbols.clone(),
                &config.us_provider,
                &config.hk_provider,
                &config.cn_provider,
                true,
            )
            .await?;
            // Then load all quotes from cache (the refreshed ones are now fresh)
            let mut all = fetch_quotes_batch_cached_with_providers(
                quote_state,
                quote_cache,
                all_symbols,
                &config.us_provider,
                &config.hk_provider,
                &config.cn_provider,
                false,
            )
            .await?;
            merge_quote_warning(&mut all.warning, targeted.warning);
            all.did_refresh |= targeted.did_refresh;
            all
        }
        Some(_) => {
            // Empty list: no refresh needed, just use cache
            fetch_quotes_batch_cached_with_providers(
                quote_state,
                quote_cache,
                all_symbols,
                &config.us_provider,
                &config.hk_provider,
                &config.cn_provider,
                false,
            )
            .await?
        }
        None => {
            // No list provided: full refresh of all symbols
            fetch_quotes_batch_cached_with_providers(
                quote_state,
                quote_cache,
                all_symbols,
                &config.us_provider,
                &config.hk_provider,
                &config.cn_provider,
                true,
            )
            .await?
        }
    };
    // Persist freshly fetched quotes to the database
    if fetch.did_refresh {
        if let Err(e) = save_quotes_to_db(db, &fetch.data) {
            warn!("Failed to persist quotes to DB: {}", e);
        }
    }
    let quote_map: std::collections::HashMap<String, StockQuote> = fetch
        .data
        .drain(..)
        .map(|q| (q.symbol.clone(), q))
        .collect();

    let result = holdings
        .into_iter()
        .map(|h| {
            let quote = quote_map.get(&h.symbol).cloned();
            let cleared = h.shares == 0.0 && !h.symbol.starts_with(CASH_SYMBOL_PREFIX);
            let (market_value, total_cost, unrealized_pnl, unrealized_pnl_percent) = if cleared {
                // Cleared position: report realized PnL from transaction history.
                let (realized_pnl, total_buy_cost) =
                    realized_pnl_map.get(&h.id).copied().unwrap_or((0.0, 0.0));
                let pnl_pct = if total_buy_cost > 0.0 {
                    Some(realized_pnl / total_buy_cost * 100.0)
                } else {
                    None
                };
                (Some(0.0), Some(total_buy_cost), Some(realized_pnl), pnl_pct)
            } else {
                let market_value = quote.as_ref().map(|q| q.current_price * h.shares);
                let total_cost = Some(h.avg_cost * h.shares);
                let unrealized_pnl = market_value.zip(total_cost).map(|(mv, tc)| mv - tc);
                let unrealized_pnl_percent =
                    unrealized_pnl.zip(total_cost).and_then(|(pnl, tc)| {
                        if tc > 0.0 {
                            Some(pnl / tc * 100.0)
                        } else {
                            None
                        }
                    });
                (
                    market_value,
                    total_cost,
                    unrealized_pnl,
                    unrealized_pnl_percent,
                )
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

    finish_quote_command(
        db,
        QuoteFetchResult {
            data: result,
            warning: fetch.warning,
            did_refresh: fetch.did_refresh,
        },
    )
}

/// Load realized PnL for every cleared non-cash position in one grouped query.
///   realized_pnl = SUM(SELL total_amount - commission) - SUM((BUY|OPEN) total_amount + commission)
///   total_buy_cost = SUM((BUY|OPEN) total_amount + commission)  [used for % calculation]
/// OPEN transactions are position-entry records (create_holding / backfill)
/// with no cash impact, but their total_amount is the position's cost basis
/// and must count toward realized PnL.
fn load_realized_pnl_by_holding(
    conn: &rusqlite::Connection,
) -> Result<std::collections::HashMap<String, (f64, f64)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT h.id,
                COALESCE(SUM(CASE
                    WHEN t.transaction_type = 'SELL' THEN t.total_amount - t.commission
                    WHEN t.transaction_type IN ('BUY', 'OPEN') THEN -(t.total_amount + t.commission)
                    ELSE 0
                END), 0.0),
                COALESCE(SUM(CASE
                    WHEN t.transaction_type IN ('BUY', 'OPEN') THEN t.total_amount + t.commission
                    ELSE 0
                END), 0.0)
           FROM holdings h
           LEFT JOIN transactions t ON t.holding_id = h.id
          WHERE h.shares = 0.0
            AND h.symbol NOT LIKE '$CASH-%'
          GROUP BY h.id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (row.get::<_, f64>(1)?, row.get::<_, f64>(2)?),
        ))
    })?;
    rows.collect()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_us_quote(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    symbol: String,
) -> Result<QuoteCommandResult<StockQuote>, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return finish_quote_command(
            &db,
            QuoteFetchResult {
                data: cached,
                warning: None,
                did_refresh: false,
            },
        );
    }
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let fetch = fetch_us_quote_with_provider(&quote_state, &symbol, &config.us_provider).await?;
    quote_cache.set(fetch.data.clone());
    if let Err(e) = save_quotes_to_db(&db, std::slice::from_ref(&fetch.data)) {
        warn!("Failed to persist quote to DB: {}", e);
    }
    finish_quote_command(&db, fetch)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_hk_quote(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    symbol: String,
) -> Result<QuoteCommandResult<StockQuote>, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return finish_quote_command(
            &db,
            QuoteFetchResult {
                data: cached,
                warning: None,
                did_refresh: false,
            },
        );
    }
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let fetch = fetch_hk_quote_with_provider(&quote_state, &symbol, &config.hk_provider).await?;
    quote_cache.set(fetch.data.clone());
    if let Err(e) = save_quotes_to_db(&db, std::slice::from_ref(&fetch.data)) {
        warn!("Failed to persist quote to DB: {}", e);
    }
    finish_quote_command(&db, fetch)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_cn_quote(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    symbol: String,
) -> Result<QuoteCommandResult<StockQuote>, String> {
    if let Some(cached) = quote_cache.get(&symbol) {
        return finish_quote_command(
            &db,
            QuoteFetchResult {
                data: cached,
                warning: None,
                did_refresh: false,
            },
        );
    }
    let config = quote_provider_service::get_quote_provider_config(&db)?;
    let fetch = fetch_cn_quote_with_provider(&quote_state, &symbol, &config.cn_provider).await?;
    quote_cache.set(fetch.data.clone());
    if let Err(e) = save_quotes_to_db(&db, std::slice::from_ref(&fetch.data)) {
        warn!("Failed to persist quote to DB: {}", e);
    }
    finish_quote_command(&db, fetch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    #[test]
    fn cache_only_quote_result_preserves_persisted_refresh_time() {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO cached_quote_refresh_time (id, updated_at) VALUES (1, '2026-09-01T10:00:00Z')",
                [],
            )
            .unwrap();
        }

        let outcome = finish_quote_command(
            &db,
            QuoteFetchResult {
                data: Vec::<StockQuote>::new(),
                warning: None,
                did_refresh: false,
            },
        )
        .unwrap();

        assert_eq!(
            outcome.refreshed_at.as_deref(),
            Some("2026-09-01T10:00:00Z")
        );
        assert_eq!(
            get_quote_refresh_time(&db).unwrap().as_deref(),
            Some("2026-09-01T10:00:00Z")
        );
    }

    #[test]
    fn refreshed_quote_result_returns_exact_persisted_time() {
        let db = Database::new(":memory:").unwrap();
        let outcome = finish_quote_command(
            &db,
            QuoteFetchResult {
                data: Vec::<StockQuote>::new(),
                warning: Some("fallback".to_string()),
                did_refresh: true,
            },
        )
        .unwrap();

        assert_eq!(outcome.warning.as_deref(), Some("fallback"));
        assert_eq!(get_quote_refresh_time(&db).unwrap(), outcome.refreshed_at);
    }

    /// Build an in-memory DB with one account and a cleared 410.HK position:
    /// an OPEN backfill entry (930,000 shares @ 2.74) fully sold at ~0.35.
    /// Mirrors the user-reported scenario (SOHO中国).
    fn db_with_cleared_position() -> (Database, String) {
        let db = Database::new(":memory:").expect("failed to create in-memory database");
        let holding_id = "h-410hk".to_string();
        let ts = now();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('a', 'Test', 'HK', ?1, ?1)",
                rusqlite::params![ts],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings (id, account_id, symbol, name, market, category_id,
                        shares, avg_cost, currency, created_at, updated_at)
                 VALUES (?1, 'a', '410.HK', 'SOHO中国', 'HK', NULL,
                         0.0, 2.74, 'HKD', ?2, ?2)",
                rusqlite::params![holding_id, ts],
            )
            .unwrap();
            // OPEN initial position entry (backfill) — 930,000 @ 2.74
            conn.execute(
                "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market,
                        transaction_type, shares, price, total_amount, commission, currency,
                        traded_at, notes, created_at)
                 VALUES ('t-open', ?1, 'a', '410.HK', 'SOHO中国', 'HK',
                         'OPEN', 930000.0, 2.74, 2548200.0, 0.0, 'HKD',
                         '2026-03-15', 'backfill:initial', ?2)",
                rusqlite::params![holding_id, ts],
            )
            .unwrap();
            // SELLs — 200k + 200k + 500 + 200k + 130k + 199.5k @ ~0.35
            let sells: [(&str, f64, f64, f64, f64); 6] = [
                ("t-s1", 200000.0, 0.35, 71000.00, 115.51),
                ("t-s2", 200000.0, 0.35, 71000.00, 115.51),
                ("t-s3", 500.0, 0.36, 182.50, 19.03),
                ("t-s4", 200000.0, 0.35, 71000.00, 115.51),
                ("t-s5", 130000.0, 0.35, 46150.00, 75.94),
                ("t-s6", 199500.0, 0.35, 70822.50, 116.41),
            ];
            for (i, (id, shares, price, amount, comm)) in sells.iter().enumerate() {
                conn.execute(
                    "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market,
                            transaction_type, shares, price, total_amount, commission, currency,
                            traded_at, notes, created_at)
                     VALUES (?1, ?2, 'a', '410.HK', 'SOHO中国', 'HK',
                             'SELL', ?3, ?4, ?5, ?6, 'HKD', ?7, NULL, ?8)",
                    rusqlite::params![
                        id,
                        holding_id,
                        shares,
                        price,
                        amount,
                        comm,
                        format!("2026-07-1{}", i + 1),
                        ts,
                    ],
                )
                .unwrap();
            }
        }
        (db, holding_id)
    }

    #[test]
    fn test_realized_pnl_includes_open_cost() {
        let (db, holding_id) = db_with_cleared_position();
        let conn = db.conn.lock().unwrap();
        let pnl_by_holding = load_realized_pnl_by_holding(&conn).unwrap();
        let (realized, total_buy_cost) = pnl_by_holding[&holding_id];
        // Cost basis: 930,000 @ 2.74 = 2,548,200 (stored as OPEN)
        assert_eq!(total_buy_cost, 2_548_200.0);
        // Sells: 330,155.00 - 557.91 = 329,597.09; realized = 329,597.09 - 2,548,200.00
        assert!(
            (realized - (-2_218_602.91)).abs() < 0.01,
            "realized PnL was {}, expected -2218602.91",
            realized
        );
    }

    #[test]
    fn test_realized_pnl_buy_sell() {
        // Regular BUY/SELL trades (no OPEN): BUY 100 @ 10 (comm 1), SELL 100 @ 12 (comm 1.2)
        let db = Database::new(":memory:").expect("failed to create in-memory database");
        let holding_id = "h-plain".to_string();
        let ts = now();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('a', 'Test', 'US', ?1, ?1)",
                rusqlite::params![ts],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings (id, account_id, symbol, name, market, category_id,
                        shares, avg_cost, currency, created_at, updated_at)
                 VALUES (?1, 'a', 'AAPL', 'Apple', 'US', NULL, 0.0, 10.0, 'USD', ?2, ?2)",
                rusqlite::params![holding_id, ts],
            )
            .unwrap();
            for (id, ttype, shares, price, amount, comm) in [
                ("t1", "BUY", 100.0, 10.0, 1000.0, 1.0),
                ("t2", "SELL", 100.0, 12.0, 1200.0, 1.2),
            ] {
                conn.execute(
                    "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market,
                            transaction_type, shares, price, total_amount, commission, currency,
                            traded_at, notes, created_at)
                     VALUES (?1, ?2, 'a', 'AAPL', 'Apple', 'US',
                             ?3, ?4, ?5, ?6, ?7, 'USD', ?8, NULL, ?9)",
                    rusqlite::params![id, holding_id, ttype, shares, price, amount, comm, ts, ts],
                )
                .unwrap();
            }
        }
        let conn = db.conn.lock().unwrap();
        let pnl_by_holding = load_realized_pnl_by_holding(&conn).unwrap();
        let (realized, total_buy_cost) = pnl_by_holding[&holding_id];
        assert_eq!(total_buy_cost, 1001.0);
        assert!(
            (realized - 197.8).abs() < 0.01,
            "realized PnL was {}",
            realized
        );
    }

    #[test]
    fn test_load_realized_pnl_groups_every_cleared_holding() {
        let (db, open_holding_id) = db_with_cleared_position();
        let ts = now();
        {
            let conn = db.conn.lock().unwrap();
            for (holding_id, symbol) in [("h-profit", "MSFT"), ("h-loss", "TSLA")] {
                conn.execute(
                    "INSERT INTO holdings (id, account_id, symbol, name, market, category_id,
                            shares, avg_cost, currency, created_at, updated_at)
                     VALUES (?1, 'a', ?2, ?2, 'US', NULL, 0.0, 10.0, 'USD', ?3, ?3)",
                    rusqlite::params![holding_id, symbol, ts],
                )
                .unwrap();
            }
            for (id, holding_id, symbol, transaction_type, amount, commission) in [
                ("profit-buy", "h-profit", "MSFT", "BUY", 1_000.0, 1.0),
                ("profit-sell", "h-profit", "MSFT", "SELL", 1_250.0, 2.0),
                ("loss-buy", "h-loss", "TSLA", "BUY", 2_000.0, 2.0),
                ("loss-sell", "h-loss", "TSLA", "SELL", 1_500.0, 1.5),
            ] {
                conn.execute(
                    "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market,
                            transaction_type, shares, price, total_amount, commission, currency,
                            traded_at, notes, created_at)
                     VALUES (?1, ?2, 'a', ?3, ?3, 'US', ?4, 100, 10, ?5, ?6,
                             'USD', ?7, NULL, ?7)",
                    rusqlite::params![
                        id,
                        holding_id,
                        symbol,
                        transaction_type,
                        amount,
                        commission,
                        ts
                    ],
                )
                .unwrap();
            }
        }

        let conn = db.conn.lock().unwrap();
        let realized = load_realized_pnl_by_holding(&conn).unwrap();

        assert_eq!(realized.len(), 3);
        assert_eq!(realized.get("h-profit"), Some(&(247.0, 1_001.0)));
        assert_eq!(realized.get("h-loss"), Some(&(-503.5, 2_002.0)));
        let (open_pnl, open_cost) = realized.get(&open_holding_id).copied().unwrap();
        assert_eq!(open_cost, 2_548_200.0);
        assert!((open_pnl - (-2_218_602.91)).abs() < 0.01);
    }
}
