//! CRUD commands for AI assistant skills. Skills are Markdown files with a
//! small frontmatter block; the service layer owns parsing and persistence.
//! See `services/skill_service` for the file format and the built-in set.

use crate::models::skill::Skill;
use crate::services::skill_service;
use std::path::PathBuf;
use tauri::AppHandle;

/// List every skill in the user skills directory (built-in + user-authored).
#[tauri::command(rename_all = "camelCase")]
pub fn list_skills(app: AppHandle) -> Result<Vec<Skill>, String> {
    skill_service::list_skills(&app)
}

/// Create or update a skill. The `id` field selects the file name; saving
/// always writes to `{id}.md` and drops the builtin marker so subsequent
/// reads report the skill as `source: "user"`.
#[tauri::command(rename_all = "camelCase")]
pub fn save_skill(app: AppHandle, skill: Skill) -> Result<(), String> {
    skill_service::save_skill(&app, &skill)
}

/// Delete a skill file. Works on both user and built-in skills; deleting a
/// built-in skill simply removes the materialised `.md` (it can be restored
/// via `reset_skills`).
#[tauri::command(rename_all = "camelCase")]
pub fn delete_skill(app: AppHandle, id: String) -> Result<(), String> {
    skill_service::delete_skill(&app, &id)
}

/// Wipe every skill file and re-materialise the built-in set, restoring the
/// factory state. Used by the "恢复内置技能" button and by `factory_reset`.
#[tauri::command(rename_all = "camelCase")]
pub fn reset_skills(app: AppHandle) -> Result<(), String> {
    skill_service::reset_all_skills(&app)
}

/// Clone an existing skill to a fresh id. Used by the "克隆" button — the
/// frontend supplies the new id (the UI suggests `{source}-copy` and lets
/// the user rename). The clone is always user-owned.
#[tauri::command(rename_all = "camelCase")]
pub fn clone_skill(app: AppHandle, source_id: String, new_id: String) -> Result<Skill, String> {
    skill_service::clone_skill(&app, &source_id, &new_id)
}

/// Export a skill to a `.md` file chosen by the user via the Tauri save
/// dialog (the dialog runs in the frontend; only the resulting path is
/// passed here). Returns the written path for confirmation toasts.
#[tauri::command(rename_all = "camelCase")]
pub fn export_skill(app: AppHandle, id: String, path: String) -> Result<String, String> {
    let skill = skill_service::get_skill(&app, &id)?;
    skill_service::export_skill_to_path(&skill, &PathBuf::from(path))
}

/// Import a skill from a `.md` file chosen by the user via the Tauri open
/// dialog. Returns the saved skill (its id may differ from the file stem if
/// the parsed id collided with an existing skill).
#[tauri::command(rename_all = "camelCase")]
pub fn import_skill(app: AppHandle, path: String) -> Result<Skill, String> {
    skill_service::import_skill_from_path(&app, &PathBuf::from(path))
}
