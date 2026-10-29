use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;
use tauri::AppHandle;

use crate::library::shared_data_dir;

static SETTINGS_FILE_LOCK: Mutex<()> = Mutex::new(());

fn settings_path(app: &AppHandle) -> PathBuf {
    shared_data_dir(app).join("nagefurin-setting.json")
}

#[tauri::command]
pub fn load_fork_settings(app: AppHandle) -> Result<Value, String> {
    let _guard = SETTINGS_FILE_LOCK.lock().map_err(|error| error.to_string())?;
    let path = settings_path(&app);
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let bytes = fs::read(&path).map_err(|error| format!("Could not read fork settings: {error}"))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Fork settings JSON is invalid: {error}"))?;
    if !value.is_object() {
        return Err("Fork settings must be a JSON object.".to_string());
    }
    Ok(value)
}

#[tauri::command]
pub fn save_fork_settings(app: AppHandle, settings: Value) -> Result<(), String> {
    if !settings.is_object() {
        return Err("Fork settings must be a JSON object.".to_string());
    }
    let _guard = SETTINGS_FILE_LOCK.lock().map_err(|error| error.to_string())?;
    let path = settings_path(&app);
    let parent = path.parent().ok_or_else(|| "Fork settings directory is unavailable.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("Could not create fork settings directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string())?;
    let temporary = path.with_file_name("nagefurin-setting.json.tmp");
    fs::write(&temporary, bytes).map_err(|error| format!("Could not write fork settings: {error}"))?;
    if path.exists() {
        fs::remove_file(&path).map_err(|error| format!("Could not replace fork settings: {error}"))?;
    }
    fs::rename(&temporary, &path).map_err(|error| format!("Could not activate fork settings file: {error}"))?;
    Ok(())
}
