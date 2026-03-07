use crate::db::Database;
use crate::models::category::{Category, CreateCategoryRequest, UpdateCategoryRequest};
use uuid::Uuid;

pub fn list_categories(db: &Database) -> Result<Vec<Category>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, name, color, icon, is_system, sort_order, created_at FROM categories ORDER BY sort_order, name")
        .map_err(|e| e.to_string())?;

    let categories = stmt
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                is_system: row.get(4)?,
                sort_order: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(categories)
}

pub fn create_category(db: &Database, req: CreateCategoryRequest) -> Result<Category, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let icon = req.icon.unwrap_or_default();
    let sort_order = req.sort_order.unwrap_or(99);

    conn.execute(
        "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
        rusqlite::params![id, req.name, req.color, icon, sort_order, now],
    ).map_err(|e| e.to_string())?;

    Ok(Category {
        id,
        name: req.name,
        color: req.color,
        icon,
        is_system: false,
        sort_order,
        created_at: now,
    })
}

pub fn update_category(db: &Database, id: &str, req: UpdateCategoryRequest) -> Result<Category, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let existing = conn
        .query_row(
            "SELECT id, name, color, icon, is_system, sort_order, created_at FROM categories WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    icon: row.get(3)?,
                    is_system: row.get(4)?,
                    sort_order: row.get(5)?,
                    created_at: row.get(6)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let name = req.name.unwrap_or(existing.name);
    let color = req.color.unwrap_or(existing.color);
    let icon = req.icon.unwrap_or(existing.icon);
    let sort_order = req.sort_order.unwrap_or(existing.sort_order);

    conn.execute(
        "UPDATE categories SET name = ?1, color = ?2, icon = ?3, sort_order = ?4 WHERE id = ?5",
        rusqlite::params![name, color, icon, sort_order, id],
    )
    .map_err(|e| e.to_string())?;

    Ok(Category {
        id: id.to_string(),
        name,
        color,
        icon,
        is_system: existing.is_system,
        sort_order,
        created_at: existing.created_at,
    })
}

pub fn delete_category(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // Check if it's a system category
    let is_system: bool = conn
        .query_row(
            "SELECT is_system FROM categories WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    if is_system {
        return Err("Cannot delete system preset categories".to_string());
    }

    conn.execute("DELETE FROM categories WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_preset_categories() {
        let db = Database::new_in_memory().unwrap();
        let categories = list_categories(&db).unwrap();
        assert_eq!(categories.len(), 4);
        assert!(categories.iter().all(|c| c.is_system));

        let names: Vec<&str> = categories.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"现金类"));
        assert!(names.contains(&"分红股"));
        assert!(names.contains(&"成长股"));
        assert!(names.contains(&"套利"));
    }

    #[test]
    fn test_create_custom_category() {
        let db = Database::new_in_memory().unwrap();

        let category = create_category(
            &db,
            CreateCategoryRequest {
                name: "科技股".to_string(),
                color: "#FF0000".to_string(),
                icon: Some("🖥️".to_string()),
                sort_order: Some(5),
            },
        )
        .unwrap();

        assert_eq!(category.name, "科技股");
        assert!(!category.is_system);

        let categories = list_categories(&db).unwrap();
        assert_eq!(categories.len(), 5);
    }

    #[test]
    fn test_cannot_delete_system_category() {
        let db = Database::new_in_memory().unwrap();
        let result = delete_category(&db, "cat-cash");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot delete system preset"));
    }

    #[test]
    fn test_delete_custom_category() {
        let db = Database::new_in_memory().unwrap();

        let category = create_category(
            &db,
            CreateCategoryRequest {
                name: "Custom".to_string(),
                color: "#000000".to_string(),
                icon: None,
                sort_order: None,
            },
        )
        .unwrap();

        delete_category(&db, &category.id).unwrap();
        let categories = list_categories(&db).unwrap();
        assert_eq!(categories.len(), 4); // Only system categories remain
    }
}
