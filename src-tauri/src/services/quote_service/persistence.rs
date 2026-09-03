use crate::db::Database;
use crate::models::StockQuote;
use chrono::Utc;
use rusqlite::OptionalExtension;

pub fn load_quotes_from_db(db: &Database) -> Result<Vec<StockQuote>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT symbol, name, market, current_price, previous_close,
                    change, change_percent, high, low, volume, updated_at
             FROM cached_quotes",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(StockQuote {
                symbol: row.get(0)?,
                name: row.get(1)?,
                market: row.get(2)?,
                current_price: row.get(3)?,
                previous_close: row.get(4)?,
                change: row.get(5)?,
                change_percent: row.get(6)?,
                high: row.get(7)?,
                low: row.get(8)?,
                volume: row.get(9)?,
                updated_at: row.get(10)?,
                ..Default::default()
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Persist quotes to the database (upsert).
pub fn save_quotes_to_db(db: &Database, quotes: &[StockQuote]) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "INSERT OR REPLACE INTO cached_quotes
                (symbol, name, market, current_price, previous_close,
                 change, change_percent, high, low, volume, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .map_err(|e| e.to_string())?;
    for q in quotes {
        stmt.execute(rusqlite::params![
            q.symbol,
            q.name,
            q.market,
            q.current_price,
            q.previous_close,
            q.change,
            q.change_percent,
            q.high,
            q.low,
            q.volume,
            q.updated_at,
        ])
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Persist the last quote refresh timestamp to the database (single-row upsert, id=1).
pub fn save_quote_refresh_time(db: &Database) -> Result<String, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO cached_quote_refresh_time (id, updated_at) VALUES (1, ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| e.to_string())?;
    Ok(now)
}

/// Load the persisted last quote refresh timestamp from the database.
/// Returns `None` if no refresh has been recorded yet.
pub fn get_quote_refresh_time(db: &Database) -> Result<Option<String>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT updated_at FROM cached_quote_refresh_time WHERE id = 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}
