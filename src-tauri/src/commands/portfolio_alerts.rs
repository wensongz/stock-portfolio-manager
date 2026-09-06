use crate::db::Database;
use crate::models::portfolio_alert::{
    PortfolioAlertEvaluation, PortfolioAlertScope, PortfolioAlertView,
    SavePortfolioAlertConfigInput,
};
use crate::services::exchange_rate_service::{load_exchange_rates_from_db, ExchangeRateCache};
use crate::services::portfolio_alert_service;
use crate::services::quote_service::QuoteCache;
use chrono::Utc;
use tauri::{AppHandle, Emitter, State};
use tracing::warn;

fn cached_exchange_rates(
    db: &Database,
    cache: &ExchangeRateCache,
) -> Result<Option<crate::models::ExchangeRates>, String> {
    match cache.get_stale() {
        Some(rates) => Ok(Some(rates)),
        None => load_exchange_rates_from_db(db),
    }
}

/// Evaluate active portfolio alerts using only in-memory or persisted rates,
/// then notify the frontend about each newly inserted breach. Quote refreshes
/// must remain successful even when alert evaluation or event delivery fails.
pub(crate) async fn evaluate_and_emit_portfolio_alerts(
    app_handle: &AppHandle,
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rate_cache: &ExchangeRateCache,
) {
    evaluate_portfolio_alerts_with_sink(db, quote_cache, exchange_rate_cache, |notification| {
        if let Err(error) = app_handle.emit("portfolio-alert-triggered", notification) {
            warn!("Failed to emit portfolio-alert-triggered event: {error}");
        }
    })
    .await;
}

pub(crate) async fn evaluate_portfolio_alerts_with_sink<F>(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rate_cache: &ExchangeRateCache,
    emit: F,
) where
    F: FnMut(crate::models::portfolio_alert::PortfolioAlertNotification),
{
    let rates = exchange_rate_cache.get_stale().or_else(|| {
        load_exchange_rates_from_db(db).unwrap_or_else(|error| {
            warn!("Unable to load persisted exchange rates for portfolio alerts: {error}");
            None
        })
    });
    match portfolio_alert_service::evaluate_all_active_portfolio_alerts(
        db,
        quote_cache,
        rates.as_ref(),
        &Utc::now().to_rfc3339(),
    )
    .await
    {
        Ok(notifications) => emit_portfolio_alert_notifications(notifications, emit),
        Err(error) => warn!("Portfolio alert evaluation after quote refresh failed: {error}"),
    }
}

