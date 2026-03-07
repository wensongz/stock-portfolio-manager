use crate::db::Database;
use crate::models::account::{Account, CreateAccountRequest, UpdateAccountRequest};
use uuid::Uuid;

pub fn create_account(db: &Database, req: CreateAccountRequest) -> Result<Account, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let description = req.description.unwrap_or_default();

    conn.execute(
        "INSERT INTO accounts (id, name, market, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, req.name, req.market, description, now, now],
    ).map_err(|e| e.to_string())?;

    Ok(Account {
        id,
        name: req.name,
        market: req.market,
        description,
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn list_accounts(db: &Database) -> Result<Vec<Account>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, name, market, description, created_at, updated_at FROM accounts ORDER BY market, name")
        .map_err(|e| e.to_string())?;

    let accounts = stmt
        .query_map([], |row| {
            Ok(Account {
                id: row.get(0)?,
                name: row.get(1)?,
                market: row.get(2)?,
                description: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(accounts)
}

pub fn get_account(db: &Database, id: &str) -> Result<Account, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, name, market, description, created_at, updated_at FROM accounts WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(Account {
                id: row.get(0)?,
                name: row.get(1)?,
                market: row.get(2)?,
                description: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn update_account(db: &Database, id: &str, req: UpdateAccountRequest) -> Result<Account, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let existing = conn
        .query_row(
            "SELECT id, name, market, description, created_at, updated_at FROM accounts WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Account {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    market: row.get(2)?,
                    description: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let name = req.name.unwrap_or(existing.name);
    let description = req.description.unwrap_or(existing.description);

    conn.execute(
        "UPDATE accounts SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![name, description, now, id],
    )
    .map_err(|e| e.to_string())?;

    Ok(Account {
        id: id.to_string(),
        name,
        market: existing.market,
        description,
        created_at: existing.created_at,
        updated_at: now,
    })
}

pub fn delete_account(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let changes = conn
        .execute("DELETE FROM accounts WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;

    if changes == 0 {
        return Err("Account not found".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_list_accounts() {
        let db = Database::new_in_memory().unwrap();

        let account = create_account(
            &db,
            CreateAccountRequest {
                name: "Robinhood".to_string(),
                market: "US".to_string(),
                description: Some("US brokerage".to_string()),
            },
        )
        .unwrap();

        assert_eq!(account.name, "Robinhood");
        assert_eq!(account.market, "US");

        let accounts = list_accounts(&db).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "Robinhood");
    }

    #[test]
    fn test_update_account() {
        let db = Database::new_in_memory().unwrap();

        let account = create_account(
            &db,
            CreateAccountRequest {
                name: "Old Name".to_string(),
                market: "CN".to_string(),
                description: None,
            },
        )
        .unwrap();

        let updated = update_account(
            &db,
            &account.id,
            UpdateAccountRequest {
                name: Some("New Name".to_string()),
                description: Some("Updated description".to_string()),
            },
        )
        .unwrap();

        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.description, "Updated description");
        assert_eq!(updated.market, "CN");
    }

    #[test]
    fn test_delete_account() {
        let db = Database::new_in_memory().unwrap();

        let account = create_account(
            &db,
            CreateAccountRequest {
                name: "To Delete".to_string(),
                market: "HK".to_string(),
                description: None,
            },
        )
        .unwrap();

        delete_account(&db, &account.id).unwrap();
        let accounts = list_accounts(&db).unwrap();
        assert_eq!(accounts.len(), 0);
    }

    #[test]
    fn test_delete_nonexistent_account() {
        let db = Database::new_in_memory().unwrap();
        let result = delete_account(&db, "nonexistent-id");
        assert!(result.is_err());
    }
}
