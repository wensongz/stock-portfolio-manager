use crate::db::Database;
use crate::models::Account;
use tauri::State;

#[tauri::command(rename_all = "snake_case")]
pub fn create_account(
    db: State<Database>,
    name: String,
    market: String,
    description: Option<String>,
) -> Result<Account, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, name, market, description, now, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(Account {
        id,
        name,
        market,
        description,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_accounts(db: State<Database>) -> Result<Vec<Account>, String> {
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

#[tauri::command(rename_all = "snake_case")]
pub fn update_account(
    db: State<Database>,
    id: String,
    name: String,
    market: String,
    description: Option<String>,
) -> Result<Account, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let rows_affected = conn
        .execute(
            "UPDATE accounts SET name = ?2, market = ?3, description = ?4, updated_at = ?5 WHERE id = ?1",
            rusqlite::params![id, name, market, description, now],
        )
        .map_err(|e| e.to_string())?;
    if rows_affected == 0 {
        return Err(format!("Account with id {} not found", id));
    }
    let created_at: String = conn
        .query_row(
            "SELECT created_at FROM accounts WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(Account {
        id,
        name,
        market,
        description,
        created_at,
        updated_at: now,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub fn delete_account(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
