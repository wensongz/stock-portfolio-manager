use super::*;
use serde_json::json;

fn database() -> Database {
    let db = Database::new(":memory:").unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts VALUES ('a','Test','US',NULL,'2026','2026')",
            [],
        )
        .unwrap();
    db
}
fn request(id: &str, rows: Vec<serde_json::Value>) -> ImportBatchRequest {
    serde_json::from_value(json!({"request_id":id,"account_id":"a","source":"broker","file_name":"trades.csv","source_content":id,"parser_version":"1","kind":"transactions","rows":rows})).unwrap()
}
fn buy(key: &str, shares: f64) -> serde_json::Value {
    json!({"key":key,"raw":"original","data":{"symbol":"AAPL","name":"Apple","market":"US","currency":"USD","transaction_type":"BUY","shares":shares,"price":10.0,"total_amount":shares*10.0,"commission":1.0,"traded_at":"2026-01-01","notes":null}})
}
fn apply_all(db: &Database, batch: &ImportBatch) -> ImportBatch {
    let keys: Vec<String> = batch
        .rows
        .iter()
        .filter(|r| r.status == "ready" || r.status == "failed")
        .map(|r| r.key.clone())
        .collect();
    apply_import_batch(db, &batch.id, &keys, &[]).unwrap()
}
fn shares(db: &Database, symbol: &str) -> f64 {
    db.conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT shares FROM holdings WHERE symbol=?1",
            [symbol],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn share_class_import_normalizes_only_us_stock_aliases() {
    for (market, symbol, expected) in [
        ("US", "BRK B", "BRK-B"),
        ("US", "BRK.B", "BRK-B"),
        ("US", "BRK_B", "BRK-B"),
        ("US", " brk  b ", "BRK-B"),
        ("US", "BRK-B", "BRK-B"),
        ("US", "BF B", "BF-B"),
        ("US", "BRK A", "BRK-A"),
        ("US", "AAPL", "AAPL"),
        ("US", "$CASH-USD", "$CASH-USD"),
        ("US", "BRK B 16JUN23 330 C", "BRK B 16JUN23 330 C"),
        ("US", "bad symbol", "BAD SYMBOL"),
        ("HK", "700.HK", "700.HK"),
        ("HK", "BRK.B", "BRK.B"),
    ] {
        let mut data = buy("one", 1.0)["data"].clone();
        data["market"] = json!(market);
        data["symbol"] = json!(symbol);
        let normalized = dedup::normalize("transactions", "a", &data).unwrap();
        assert_eq!(normalized["symbol"], expected, "{market}: {symbol}");
    }
}

#[test]
fn share_class_trade_import_merges_position_and_keeps_source_and_undo() {
    let db = database();
    let mut first = buy("1", 600.0);
    first["data"]["symbol"] = json!("BRK-B");
    first["data"]["price"] = json!(489.4);
    first["data"]["total_amount"] = json!(293640.0);
    let initial = preview_import_batch(&db, &request("first", vec![first])).unwrap();
    apply_all(&db, &initial);
    let before = state::capture(&db.conn.lock().unwrap(), "a").unwrap();
    let original_id = before
        .holdings
        .iter()
        .find(|h| h["symbol"] == "BRK-B")
        .unwrap()["id"]
        .clone();

    let mut row = buy("2", 100.0);
    row["raw"] = json!("BRK B,BUY,100,501.33");
    row["data"]["symbol"] = json!("BRK B");
    row["data"]["price"] = json!(501.33);
    row["data"]["total_amount"] = json!(50133.0);
    row["data"]["traded_at"] = json!("2026-10-02");
    let p = preview_import_batch(&db, &request("second", vec![row])).unwrap();
    assert_eq!(p.rows[0].data["symbol"], "BRK-B");
    assert_eq!(p.rows[0].raw, "BRK B,BUY,100,501.33");
    let applied = apply_all(&db, &p);
    assert_eq!(applied.rows[0].status, "imported");
    let after = state::capture(&db.conn.lock().unwrap(), "a").unwrap();
    let stock = after
        .holdings
        .iter()
        .find(|h| h["symbol"] == "BRK-B")
        .unwrap();
    assert_eq!(stock["id"], original_id);
    assert_eq!(stock["shares"], 700.0);
    assert!((stock["avg_cost"].as_f64().unwrap() - 491.10714285714283).abs() < 1e-9);
    assert_eq!(after.holdings.len(), 2);
    assert_eq!(shares(&db, "$CASH-USD"), -343775.0);
    assert!(after
        .transactions
        .iter()
        .all(|t| t["symbol"] == "BRK-B" && t["holding_id"] == original_id));
    undo_import_batch(&db, &applied.id).unwrap();
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "a").unwrap(),
        before
    );
}

