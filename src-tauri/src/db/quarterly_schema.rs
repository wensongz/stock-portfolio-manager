use crate::services::historical_exchange_rate_service::{parse_rates, record_rates_in};
use rusqlite::{Connection, Result};

/// Run before v7's daily-cache cleanup when upgrading an older database.
pub(super) fn preserve_historical_rates(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS historical_exchange_rates (
           rate_date TEXT NOT NULL,
           observed_at TEXT NOT NULL,
           source TEXT NOT NULL,
           exchange_rates TEXT NOT NULL,
           recorded_at TEXT NOT NULL,
           PRIMARY KEY (observed_at, source)
         );
         CREATE INDEX IF NOT EXISTS idx_historical_exchange_rates_date
           ON historical_exchange_rates(rate_date, observed_at);",
    )?;
    for (table, source) in [
        ("quarterly_snapshots", "legacy_quarterly_snapshot"),
        ("daily_portfolio_values", "legacy_daily_snapshot"),
    ] {
        let mut stmt = conn.prepare(&format!(
            "SELECT exchange_rates FROM {table} ORDER BY rowid"
        ))?;
        let jsons = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>>>()?;
        for json in jsons {
            let Ok(rates) = parse_rates(&json, source) else {
                continue;
            };
            let valid_date: Option<String> =
                conn.query_row("SELECT DATE(?1)", [&rates.updated_at], |row| row.get(0))?;
            if valid_date.is_none() {
                continue;
            }
            record_rates_in(conn, &rates, source).map_err(rusqlite::Error::InvalidParameterName)?;
        }
    }
    let rates = conn
        .prepare("SELECT usd_cny,usd_hkd,cny_hkd,updated_at FROM cached_exchange_rates")?
        .query_map([], |row| {
            Ok(crate::models::ExchangeRates {
                usd_cny: row.get(0)?,
                usd_hkd: row.get(1)?,
                cny_hkd: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    for rate in rates {
        let json = serde_json::to_string(&rate).unwrap_or_default();
        let date: Option<String> =
            conn.query_row("SELECT DATE(?1)", [&rate.updated_at], |row| row.get(0))?;
        if parse_rates(&json, "observed").is_ok() && date.is_some() {
            record_rates_in(conn, &rate, "observed")
                .map_err(rusqlite::Error::InvalidParameterName)?;
        }
    }
    Ok(())
}

pub(super) fn migrate_currency(conn: &Connection) -> Result<()> {
    if !super::migrations::column_exists(conn, "quarterly_holding_snapshots", "currency")? {
        conn.execute_batch(
            "ALTER TABLE quarterly_holding_snapshots ADD COLUMN currency TEXT NOT NULL DEFAULT '';",
        )?;
    }
    conn.execute_batch(
        "UPDATE quarterly_holding_snapshots SET currency =
           CASE WHEN UPPER(symbol) IN ('$CASH-USD','$CASH-CNY','$CASH-HKD') THEN SUBSTR(UPPER(symbol),7)
                WHEN market='CN' THEN 'CNY' WHEN market='HK' THEN 'HKD' ELSE 'USD' END
         WHERE currency='';",
    )
}
