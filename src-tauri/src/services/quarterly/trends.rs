use super::*;

/// Get multi-quarter trend data.
pub fn get_quarterly_trends(db: &Database) -> Result<QuarterlyTrends, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Get all snapshots ordered by quarter
    let mut stmt = conn
        .prepare(
            "SELECT id, quarter, total_value, total_cost, total_pnl,
                    us_value, cn_value, hk_value
             FROM quarterly_snapshots
             ORDER BY quarter ASC",
        )
        .map_err(|e| e.to_string())?;

    struct SnapRow {
        id: String,
        quarter: String,
        total_value: f64,
        total_cost: f64,
        total_pnl: f64,
        us_value: f64,
        cn_value: f64,
        hk_value: f64,
    }

    let snap_rows: Vec<SnapRow> = stmt
        .query_map([], |row| {
            Ok(SnapRow {
                id: row.get(0)?,
                quarter: row.get(1)?,
                total_value: row.get(2)?,
                total_cost: row.get(3)?,
                total_pnl: row.get(4)?,
                us_value: row.get(5)?,
                cn_value: row.get(6)?,
                hk_value: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if snap_rows.is_empty() {
        return Ok(QuarterlyTrends {
            quarters: vec![],
            total_values: vec![],
            total_costs: vec![],
            total_pnls: vec![],
            market_values: HashMap::new(),
            category_values: HashMap::new(),
            holding_counts: vec![],
        });
    }

    let quarters: Vec<String> = snap_rows.iter().map(|r| r.quarter.clone()).collect();
    let total_values: Vec<f64> = snap_rows.iter().map(|r| r.total_value).collect();
    let total_costs: Vec<f64> = snap_rows.iter().map(|r| r.total_cost).collect();
    let total_pnls: Vec<f64> = snap_rows.iter().map(|r| r.total_pnl).collect();

    let mut market_values: HashMap<String, Vec<f64>> = HashMap::new();
    market_values.insert(
        "US".to_string(),
        snap_rows.iter().map(|r| r.us_value).collect(),
    );
    market_values.insert(
        "CN".to_string(),
        snap_rows.iter().map(|r| r.cn_value).collect(),
    );
    market_values.insert(
        "HK".to_string(),
        snap_rows.iter().map(|r| r.hk_value).collect(),
    );

    // Get category breakdown per snapshot
    let all_cats: Vec<(String, String)> = {
        let mut cats: Vec<(String, String)> = Vec::new();
        let mut cat_stmt = conn
            .prepare(
                "SELECT DISTINCT category_name, category_color FROM quarterly_holding_snapshots",
            )
            .map_err(|e| e.to_string())?;
        cat_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .for_each(|c| {
                if !cats.iter().any(|(n, _)| n == &c.0) {
                    cats.push(c);
                }
            });
        cats
    };

    let mut category_values: HashMap<String, Vec<f64>> = HashMap::new();
    for (cat_name, _) in &all_cats {
        let mut values = Vec::new();
        for snap in &snap_rows {
            let sum: f64 = {
                conn.query_row(
                    "SELECT COALESCE(SUM(market_value), 0) FROM quarterly_holding_snapshots
                     WHERE quarterly_snapshot_id = ?1 AND category_name = ?2",
                    rusqlite::params![snap.id, cat_name],
                    |row| row.get(0),
                )
                .unwrap_or(0.0)
            };
            values.push(sum);
        }
        category_values.insert(cat_name.clone(), values);
    }

    // Holding counts per snapshot (unique symbols only)
    let holding_counts: Vec<usize> = snap_rows
        .iter()
        .map(|snap| {
            conn.query_row(
                "SELECT COUNT(DISTINCT symbol) FROM quarterly_holding_snapshots WHERE quarterly_snapshot_id = ?1",
                rusqlite::params![snap.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
        })
        .collect();

    Ok(QuarterlyTrends {
        quarters,
        total_values,
        total_costs,
        total_pnls,
        market_values,
        category_values,
        holding_counts,
    })
}
