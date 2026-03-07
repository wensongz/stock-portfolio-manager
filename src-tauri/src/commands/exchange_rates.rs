use crate::models::ExchangeRates;
use crate::services::exchange_rate_service::{
    convert_currency, get_cached_rates, ExchangeRateCache,
};
use tauri::State;

#[tauri::command]
pub async fn get_exchange_rates(
    cache: State<'_, ExchangeRateCache>,
) -> Result<ExchangeRates, String> {
    get_cached_rates(&cache).await
}

#[tauri::command]
pub async fn convert_amount(
    amount: f64,
    from_currency: String,
    to_currency: String,
    cache: State<'_, ExchangeRateCache>,
) -> Result<f64, String> {
    let rates = get_cached_rates(&cache).await?;
    Ok(convert_currency(amount, &from_currency, &to_currency, &rates))
}
