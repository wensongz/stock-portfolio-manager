use crate::db::Database;
use crate::models::Category;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn create_category(
    db: State<Database>,
    name: String,
    color: String,
    icon: String,
    sort_order: Option<i32>,
) -> Result<Category, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let order = sort_order.unwrap_or(100);
    conn.execute(
        "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
        rusqlite::params![id, name, color, icon, order, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(Category {
        id,
        name,
        color,
        icon,
        is_system: false,
        sort_order: order,
        created_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_categories(db: State<Database>) -> Result<Vec<Category>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, color, icon, is_system, sort_order, created_at
             FROM categories ORDER BY sort_order, name",
        )
        .map_err(|e| e.to_string())?;
    let categories = stmt
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                is_system: row.get::<_, i32>(4)? != 0,
                sort_order: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(categories)
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_category(
    db: State<Database>,
    id: String,
    name: String,
    color: String,
    icon: String,
    sort_order: Option<i32>,
) -> Result<Category, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let order = sort_order.unwrap_or(100);
    let rows_affected = conn
        .execute(
            "UPDATE categories SET name = ?2, color = ?3, icon = ?4, sort_order = ?5 WHERE id = ?1",
            rusqlite::params![id, name, color, icon, order],
        )
        .map_err(|e| e.to_string())?;
    if rows_affected == 0 {
        return Err(format!("Category with id {} not found", id));
    }
    let (is_system, created_at): (i32, String) = conn
        .query_row(
            "SELECT is_system, created_at FROM categories WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    Ok(Category {
        id,
        name,
        color,
        icon,
        is_system: is_system != 0,
        sort_order: order,
        created_at,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_category(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    // Prevent deletion of system categories
    let is_system: i32 = conn
        .query_row(
            "SELECT is_system FROM categories WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if is_system != 0 {
        return Err("Cannot delete system categories".to_string());
    }
    conn.execute("DELETE FROM categories WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
