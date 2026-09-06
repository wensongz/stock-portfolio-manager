use crate::db::Database;
use crate::models::portfolio_alert::{
    PortfolioAlertEvaluation, PortfolioAlertScope, PortfolioAlertView,
    SavePortfolioAlertConfigInput,
};
use crate::services::exchange_rate_service::{load_exchange_rates_from_db, ExchangeRateCache};
use crate::services::portfolio_alert_service;
use crate::services::quote_service::QuoteCache;
use chrono::Utc;
use tauri::State;

fn cached_exchange_rates(
    db: &Database,
    cache: &ExchangeRateCache,
) -> Result<Option<crate::models::ExchangeRates>, String> {
    match cache.get_stale() {
        Some(rates) => Ok(Some(rates)),
        None => load_exchange_rates_from_db(db),
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
