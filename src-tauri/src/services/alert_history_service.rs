use crate::db::Database;
use crate::models::alert_history::{AlertHistoryMessage, AlertHistoryPage, AlertHistoryQuery};
use rusqlite::Connection;

pub fn append_alert_history(
    connection: &Connection,
    message: &AlertHistoryMessage,
) -> Result<(), String> {
    validate_kind(&message.kind)?;
    let details_json = serde_json::to_string(&message.details)
        .map_err(|error| format!("failed to serialize alert history details: {error}"))?;
    connection
        .execute(
            "INSERT INTO alert_history
               (id, kind, title, message, scope_name, account_name, triggered_at, details_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                message.id,
                message.kind,
                message.title,
                message.message,
                message.scope_name,
                message.account_name,
                message.triggered_at,
                details_json,
            ],
        )
        .map_err(|error| format!("failed to append alert history: {error}"))?;
    Ok(())
}

pub fn get_alert_history(
    db: &Database,
    query: AlertHistoryQuery,
) -> Result<AlertHistoryPage, String> {
    if let Some(kind) = query.kind.as_deref() {
        validate_kind(kind)?;
    }

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = u64::from(page - 1)
        .checked_mul(u64::from(page_size))
        .ok_or_else(|| "alert history page offset is too large".to_string())?;
    let offset =
        i64::try_from(offset).map_err(|_| "alert history page offset is too large".to_string())?;
    let kind = query.kind.as_deref();
    let search = query.search.as_deref();

    let connection = db.conn.lock().map_err(|error| error.to_string())?;
    let where_clause = "WHERE (?1 IS NULL OR kind = ?1)
           AND (
             ?2 IS NULL
             OR instr(lower(title), lower(?2)) > 0
             OR instr(lower(message), lower(?2)) > 0
             OR instr(lower(scope_name), lower(?2)) > 0
             OR instr(lower(COALESCE(account_name, '')), lower(?2)) > 0
             OR instr(lower(details_json), lower(?2)) > 0
           )";
    let total: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM alert_history {where_clause}"),
            rusqlite::params![kind, search],
            |row| row.get(0),
        )
        .map_err(|error| format!("failed to count alert history: {error}"))?;

    let sql = format!(
        "SELECT id, kind, title, message, scope_name, account_name, triggered_at, details_json
         FROM alert_history {where_clause}
         ORDER BY triggered_at DESC, id DESC
         LIMIT ?3 OFFSET ?4"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("failed to prepare alert history query: {error}"))?;
    let rows = statement
        .query_map(
            rusqlite::params![kind, search, i64::from(page_size), offset],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|error| format!("failed to query alert history: {error}"))?;

    let mut items = Vec::new();
    for row in rows {
        let (id, kind, title, message, scope_name, account_name, triggered_at, details_json) =
            row.map_err(|error| format!("failed to read alert history: {error}"))?;
        let details = serde_json::from_str(&details_json)
            .map_err(|error| format!("failed to parse alert history details for {id}: {error}"))?;
        items.push(AlertHistoryMessage {
            id,
            kind,
            title,
            message,
            scope_name,
            account_name,
            triggered_at,
            details,
        });
    }

    Ok(AlertHistoryPage {
        items,
        total: u64::try_from(total)
            .map_err(|_| "alert history count cannot be negative".to_string())?,
        page,
        page_size,
    })
}

