use super::*;

/// Update the notes for a specific holding in a quarterly snapshot.
pub fn update_holding_notes(
    db: &Database,
    snapshot_id: &str,
    holding_snapshot_id: &str,
    notes: &str,
) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let rows = conn
        .execute(
            "UPDATE quarterly_holding_snapshots SET notes = ?1
             WHERE quarterly_snapshot_id = ?2 AND id = ?3",
            rusqlite::params![notes, snapshot_id, holding_snapshot_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(rows > 0)
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
