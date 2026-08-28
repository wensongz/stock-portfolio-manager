use rusqlite::{Connection, Result};
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
    pub path: String,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database {
            conn: Mutex::new(conn),
            path: path.to_string(),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                description TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS categories (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                color TEXT NOT NULL,
                icon TEXT NOT NULL,
                is_system INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
        ",
        )?;

        conn.execute_batch(
            "
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
        ",
        )?;

        conn.execute_batch("
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
        ")?;

        // Seed system categories (ignore if already exist)
        let categories = [
            (
                uuid::Uuid::new_v4().to_string(),
                "现金类",
                "#22C55E",
                "💵",
                1,
                1,
            ),
            (
                uuid::Uuid::new_v4().to_string(),
                "分红股",
                "#3B82F6",
                "💰",
                1,
                2,
            ),
            (
                uuid::Uuid::new_v4().to_string(),
                "成长股",
                "#F97316",
                "🚀",
                1,
                3,
            ),
            (
                uuid::Uuid::new_v4().to_string(),
                "套利",
                "#8B5CF6",
                "🔄",
                1,
                4,
            ),
        ];

        let now = chrono::Utc::now().to_rfc3339();
        for (id, name, color, icon, is_system, sort_order) in &categories {
            conn.execute(
                "INSERT OR IGNORE INTO categories (id, name, color, icon, is_system, sort_order, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
                 WHERE NOT EXISTS (SELECT 1 FROM categories WHERE name = ?2 AND is_system = 1)",
                rusqlite::params![id, name, color, icon, is_system, sort_order, now],
            )?;
        }

        conn.execute_batch(
            "
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
        ",
        )?;

        conn.execute_batch(
            "
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
        ",
        )?;

        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_daily_holding_snapshots_date
            ON daily_holding_snapshots(date);
        ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS benchmark_daily_prices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                symbol TEXT NOT NULL,
                date TEXT NOT NULL,
                close_price REAL NOT NULL DEFAULT 0,
                change_percent REAL NOT NULL DEFAULT 0,
                UNIQUE(symbol, date)
            );
        ",
        )?;

        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_benchmark_daily_prices_symbol_date
            ON benchmark_daily_prices(symbol, date);
        ",
        )?;

        conn.execute_batch(
            "
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
        ",
        )?;

        conn.execute_batch("
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
                notes TEXT
            );
        ")?;

        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_quarterly_holding_snapshots_snapshot_id
            ON quarterly_holding_snapshots(quarterly_snapshot_id);
        ",
        )?;

        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_quarterly_holding_snapshots_symbol
            ON quarterly_holding_snapshots(symbol);
        ",
        )?;

        // Add decision_quality column if not exists (migration)
        let _ = conn.execute_batch(
            "
            ALTER TABLE quarterly_holding_snapshots ADD COLUMN decision_quality TEXT;
        ",
        );

        conn.execute_batch("
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
        ")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS quote_provider_config (
                id INTEGER PRIMARY KEY DEFAULT 1,
                us_provider TEXT NOT NULL DEFAULT 'xueqiu',
                hk_provider TEXT NOT NULL DEFAULT 'xueqiu',
                cn_provider TEXT NOT NULL DEFAULT 'xueqiu',
                updated_at TEXT NOT NULL DEFAULT ''
            );
        ",
        )?;

        // Add xueqiu_cookie column if not exists (migration)
        let _ = conn.execute_batch(
            "
            ALTER TABLE quote_provider_config ADD COLUMN xueqiu_cookie TEXT;
        ",
        );

        // Add xueqiu_u column if not exists (migration)
        let _ = conn.execute_batch(
            "
            ALTER TABLE quote_provider_config ADD COLUMN xueqiu_u TEXT;
        ",
        );
        // NOTE: xueqiu_cookie (xq_a_token) and xueqiu_u (user ID) are
        // different values – do NOT copy one into the other.  Users who
        // previously only had xueqiu_cookie set will need to enter their
        // u value separately via the settings UI.

        // Add per-market cost adjustment setting columns (migration).
        // CN defaults to 1 (true) because A-share investors traditionally adjust
        // cost basis on every transaction. US and HK default to 0 (false) because
        // those markets realise gains on SELL and dividends are taxed separately.
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN cn_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 1;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN us_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN hk_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0;
        ");

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS ai_config (
                id INTEGER PRIMARY KEY DEFAULT 1,
                provider TEXT NOT NULL DEFAULT 'openai',
                api_key TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                base_url TEXT,
                -- The default system prompt lives in
                -- `models::ai_config::DEFAULT_SYSTEM_PROMPT`; `get_ai_config`
                -- falls back to it when no row is present.
                system_prompt TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL
            );
        ",
        )?;

        // Migration: add tools_enabled column if missing.
        let _ = conn.execute_batch(
            "ALTER TABLE ai_config ADD COLUMN tools_enabled INTEGER NOT NULL DEFAULT 1;",
        );

        // AI chat sessions & messages. Messages cascade-delete with their
        // session via the foreign key (FK enforcement is enabled at the top
        // of this function). Sessions are user-created named conversations;
        // each session owns a full ordered message history.
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS chat_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS chat_messages (
                id TEXT PRIMARY KEY NOT NULL,
                session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                cached_tokens INTEGER NOT NULL DEFAULT 0,
                -- Chain-of-thought text (reasoning_content) for thinking models.
                -- NULL for messages without reasoning. Assistant turns only.
                reasoning TEXT,
                -- JSON array of ToolCallInfo (status/args/result/duration) for
                -- assistant turns that called tools. NULL when none were used.
                tool_calls TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chat_messages_session
                ON chat_messages(session_id, created_at);
        ",
        )?;

        // Add reasoning + tool_calls columns to existing chat_messages tables
        // (migration for databases created before these columns existed). Both
        // are nullable TEXT — older rows simply have no reasoning/tool data.
        let _ = conn.execute_batch("ALTER TABLE chat_messages ADD COLUMN reasoning TEXT;");
        let _ = conn.execute_batch("ALTER TABLE chat_messages ADD COLUMN tool_calls TEXT;");

        conn.execute_batch(
            "
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
                updated_at TEXT NOT NULL
            );
        ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cached_exchange_rates (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                usd_cny REAL NOT NULL,
                usd_hkd REAL NOT NULL,
                cny_hkd REAL NOT NULL,
                updated_at TEXT NOT NULL
            );
        ",
        )?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cached_quote_refresh_time (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                updated_at TEXT NOT NULL
            );
        ",
        )?;

        conn.execute_batch(
            "
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
                created_at TEXT NOT NULL
            );
        ",
        )?;

        // Stock splits configuration for option contract matching
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS stock_splits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                stock_code TEXT NOT NULL,
                split_date TEXT NOT NULL,
                ratio_from INTEGER NOT NULL DEFAULT 1,
                ratio_to INTEGER NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;

        // Migration: add contract_status column to mark each open record's status
        let _ = conn.execute_batch(
            "ALTER TABLE option_records ADD COLUMN contract_status TEXT NOT NULL DEFAULT 'active';",
        );

        // Option share lot configuration: shares per contract per underlying
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS option_share_lots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                stock_code TEXT NOT NULL UNIQUE,
                shares_per_contract INTEGER NOT NULL DEFAULT 100,
                created_at TEXT NOT NULL
            );",
        )?;

        migrate_transactions_check_constraint(&conn)?;

        // Convert synthetic BUY records to OPEN type so they are correctly
        // treated as zero-cash-impact position entries everywhere.
        //
        // These migrations are idempotent (UPDATE with 0 rows is not an error)
        // and failures are tolerated – if they don't apply, the frontend filter
        // below provides a fallback by explicitly excluding 'backfill:initial'.
        //
        // 1. Records created by the backfill_open_transactions tool:
        //    these always carry notes = 'backfill:initial'.
        let _ = conn.execute_batch(
            "
            UPDATE transactions
            SET transaction_type = 'OPEN'
            WHERE transaction_type = 'BUY'
              AND notes = 'backfill:initial'
              AND symbol NOT LIKE '$CASH-%';
        ",
        );

        // 2. Records created by create_holding (initial position entries):
        //    identified by notes IS NULL, commission = 0, and the transaction's
        //    traded_at matching its parent holding's created_at exactly (because
        //    create_holding sets both to `now` in the same operation).
        let _ = conn.execute_batch(
            "
            UPDATE transactions
            SET transaction_type = 'OPEN'
            WHERE transaction_type = 'BUY'
              AND notes IS NULL
              AND commission = 0.0
              AND symbol NOT LIKE '$CASH-%'
              AND holding_id IS NOT NULL
              AND traded_at = (
                  SELECT h.created_at FROM holdings h WHERE h.id = holding_id
              );
        ",
        );

        conn.execute_batch(
            "
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
            );

            CREATE TABLE IF NOT EXISTS stock_market_sessions (
              market TEXT NOT NULL CHECK(market IN ('US','CN','HK')),
              date TEXT NOT NULL,
              is_session INTEGER NOT NULL DEFAULT 1 CHECK(is_session IN (0,1)),
              source TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              PRIMARY KEY (market, date)
            );

            CREATE TABLE IF NOT EXISTS stock_market_calendar_coverage (
              market TEXT PRIMARY KEY CHECK(market IN ('US','CN','HK')),
              source TEXT NOT NULL,
              complete_start TEXT NOT NULL,
              complete_through TEXT NOT NULL,
              revision TEXT NOT NULL,
              encodes_closed_dates INTEGER NOT NULL DEFAULT 0 CHECK(encodes_closed_dates IN (0,1)),
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS stock_review_annotations (
              id TEXT PRIMARY KEY,
              scope_type TEXT NOT NULL CHECK(scope_type IN ('period','stock','campaign','action')),
              scope_key TEXT NOT NULL,
              account_id TEXT,
              symbol TEXT,
              annotation_type TEXT NOT NULL,
              value_json TEXT NOT NULL,
              source TEXT NOT NULL CHECK(source IN ('user','ai_confirmed')),
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS stock_review_overrides (
              id TEXT PRIMARY KEY,
              override_type TEXT NOT NULL CHECK(override_type IN ('transfer','duplicate','same_day_order','non_trade')),
              transaction_ids_json TEXT NOT NULL,
              value_json TEXT NOT NULL,
              reference_fingerprint_json TEXT NOT NULL DEFAULT '[]',
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
        ",
        )?;

        let _ = conn.execute_batch(
            "ALTER TABLE stock_market_sessions ADD COLUMN is_session INTEGER NOT NULL DEFAULT 1 CHECK(is_session IN (0,1));",
        );

        // Existing installations created before durable override reference
        // snapshots need the column before corrections can be safely replayed.
        let has_override_reference_fingerprints = conn
            .prepare("PRAGMA table_info(stock_review_overrides)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?
            .iter()
            .any(|name| name == "reference_fingerprint_json");
        if !has_override_reference_fingerprints {
            conn.execute(
                "ALTER TABLE stock_review_overrides ADD COLUMN reference_fingerprint_json TEXT NOT NULL DEFAULT '[]'",
                [],
            )?;
        }

        Ok(())
    }
}

fn migrate_transactions_check_constraint(conn: &Connection) -> Result<()> {
    // Check if the transactions table already has 'PAY' in its CHECK constraint.
    // If not, recreate the table with the updated constraint.
    let schema: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='transactions'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if schema.contains("'PAY'") {
        return Ok(());
    }

    conn.execute_batch("
        PRAGMA foreign_keys = OFF;

        CREATE TABLE transactions_new (
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

        INSERT INTO transactions_new SELECT * FROM transactions;
        DROP TABLE transactions;
        ALTER TABLE transactions_new RENAME TO transactions;

        PRAGMA foreign_keys = ON;
    ")?;

    Ok(())
}

#[cfg(test)]
mod tests;
