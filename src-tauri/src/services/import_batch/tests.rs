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