pub(crate) fn emit_portfolio_alert_notifications<F>(
    notifications: Vec<crate::models::portfolio_alert::PortfolioAlertNotification>,
    mut emit: F,
) where
    F: FnMut(crate::models::portfolio_alert::PortfolioAlertNotification),
{
    for notification in notifications {
        emit(notification);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::quotes::{
        persist_holding_quote_refresh, run_alert_evaluation_after_holding_refresh,
    };
    use crate::db::Database;
    use crate::models::portfolio_alert::{
        PortfolioAlertBreachDirection, PortfolioAlertBreachKind, PortfolioAlertScope,
        PortfolioAlertScopeKind, PortfolioAlertTarget, SavePortfolioAlertConfigInput,
    };
    use crate::models::StockQuote;
    use crate::services::portfolio_alert_service;
    use crate::services::quote_service::{classify_refresh_complete, QuoteCache, QuoteFetchResult};

    fn fixture() -> (Database, QuoteCache, String, Vec<StockQuote>) {
        let db = Database::new(":memory:").unwrap();
        let config_id = "config-us".to_string();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('acct-us', 'US', 'US', '2026-09-06', '2026-09-06')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO categories (id, name, color, icon, created_at)
                 VALUES ('growth', 'Growth', '#00AA00', 'growth', '2026-09-06')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', 'growth', 10, 1, 'USD', '2026-09-06', '2026-09-06')",
                [],
            )
            .unwrap();
        }
        portfolio_alert_service::save_portfolio_alert_config(
            &db,
            SavePortfolioAlertConfigInput {
                id: Some(config_id.clone()),
                scope: PortfolioAlertScope {
                    kind: PortfolioAlertScopeKind::Market,
                    market: Some("US".to_string()),
                    account_id: None,
                },
                base_currency: "USD".to_string(),
                deviation_threshold: 20.0,
                concentration_threshold: 60.0,
                is_active: true,
                targets: vec![PortfolioAlertTarget {
                    category_id: "growth".to_string(),
                    target_percent: 100.0,
                }],
            },
        )
        .unwrap();
        let quotes = vec![StockQuote {
            market: "US".to_string(),
            symbol: "AAPL".to_string(),
            current_price: 100.0,
            ..StockQuote::default()
        }];
        let quote_cache = QuoteCache::new();
        quote_cache.set_batch(&quotes);
        (db, quote_cache, config_id, quotes)
    }

    #[tokio::test]
    async fn persisted_complete_refresh_evaluates_real_breaches_and_emits_exact_payloads() {
        // This catches disconnected orchestration tests: all notifications here
        // must come from real persisted breach transitions, not fixture values.
        let (db, quote_cache, config_id, quotes) = fixture();
        let requested = vec![("AAPL".to_string(), "US".to_string())];
        let refresh_complete = classify_refresh_complete(&requested, &quotes);
        let complete = QuoteFetchResult {
            data: quotes,
            warning: None,
            did_refresh: true,
            refresh_complete,
        };
        let persisted = persist_holding_quote_refresh(&db, &complete);
        assert!(persisted);
        let rate_cache = ExchangeRateCache::new();
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = emitted.clone();

        run_alert_evaluation_after_holding_refresh(None, &complete, persisted, || async {
            assert_eq!(
                crate::services::quote_service::load_quotes_from_db(&db)
                    .unwrap()
                    .len(),
                1
            );
            evaluate_portfolio_alerts_with_sink(&db, &quote_cache, &rate_cache, |notification| {
                captured.lock().unwrap().push(notification);
            })
            .await;
        })
        .await;

        let emitted = emitted.lock().unwrap();
        assert_eq!(emitted.len(), 1);
        let notification = &emitted[0];
        assert_eq!(notification.config_id, config_id);
        assert_eq!(
            notification.scope,
            PortfolioAlertScope {
                kind: PortfolioAlertScopeKind::Market,
                market: Some("US".to_string()),
                account_id: None,
            }
        );
        assert_eq!(notification.breach.config_id, notification.config_id);
        assert_eq!(notification.breach.breach_key, "security:US:AAPL");
        assert_eq!(
            notification.breach.breach_kind,
            PortfolioAlertBreachKind::Concentration
        );
        assert_eq!(
            notification.breach.direction,
            PortfolioAlertBreachDirection::AboveLimit
        );
        assert_eq!(notification.message, "持仓集中度预警：security:US:AAPL");
        assert_eq!(
            notification.triggered_at,
            notification.breach.first_triggered_at
        );
        assert_eq!(notification.triggered_at, notification.breach.last_seen_at);
        assert!(chrono::DateTime::parse_from_rfc3339(&notification.triggered_at).is_ok());

        let persisted = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT breach_key, breach_kind, direction, first_triggered_at, last_seen_at
                 FROM portfolio_alert_breaches WHERE config_id = ?1",
                [&notification.config_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(persisted.0, notification.breach.breach_key);
        assert_eq!(persisted.1, "CONCENTRATION");
        assert_eq!(persisted.2, "ABOVE_LIMIT");
        assert_eq!(persisted.3, notification.breach.first_triggered_at);
        assert_eq!(persisted.4, notification.breach.last_seen_at);
    }

    #[tokio::test]
    async fn incomplete_cache_only_and_unpersisted_refreshes_skip_real_evaluation_and_emission() {
        let requested = vec![("AAPL".to_string(), "US".to_string())];
        let rate_cache = ExchangeRateCache::new();

        // A stale/unresolved final result is classified from the final fresh
        // quote set and must never create a persisted breach.
        let (db, quote_cache, _, quotes) = fixture();
        let partial = QuoteFetchResult {
            data: Vec::<StockQuote>::new(),
            warning: Some("stale fallback".to_string()),
            did_refresh: true,
            refresh_complete: classify_refresh_complete(&requested, &[]),
        };
        assert!(!partial.refresh_complete);
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = emitted.clone();
        run_alert_evaluation_after_holding_refresh(None, &partial, true, || async {
            evaluate_portfolio_alerts_with_sink(&db, &quote_cache, &rate_cache, |notification| {
                captured.lock().unwrap().push(notification);
            })
            .await;
        })
        .await;
        assert!(emitted.lock().unwrap().is_empty());
        assert_eq!(breach_count(&db), 0);

        // The public `Some(vec![])` cache-only path is never a real refresh,
        // even if its caller happens to carry a complete prior result.
        let (db, quote_cache, _, _) = fixture();
        let complete = QuoteFetchResult {
            data: quotes.clone(),
            warning: None,
            did_refresh: true,
            refresh_complete: classify_refresh_complete(&requested, &quotes),
        };
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = emitted.clone();
        run_alert_evaluation_after_holding_refresh(Some(&[]), &complete, true, || async {
            evaluate_portfolio_alerts_with_sink(&db, &quote_cache, &rate_cache, |notification| {
                captured.lock().unwrap().push(notification);
            })
            .await;
        })
        .await;
        assert!(emitted.lock().unwrap().is_empty());
        assert_eq!(breach_count(&db), 0);

        // Failure to save the fresh quotes is also a hard gate: the real
        // evaluator/sink remains untouched despite a complete result.
        let (db, quote_cache, _, quotes) = fixture();
        let complete = QuoteFetchResult {
            data: quotes,
            warning: None,
            did_refresh: true,
            refresh_complete: classify_refresh_complete(&requested, &complete_quotes(&quote_cache)),
        };
        db.conn
            .lock()
            .unwrap()
            .execute("DROP TABLE cached_quotes", [])
            .unwrap();
        assert!(!persist_holding_quote_refresh(&db, &complete));
        let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = emitted.clone();
        run_alert_evaluation_after_holding_refresh(None, &complete, false, || async {
            evaluate_portfolio_alerts_with_sink(&db, &quote_cache, &rate_cache, |notification| {
                captured.lock().unwrap().push(notification);
            })
            .await;
        })
        .await;
        assert!(emitted.lock().unwrap().is_empty());
        assert_eq!(breach_count(&db), 0);
    }

    fn complete_quotes(quote_cache: &QuoteCache) -> Vec<StockQuote> {
        quote_cache.get("US", "AAPL").into_iter().collect()
    }

    fn breach_count(db: &Database) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM portfolio_alert_breaches", [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_portfolio_alert_view(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    scope: PortfolioAlertScope,
) -> Result<PortfolioAlertView, String> {
    let Some(config) = portfolio_alert_service::get_portfolio_alert_config_by_scope(&db, &scope)?
    else {
        return Ok(PortfolioAlertView {
            config: None,
            evaluation: None,
        });
    };
    if !config.is_active {
        return Ok(PortfolioAlertView {
            config: Some(config),
            evaluation: None,
        });
    }
    let rates = cached_exchange_rates(&db, &exchange_rate_cache)?;
    let evaluation = portfolio_alert_service::evaluate_portfolio_alert(
        &db,
        &quote_cache,
        rates.as_ref(),
        &config.id,
        &Utc::now().to_rfc3339(),
    )
    .await?;
    Ok(PortfolioAlertView {
        config: Some(portfolio_alert_service::get_portfolio_alert_config_by_id(
            &db, &config.id,
        )?),
        evaluation: Some(evaluation),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_portfolio_alert_config(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    input: SavePortfolioAlertConfigInput,
) -> Result<PortfolioAlertView, String> {
    let rates = cached_exchange_rates(&db, &exchange_rate_cache)?;
    portfolio_alert_service::save_and_evaluate_portfolio_alert_config(
        &db,
        &quote_cache,
        rates.as_ref(),
        input,
        &Utc::now().to_rfc3339(),
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn set_portfolio_alert_active(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    config_id: String,
    is_active: bool,
) -> Result<PortfolioAlertView, String> {
    let rates = if is_active {
        cached_exchange_rates(&db, &exchange_rate_cache)?
    } else {
        None
    };
    portfolio_alert_service::set_portfolio_alert_active_and_evaluate(
        &db,
        &quote_cache,
        rates.as_ref(),
        &config_id,
        is_active,
        &Utc::now().to_rfc3339(),
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn evaluate_portfolio_alert(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    config_id: String,
) -> Result<PortfolioAlertEvaluation, String> {
    let rates = cached_exchange_rates(&db, &exchange_rate_cache)?;
    portfolio_alert_service::evaluate_portfolio_alert(
        &db,
        &quote_cache,
        rates.as_ref(),
        &config_id,
        &Utc::now().to_rfc3339(),
    )
    .await
}
