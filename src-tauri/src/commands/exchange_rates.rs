use crate::db::Database;
use crate::models::ExchangeRates;
use crate::services::exchange_rate_service::{get_cached_rates, ExchangeRateCache};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn get_exchange_rates(
    cache: State<'_, ExchangeRateCache>,
    db: State<'_, Database>,
) -> Result<ExchangeRates, String> {
    get_cached_rates(&cache, &db).await
}
