mod commands;
pub mod db;
pub mod models;
pub mod services;

use commands::{account_commands, category_commands, holding_commands, transaction_commands};
use db::Database;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Initialize database in app data directory
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data directory");
            std::fs::create_dir_all(&app_dir).expect("Failed to create app data directory");
            let db_path = app_dir.join("portfolio.db");

            let database =
                Database::new(&db_path).expect("Failed to initialize database");

            app.manage(database);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            account_commands::create_account,
            account_commands::list_accounts,
            account_commands::get_account,
            account_commands::update_account,
            account_commands::delete_account,
            category_commands::list_categories,
            category_commands::create_category,
            category_commands::update_category,
            category_commands::delete_category,
            holding_commands::create_holding,
            holding_commands::list_holdings,
            holding_commands::get_holding,
            holding_commands::update_holding,
            holding_commands::delete_holding,
            transaction_commands::create_transaction,
            transaction_commands::list_transactions,
            transaction_commands::delete_transaction,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
