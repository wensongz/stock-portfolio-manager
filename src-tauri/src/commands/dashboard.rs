use crate::commands::quotes::QuoteCommandResult;
use crate::db::Database;
use crate::models::DashboardReport;
use crate::services::exchange_rate_service::{get_cached_rates, ExchangeRateCache};
use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
use crate::services::quote_service::{
    get_quote_refresh_time, save_quote_refresh_time, QuoteCache, QuoteServiceState,
};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn get_dashboard_report(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    base_currency: Option<String>,
) -> Result<QuoteCommandResult<DashboardReport>, String> {
    let base = base_currency.unwrap_or_else(|| "USD".to_string());
    let rates = get_cached_rates(&cache, &db).await?;
    let model = PortfolioReadModel::load(
        &db,
        &quote_cache,
        Some(&quote_state),
        QuoteReadMode::RefreshMissing,
    )
    .await?;
    let warning = model.quote_warning().map(str::to_string);
    let refreshed_at = if model.quotes_refreshed() {
        Some(save_quote_refresh_time(&db)?)
    } else {
        get_quote_refresh_time(&db)?
    };
    Ok(QuoteCommandResult {
        data: model.dashboard_report(rates, base),
        warning,
        refreshed_at,
    })
}
