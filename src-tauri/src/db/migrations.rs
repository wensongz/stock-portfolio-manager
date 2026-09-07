use super::schema;
use rusqlite::{Connection, Error, OptionalExtension, Result};

pub(crate) const CURRENT_SCHEMA_VERSION: i64 = 8;

pub(crate) fn run_migrations(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(Error::InvalidParameterName(format!(
            "database schema version {version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        )));
    }
    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = conn.transaction()?;
    schema::create_current_schema(&transaction)?;
    if version < 1 {
        migrate_unversioned_database(&transaction)?;
    }
    if version < 2 {
        migrate_v2(&transaction)?;
    }
    if version < 3 {
        migrate_v3(&transaction)?;
    }
    if version < 4 {
        migrate_v4(&transaction)?;
    }
    if version < 5 {
        migrate_v5(&transaction)?;
    }
    if version < 6 {
        schema::create_import_batch_schema(&transaction)?;
    }
    if version < 8 {
        super::quarterly_schema::preserve_historical_rates(&transaction)?;
    }
    if version < 7 {
        super::snapshot_cache_schema::migrate_v7(&transaction)?;
    }
    if version < 8 {
        super::quarterly_schema::migrate_currency(&transaction)?;
    }
    transaction.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
    transaction.commit()
}

fn migrate_v2(conn: &Connection) -> Result<()> {
    schema::create_portfolio_query_indexes(conn)
}

fn migrate_v3(conn: &Connection) -> Result<()> {
    for (column, definition) in [
        ("pe_ttm", "REAL"),
        ("pb", "REAL"),
        ("market_cap", "REAL"),
        ("dividend_yield", "REAL"),
        ("eps", "REAL"),
        ("roe", "REAL"),
        ("turnover_rate", "REAL"),
    ] {
        add_column_if_missing(
            conn,
            "cached_quotes",
            column,
            &format!("ALTER TABLE cached_quotes ADD COLUMN {column} {definition}"),
        )?;
    }
    Ok(())
}

fn migrate_v4(conn: &Connection) -> Result<()> {
    schema::create_portfolio_alert_schema(conn)
}

fn migrate_v5(conn: &Connection) -> Result<()> {
    let primary_key_columns: Vec<String> = conn
        .prepare(
            "SELECT name FROM pragma_table_info('cached_quotes')
             WHERE pk > 0 ORDER BY pk",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_>>()?;
    if primary_key_columns == ["market", "symbol"] {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TABLE cached_quotes_v5 (
           symbol TEXT NOT NULL,
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
           turnover_rate REAL,
           PRIMARY KEY (market, symbol)
         );
         INSERT INTO cached_quotes_v5 (
           symbol, name, market, current_price, previous_close, change,
           change_percent, high, low, volume, updated_at, pe_ttm, pb,
           market_cap, dividend_yield, eps, roe, turnover_rate
         )
         SELECT
           symbol, name, market, current_price, previous_close, change,
           change_percent, high, low, volume, updated_at, pe_ttm, pb,
           market_cap, dividend_yield, eps, roe, turnover_rate
         FROM cached_quotes;
         DROP TABLE cached_quotes;
         ALTER TABLE cached_quotes_v5 RENAME TO cached_quotes;",
    )
}

pub(crate) fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2 LIMIT 1",
        rusqlite::params![table, column],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
}

fn migrate_unversioned_database(conn: &Connection) -> Result<()> {
    add_column_if_missing(
        conn,
        "quarterly_holding_snapshots",
        "decision_quality",
        "ALTER TABLE quarterly_holding_snapshots ADD COLUMN decision_quality TEXT",
    )?;
    add_column_if_missing(
        conn,
        "quote_provider_config",
        "xueqiu_cookie",
        "ALTER TABLE quote_provider_config ADD COLUMN xueqiu_cookie TEXT",
    )?;
    add_column_if_missing(
        conn,
        "quote_provider_config",
        "xueqiu_u",
        "ALTER TABLE quote_provider_config ADD COLUMN xueqiu_u TEXT",
    )?;
    add_column_if_missing(
        conn,
        "quote_provider_config",
        "cn_adjust_sell_pay_cost",
        "ALTER TABLE quote_provider_config ADD COLUMN cn_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "quote_provider_config",
        "us_adjust_sell_pay_cost",
        "ALTER TABLE quote_provider_config ADD COLUMN us_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        conn,
        "quote_provider_config",
        "hk_adjust_sell_pay_cost",
        "ALTER TABLE quote_provider_config ADD COLUMN hk_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        conn,
        "ai_config",
        "tools_enabled",
        "ALTER TABLE ai_config ADD COLUMN tools_enabled INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "chat_messages",
        "reasoning",
        "ALTER TABLE chat_messages ADD COLUMN reasoning TEXT",
    )?;
    add_column_if_missing(
        conn,
        "chat_messages",
        "tool_calls",
        "ALTER TABLE chat_messages ADD COLUMN tool_calls TEXT",
    )?;
    add_column_if_missing(
        conn,
        "option_records",
        "contract_status",
        "ALTER TABLE option_records ADD COLUMN contract_status TEXT NOT NULL DEFAULT 'active'",
    )?;

    migrate_transactions_check_constraint(conn)?;
    conn.execute_batch(
        "UPDATE transactions
         SET transaction_type = 'OPEN'
         WHERE transaction_type = 'BUY'
           AND notes = 'backfill:initial'
           AND symbol NOT LIKE '$CASH-%';

         UPDATE transactions
         SET transaction_type = 'OPEN'
         WHERE transaction_type = 'BUY'
           AND notes IS NULL
           AND commission = 0.0
           AND symbol NOT LIKE '$CASH-%'
           AND holding_id IS NOT NULL
           AND traded_at = (
             SELECT holdings.created_at FROM holdings WHERE holdings.id = transactions.holding_id
           );",
    )?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    statement: &str,
) -> Result<()> {
    if !column_exists(conn, table, column)? {
        conn.execute_batch(statement)?;
    }
    Ok(())
}

fn migrate_transactions_check_constraint(conn: &Connection) -> Result<()> {
    let definition: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'transactions'",
        [],
        |row| row.get(0),
    )?;
    if definition.contains("'OPEN'") && definition.contains("'PAY'") {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TABLE transactions_new (
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
         INSERT INTO transactions_new (
           id, holding_id, account_id, symbol, name, market, transaction_type,
           shares, price, total_amount, commission, currency, traded_at, notes, created_at
         )
         SELECT
           id, holding_id, account_id, symbol, name, market, transaction_type,
           shares, price, total_amount, commission, currency, traded_at, notes, created_at
         FROM transactions;
         DROP TABLE transactions;
         ALTER TABLE transactions_new RENAME TO transactions;",
    )
}
