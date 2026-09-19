use rusqlite::{Connection, Result};

pub(super) fn migrate_v10(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS alert_history (
           id TEXT PRIMARY KEY NOT NULL,
           kind TEXT NOT NULL CHECK(kind IN ('PRICE', 'PORTFOLIO')),
           title TEXT NOT NULL,
           message TEXT NOT NULL,
           scope_name TEXT NOT NULL,
           account_name TEXT,
           triggered_at TEXT NOT NULL,
           details_json TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_alert_history_triggered_at
           ON alert_history(triggered_at DESC, id DESC);
         CREATE INDEX IF NOT EXISTS idx_alert_history_kind_triggered_at
           ON alert_history(kind, triggered_at DESC, id DESC);",
    )
}

#[cfg(test)]
mod tests {
    use crate::db::migrations::{run_migrations, CURRENT_SCHEMA_VERSION};
    use rusqlite::Connection;

    #[test]
    fn v9_migration_adds_alert_history_table_and_indexes_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("v9.sqlite");
        {
            let mut conn = Connection::open(&path).unwrap();
            crate::db::schema::create_current_schema(&conn).unwrap();
            conn.execute("INSERT INTO accounts (id,name,market,created_at,updated_at) VALUES ('kept','Kept','US','old','old')", []).unwrap();
            conn.pragma_update(None, "user_version", 9).unwrap();
            run_migrations(&mut conn).unwrap();
            assert_eq!(
                conn.pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
                    .unwrap(),
                CURRENT_SCHEMA_VERSION
            );
        }

        let reopened = Connection::open(&path).unwrap();
        let objects = reopened
            .prepare(
                "SELECT name FROM sqlite_master WHERE name LIKE '%alert_history%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            objects,
            vec![
                "alert_history",
                "idx_alert_history_kind_triggered_at",
                "idx_alert_history_triggered_at",
                "sqlite_autoindex_alert_history_1",
            ]
        );
        assert_eq!(
            reopened
                .query_row("SELECT name FROM accounts WHERE id='kept'", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "Kept"
        );
    }

    #[test]
    fn failed_v10_migration_does_not_advance_version_or_leave_partial_indexes() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_current_schema(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE alert_history (id TEXT PRIMARY KEY);
             PRAGMA user_version = 9;",
        )
        .unwrap();

        assert!(run_migrations(&mut conn).is_err());
        assert_eq!(
            conn.pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            9
        );
        let new_indexes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name LIKE 'idx_alert_history_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new_indexes, 0);
    }
}
