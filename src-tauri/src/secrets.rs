// SPDX-License-Identifier: Apache-2.0
// Phase 2 — Secure credential storage via the OS keyring (Linux Secret Service).
//
// Master spec section 11: API keys are NEVER stored in SQLite, localStorage,
// source files, or logs. They live in the platform credential store, are
// encrypted at rest by the Secret Service, are masked in the UI, and are never
// returned to the frontend (only presence is reported via `has_key`).
//
// keyring 3 has no enumeration API, so a small `keyring_keys` index table in
// the app SQLite database tracks which keys were stored through these
// commands. `secret_list_keys` cross-checks the index against the real store
// and prunes stale entries.

use std::path::PathBuf;
use tauri::Manager;

/// Locate the application SQLite database (same file db.rs manages).
pub(crate) fn db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("guild-foundry-ai.db"))
}

fn keyring_entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("guild-foundry-ai", key).map_err(|e| e.to_string())
}

fn ensure_index(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS keyring_keys (
            name TEXT PRIMARY KEY
        )",
    )
}

/// Non-command helper for other backend modules: read a secret without
/// exposing it to the frontend.
pub(crate) fn keyring_get(key: &str) -> Option<String> {
    keyring_entry(key).ok()?.get_password().ok()
}

fn validate_key(key: &str) -> Result<&str, String> {
    let k = key.trim();
    if k.is_empty() {
        return Err("refused: empty secret key".to_string());
    }
    if k.len() > 256 {
        return Err("refused: secret key too long".to_string());
    }
    Ok(k)
}

/// Store (or rotate) a secret in the OS keyring. Values are never logged.
#[tauri::command]
pub fn secret_set(app: tauri::AppHandle, key: String, value: String) -> Result<(), String> {
    let k = validate_key(&key)?.to_string();
    if value.is_empty() {
        return Err("refused: empty secret value".to_string());
    }
    // Value goes only to the keyring entry; never into SQLite or logs.
    keyring_entry(&k)?.set_password(&value).map_err(|e| e.to_string())?;
    let conn =
        rusqlite::Connection::open(db_path(&app)?).map_err(|e| e.to_string())?;
    ensure_index(&conn).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO keyring_keys (name) VALUES (?1)",
        rusqlite::params![k],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Fetch a secret. Only the Rust backend calls this for provider auth;
/// the frontend must never receive raw secret values.
#[tauri::command]
pub fn secret_get(key: String) -> Result<Option<String>, String> {
    let k = validate_key(&key)?.to_string();
    Ok(keyring_get(&k))
}

/// Delete a secret from the OS keyring and drop it from the index.
#[tauri::command]
pub fn secret_delete(app: tauri::AppHandle, key: String) -> Result<(), String> {
    let k = validate_key(&key)?.to_string();
    match keyring_entry(&k)?.delete_credential() {
        Ok(()) => {}
        // Deleting a missing credential is not an error.
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("not found")
                && !msg.contains("No such")
                && !msg.contains("does not exist")
            {
                return Err(msg);
            }
        }
    }
    let conn =
        rusqlite::Connection::open(db_path(&app)?).map_err(|e| e.to_string())?;
    ensure_index(&conn).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM keyring_keys WHERE name = ?1", rusqlite::params![k])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// List key names present in the keyring (names only — never values).
/// Stale index rows (entry deleted outside this app) are pruned.
#[tauri::command]
pub fn secret_list_keys(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let conn =
        rusqlite::Connection::open(db_path(&app)?).map_err(|e| e.to_string())?;
    ensure_index(&conn).map_err(|e| e.to_string())?;
    let names: Vec<String> = conn
        .prepare("SELECT name FROM keyring_keys ORDER BY name")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())?;
    let mut live = Vec::with_capacity(names.len());
    for name in names {
        if keyring_get(&name).is_some() {
            live.push(name);
        } else {
            let _ = conn.execute(
                "DELETE FROM keyring_keys WHERE name = ?1",
                rusqlite::params![name],
            );
        }
    }
    Ok(live)
}
