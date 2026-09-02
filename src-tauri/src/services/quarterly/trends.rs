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

    // Load category totals and distinct holding counts for every snapshot in
    // one grouped query. Missing aggregate rows are legitimate zeros; query
    // and decoding failures remain errors.
    let snapshot_indexes: HashMap<&str, usize> = snap_rows
        .iter()
        .enumerate()
        .map(|(index, snapshot)| (snapshot.id.as_str(), index))
        .collect();
    let mut category_values: HashMap<String, Vec<f64>> = HashMap::new();
    let mut holding_counts = vec![0usize; snap_rows.len()];
    let mut aggregate_stmt = conn
        .prepare(
            "WITH category_totals AS (
                 SELECT quarterly_snapshot_id, category_name, SUM(market_value) AS category_value
                 FROM quarterly_holding_snapshots
                 GROUP BY quarterly_snapshot_id, category_name
             ), holding_counts AS (
                 SELECT quarterly_snapshot_id, COUNT(DISTINCT symbol) AS holding_count
                 FROM quarterly_holding_snapshots
                 GROUP BY quarterly_snapshot_id
             )
             SELECT c.quarterly_snapshot_id, c.category_name, c.category_value, h.holding_count
             FROM category_totals c
             JOIN holding_counts h ON h.quarterly_snapshot_id = c.quarterly_snapshot_id
             ORDER BY c.quarterly_snapshot_id, c.category_name",
        )
        .map_err(|error| error.to_string())?;
    let aggregates = aggregate_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (snapshot_id, category_name, value, count) in aggregates {
        let Some(index) = snapshot_indexes.get(snapshot_id.as_str()).copied() else {
            return Err(format!(
                "quarterly aggregate references unknown snapshot {snapshot_id}"
            ));
        };
        if count < 0 {
            return Err(format!(
                "negative holding count for quarterly snapshot {snapshot_id}"
            ));
        }
        holding_counts[index] = count as usize;
        category_values
            .entry(category_name)
            .or_insert_with(|| vec![0.0; snap_rows.len()])[index] = value;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_snapshot(db: &Database, id: &str, quarter: &str, total_value: f64) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_snapshots
             (id, quarter, snapshot_date, total_value, exchange_rates, created_at)
             VALUES (?1, ?2, '2025-03-31', ?3, '{}', '2025-04-01')",
            rusqlite::params![id, quarter, total_value],
        )
        .unwrap();
    }

    fn insert_holding(
        db: &Database,
        id: &str,
        snapshot_id: &str,
        symbol: &str,
        category: &str,
        value: f64,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_holding_snapshots
             (id, quarterly_snapshot_id, account_id, symbol, name, market,
              category_name, category_color, market_value)
             VALUES (?1, ?2, 'acct', ?3, ?3, 'US', ?4, '#fff', ?5)",
            rusqlite::params![id, snapshot_id, symbol, category, value],
        )
        .unwrap();
    }

    #[test]
    fn trends_fill_grouped_category_values_and_unique_counts() {
        let db = Database::new(":memory:").unwrap();
        insert_snapshot(&db, "q1", "2025-Q1", 30.0);
        insert_snapshot(&db, "q2", "2025-Q2", 70.0);
        insert_holding(&db, "q1-a", "q1", "AAPL", "成长", 10.0);
        insert_holding(&db, "q1-b", "q1", "MSFT", "分红", 20.0);
        insert_holding(&db, "q2-a", "q2", "AAPL", "成长", 30.0);
        insert_holding(&db, "q2-b", "q2", "AAPL", "分红", 40.0);

        let trends = get_quarterly_trends(&db).unwrap();

        assert_eq!(trends.quarters, vec!["2025-Q1", "2025-Q2"]);
        assert_eq!(trends.category_values["成长"], vec![10.0, 30.0]);
        assert_eq!(trends.category_values["分红"], vec![20.0, 40.0]);
        assert_eq!(trends.holding_counts, vec![2, 1]);
    }

    #[test]
    fn trends_propagate_malformed_category_rows() {
        let db = Database::new(":memory:").unwrap();
        insert_snapshot(&db, "q1", "2025-Q1", 10.0);
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_holding_snapshots
             (id, quarterly_snapshot_id, account_id, symbol, name, market,
              category_name, category_color, market_value)
             VALUES ('row', 'q1', 'acct', 'AAPL', 'Apple', 'US', X'FF', '#fff', 10)",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(get_quarterly_trends(&db).is_err());
    }
}
