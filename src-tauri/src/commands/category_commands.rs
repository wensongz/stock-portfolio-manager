use crate::db::Database;
use crate::models::category::{Category, CreateCategoryRequest, UpdateCategoryRequest};
use crate::services::category_service;
use tauri::State;

#[tauri::command]
pub fn list_categories(db: State<'_, Database>) -> Result<Vec<Category>, String> {
    category_service::list_categories(&db)
}

#[tauri::command]
pub fn create_category(
    db: State<'_, Database>,
    request: CreateCategoryRequest,
) -> Result<Category, String> {
    category_service::create_category(&db, request)
}

#[tauri::command]
pub fn update_category(
    db: State<'_, Database>,
    id: String,
    request: UpdateCategoryRequest,
) -> Result<Category, String> {
    category_service::update_category(&db, &id, request)
}

#[tauri::command]
pub fn delete_category(db: State<'_, Database>, id: String) -> Result<(), String> {
    category_service::delete_category(&db, &id)
}
