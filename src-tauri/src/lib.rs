mod commands;
mod db;
mod menu;
mod models;
mod services;

use db::Database;
use services::exchange_rate_service::ExchangeRateCache;
use services::quote_service::{QuoteCache, QuoteServiceState};
use tauri::{Emitter, Manager};
use tracing::warn;

/// Initialize the global tracing subscriber.
///
/// Default level: `info` for this crate, `warn` for everything else. Override
/// with the `RUST_LOG` env var (e.g. `RUST_LOG=debug` or
/// `RUST_LOG=stock_portfolio_manager_lib=trace,reqwest=warn`). Safe to call
/// more than once — later calls are no-ops once a subscriber is installed.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,reqwest=warn,rusqlite=warn,hyper=warn")
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    tracing::info!("starting stock-portfolio-manager");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .menu(menu::build_menu)
        .setup(|app| {
            // Register the Window submenu with NSApp so macOS injects
            // "Bring All to Front", "Move to [Display]", and the window list.
            #[cfg(target_os = "macos")]
            menu::register_window_menu_for_nsapp();

            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&app_dir)?;
            let db_path = app_dir.join("portfolio.db");
            let db =
                Database::new(db_path.to_str().unwrap()).expect("failed to initialize database");
            let quote_state = QuoteServiceState::new();
            if let Ok(config) = services::quote_provider_service::get_quote_provider_config(&db) {
                services::quote_service::set_xueqiu_user_cookie(&quote_state, config.xueqiu_cookie);
                services::quote_service::set_xueqiu_user_u(&quote_state, config.xueqiu_u);
            }
            app.manage(db);
            app.manage(ExchangeRateCache::new());
            app.manage(QuoteCache::new());
            app.manage(quote_state);

            // Load persisted quote cache from the database so the UI can
            // render immediately with the last-known prices.
            {
                let db = app.state::<Database>();
                let cache = app.state::<QuoteCache>();
                match services::quote_service::load_quotes_from_db(&db) {
                    Ok(quotes) if !quotes.is_empty() => {
                        cache.set_batch(&quotes);
                    }
                    Ok(_) => {}
                    Err(e) => warn!("Failed to load cached quotes from DB: {}", e),
                }
            }

            // Pre-load persisted exchange rates so the in-memory cache is
            // warm immediately, even before a network fetch succeeds.
            {
                let db = app.state::<Database>();
                let rate_cache = app.state::<ExchangeRateCache>();
                match services::exchange_rate_service::load_exchange_rates_from_db(&db) {
                    Ok(Some(rates)) => {
                        rate_cache.set(rates);
                    }
                    Ok(None) => {}
                    Err(e) => warn!("Failed to load cached exchange rates from DB: {}", e),
                }
            }

            // Auto-backup check on startup (background)
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    crate::commands::backup::auto_backup_if_needed(&handle);
                });
            }

            // Materialise built-in AI skills into the user skills directory on
            // first launch so the user can see, edit, and delete them. Existing
            // files are never overwritten — user edits win.
            {
                let handle = app.handle().clone();
                if let Err(e) = crate::services::skill_service::export_builtin_skills(&handle) {
                    warn!("Failed to export built-in skills: {}", e);
                }
            }

            // Spawn a background task to refresh holding quotes from the API.
            // This runs after startup so the UI is not blocked.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Small delay to let the window finish loading.
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                let db = handle.state::<Database>();
                let cache = handle.state::<QuoteCache>();
                let quote_state = handle.state::<QuoteServiceState>();

                // Collect all holding symbols.
                let symbols: Vec<(String, String)> = {
                    let conn = match db.conn.lock() {
                        Ok(c) => c,
                        Err(e) => {
                            warn!("Background refresh: failed to acquire DB lock: {}", e);
                            return;
                        }
                    };
                    let mut stmt =
                        match conn.prepare("SELECT DISTINCT symbol, market FROM holdings") {
                            Ok(s) => s,
                            Err(e) => {
                                warn!("Background refresh: failed to prepare query: {}", e);
                                return;
                            }
                        };
                    let rows = match stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    }) {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("Background refresh: failed to query holdings: {}", e);
                            return;
                        }
                    };
                    rows.filter_map(|r| r.ok()).collect()
                };

                if symbols.is_empty() {
                    return;
                }

                // Determine quote providers.
                let config = services::quote_provider_service::get_quote_provider_config(&db)
                    .unwrap_or_default();

                // Load user-provided Xueqiu cookie and `u` value (if any) so that
                // API requests from the background refresh can use them.
                services::quote_service::set_xueqiu_user_cookie(
                    &quote_state,
                    config.xueqiu_cookie.clone(),
                );
                services::quote_service::set_xueqiu_user_u(&quote_state, config.xueqiu_u.clone());

                // Force-refresh all holding quotes from the upstream API.
                match services::quote_service::fetch_quotes_batch_cached_with_providers(
                    &quote_state,
                    &cache,
                    symbols,
                    &config.us_provider,
                    &config.hk_provider,
                    &config.cn_provider,
                    true,
                )
                .await
                {
                    Ok(quotes) => {
                        // Persist the freshly fetched quotes to the database.
                        let _ = services::quote_service::save_quotes_to_db(&db, &quotes);
                        let _ = services::quote_service::save_quote_refresh_time(&db);
                        // Peek at any warning (without consuming it) so we can
                        // include it in the quote-warning event.  The warning stays
                        // in managed state so the frontend's take_quote_warning
                        // command can still read it as a fallback if the event is
                        // missed, e.g. due to listener registration timing.
                        if let Some(warning) =
                            services::quote_service::peek_quote_warning(&quote_state)
                        {
                            let _ = handle.emit("quote-warning", warning);
                        }
                        // Notify the frontend so it can re-render with fresh prices.
                        let _ = handle.emit("quotes-refreshed", ());
                    }
                    Err(e) => warn!("Background quote refresh failed: {}", e),
                }
            });

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
            commands::transactions::update_transaction,
            commands::transactions::delete_transaction,
            commands::transactions::recalculate_holdings_cost,
            commands::quotes::get_real_time_quotes,
            commands::quotes::get_holding_quotes,
            commands::quotes::get_us_quote,
            commands::quotes::get_hk_quote,
            commands::quotes::get_cn_quote,
            commands::quotes::take_quote_warning,
            commands::quotes::get_last_quote_refresh_time,
            commands::exchange_rates::get_exchange_rates,
            commands::exchange_rates::convert_amount,
            commands::snapshots::backfill_snapshots,
            commands::dashboard::get_dashboard_summary,
            commands::dashboard::get_holdings_with_quotes,
            commands::statistics::get_statistics_overview,
            commands::statistics::get_statistics_by_market,
            commands::statistics::get_statistics_by_account,
            commands::dividends::get_dividend_analysis,
            commands::dividends::get_dividend_years,
            commands::statistics::get_statistics_by_category,
            commands::performance::get_performance_summary,
            commands::performance::get_performance_report,
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
            commands::quarterly::refresh_quarterly_snapshot,
            commands::quarterly::check_missing_snapshots,
            commands::quarterly::ensure_current_quarter_snapshot,
            commands::quarterly::compare_quarters,
            commands::quarterly::update_holding_notes,
            commands::quarterly::get_holding_notes_history,
            commands::quarterly::update_quarterly_notes,
            commands::quarterly::get_quarterly_notes_history,
            commands::quarterly::get_quarterly_trends,
            commands::quarterly::get_quarterly_transactions,
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
            commands::review::get_stock_operation_review,
            // Phase 6: AI Config
            commands::ai::get_ai_config,
            commands::ai::update_ai_config,
            commands::ai::get_default_system_prompt,
            commands::ai::fetch_ai_models,
            // AI Assistant (streaming chat)
            commands::ai::chat_with_ai,
            commands::ai::stop_ai_chat,
            // AI chat sessions & persisted messages
            commands::chat_sessions::create_chat_session,
            commands::chat_sessions::get_chat_sessions,
            commands::chat_sessions::rename_chat_session,
            commands::chat_sessions::delete_chat_session,
            commands::chat_sessions::touch_chat_session,
            commands::chat_sessions::get_chat_messages,
            commands::chat_sessions::save_chat_messages,
            commands::chat_sessions::clear_chat_session,
            commands::chat_sessions::generate_session_title,
            // AI Assistant skills (Markdown skill files)
            commands::skills::list_skills,
            commands::skills::get_skill,
            commands::skills::save_skill,
            commands::skills::delete_skill,
            commands::skills::reset_skills,
            commands::skills::clone_skill,
            commands::skills::export_skill,
            commands::skills::import_skill,
            // Quote Provider Config
            commands::quote_provider::get_quote_provider_config,
            commands::quote_provider::update_quote_provider_config,
            commands::quote_provider::capture_xueqiu_cookies,
            commands::quote_provider::parse_xueqiu_cookie_text,
            // OCR: import trades from 同花顺 screenshots
            commands::ocr::parse_trade_image,
            commands::ocr::lookup_cn_stock_code,
            commands::ocr::lookup_stock_name_by_symbol,
            // Options Management
            commands::options::import_options_csv,
            commands::options::get_option_contracts,
            commands::options::get_expired_option_stats,
            commands::options::simulate_sell_put,
            commands::options::simulate_sell_call,
            commands::options::delete_option_records,
            commands::options::export_options_csv,
            commands::options::parse_options_csv,
            commands::option_review::get_option_review,
            // Stock Splits (for option contract matching)
            commands::stock_splits::get_stock_splits,
            commands::stock_splits::add_stock_split,
            commands::stock_splits::delete_stock_split,
            commands::stock_splits::get_option_share_lots,
            commands::stock_splits::add_option_share_lot,
            commands::stock_splits::delete_option_share_lot,
            // Backup
            commands::backup::get_backup_config,
            commands::backup::set_backup_config,
            commands::backup::backup_database_now,
            // Factory reset (wipe all data & restore default settings)
            commands::reset::factory_reset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
