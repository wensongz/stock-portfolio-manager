use super::*;

/// Update the notes for a specific holding in a quarterly snapshot.
pub fn update_holding_notes(
    db: &Database,
    snapshot_id: &str,
    symbol: &str,
    notes: &str,
) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let rows = conn
        .execute(
            "UPDATE quarterly_holding_snapshots SET notes = ?1
             WHERE quarterly_snapshot_id = ?2 AND symbol = ?3",
            rusqlite::params![notes, snapshot_id, symbol],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}

/// Get the notes history for a specific symbol across all quarterly snapshots.
pub fn get_holding_notes_history(
    db: &Database,
    symbol: &str,
) -> Result<Vec<HoldingNoteHistory>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT qs.quarter, qs.snapshot_date, qhs.shares, qhs.avg_cost,
                    qhs.close_price, qhs.pnl_percent, COALESCE(qhs.notes, '')
             FROM quarterly_holding_snapshots qhs
             JOIN quarterly_snapshots qs ON qhs.quarterly_snapshot_id = qs.id
             WHERE qhs.symbol = ?1
             ORDER BY qs.quarter DESC",
        )
        .map_err(|e| e.to_string())?;

    let history = stmt
        .query_map(rusqlite::params![symbol], |row| {
            Ok(HoldingNoteHistory {
                quarter: row.get(0)?,
                snapshot_date: row.get(1)?,
                shares: row.get(2)?,
                avg_cost: row.get(3)?,
                close_price: row.get(4)?,
                pnl_percent: row.get(5)?,
                notes: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(history)
}

/// Update the overall notes for a quarterly snapshot.
pub fn update_quarterly_notes(
    db: &Database,
    snapshot_id: &str,
    notes: &str,
) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let rows = conn
        .execute(
            "UPDATE quarterly_snapshots SET overall_notes = ?1 WHERE id = ?2",
            rusqlite::params![notes, snapshot_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
}
