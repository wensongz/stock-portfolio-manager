use crate::db::Database;
use crate::models::DashboardReport;
use crate::services::exchange_rate_service::{get_cached_rates, ExchangeRateCache};
use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
use crate::services::quote_service::{QuoteCache, QuoteServiceState};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn get_dashboard_report(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    base_currency: Option<String>,
) -> Result<DashboardReport, String> {
    let base = base_currency.unwrap_or_else(|| "USD".to_string());
    let rates = get_cached_rates(&cache, &db).await?;
    let model = PortfolioReadModel::load(
        &db,
        &quote_cache,
        Some(&quote_state),
        QuoteReadMode::RefreshMissing,
    )
    .await?;
    Ok(model.dashboard_report(rates, base))
}