#[test]
fn share_class_reimport_matches_legacy_batch_fingerprints_without_rewriting_audit() {
    let db = database();
    let mut row = buy("1", 10.0);
    row["data"]["symbol"] = json!("BRK-B");
    row["external_id"] = json!("exec-1");
    let req = request("legacy", vec![row.clone()]);
    let batch = preview_import_batch(&db, &req).unwrap();
    apply_all(&db, &batch);
    // Reproduce an old parser's persisted audit, including its pre-fix fingerprint.
    let old_fp = json!([
        "BRK B",
        "US",
        "USD",
        "BUY",
        "2026-01-01T00:00:00+00:00",
        10.0,
        10.0,
        100.0,
        1.0
    ])
    .to_string();
    db.conn.lock().unwrap().execute(
        "UPDATE import_batch_rows SET data=json_set(data,'$.symbol','BRK B'),fingerprint=?2 WHERE batch_id=?1",
        params![batch.id, old_fp],
    ).unwrap();
    let old_data = get_import_batch(&db, &batch.id).unwrap().rows[0]
        .data
        .clone();

    let mut same_file = req.clone();
    same_file.request_id = "same-file".into();
    same_file.rows[0].external_id = None;
    assert_eq!(
        preview_import_batch(&db, &same_file).unwrap().rows[0].status,
        "duplicate"
    );
    let same_execution = request("same-execution", vec![row.clone()]);
    assert_eq!(
        preview_import_batch(&db, &same_execution).unwrap().rows[0].status,
        "duplicate"
    );
    row["external_id"] = json!("exec-2");
    assert_eq!(
        preview_import_batch(&db, &request("distinct", vec![row.clone()]))
            .unwrap()
            .rows[0]
            .status,
        "ready"
    );
    row["external_id"] = json!("exec-1");
    row["data"]["price"] = json!(11.0);
    assert_eq!(
        preview_import_batch(&db, &request("conflict", vec![row]))
            .unwrap()
            .rows[0]
            .status,
        "failed"
    );
    assert_eq!(
        get_import_batch(&db, &batch.id).unwrap().rows[0].data,
        old_data
    );
    let persisted_fp: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT fingerprint FROM import_batch_rows WHERE batch_id=?1",
            [&batch.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(persisted_fp, old_fp);
}

#[test]
fn share_class_import_detects_legacy_manual_transactions_as_suspected_duplicates() {
    let db = database();
    let mut row = buy("1", 10.0);
    row["data"]["symbol"] = json!("BRK B");
    row["data"]["account_id"] = json!("a");
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        create_transaction_in(&tx, &serde_json::from_value(row["data"].clone()).unwrap()).unwrap();
        tx.commit().unwrap();
    }
    row["data"]["symbol"] = json!("BRK-B");
    let preview = preview_import_batch(&db, &request("reimport", vec![row])).unwrap();
    assert_eq!(preview.rows[0].status, "suspected");
}

#[test]
fn share_class_holding_import_rejects_a_second_snapshot_of_existing_position() {
    let db = database();
    let mut req = request(
        "holding",
        vec![json!({"key":"1","raw":"BRK-B,600,489.4","data":{
            "symbol":"BRK-B","name":"Berkshire","market":"US","currency":"USD",
            "shares":600.0,"avg_cost":489.4,"category_id":null
        }})],
    );
    req.kind = "holdings".into();
    let first = preview_import_batch(&db, &req).unwrap();
    apply_all(&db, &first);
    req.request_id = "duplicate-holding".into();
    req.source_content = "different-file".into();
    req.rows[0].data["symbol"] = json!("BRK B");
    req.rows[0].data["shares"] = json!(700.0);
    let p = preview_import_batch(&db, &req).unwrap();
    assert_eq!(p.rows[0].data["symbol"], "BRK-B");
    let applied = apply_all(&db, &p);
    assert_eq!(applied.rows[0].status, "failed");
    assert_eq!(shares(&db, "BRK-B"), 600.0);
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "a")
            .unwrap()
            .holdings
            .len(),
        1
    );
}

