use crate::db::Database;
use crate::models::review::{DecisionStatistics, HoldingReview, QuarterlyHoldingStatus};

pub fn get_holding_review(db: &Database, symbol: &str) -> Result<HoldingReview, String> {
    let conn = db.conn.lock().unwrap();

    // Check if this is a current holding
    let is_current: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM holdings WHERE symbol = ?1",
            rusqlite::params![symbol],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    // Get name and market from quarterly snapshots
    let (name, market) = conn
        .query_row(
            "SELECT name, market FROM quarterly_holding_snapshots WHERE symbol = ?1 LIMIT 1",
            rusqlite::params![symbol],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap_or_else(|_| (symbol.to_string(), "US".to_string()));

    // Build quarterly timeline
    let mut stmt = conn
        .prepare(
            "SELECT qs.id, qs.quarter, qhs.shares, qhs.avg_cost, qhs.close_price, qhs.pnl_percent,
                    qhs.notes, qhs.decision_quality
             FROM quarterly_holding_snapshots qhs
             JOIN quarterly_snapshots qs ON qs.id = qhs.quarterly_snapshot_id
             WHERE qhs.symbol = ?1
             ORDER BY qs.quarter ASC",
        )
        .map_err(|e| e.to_string())?;

    let timeline = stmt
        .query_map(rusqlite::params![symbol], |row| {
            Ok(QuarterlyHoldingStatus {
                snapshot_id: row.get(0)?,
                quarter: row.get(1)?,
                shares: row.get(2)?,
                avg_cost: row.get(3)?,
                close_price: row.get(4)?,
                pnl_percent: row.get(5)?,
                notes: row.get(6)?,
                decision_quality: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(HoldingReview {
        symbol: symbol.to_string(),
        name,
        market,
        is_current_holding: is_current,
        quarterly_timeline: timeline,
    })
}

pub fn update_decision_quality(
    db: &Database,
    snapshot_id: &str,
    symbol: &str,
    quality: &str,
) -> Result<bool, String> {
    let conn = db.conn.lock().unwrap();
    let rows = conn
        .execute(
            "UPDATE quarterly_holding_snapshots SET decision_quality = ?1
             WHERE quarterly_snapshot_id = ?2 AND symbol = ?3",
            rusqlite::params![quality, snapshot_id, symbol],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}

pub fn get_decision_statistics(db: &Database) -> Result<DecisionStatistics, String> {
    let conn = db.conn.lock().unwrap();

    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quarterly_holding_snapshots WHERE decision_quality IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let correct: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quarterly_holding_snapshots WHERE decision_quality = 'correct'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let wrong: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quarterly_holding_snapshots WHERE decision_quality = 'wrong'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM quarterly_holding_snapshots WHERE decision_quality = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let accuracy_rate = if correct + wrong > 0 {
        correct as f64 / (correct + wrong) as f64
    } else {
        0.0
    };

    Ok(DecisionStatistics {
        total_decisions: total as usize,
        correct_count: correct as usize,
        wrong_count: wrong as usize,
        pending_count: pending as usize,
        accuracy_rate,
    })
}

/// Get all symbols that have appeared in quarterly snapshots
pub fn get_reviewed_symbols(db: &Database) -> Result<Vec<(String, String, String)>, String> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT symbol, name, market FROM quarterly_holding_snapshots ORDER BY symbol",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}
