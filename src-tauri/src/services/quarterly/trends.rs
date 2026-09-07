use super::*;

/// Get multi-quarter trend data.
pub fn get_quarterly_trends(db: &Database) -> Result<QuarterlyTrends, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Get all snapshots ordered by quarter
    let mut stmt = conn
        .prepare(
            "SELECT id, quarter, total_value, total_cost, total_pnl,
                    exchange_rates
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
        exchange_rates: String,
    }

    let snap_rows: Vec<SnapRow> = stmt
        .query_map([], |row| {
            Ok(SnapRow {
                id: row.get(0)?,
                quarter: row.get(1)?,
                total_value: row.get(2)?,
                total_cost: row.get(3)?,
                total_pnl: row.get(4)?,
                exchange_rates: row.get(5)?,
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

    let mut market_values: HashMap<String, Vec<f64>> = ["US", "CN", "HK"]
        .into_iter()
        .map(|market| (market.to_string(), vec![0.0; snap_rows.len()]))
        .collect();
    let snapshot_rates = snap_rows
        .iter()
        .map(|snapshot| {
            currency::SnapshotRates::from_json(&snapshot.exchange_rates)
                .map_err(|error| format!("Quarter {}: {error}", snapshot.quarter))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Load all holdings in one query, preserving their currencies until each
    // amount has been converted with its own snapshot's saved exchange rates.
    // An absent holding is zero; SQL/decode/conversion failures remain errors.
    let snapshot_indexes: HashMap<&str, usize> = snap_rows
        .iter()
        .enumerate()
        .map(|(index, snapshot)| (snapshot.id.as_str(), index))
        .collect();
    let mut category_values: HashMap<String, Vec<f64>> = HashMap::new();
    let mut distinct_holdings = vec![HashSet::new(); snap_rows.len()];
    let mut holdings_stmt = conn
        .prepare(
            "SELECT quarterly_snapshot_id, symbol, market, category_name, market_value, currency
             FROM quarterly_holding_snapshots
             ORDER BY quarterly_snapshot_id, category_name",
        )
        .map_err(|error| error.to_string())?;
    let holdings = holdings_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (snapshot_id, symbol, market, category_name, value, explicit_currency) in holdings {
        let Some(index) = snapshot_indexes.get(snapshot_id.as_str()).copied() else {
            return Err(format!(
                "quarterly holding references unknown snapshot {snapshot_id}"
            ));
        };
        let holding_currency =
            currency::currency_for_holding(&symbol, &market, &explicit_currency)?;
        let usd_value = snapshot_rates[index]
            .convert(value, &holding_currency, "USD")
            .map_err(|error| format!("Quarter {}: {error}", snap_rows[index].quarter))?;
        if !symbol.starts_with("$CASH-") {
            distinct_holdings[index].insert((market.clone(), symbol));
        }
        market_values
            .entry(market)
            .or_insert_with(|| vec![0.0; snap_rows.len()])[index] += usd_value;
        category_values
            .entry(category_name)
            .or_insert_with(|| vec![0.0; snap_rows.len()])[index] += usd_value;
    }
    let holding_counts = distinct_holdings.iter().map(HashSet::len).collect();

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

    // USD 100 + CNY 700 is USD 200 at 7, and USD 150 at 14.
    // This catches adding native-currency amounts or reusing the latest rates.
    fn mixed_currency_quarters() -> Database {
        let db = Database::new(":memory:").unwrap();
        for (id, quarter, rate, total, cost) in [
            ("q1", "2025-Q1", 7.0, 200.0, 160.0),
            ("q2", "2025-Q2", 14.0, 150.0, 120.0),
        ] {
            insert_snapshot(&db, id, quarter, total);
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE quarterly_snapshots SET total_cost = ?2, total_pnl = ?3,
                 us_value = 100, cn_value = 700, us_cost = 80, cn_cost = 560,
                 exchange_rates = ?4 WHERE id = ?1",
                rusqlite::params![
                    id,
                    cost,
                    total - cost,
                    serde_json::json!({"usd_cny": rate, "usd_hkd": 7.8}).to_string()
                ],
            )
            .unwrap();
            for (suffix, symbol, market, value, holding_cost) in [
                ("us", "AAPL", "US", 100.0, 80.0),
                ("cn", "600000", "CN", 700.0, 560.0),
            ] {
                conn.execute(
                    "INSERT INTO quarterly_holding_snapshots
                     (id, quarterly_snapshot_id, account_id, symbol, name, market,
                      category_name, category_color, market_value, cost_value, shares)
                     VALUES (?1, ?2, 'acct', ?3, ?3, ?4, 'Shared category', '#fff', ?5, ?6, 1)",
                    rusqlite::params![
                        format!("{id}-{suffix}"),
                        id,
                        symbol,
                        market,
                        value,
                        holding_cost
                    ],
                )
                .unwrap();
            }
        }
        db
    }

    #[test]
    fn reports_use_each_snapshots_rates_for_market_and_category_totals() {
        let db = mixed_currency_quarters();
        let comparison = compare_quarters(&db, "2025-Q1", "2025-Q2").unwrap();
        let cn = comparison
            .by_market
            .iter()
            .find(|row| row.market == "CN")
            .unwrap();
        assert_eq!((cn.q1_value, cn.q2_value), (100.0, 50.0));
        assert_eq!((cn.q1_cost, cn.q2_cost), (80.0, 40.0));
        assert_eq!(comparison.by_category[0].q1_value, 200.0);
        assert_eq!(comparison.by_category[0].q2_value, 150.0);
        assert_eq!(comparison.by_category[0].q1_cost, 160.0);
        assert_eq!(comparison.by_category[0].q2_cost, 120.0);
        assert_eq!(
            comparison
                .by_market
                .iter()
                .map(|row| row.q1_value)
                .sum::<f64>(),
            comparison.overview.q1_total_value
        );
        assert_eq!(
            comparison
                .by_market
                .iter()
                .map(|row| row.q2_value)
                .sum::<f64>(),
            comparison.overview.q2_total_value
        );

        let trends = get_quarterly_trends(&db).unwrap();
        assert_eq!(trends.market_values["CN"], vec![100.0, 50.0]);
        assert_eq!(trends.market_values["US"], vec![100.0, 100.0]);
        assert_eq!(
            trends.category_values["Shared category"],
            vec![200.0, 150.0]
        );
        assert_eq!(trends.total_values, vec![200.0, 150.0]);
        for index in 0..2 {
            assert_eq!(
                trends
                    .market_values
                    .values()
                    .map(|values| values[index])
                    .sum::<f64>(),
                trends.total_values[index]
            );
            assert_eq!(
                trends
                    .category_values
                    .values()
                    .map(|values| values[index])
                    .sum::<f64>(),
                trends.total_values[index]
            );
        }
        let detail = get_quarterly_snapshot_detail(&db, "q1").unwrap();
        assert_eq!(detail.snapshot.cn_value, 700.0);
        assert_eq!(
            detail
                .holdings
                .iter()
                .find(|row| row.market == "CN")
                .unwrap()
                .currency,
            "CNY"
        );
        assert_eq!(
            detail
                .holdings
                .iter()
                .find(|row| row.market == "CN")
                .unwrap()
                .market_value,
            700.0
        );
    }

    #[test]
    fn zero_foreign_cash_reports_need_no_exchange_pair_but_reject_unknown_currency() {
        let db = Database::new(":memory:").unwrap();
        for (id, quarter) in [("q1", "2025-Q1"), ("q2", "2025-Q2")] {
            insert_snapshot(&db, id, quarter, 0.0);
            insert_holding(&db, &format!("{id}-cash"), id, "$CASH-CNY", "Cash", 0.0);
        }
        let comparison = compare_quarters(&db, "2025-Q1", "2025-Q2").unwrap();
        assert_eq!(
            (
                comparison.by_category[0].q1_value,
                comparison.by_category[0].q2_value
            ),
            (0.0, 0.0)
        );
        assert_eq!(
            (
                comparison.overview.q1_holding_count,
                comparison.overview.q2_holding_count
            ),
            (0, 0)
        );
        let trends = get_quarterly_trends(&db).unwrap();
        assert_eq!(trends.category_values["Cash"], vec![0.0, 0.0]);
        assert_eq!(trends.market_values["US"], vec![0.0, 0.0]);
        assert_eq!(trends.holding_counts, vec![0, 0]);
        let detail = get_quarterly_snapshot_detail(&db, "q2").unwrap();
        assert_eq!(detail.holdings[0].currency, "CNY");
        assert_eq!(detail.holdings[0].market_value, 0.0);

        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE quarterly_holding_snapshots SET currency = 'UNKNOWN' WHERE id = 'q1-cash'",
                [],
            )
            .unwrap();
        assert!(compare_quarters(&db, "2025-Q1", "2025-Q2")
            .unwrap_err()
            .contains("Unsupported"));
        assert!(get_quarterly_trends(&db)
            .unwrap_err()
            .contains("Unsupported"));
    }

    #[test]
    fn reports_reject_missing_or_invalid_required_snapshot_rates() {
        for rates in [
            "{}",
            "{\"usd_cny\":0}",
            "{\"usd_cny\":-7}",
            "{\"usd_cny\":\"bad\"}",
            "not-json",
        ] {
            let db = mixed_currency_quarters();
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE quarterly_snapshots SET exchange_rates = ?1 WHERE id = 'q1'",
                    [rates],
                )
                .unwrap();
            let comparison_error = compare_quarters(&db, "2025-Q1", "2025-Q2").unwrap_err();
            assert!(
                comparison_error.contains("exchange rate"),
                "{comparison_error}"
            );
            let trend_error = get_quarterly_trends(&db).unwrap_err();
            assert!(trend_error.contains("exchange rate"), "{trend_error}");
        }
    }

    #[test]
    fn reports_include_negative_cash_using_its_currency_and_exclude_cash_from_stock_counts() {
        let db = mixed_currency_quarters();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_holding_snapshots
             (id, quarterly_snapshot_id, account_id, symbol, name, market,
              category_name, category_color, market_value, cost_value)
             VALUES ('cash', 'q1', 'acct', '$CASH-CNY', 'Cash', 'US', 'Cash', '#fff', -70, -70)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE quarterly_snapshots SET total_value = 190, total_cost = 150, us_value = 90, us_cost = 70 WHERE id = 'q1'", []).unwrap();
        drop(conn);
        let comparison = compare_quarters(&db, "2025-Q1", "2025-Q2").unwrap();
        let cash = comparison
            .by_category
            .iter()
            .find(|row| row.category_name == "Cash")
            .unwrap();
        assert_eq!(
            (cash.q1_value, cash.q1_cost, cash.q1_pnl),
            (-10.0, -10.0, 0.0)
        );
        assert_eq!(comparison.overview.q1_holding_count, 2);
        assert_eq!(
            comparison
                .by_market
                .iter()
                .find(|row| row.market == "US")
                .unwrap()
                .q1_value,
            90.0
        );
        let trends = get_quarterly_trends(&db).unwrap();
        assert_eq!(trends.category_values["Cash"], vec![-10.0, 0.0]);
        assert_eq!(trends.holding_counts, vec![2, 2]);
        assert_eq!(
            get_quarterly_snapshot_detail(&db, "q1")
                .unwrap()
                .snapshot
                .holding_count,
            2
        );
        assert_eq!(get_quarterly_snapshots(&db).unwrap()[1].holding_count, 2);
    }

    #[test]
    fn reports_use_explicit_currency_even_when_it_differs_from_market() {
        let db = mixed_currency_quarters();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE quarterly_holding_snapshots SET currency = 'USD' WHERE id = 'q1-cn'",
            [],
        )
        .unwrap();
        conn.execute("UPDATE quarterly_snapshots SET total_value = 800, total_cost = 640, total_pnl = 160, cn_value = 4900, cn_cost = 3920 WHERE id = 'q1'", []).unwrap();
        drop(conn);
        let comparison = compare_quarters(&db, "2025-Q1", "2025-Q2").unwrap();
        assert_eq!(
            comparison
                .by_market
                .iter()
                .find(|row| row.market == "CN")
                .unwrap()
                .q1_value,
            700.0
        );
        assert_eq!(comparison.by_category[0].q1_value, 800.0);
        let trends = get_quarterly_trends(&db).unwrap();
        assert_eq!(trends.market_values["CN"], vec![700.0, 50.0]);
        assert_eq!(
            trends.category_values["Shared category"],
            vec![800.0, 150.0]
        );
        let detail = get_quarterly_snapshot_detail(&db, "q1").unwrap();
        let holding = detail
            .holdings
            .iter()
            .find(|row| row.market == "CN")
            .unwrap();
        assert_eq!(holding.currency, "USD");
        assert_eq!(holding.market_value, 700.0);
        assert_eq!(detail.snapshot.cn_value, 4900.0);
        let changes = &comparison.holding_changes.unchanged;
        let cn_change = changes.iter().find(|row| row.market == "CN").unwrap();
        assert_eq!(
            (cn_change.q1_value, cn_change.q2_value),
            (Some(4900.0), Some(700.0))
        );
        let q2_detail = get_quarterly_snapshot_detail(&db, "q2").unwrap();
        let changes = q2_detail.holding_changes.unwrap();
        let cn_change = changes
            .unchanged
            .iter()
            .find(|row| row.market == "CN")
            .unwrap();
        assert_eq!(
            (cn_change.q1_value, cn_change.q2_value),
            (Some(4900.0), Some(700.0))
        );
    }

    #[test]
    fn stock_transfers_appear_in_quarterly_holding_changes() {
        let db = mixed_currency_quarters();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "DELETE FROM quarterly_holding_snapshots WHERE id IN ('q1-cn', 'q2-us')",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO accounts (id,name,market,created_at,updated_at) VALUES ('acct','Account','US','2025-01-01','2025-01-01')", []).unwrap();
            for (kind, symbol, market, currency) in [
                ("STOCK_IN", "600000", "CN", "CNY"),
                ("STOCK_OUT", "AAPL", "US", "USD"),
            ] {
                conn.execute("INSERT INTO transactions (id,account_id,symbol,name,market,transaction_type,shares,price,total_amount,currency,traded_at,created_at) VALUES (?1,'acct',?2,?2,?3,?1,1,0,0,?4,'2025-04-01','2025-04-01')", rusqlite::params![kind,symbol,market,currency]).unwrap();
            }
        }
        let changes = get_quarterly_snapshot_detail(&db, "q2")
            .unwrap()
            .holding_changes
            .unwrap();
        let added = changes
            .new_holdings
            .iter()
            .find(|h| h.symbol == "600000")
            .unwrap();
        assert_eq!(added.q2_shares, Some(1.0));
        let removed = changes
            .closed_holdings
            .iter()
            .find(|h| h.symbol == "AAPL")
            .unwrap();
        assert_eq!(removed.shares_change, -1.0);
    }

    #[test]
    fn detail_holding_change_fallback_converts_each_transaction_to_market_currency() {
        let db = mixed_currency_quarters();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE quarterly_holding_snapshots SET currency = 'USD' WHERE id = 'q1-cn'",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM quarterly_holding_snapshots WHERE id = 'q2-cn'",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES ('acct', 'Account', 'CN', '2025-01-01', '2025-01-01')", []).unwrap();
        // Same symbol, different currencies: USD 100 = CNY 1,400 in Q2;
        // another CNY 70 trade must stay CNY 70 rather than inherit USD.
        for (id, currency, amount) in [("usd", "USD", 100.0), ("cny", "CNY", 70.0)] {
            conn.execute(
                "INSERT INTO transactions
                 (id, account_id, symbol, name, market, transaction_type, shares,
                  price, total_amount, currency, traded_at, created_at)
                 VALUES (?1, 'acct', '600000', 'Stock', 'CN', 'BUY', 1, ?2, ?2, ?3, '2025-04-01', '2025-04-01')",
                rusqlite::params![id, amount, currency],
            ).unwrap();
        }
        drop(conn);
        let detail = get_quarterly_snapshot_detail(&db, "q2").unwrap();
        let changes = detail.holding_changes.unwrap();
        let cn_change = changes
            .increased
            .iter()
            .find(|row| row.market == "CN")
            .unwrap();
        assert_eq!(cn_change.q1_value, Some(4900.0));
        assert_eq!(cn_change.q2_value, Some(6370.0));
        assert_eq!(cn_change.value_change, 1470.0);
    }

    #[test]
    fn stock_counts_keep_markets_distinct_and_cash_only_quarters_at_zero() {
        let db = mixed_currency_quarters();
        let conn = db.conn.lock().unwrap();
        conn.execute("UPDATE quarterly_holding_snapshots SET symbol = 'SAME' WHERE quarterly_snapshot_id = 'q1'", []).unwrap();
        conn.execute("UPDATE quarterly_holding_snapshots SET symbol = CASE market WHEN 'CN' THEN '$CASH-CNY' ELSE '$CASH-USD' END WHERE quarterly_snapshot_id = 'q2'", []).unwrap();
        drop(conn);
        let comparison = compare_quarters(&db, "2025-Q1", "2025-Q2").unwrap();
        assert_eq!(
            (
                comparison.overview.q1_holding_count,
                comparison.overview.q2_holding_count
            ),
            (2, 0)
        );
        assert_eq!(
            get_quarterly_trends(&db).unwrap().holding_counts,
            vec![2, 0]
        );
        assert_eq!(
            get_quarterly_snapshot_detail(&db, "q2")
                .unwrap()
                .snapshot
                .holding_count,
            0
        );
        assert_eq!(get_quarterly_snapshots(&db).unwrap()[0].holding_count, 0);
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