#[test]
fn duplicate_request_and_file_cannot_double_book() {
    let db = database();
    let req = request("one", vec![buy("1", 10.0)]);
    let p = preview_import_batch(&db, &req).unwrap();
    let applied = apply_all(&db, &p);
    assert_eq!(applied.rows[0].status, "imported");
    assert_eq!(shares(&db, "$CASH-USD"), -101.0);
    assert_eq!(preview_import_batch(&db, &req).unwrap().id, p.id);
    apply_import_batch(&db, &p.id, &["1".into()], &[]).unwrap();
    let mut again = req.clone();
    again.request_id = "two".into();
    let repeat = preview_import_batch(&db, &again).unwrap();
    assert_eq!(repeat.rows[0].status, "duplicate");
    assert_eq!(shares(&db, "AAPL"), 10.0);
}
#[test]
fn ambiguous_matches_require_consent_but_distinct_execution_ids_do_not() {
    let db = database();
    let p = preview_import_batch(&db, &request("one", vec![buy("1", 10.0)])).unwrap();
    apply_all(&db, &p);
    let q = preview_import_batch(&db, &request("two", vec![buy("2", 10.0)])).unwrap();
    assert_eq!(q.rows[0].status, "suspected");
    let denied = apply_import_batch(&db, &q.id, &["2".into()], &[]).unwrap();
    assert_ne!(denied.rows[0].status, "imported");
    let accepted = apply_import_batch(&db, &q.id, &["2".into()], &["2".into()]).unwrap();
    assert_eq!(accepted.rows[0].status, "imported");
    assert_eq!(shares(&db, "AAPL"), 20.0);
    let db = database();
    let mut first = buy("1", 10.0);
    first["external_id"] = json!("exec-1");
    let p = preview_import_batch(&db, &request("one", vec![first])).unwrap();
    apply_all(&db, &p);
    let mut second = buy("2", 10.0);
    second["external_id"] = json!("exec-2");
    assert_eq!(
        preview_import_batch(&db, &request("two", vec![second]))
            .unwrap()
            .rows[0]
            .status,
        "ready"
    );
}
#[test]
fn execution_id_conflict_is_not_silently_skipped() {
    let db = database();
    let mut row = buy("1", 10.0);
    row["external_id"] = json!("exec-1");
    let p = preview_import_batch(&db, &request("one", vec![row.clone()])).unwrap();
    apply_all(&db, &p);
    row["data"]["shares"] = json!(20.0);
    let p = preview_import_batch(&db, &request("two", vec![row])).unwrap();
    assert_eq!(p.rows[0].status, "failed");
    assert!(p.rows[0].error.as_ref().unwrap().contains("编号"));
}
#[test]
fn retry_only_failed_rows_and_undo_restore_exact_before_state() {
    let db = database();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_msft BEFORE INSERT ON transactions WHEN NEW.symbol='MSFT' BEGIN SELECT RAISE(ABORT,'temporary failure'); END;").unwrap();
    let mut msft = buy("2", 3.0);
    msft["data"]["symbol"] = json!("MSFT");
    let p = preview_import_batch(&db, &request("one", vec![buy("1", 10.0), msft])).unwrap();
    let a = apply_all(&db, &p);
    assert_eq!(a.rows[0].status, "imported");
    assert_eq!(a.rows[1].status, "failed");
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_msft;")
        .unwrap();
    let a = apply_all(&db, &a);
    assert_eq!(shares(&db, "AAPL"), 10.0);
    assert_eq!(shares(&db, "MSFT"), 3.0);
    assert_eq!(shares(&db, "$CASH-USD"), -132.0);
    assert!(a
        .reconciliation
        .iter()
        .all(|r| r.expected_shares.is_none() && r.difference.is_none()));
    let a = reconcile_import_batch(
        &db,
        &a.id,
        &[ExpectedBalance {
            symbol: "AAPL".into(),
            expected_shares: 12.0,
        }],
    )
    .unwrap();
    assert_eq!(
        a.reconciliation
            .iter()
            .find(|r| r.symbol == "AAPL")
            .unwrap()
            .difference,
        Some(-2.0)
    );
    assert!(a.can_undo);
    let undone = undo_import_batch(&db, &a.id).unwrap();
    assert_eq!(undone.status, "undone");
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM holdings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert!(apply_import_batch(&db, &a.id, &["2".into()], &[]).is_err());
}
#[test]
fn later_changes_block_undo_without_partial_writes() {
    let db = database();
    let p = preview_import_batch(&db, &request("one", vec![buy("1", 10.0)])).unwrap();
    let a = apply_all(&db, &p);
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE holdings SET shares=11 WHERE symbol='AAPL'", [])
        .unwrap();
    assert!(!get_import_batch(&db, &a.id).unwrap().can_undo);
    assert!(undo_import_batch(&db, &a.id).is_err());
    assert_eq!(shares(&db, "AAPL"), 11.0);
    assert_eq!(shares(&db, "$CASH-USD"), -101.0);
}
#[test]
fn overlapping_previews_recheck_duplicates_at_commit() {
    let db = database();
    let mut one = request("one", vec![buy("1", 10.0)]);
    one.rows[0].external_id = Some("exec-1".into());
    let a = preview_import_batch(&db, &one).unwrap();
    one.request_id = "two".into();
    one.source_content = "two".into();
    let b = preview_import_batch(&db, &one).unwrap();
    apply_all(&db, &a);
    let b = apply_all(&db, &b);
    assert_eq!(b.rows[0].status, "duplicate");
    assert_eq!(shares(&db, "AAPL"), 10.0);
}

