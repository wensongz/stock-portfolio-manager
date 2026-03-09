mod commands;
mod db;
mod models;
mod services;

use db::Database;
use services::exchange_rate_service::ExchangeRateCache;
use services::quote_service::QuoteCache;
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
            app.manage(ExchangeRateCache::new());
            app.manage(QuoteCache::new());
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
            commands::quotes::get_real_time_quotes,
            commands::quotes::get_holding_quotes,
            commands::quotes::get_us_quote,
            commands::quotes::get_hk_quote,
            commands::quotes::get_cn_quote,
            commands::exchange_rates::get_exchange_rates,
            commands::exchange_rates::convert_amount,
            commands::snapshots::take_snapshot,
            commands::snapshots::get_portfolio_history,
            commands::snapshots::backfill_snapshots,
            commands::dashboard::get_dashboard_summary,
            commands::dashboard::get_holdings_with_quotes,
            commands::statistics::get_statistics_overview,
            commands::statistics::get_statistics_by_market,
            commands::statistics::get_statistics_by_account,
            commands::statistics::get_statistics_by_category,
            commands::performance::get_performance_summary,
            commands::performance::get_return_series,
            commands::performance::get_benchmark_return_series,
            commands::performance::get_return_attribution,
            commands::performance::get_monthly_returns,
            commands::performance::get_holding_performance_ranking,
            commands::performance::get_risk_metrics,
            commands::performance::get_drawdown_analysis,
            commands::quarterly::create_quarterly_snapshot,
            commands::quarterly::get_quarterly_snapshots,
            commands::quarterly::get_quarterly_snapshot_detail,
            commands::quarterly::delete_quarterly_snapshot,
            commands::quarterly::check_missing_snapshots,
            commands::quarterly::compare_quarters,
            commands::quarterly::update_holding_notes,
            commands::quarterly::get_holding_notes_history,
            commands::quarterly::update_quarterly_notes,
            commands::quarterly::get_quarterly_notes_history,
            commands::quarterly::get_quarterly_trends,
            // Phase 6: Import/Export
            commands::import_export::export_holdings_csv,
            commands::import_export::export_transactions_csv,
            commands::import_export::get_import_template,
            commands::import_export::parse_import_csv,
            commands::import_export::confirm_import,
            // Phase 6: Price Alerts
            commands::alerts::create_alert,
            commands::alerts::get_alerts,
            commands::alerts::update_alert,
            commands::alerts::delete_alert,
            commands::alerts::check_alerts,
            // Phase 6: Review
            commands::review::get_holding_review,
            commands::review::update_decision_quality,
            commands::review::get_decision_statistics,
            commands::review::get_reviewed_symbols,
            // Phase 6: AI Config
            commands::ai::get_ai_config,
            commands::ai::update_ai_config,
            // Quote Provider Config
            commands::quote_provider::get_quote_provider_config,
            commands::quote_provider::update_quote_provider_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