pub fn delete_alert_history(db: &Database, id: &str) -> Result<bool, String> {
    let connection = db.conn.lock().map_err(|error| error.to_string())?;
    let deleted = connection
        .execute(
            "DELETE FROM alert_history WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|error| format!("failed to delete alert history: {error}"))?;
    Ok(deleted > 0)
}

fn validate_kind(kind: &str) -> Result<(), String> {
    if matches!(kind, "PRICE" | "PORTFOLIO") {
        Ok(())
    } else {
        Err("alert history kind must be PRICE or PORTFOLIO".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{append_alert_history, delete_alert_history, get_alert_history};
    use crate::db::Database;
    use crate::models::alert_history::{
        AlertHistoryDetail, AlertHistoryMessage, AlertHistoryQuery,
    };

    fn message(
        id: &str,
        kind: &str,
        title: &str,
        scope_name: &str,
        account_name: Option<&str>,
        triggered_at: &str,
        detail_value: &str,
    ) -> AlertHistoryMessage {
        AlertHistoryMessage {
            id: id.into(),
            kind: kind.into(),
            title: title.into(),
            message: format!("message for {title}"),
            scope_name: scope_name.into(),
            account_name: account_name.map(str::to_owned),
            triggered_at: triggered_at.into(),
            details: vec![AlertHistoryDetail {
                label: "Value".into(),
                value: detail_value.into(),
            }],
        }
    }

    fn append(db: &Database, message: &AlertHistoryMessage) {
        append_alert_history(&db.conn.lock().unwrap(), message).unwrap();
    }

    #[test]
    fn alert_history_orders_stably_and_paginates_with_defaults_and_cap() {
        let db = Database::new(":memory:").unwrap();
        append(
            &db,
            &message("a", "PRICE", "A", "A", None, "2026-09-19T10:00:00Z", "1"),
        );
        append(
            &db,
            &message(
                "c",
                "PORTFOLIO",
                "C",
                "C",
                None,
                "2026-09-19T10:00:00Z",
                "3",
            ),
        );
        append(
            &db,
            &message("b", "PRICE", "B", "B", None, "2026-09-19T09:00:00Z", "2"),
        );

        let first = get_alert_history(
            &db,
            AlertHistoryQuery {
                page_size: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.total, 3);
        assert_eq!(first.page, 1);
        assert_eq!(first.page_size, 2);
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "a"]
        );

        let second = get_alert_history(
            &db,
            AlertHistoryQuery {
                page: Some(2),
                page_size: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            second
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );

        let capped = get_alert_history(
            &db,
            AlertHistoryQuery {
                page: Some(0),
                page_size: Some(u32::MAX),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(capped.page, 1);
        assert_eq!(capped.page_size, 100);

        let huge = get_alert_history(
            &db,
            AlertHistoryQuery {
                page: Some(u32::MAX),
                page_size: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(huge.items.is_empty());
        assert_eq!(huge.total, 3);
    }

    #[test]
    fn alert_history_filters_kind_and_searches_snapshot_text_literally() {
        let db = Database::new(":memory:").unwrap();
        append(
            &db,
            &message(
                "percent",
                "PRICE",
                "Move 10%",
                "Apple",
                Some("Income_Account"),
                "2026-09-19T10:00:00Z",
                "USD 123.45",
            ),
        );
        append(
            &db,
            &message(
                "portfolio",
                "PORTFOLIO",
                "Concentration",
                "US Portfolio",
                Some("Retirement"),
                "2026-09-19T11:00:00Z",
                "Technology",
            ),
        );

        for (search, expected) in [
            ("10%", vec!["percent"]),
            ("Income_", vec!["percent"]),
            ("retireMENT", vec!["portfolio"]),
            ("123.45", vec!["percent"]),
            ("US Portfolio", vec!["portfolio"]),
        ] {
            let page = get_alert_history(
                &db,
                AlertHistoryQuery {
                    search: Some(search.into()),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                page.items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "search {search}"
            );
        }

        let price = get_alert_history(
            &db,
            AlertHistoryQuery {
                kind: Some("PRICE".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            price
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["percent"]
        );

        for invalid in ["price", "OTHER", ""] {
            let error = get_alert_history(
                &db,
                AlertHistoryQuery {
                    kind: Some(invalid.into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(
                error.contains("PRICE") && error.contains("PORTFOLIO"),
                "{error}"
            );
        }
    }

    #[test]
    fn alert_history_is_permanent_and_keeps_names_after_source_rows_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        {
            let db = Database::new(path.to_str().unwrap()).unwrap();
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO accounts (id,name,market,created_at,updated_at)
                 VALUES ('opaque-account-id','Original account','US','old','old');
                 INSERT INTO price_alerts
                   (id,symbol,name,market,alert_type,threshold,is_active,is_triggered,created_at)
                 VALUES ('rule-id','AAPL','Apple','US','PRICE_ABOVE',100,1,1,'old');",
            )
            .unwrap();
            append_alert_history(
                &conn,
                &message(
                    "saved",
                    "PRICE",
                    "Apple price",
                    "Apple (AAPL)",
                    Some("Original account"),
                    "2026-09-19T10:00:00Z",
                    "101.00 USD",
                ),
            )
            .unwrap();
            conn.execute_batch("DELETE FROM price_alerts; DELETE FROM accounts;")
                .unwrap();
        }

        let reopened = Database::new(path.to_str().unwrap()).unwrap();
        let page = get_alert_history(&reopened, Default::default()).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(
            page.items[0].account_name.as_deref(),
            Some("Original account")
        );
        assert_eq!(page.items[0].scope_name, "Apple (AAPL)");
        assert_eq!(page.items[0].details[0].value, "101.00 USD");
    }

    #[test]
    fn deleting_one_history_message_keeps_other_messages_and_source_rule() {
        let db = Database::new(":memory:").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO price_alerts
                   (id, symbol, name, market, alert_type, threshold, is_active, is_triggered, created_at)
                 VALUES ('source-rule', 'AAPL', 'Apple', 'US', 'PRICE_ABOVE', 100, 1, 1, 'old');",
            )
            .unwrap();
        append(
            &db,
            &message(
                "remove-me",
                "PRICE",
                "Remove",
                "Apple",
                None,
                "2026-09-19T10:00:00Z",
                "101",
            ),
        );
        append(
            &db,
            &message(
                "keep-me",
                "PRICE",
                "Keep",
                "Apple",
                None,
                "2026-09-19T11:00:00Z",
                "102",
            ),
        );

        assert!(delete_alert_history(&db, "remove-me").unwrap());
        assert!(!delete_alert_history(&db, "remove-me").unwrap());
        assert!(!delete_alert_history(&db, "missing").unwrap());

        let page = get_alert_history(&db, Default::default()).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, "keep-me");
        let source_rule_count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM price_alerts WHERE id = 'source-rule'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_rule_count, 1);
    }

    #[test]
    fn deleting_sql_special_character_id_is_exact_and_persists_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("delete-history.sqlite");
        let special_id = "target' OR 1=1 --";
        {
            let db = Database::new(path.to_str().unwrap()).unwrap();
            append(
                &db,
                &message(
                    special_id,
                    "PRICE",
                    "Special",
                    "Apple",
                    None,
                    "2026-09-19T10:00:00Z",
                    "101",
                ),
            );
            append(
                &db,
                &message(
                    "target",
                    "PRICE",
                    "Keep",
                    "Apple",
                    None,
                    "2026-09-19T11:00:00Z",
                    "102",
                ),
            );

            assert!(delete_alert_history(&db, special_id).unwrap());
        }

        let reopened = Database::new(path.to_str().unwrap()).unwrap();
        let page = get_alert_history(&reopened, Default::default()).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, "target");
    }
}