#[test]
fn edited_reimport_of_same_source_row_reports_conflict() {
    let db = database();
    let req = request("one", vec![buy("1", 10.0)]);
    let p = preview_import_batch(&db, &req).unwrap();
    apply_all(&db, &p);
    let mut changed = req.clone();
    changed.request_id = "two".into();
    changed.rows[0].data["price"] = json!(11.0);
    let p = preview_import_batch(&db, &changed).unwrap();
    assert_eq!(p.rows[0].status, "failed");
    assert_eq!(shares(&db, "AAPL"), 10.0);
}
#[test]
fn rollback_failure_preserves_the_whole_account_and_audit_status() {
    let db = database();
    let p = preview_import_batch(&db, &request("one", vec![buy("1", 10.0)])).unwrap();
    let a = apply_all(&db, &p);
    let before = state::capture(&db.conn.lock().unwrap(), "a").unwrap();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_undo BEFORE UPDATE OF status ON import_batches WHEN NEW.status='undone' BEGIN SELECT RAISE(ABORT,'cannot update audit'); END;").unwrap();
    assert!(undo_import_batch(&db, &a.id).is_err());
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "a").unwrap(),
        before
    );
    assert_eq!(get_import_batch(&db, &a.id).unwrap().status, "applied");
}
#[test]
fn holding_batch_undo_restores_preexisting_account_and_other_account() {
    let db = database();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts VALUES ('b','Other','US',NULL,'2026','2026')",
            [],
        )
        .unwrap();
    let mut other = request("other", vec![buy("x", 2.0)]);
    other.account_id = "b".into();
    let b = preview_import_batch(&db, &other).unwrap();
    apply_all(&db, &b);
    let initial = state::capture(&db.conn.lock().unwrap(), "a").unwrap();
    let other_initial = state::capture(&db.conn.lock().unwrap(), "b").unwrap();
    let mut req = request(
        "holding",
        vec![
            json!({"key":"1","raw":"cash","data":{"symbol":"$CASH-USD","name":"Cash","market":"US","currency":"USD","shares":200.0,"avg_cost":1.0,"category_id":null}}),
            json!({"key":"2","raw":"stock","data":{"symbol":"AAPL","name":"Apple","market":"US","currency":"USD","shares":5.0,"avg_cost":20.0,"category_id":null}}),
        ],
    );
    req.kind = "holdings".into();
    let p = preview_import_batch(&db, &req).unwrap();
    let a = apply_all(&db, &p);
    assert!(a.rows.iter().all(|r| r.status == "imported"));
    undo_import_batch(&db, &a.id).unwrap();
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "a").unwrap(),
        initial
    );
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "b").unwrap(),
        other_initial
    );
}
#[test]
fn batches_can_be_undone_in_reverse_order() {
    let db = database();
    let p = preview_import_batch(&db, &request("one", vec![buy("1", 10.0)])).unwrap();
    let a = apply_all(&db, &p);
    let p = preview_import_batch(&db, &request("two", vec![buy("2", 2.0)])).unwrap();
    let b = apply_all(&db, &p);
    assert!(undo_import_batch(&db, &a.id).is_err());
    undo_import_batch(&db, &b.id).unwrap();
    assert!(undo_import_batch(&db, &a.id).is_ok());
}
#[test]
fn csv_batches_survive_restart_and_keep_all_rows_and_raw_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.db");
    let db = Database::new(path.to_str().unwrap()).unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts VALUES ('a','Test','US',NULL,'2026','2026')",
            [],
        )
        .unwrap();
    let mut csv = "symbol,shares,avg_cost\n".to_string();
    for i in 0..25 {
        csv.push_str(&format!("S{i},1,10\n"));
    }
    let batch = crate::services::import_export_service::preview_csv_import_batch(
        &db, &csv, "holdings", "a", "test.csv", "req",
    )
    .unwrap();
    let a = apply_all(&db, &batch);
    assert_eq!(a.rows.iter().filter(|r| r.status == "imported").count(), 25);
    drop(db);
    let db = Database::new(path.to_str().unwrap()).unwrap();
    let restored = get_import_batch(&db, &a.id).unwrap();
    assert!(restored.can_undo);
    assert_eq!(restored.rows.len(), 25);
    assert_eq!(list_import_batches(&db, Some("a")).unwrap().len(), 1);
    assert_eq!(
        request_for(&db.conn.lock().unwrap(), &a.id)
            .unwrap()
            .source_content,
        csv
    );
    undo_import_batch(&db, &a.id).unwrap();
}
#[test]
fn migration_from_v5_preserves_original_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let db = Database::new(path.to_str().unwrap()).unwrap();
    db.conn.lock().unwrap().execute_batch("DROP TABLE import_batch_rows; DROP TABLE import_batches; INSERT INTO accounts VALUES ('a','Original','US',NULL,'2026','2026'); PRAGMA user_version=5;").unwrap();
    drop(db);
    let db = Database::new(path.to_str().unwrap()).unwrap();
    let conn = db.conn.lock().unwrap();
    assert_eq!(
        conn.query_row("SELECT name FROM accounts WHERE id='a'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "Original"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn arithmetic_overflow_rolls_back_only_the_invalid_row() {
    let db = database();
    let mut req = request(
        "overflow",
        vec![
            json!({"key":"bad","raw":"bad","data":{"symbol":"HUGE","name":"Huge","market":"US","currency":"USD","shares":1e308,"avg_cost":1e308}}),
            json!({"key":"good","raw":"good","data":{"symbol":"AAPL","name":"Apple","market":"US","currency":"USD","shares":2.0,"avg_cost":10.0}}),
        ],
    );
    req.kind = "holdings".into();
    let p = preview_import_batch(&db, &req).unwrap();
    let a = apply_all(&db, &p);
    assert_eq!(a.rows[0].status, "failed");
    assert_eq!(a.rows[1].status, "imported");
    let n: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM holdings WHERE symbol='HUGE'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    undo_import_batch(&db, &a.id).unwrap();
}

#[test]
fn fractional_account_state_remains_undoable_after_reads_and_repeated_submit() {
    let db = database();
    create_holding_in(
        &db.conn.lock().unwrap(),
        &CreateHoldingInput {
            account_id: "a".into(),
            symbol: "BASE".into(),
            name: "Existing position".into(),
            market: "US".into(),
            currency: "USD".into(),
            category_id: None,
            shares: 2.0,
            avg_cost: 440.84788000000003,
        },
    )
    .unwrap();
    let initial = state::capture(&db.conn.lock().unwrap(), "a").unwrap();
    let mut row = buy("1", 5.0);
    row["data"]["price"] = json!(24.9);
    row["data"]["total_amount"] = json!(124.5);
    row["data"]["commission"] = json!(2.8284000000000002);
    let req = request("fractional", vec![row]);
    let preview = preview_import_batch(&db, &req).unwrap();
    let applied = apply_all(&db, &preview);
    assert_eq!(applied.rows[0].status, "imported");
    assert_eq!(applied.conflict, None);
    assert!(applied.can_undo);
    let after = state::capture(&db.conn.lock().unwrap(), "a").unwrap();

    for _ in 0..2 {
        let read = get_import_batch(&db, &applied.id).unwrap();
        assert_eq!(read.conflict, None);
        assert!(read.can_undo);
        assert!(list_import_batches(&db, Some("a")).unwrap()[0].can_undo);
        assert_eq!(preview_import_batch(&db, &req).unwrap().id, applied.id);
        let repeated = apply_import_batch(&db, &applied.id, &["1".into()], &[]).unwrap();
        assert_eq!(repeated.rows[0].record_id, applied.rows[0].record_id);
        assert_eq!(
            state::capture(&db.conn.lock().unwrap(), "a").unwrap(),
            after
        );
    }

    assert_eq!(
        undo_import_batch(&db, &applied.id).unwrap().status,
        "undone"
    );
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "a").unwrap(),
        initial
    );
}

