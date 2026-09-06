use rusqlite::{Connection, Result};

pub(crate) const SYSTEM_CATEGORIES: [(&str, &str, &str, i64); 4] = [
    ("现金类", "#22C55E", "💵", 1),
    ("分红股", "#3B82F6", "💰", 2),
    ("成长股", "#F97316", "🚀", 3),
    ("套利", "#8B5CF6", "🔄", 4),
];

pub(super) fn create_current_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounts (
           id TEXT PRIMARY KEY NOT NULL,
           name TEXT NOT NULL,
           market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
           description TEXT,
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS categories (
           id TEXT PRIMARY KEY NOT NULL,
           name TEXT NOT NULL,
           color TEXT NOT NULL,
           icon TEXT NOT NULL,
           is_system INTEGER NOT NULL DEFAULT 0,
           sort_order INTEGER NOT NULL DEFAULT 0,
           created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS holdings (
           id TEXT PRIMARY KEY NOT NULL,
           account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
           symbol TEXT NOT NULL,
           name TEXT NOT NULL,
           market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
           category_id TEXT REFERENCES categories(id) ON DELETE SET NULL,
           shares REAL NOT NULL DEFAULT 0,
           avg_cost REAL NOT NULL DEFAULT 0,
           currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS transactions (
           id TEXT PRIMARY KEY NOT NULL,
           holding_id TEXT REFERENCES holdings(id) ON DELETE SET NULL,
           account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
           symbol TEXT NOT NULL,
           name TEXT NOT NULL,
           market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
           transaction_type TEXT NOT NULL CHECK(transaction_type IN ('BUY', 'SELL', 'OPEN', 'PAY')),
           shares REAL NOT NULL,
           price REAL NOT NULL,
           total_amount REAL NOT NULL,
           commission REAL NOT NULL DEFAULT 0,
           currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
           traded_at TEXT NOT NULL,
           notes TEXT,
           created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS daily_portfolio_values (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           date TEXT NOT NULL UNIQUE,
           total_cost REAL NOT NULL DEFAULT 0,
           total_value REAL NOT NULL DEFAULT 0,
           us_cost REAL NOT NULL DEFAULT 0,
           us_value REAL NOT NULL DEFAULT 0,
           cn_cost REAL NOT NULL DEFAULT 0,
           cn_value REAL NOT NULL DEFAULT 0,
           hk_cost REAL NOT NULL DEFAULT 0,
           hk_value REAL NOT NULL DEFAULT 0,
           exchange_rates TEXT NOT NULL DEFAULT '{}',
           daily_pnl REAL NOT NULL DEFAULT 0,
           cumulative_pnl REAL NOT NULL DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS daily_holding_snapshots (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           date TEXT NOT NULL,
           account_id TEXT NOT NULL,
           symbol TEXT NOT NULL,
           market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
           category_name TEXT,
           shares REAL NOT NULL DEFAULT 0,
           avg_cost REAL NOT NULL DEFAULT 0,
           close_price REAL NOT NULL DEFAULT 0,
           market_value REAL NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_daily_holding_snapshots_date
           ON daily_holding_snapshots(date);

         CREATE TABLE IF NOT EXISTS benchmark_daily_prices (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           symbol TEXT NOT NULL,
           date TEXT NOT NULL,
           close_price REAL NOT NULL DEFAULT 0,
           change_percent REAL NOT NULL DEFAULT 0,
           UNIQUE(symbol, date)
         );
         CREATE INDEX IF NOT EXISTS idx_benchmark_daily_prices_symbol_date
           ON benchmark_daily_prices(symbol, date);

         CREATE TABLE IF NOT EXISTS quarterly_snapshots (
           id TEXT PRIMARY KEY NOT NULL,
           quarter TEXT NOT NULL UNIQUE,
           snapshot_date TEXT NOT NULL,
           total_value REAL NOT NULL DEFAULT 0,
           total_cost REAL NOT NULL DEFAULT 0,
           total_pnl REAL NOT NULL DEFAULT 0,
           us_value REAL NOT NULL DEFAULT 0,
           us_cost REAL NOT NULL DEFAULT 0,
           cn_value REAL NOT NULL DEFAULT 0,
           cn_cost REAL NOT NULL DEFAULT 0,
           hk_value REAL NOT NULL DEFAULT 0,
           hk_cost REAL NOT NULL DEFAULT 0,
           exchange_rates TEXT NOT NULL DEFAULT '{}',
           overall_notes TEXT,
           created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS quarterly_holding_snapshots (
           id TEXT PRIMARY KEY NOT NULL,
           quarterly_snapshot_id TEXT NOT NULL REFERENCES quarterly_snapshots(id) ON DELETE CASCADE,
           account_id TEXT NOT NULL,
           account_name TEXT NOT NULL DEFAULT '',
           symbol TEXT NOT NULL,
           name TEXT NOT NULL,
           market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
           category_name TEXT NOT NULL DEFAULT '未分类',
           category_color TEXT NOT NULL DEFAULT '#8B8B8B',
           shares REAL NOT NULL DEFAULT 0,
           avg_cost REAL NOT NULL DEFAULT 0,
           close_price REAL NOT NULL DEFAULT 0,
           market_value REAL NOT NULL DEFAULT 0,
           cost_value REAL NOT NULL DEFAULT 0,
           pnl REAL NOT NULL DEFAULT 0,
           pnl_percent REAL NOT NULL DEFAULT 0,
           weight REAL NOT NULL DEFAULT 0,
           notes TEXT,
           decision_quality TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_quarterly_holding_snapshots_snapshot_id
           ON quarterly_holding_snapshots(quarterly_snapshot_id);
         CREATE INDEX IF NOT EXISTS idx_quarterly_holding_snapshots_symbol
           ON quarterly_holding_snapshots(symbol);

         CREATE TABLE IF NOT EXISTS price_alerts (
           id TEXT PRIMARY KEY NOT NULL,
           holding_id TEXT,
           symbol TEXT NOT NULL,
           name TEXT NOT NULL,
           market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
           alert_type TEXT NOT NULL CHECK(alert_type IN ('PRICE_ABOVE', 'PRICE_BELOW', 'CHANGE_ABOVE', 'CHANGE_BELOW', 'PNL_ABOVE', 'PNL_BELOW')),
           threshold REAL NOT NULL,
           is_active INTEGER NOT NULL DEFAULT 1,
           is_triggered INTEGER NOT NULL DEFAULT 0,
           triggered_at TEXT,
           created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS quote_provider_config (
           id INTEGER PRIMARY KEY DEFAULT 1,
           us_provider TEXT NOT NULL DEFAULT 'xueqiu',
           hk_provider TEXT NOT NULL DEFAULT 'xueqiu',
           cn_provider TEXT NOT NULL DEFAULT 'xueqiu',
           updated_at TEXT NOT NULL DEFAULT '',
           xueqiu_cookie TEXT,
           xueqiu_u TEXT,
           cn_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 1,
           us_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0,
           hk_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS ai_config (
           id INTEGER PRIMARY KEY DEFAULT 1,
           provider TEXT NOT NULL DEFAULT 'openai',
           api_key TEXT NOT NULL DEFAULT '',
           model TEXT NOT NULL DEFAULT '',
           base_url TEXT,
           system_prompt TEXT NOT NULL DEFAULT '',
           updated_at TEXT NOT NULL,
           tools_enabled INTEGER NOT NULL DEFAULT 1
         );

         CREATE TABLE IF NOT EXISTS chat_sessions (
           id TEXT PRIMARY KEY NOT NULL,
           name TEXT NOT NULL,
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS chat_messages (
           id TEXT PRIMARY KEY NOT NULL,
           session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
           role TEXT NOT NULL,
           content TEXT NOT NULL,
           prompt_tokens INTEGER NOT NULL DEFAULT 0,
           completion_tokens INTEGER NOT NULL DEFAULT 0,
           total_tokens INTEGER NOT NULL DEFAULT 0,
           cached_tokens INTEGER NOT NULL DEFAULT 0,
           reasoning TEXT,
           tool_calls TEXT,
           created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_chat_messages_session
           ON chat_messages(session_id, created_at);

         CREATE TABLE IF NOT EXISTS cached_quotes (
           symbol TEXT PRIMARY KEY NOT NULL,
           name TEXT NOT NULL,
           market TEXT NOT NULL,
           current_price REAL NOT NULL DEFAULT 0,
           previous_close REAL NOT NULL DEFAULT 0,
           change REAL NOT NULL DEFAULT 0,
           change_percent REAL NOT NULL DEFAULT 0,
           high REAL NOT NULL DEFAULT 0,
           low REAL NOT NULL DEFAULT 0,
           volume INTEGER NOT NULL DEFAULT 0,
           updated_at TEXT NOT NULL,
           pe_ttm REAL,
           pb REAL,
           market_cap REAL,
           dividend_yield REAL,
           eps REAL,
           roe REAL,
           turnover_rate REAL
         );

         CREATE TABLE IF NOT EXISTS cached_exchange_rates (
           id INTEGER PRIMARY KEY CHECK (id = 1),
           usd_cny REAL NOT NULL,
           usd_hkd REAL NOT NULL,
           cny_hkd REAL NOT NULL,
           updated_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS cached_quote_refresh_time (
           id INTEGER PRIMARY KEY CHECK (id = 1),
           updated_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS option_records (
           id TEXT PRIMARY KEY NOT NULL,
           account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
           option_symbol TEXT NOT NULL,
           underlying TEXT NOT NULL,
           expiry_date TEXT NOT NULL,
           strike_price REAL NOT NULL,
           option_type TEXT NOT NULL CHECK(option_type IN ('P', 'C')),
           action TEXT NOT NULL CHECK(action IN ('SELL', 'BUY')),
           code TEXT NOT NULL,
           quantity INTEGER NOT NULL,
           price REAL NOT NULL,
           amount REAL NOT NULL,
           commission REAL NOT NULL DEFAULT 0,
           fee REAL NOT NULL DEFAULT 0,
           traded_at TEXT,
           settled_at TEXT,
           created_at TEXT NOT NULL,
           contract_status TEXT NOT NULL DEFAULT 'active'
         );

         CREATE TABLE IF NOT EXISTS stock_splits (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           stock_code TEXT NOT NULL,
           split_date TEXT NOT NULL,
           ratio_from INTEGER NOT NULL DEFAULT 1,
           ratio_to INTEGER NOT NULL,
           created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS option_share_lots (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           stock_code TEXT NOT NULL UNIQUE,
           shares_per_contract INTEGER NOT NULL DEFAULT 100,
           created_at TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS stock_daily_prices (
           symbol TEXT NOT NULL,
           market TEXT NOT NULL,
           date TEXT NOT NULL,
           open REAL,
           high REAL,
           low REAL,
           close REAL NOT NULL,
           volume REAL,
           adjusted_close REAL,
           dividend REAL,
           source TEXT NOT NULL,
           updated_at TEXT NOT NULL,
           PRIMARY KEY (symbol, market, date)
         );",
    )?;

    create_portfolio_alert_schema(conn)?;
    create_portfolio_query_indexes(conn)?;

    let now = chrono::Utc::now().to_rfc3339();
    for (name, color, icon, sort_order) in SYSTEM_CATEGORIES {
        conn.execute(
            "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at)
             SELECT ?1, ?2, ?3, ?4, 1, ?5, ?6
             WHERE NOT EXISTS (
               SELECT 1 FROM categories WHERE name = ?2 AND is_system = 1
             )",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                name,
                color,
                icon,
                sort_order,
                now
            ],
        )?;
    }
    Ok(())
}

pub(super) fn create_portfolio_alert_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS portfolio_alert_configs (
           id TEXT PRIMARY KEY,
           scope_key TEXT NOT NULL UNIQUE,
           scope_kind TEXT NOT NULL CHECK (scope_kind IN ('OVERALL', 'MARKET', 'ACCOUNT')),
           market TEXT,
           account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
           base_currency TEXT NOT NULL CHECK (base_currency IN ('USD', 'CNY', 'HKD')),
           deviation_threshold REAL NOT NULL DEFAULT 20 CHECK (deviation_threshold >= 0 AND deviation_threshold <= 100),
           concentration_threshold REAL NOT NULL DEFAULT 20 CHECK (concentration_threshold > 0 AND concentration_threshold <= 100),
           is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
           last_snapshot_json TEXT,
           last_evaluated_at TEXT,
           created_at TEXT NOT NULL,
           updated_at TEXT NOT NULL,
           CHECK (
               (scope_kind = 'OVERALL' AND market IS NULL AND account_id IS NULL) OR
               (scope_kind = 'MARKET' AND market IN ('CN', 'US', 'HK') AND account_id IS NULL) OR
               (scope_kind = 'ACCOUNT' AND market IS NULL AND account_id IS NOT NULL)
           )
         );
         CREATE INDEX IF NOT EXISTS idx_portfolio_alert_configs_is_active
           ON portfolio_alert_configs(is_active);

         CREATE TABLE IF NOT EXISTS portfolio_alert_targets (
           config_id TEXT NOT NULL REFERENCES portfolio_alert_configs(id) ON DELETE CASCADE,
           category_id TEXT NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
           target_percent REAL NOT NULL CHECK (target_percent >= 0 AND target_percent <= 100),
           PRIMARY KEY (config_id, category_id)
         );

         CREATE TABLE IF NOT EXISTS portfolio_alert_breaches (
           config_id TEXT NOT NULL REFERENCES portfolio_alert_configs(id) ON DELETE CASCADE,
           breach_key TEXT NOT NULL,
           breach_kind TEXT NOT NULL CHECK (breach_kind IN ('CATEGORY_DEVIATION', 'CONCENTRATION')),
           direction TEXT NOT NULL CHECK (direction IN ('OVERWEIGHT', 'UNDERWEIGHT', 'ABOVE_LIMIT')),
           first_triggered_at TEXT NOT NULL,
           last_seen_at TEXT NOT NULL,
           PRIMARY KEY (config_id, breach_key)
         );
         CREATE INDEX IF NOT EXISTS idx_portfolio_alert_breaches_config_id
           ON portfolio_alert_breaches(config_id);",
    )
}

pub(super) fn create_portfolio_query_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_transactions_account_symbol_traded_at
           ON transactions(account_id, UPPER(symbol), traded_at);
         CREATE INDEX IF NOT EXISTS idx_transactions_account_traded_at
           ON transactions(account_id, traded_at DESC);
         CREATE INDEX IF NOT EXISTS idx_transactions_holding_id
           ON transactions(holding_id);
         CREATE INDEX IF NOT EXISTS idx_option_records_account_symbol_traded_at
           ON option_records(account_id, option_symbol, traded_at);",
    )
}
