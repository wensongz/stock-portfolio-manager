use crate::db::Database;
use rusqlite::{backup::Backup, Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

pub fn backup_database(source: &Database, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!(
            "Backup destination already exists: {}",
            destination.display()
        ));
    }

    let backup_result = (|| -> Result<(), String> {
        let source_conn = source.conn.lock().map_err(|error| error.to_string())?;
        let mut destination_conn = Connection::open_with_flags(
            destination,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("Unable to create backup database: {error}"))?;
        {
            let backup = Backup::new(&source_conn, &mut destination_conn)
                .map_err(|error| format!("Unable to start SQLite backup: {error}"))?;
            backup
                .run_to_completion(100, Duration::from_millis(10), None)
                .map_err(|error| format!("SQLite backup failed: {error}"))?;
        }
        destination_conn
            .close()
            .map_err(|(_, error)| format!("Unable to close backup database: {error}"))?;

        verify_backup(destination)
    })();

    if let Err(error) = backup_result {
        let _ = std::fs::remove_file(destination);
        return Err(error);
    }
    Ok(())
}

fn verify_backup(path: &Path) -> Result<(), String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Unable to reopen backup database: {error}"))?;
    let mut statement = connection
        .prepare("PRAGMA integrity_check")
        .map_err(|error| format!("Unable to prepare backup integrity check: {error}"))?;
    let messages: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .map_err(|error| format!("Unable to run backup integrity check: {error}"))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("Unable to read backup integrity result: {error}"))?;
    if messages.as_slice() != ["ok"] {
        return Err(format!(
            "Backup database failed integrity_check: {}",
            messages.join("; ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::backup_database;
    use crate::db::Database;
    use rusqlite::{Connection, OpenFlags};

    #[test]
    fn live_backup_is_consistent_and_contains_committed_rows() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_path = temp_dir.path().join("portfolio.db");
        let destination_path = temp_dir.path().join("portfolio-backup.db");
        let db = Database::new(source_path.to_str().unwrap()).unwrap();

        {
            let conn = db.conn.lock().unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('account-1', 'Backup', 'US', '2026-09-01', '2026-09-01')",
                [],
            )
            .unwrap();
        }

        backup_database(&db, &destination_path).unwrap();

        let backup = Connection::open_with_flags(
            &destination_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let integrity: Vec<String> = backup
            .prepare("PRAGMA integrity_check")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let account_count: i64 = backup
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .unwrap();

        assert_eq!(integrity, ["ok"]);
        assert_eq!(account_count, 1);
    }
}