#[test]
fn fractional_computed_cost_does_not_block_retrying_failed_rows() {
    let db = database();
    db.conn.lock().unwrap().execute_batch("CREATE TRIGGER fail_msft BEFORE INSERT ON transactions WHEN NEW.symbol='MSFT' BEGIN SELECT RAISE(ABORT,'temporary failure'); END;").unwrap();
    let mut first = buy("1", 1.0);
    first["data"]["price"] = json!(440.04);
    first["data"]["total_amount"] = json!(440.04);
    first["data"]["commission"] = json!(0.80788);
    let mut second = buy("2", 3.0);
    second["data"]["symbol"] = json!("MSFT");
    let preview =
        preview_import_batch(&db, &request("retry-fractional", vec![first, second])).unwrap();
    let applied = apply_all(&db, &preview);
    assert_eq!(applied.rows[0].status, "imported");
    assert_eq!(applied.rows[1].status, "failed");
    let cost: f64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT avg_cost FROM holdings WHERE symbol='AAPL'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cost, 440.84788000000003);
    assert_eq!(applied.conflict, None);
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_msft;")
        .unwrap();

    let retried = apply_all(&db, &applied);
    assert!(retried.rows.iter().all(|r| r.status == "imported"));
    assert_eq!(retried.rows[0].record_id, applied.rows[0].record_id);
    assert_eq!(shares(&db, "AAPL"), 1.0);
    assert_eq!(shares(&db, "MSFT"), 3.0);
    assert_eq!(retried.conflict, None);
    assert!(retried.can_undo);
    undo_import_batch(&db, &retried.id).unwrap();
    let restored = state::capture(&db.conn.lock().unwrap(), "a").unwrap();
    assert!(restored.holdings.is_empty());
    assert!(restored.transactions.is_empty());
}

#[test]
fn one_ulp_account_edit_still_blocks_retry_and_undo_without_writes() {
    let db = database();
    let preview = preview_import_batch(&db, &request("edited", vec![buy("1", 10.0)])).unwrap();
    let applied = apply_all(&db, &preview);
    let changed_cost = f64::from_bits(10.1_f64.to_bits() + 1);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE holdings SET avg_cost=?1 WHERE symbol='AAPL'",
            [changed_cost],
        )
        .unwrap();
    let edited = state::capture(&db.conn.lock().unwrap(), "a").unwrap();

    let read = get_import_batch(&db, &applied.id).unwrap();
    assert!(read.conflict.is_some());
    assert!(!read.can_undo);
    assert!(apply_import_batch(&db, &applied.id, &["1".into()], &[]).is_err());
    assert!(undo_import_batch(&db, &applied.id).is_err());
    assert_eq!(
        state::capture(&db.conn.lock().unwrap(), "a").unwrap(),
        edited
    );
}
