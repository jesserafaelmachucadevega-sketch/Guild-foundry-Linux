// Phase 1 — Non-secret settings persistence (system prompts, UI prefs).
//
// Secrets (API keys) NEVER live here — they go to the keyring (Phase 2,
// secrets.rs). This store holds only non-sensitive configuration.

use std::collections::HashMap;
use tauri::Manager;
use tauri_plugin_store::StoreExt;

#[tauri::command]
pub fn settings_save_prompts(
    app: tauri::AppHandle,
    prompts: HashMap<String, String>,
) -> Result<(), String> {
    let store = app.store("settings.json").map_err(|e| e.to_string())?;
    store.set(
        "system_prompts",
        serde_json::to_value(&prompts).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn settings_load_prompts(
    app: tauri::AppHandle,
) -> Result<HashMap<String, String>, String> {
    let store = app.store("settings.json").map_err(|e| e.to_string())?;
    match store.get("system_prompts") {
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string()),
        None => Ok(HashMap::new()),
    }
}

#[tauri::command]
pub fn settings_set(app: tauri::AppHandle, key: String, value: serde_json::Value) -> Result<(), String> {
    if key.starts_with("secret") || key.contains("api_key") || key.contains("token") {
        return Err("refused: secrets must use the keyring, not settings".to_string());
    }
    let store = app.store("settings.json").map_err(|e| e.to_string())?;
    store.set(key, value);
    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn settings_get(app: tauri::AppHandle, key: String) -> Result<Option<serde_json::Value>, String> {
    let store = app.store("settings.json").map_err(|e| e.to_string())?;
    Ok(store.get(key))
}
