use chrono::{Days, NaiveDate};
use rusqlite::Connection;

/// Backfill enough prior days to recover the period's opening valuation across
/// a weekend or an extended exchange holiday. The report keeps its own start.
pub(crate) fn backfill_start_with_baseline(start: NaiveDate) -> NaiveDate {
    start
        .checked_sub_days(Days::new(14))
        .unwrap_or(NaiveDate::MIN)
}

/// Use SQLite's UTC date conversion, just like historical transaction queries.
/// An unparseable legacy date invalidates all days rather than retaining stale data.
pub(crate) fn snapshot_date(conn: &Connection, traded_at: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT COALESCE(DATE(?1), '0000-01-01')",
        [traded_at],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

/// Call within the ledger mutation's transaction so cache and ledger commit together.
pub(crate) fn invalidate_from(conn: &Connection, traded_at: &str) -> Result<(), String> {
    let date = snapshot_date(conn, traded_at)?;
    conn.execute(
        "DELETE FROM daily_holding_snapshots WHERE date >= ?1",
        [&date],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "DELETE FROM daily_portfolio_values WHERE date >= ?1",
        [&date],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Revision triggers also cover bulk imports, undo and direct holding changes.
pub(crate) fn current_revision(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT revision FROM snapshot_cache_state WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}
