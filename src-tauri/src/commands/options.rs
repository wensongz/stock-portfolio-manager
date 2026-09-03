use crate::db::Database;
use crate::models::option::{OptionContract, SellCallSimulation, SellPutSimulation};
use tauri::State;

mod contracts;
mod csv;
mod simulation;

pub use contracts::get_option_contracts_inner;
pub use csv::ImportOptionsResult;
pub use simulation::StockPriceInput;

#[cfg(test)]
mod tests;

#[tauri::command(rename_all = "camelCase")]
pub fn import_options_csv(
    db: State<Database>,
    account_id: String,
    csv_content: String,
) -> Result<ImportOptionsResult, String> {
    csv::import_options_csv_inner(&db, &account_id, &csv_content)
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_option_contracts(
    db: State<Database>,
    account_id: String,
) -> Result<Vec<OptionContract>, String> {
    get_option_contracts_inner(&db, &account_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn simulate_sell_put(
    db: State<Database>,
    account_id: String,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellPutSimulation>, String> {
    simulation::simulate_sell_put_inner(&db, &account_id, stock_prices)
}

#[tauri::command(rename_all = "camelCase")]
pub fn simulate_sell_call(
    db: State<Database>,
    account_id: String,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellCallSimulation>, String> {
    simulation::simulate_sell_call_inner(&db, &account_id, stock_prices)
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_option_records(db: State<Database>, account_id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM option_records WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub fn export_options_csv(db: State<Database>, account_id: String) -> Result<String, String> {
    csv::export_options_csv_inner(&db, &account_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn parse_options_csv(
    csv_content: String,
) -> Result<crate::models::import_export::ImportPreview, String> {
    csv::parse_options_csv_inner(&csv_content)
}
