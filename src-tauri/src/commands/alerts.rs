use crate::db::Database;
use crate::models::alert::{PriceAlert, TriggeredAlert};
use crate::services::alert_service;
use std::collections::HashMap;
use tauri::State;

#[tauri::command(rename_all = "snake_case")]
pub async fn create_alert(
    db: State<'_, Database>,
    holding_id: Option<String>,
    symbol: String,
    name: String,
    market: String,
    alert_type: String,
    threshold: f64,
) -> Result<PriceAlert, String> {
    alert_service::create_alert(&db, holding_id, symbol, name, market, alert_type, threshold)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_alerts(db: State<'_, Database>) -> Result<Vec<PriceAlert>, String> {
    alert_service::get_alerts(&db)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_alert(
    db: State<'_, Database>,
    id: String,
    is_active: bool,
) -> Result<PriceAlert, String> {
    alert_service::update_alert(&db, &id, is_active)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_alert(db: State<'_, Database>, id: String) -> Result<bool, String> {
    alert_service::delete_alert(&db, &id)
}

/// quotes_json: JSON object { symbol: [price, change_percent, pnl_percent] }
#[tauri::command(rename_all = "snake_case")]
pub async fn check_alerts(
    db: State<'_, Database>,
    quotes_json: serde_json::Value,
) -> Result<Vec<TriggeredAlert>, String> {
    let mut quotes: HashMap<String, (f64, f64, f64)> = HashMap::new();

    if let Some(obj) = quotes_json.as_object() {
        for (symbol, arr) in obj {
            if let Some(arr) = arr.as_array() {
                let price = arr.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
                let change = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let pnl = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                quotes.insert(symbol.clone(), (price, change, pnl));
            }
        }
    }

    alert_service::check_alerts(&db, &quotes)
}
