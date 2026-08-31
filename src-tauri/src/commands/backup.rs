use crate::db::Database;
use crate::services::backup_service::backup_database;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use tauri::{Manager, State};
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BackupConfig {
    pub directory: Option<String>,
    pub auto_backup: bool,
    pub last_backup_mtime: Option<u64>,
    pub last_backup_size: Option<u64>,
    pub last_backup_time: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupResult {
    pub success: bool,
    pub path: Option<String>,
    pub message: Option<String>,
}

pub(crate) fn config_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("failed to get app data dir")
        .join("backup_config.json")
}

fn load_config(app: &tauri::AppHandle) -> BackupConfig {
    let path = config_path(app);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub(crate) fn save_config(app: &tauri::AppHandle, config: &BackupConfig) -> Result<(), String> {
    let path = config_path(app);
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    atomic_write(&path, json.as_bytes())
}

fn atomic_write(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("配置文件没有父目录: {}", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("backup_config"),
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|error| error.to_string())?;
        file.write_all(contents)
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        std::fs::rename(&temporary_path, path).map_err(|error| error.to_string())?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result
}

/// Get file mtime as seconds since epoch, or None if unavailable.
fn file_mtime_secs(path: &str) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_secs())
}

fn database_mtime_secs(path: &str) -> Option<u64> {
    [path.to_string(), format!("{path}-wal")]
        .iter()
        .filter_map(|candidate| file_mtime_secs(candidate))
        .max()
}

fn database_size(path: &str) -> Option<u64> {
    let database_size = std::fs::metadata(path).ok()?.len();
    let wal_size = std::fs::metadata(format!("{path}-wal"))
        .ok()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Some(database_size + wal_size)
}

/// Check if DB has changed since last backup.
fn db_changed(db_path: &str, config: &BackupConfig) -> bool {
    let mtime = database_mtime_secs(db_path);
    let size = database_size(db_path);
    mtime != config.last_backup_mtime || size != config.last_backup_size
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_backup_config(app: tauri::AppHandle) -> BackupConfig {
    load_config(&app)
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_backup_config(app: tauri::AppHandle, config: BackupConfig) -> Result<(), String> {
    save_config(&app, &config)
}

#[tauri::command(rename_all = "camelCase")]
pub fn backup_database_now(
    app: tauri::AppHandle,
    db: State<Database>,
) -> Result<BackupResult, String> {
    let config = load_config(&app);
    let dir = config
        .directory
        .as_ref()
        .ok_or_else(|| "请先设置备份目录".to_string())?;
    let backup_dir = PathBuf::from(dir);
    std::fs::create_dir_all(&backup_dir).map_err(|e| format!("无法创建备份目录: {}", e))?;

    let db_path = &db.path;

    if !db_changed(db_path, &config) {
        return Ok(BackupResult {
            success: true,
            path: None,
            message: Some("数据库无变化，跳过备份".to_string()),
        });
    }

    let now = chrono::Local::now();
    let filename = format!("portfolio_{}.db", now.format("%Y-%m-%d_%H-%M-%S"));
    let dest = backup_dir.join(&filename);

    backup_database(&db, &dest).map_err(|e| format!("备份失败: {}", e))?;

    let mut new_config = config;
    new_config.last_backup_mtime = database_mtime_secs(db_path);
    new_config.last_backup_size = database_size(db_path);
    new_config.last_backup_time = Some(now.to_rfc3339());
    save_config(&app, &new_config)?;

    Ok(BackupResult {
        success: true,
        path: Some(dest.to_string_lossy().to_string()),
        message: None,
    })
}

/// Called on app startup to perform auto-backup if enabled and needed.
pub fn auto_backup_if_needed(app: &tauri::AppHandle) {
    let config = load_config(app);
    if !config.auto_backup || config.directory.is_none() {
        return;
    }

    let db_path = app
        .path()
        .app_data_dir()
        .expect("app data dir")
        .join("portfolio.db");
    let db_path_str = db_path.to_string_lossy().to_string();

    if !db_changed(&db_path_str, &config) {
        return;
    }

    // Check if last backup was > 7 days ago
    if let Some(ref last_time) = config.last_backup_time {
        if let Ok(last) = chrono::DateTime::parse_from_rfc3339(last_time) {
            let last_utc: chrono::DateTime<chrono::Utc> = last.into();
            let days_since = (chrono::Utc::now() - last_utc).num_days();
            if days_since < 7 {
                return;
            }
        }
    }

    let backup_dir = std::path::PathBuf::from(config.directory.as_ref().unwrap());
    if let Err(e) = std::fs::create_dir_all(&backup_dir) {
        warn!("[auto-backup] failed to create dir: {}", e);
        return;
    }

    let now = chrono::Local::now();
    let filename = format!("portfolio_{}.db", now.format("%Y-%m-%d_%H-%M-%S"));
    let dest = backup_dir.join(&filename);

    let db = app.state::<Database>();
    match backup_database(&db, &dest) {
        Ok(()) => {
            info!("[auto-backup] saved to {}", dest.display());
            let mut new_config = config;
            new_config.last_backup_mtime = database_mtime_secs(&db_path_str);
            new_config.last_backup_size = database_size(&db_path_str);
            new_config.last_backup_time = Some(chrono::Utc::now().to_rfc3339());
            if let Err(e) = save_config(app, &new_config) {
                warn!("[auto-backup] failed to save config: {}", e);
            }
        }
        Err(e) => warn!("[auto-backup] failed: {}", e),
    }
}
