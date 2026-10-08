// SPDX-License-Identifier: Apache-2.0
// Agent-tab avatar images: custom uploads and model-generated redesigns.
// The custom portrait (app-data/avatars/custom.png) overrides the picked
// boy/girl default when the `avatar.custom` setting is "1".

use tauri::Manager;

pub const AVATAR_DIR: &str = "avatars";
pub const CUSTOM_FILE: &str = "custom.png";
pub const CUSTOM_FLAG: &str = "avatar.custom";

fn avatars_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(AVATAR_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn is_image(bytes: &[u8]) -> bool {
    // PNG, JPEG, WebP magic bytes.
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP"
}

fn save_custom(app: &tauri::AppHandle, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 10 * 1024 * 1024 {
        return Err("image must be 1 byte–10 MB".to_string());
    }
    if !is_image(bytes) {
        return Err("not a recognized image (PNG/JPEG/WebP)".to_string());
    }
    let path = avatars_dir(app)?.join(CUSTOM_FILE);
    std::fs::write(&path, bytes).map_err(|e| format!("save failed: {}", e))?;
    crate::settings::settings_set(app.clone(), CUSTOM_FLAG.to_string(), serde_json::json!("1"))?;
    Ok(())
}

/// Upload a custom avatar from the frontend (base64 data URL or raw base64).
#[tauri::command]
pub fn avatar_upload_image(app: tauri::AppHandle, data_base64: String) -> Result<(), String> {
    use base64::Engine;
    let raw = data_base64.trim();
    let b64 = raw.split(',').next_back().unwrap_or(raw);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("bad base64 image: {}", e))?;
    save_custom(&app, &bytes)
}

/// Let the agent set the avatar from a generated/saved image file.
/// Used after `media.generate_image` redesigns the avatar.
pub fn set_avatar_from_path(
    app: &tauri::AppHandle,
    source_path: &str,
) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(source_path).map_err(|e| format!("read failed: {}", e))?;
    save_custom(app, &bytes)?;
    Ok(serde_json::json!({ "ok": true, "avatar": "custom" }))
}

/// Tauri command wrapper.
#[tauri::command]
pub fn avatar_set_image(app: tauri::AppHandle, source_path: String) -> Result<(), String> {
    set_avatar_from_path(&app, &source_path)?;
    Ok(())
}

/// Revert to the picked boy/girl default.
#[tauri::command]
pub fn avatar_clear_custom(app: tauri::AppHandle) -> Result<(), String> {
    let path = avatars_dir(&app)?.join(CUSTOM_FILE);
    let _ = std::fs::remove_file(&path);
    crate::settings::settings_set(app.clone(), CUSTOM_FLAG.to_string(), serde_json::json!("0"))?;
    Ok(())
}
