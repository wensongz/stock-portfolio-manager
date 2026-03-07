mod commands;
mod db;
mod models;

use db::Database;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("portfolio.db");
            let db = Database::new(db_path.to_str().unwrap())
                .expect("failed to initialize database");
            app.manage(db);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts::create_account,
            commands::accounts::get_accounts,
            commands::accounts::update_account,
            commands::accounts::delete_account,
            commands::categories::create_category,
            commands::categories::get_categories,
            commands::categories::update_category,
            commands::categories::delete_category,
            commands::holdings::create_holding,
            commands::holdings::get_holdings,
            commands::holdings::update_holding,
            commands::holdings::delete_holding,
            commands::transactions::create_transaction,
            commands::transactions::get_transactions,
            commands::transactions::delete_transaction,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
