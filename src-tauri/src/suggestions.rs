// SPDX-License-Identifier: Apache-2.0
// Suggested to-do: dynamic follow-up actions the user can run or dismiss.
// Generated at the end of an agent turn (see src/lib/suggestions.ts), stored
// per conversation. Tapping a suggestion sends its stored prompt as a new
// user message through the normal permission gates — suggestions never
// auto-run. Dismissed suggestions are kept so generation can avoid repeats.

use serde::{Deserialize, Serialize};
use tauri::Manager;

pub const MIGRATION_SUGGESTIONS_V3: &str = r#"
CREATE TABLE IF NOT EXISTS suggestions (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    label           TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',  -- pending | accepted | dismissed
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_suggestions_conversation ON suggestions(conversation_id);
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub conversation_id: String,
    pub label: String,
    pub prompt: String,
    pub status: String,
    pub created_at: String,
}

fn db_conn(app: &tauri::AppHandle) -> Result<rusqlite::Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let db_path = dir.join("guild-foundry-ai.db");
    rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("sug-{}-{}", nanos, rand_suffix())
}

fn rand_suffix() -> String {
    // Small non-crypto suffix; uniqueness comes from the timestamp prefix.
    let mut x: u64 = 0x9E3779B97F4A7C15;
    for _ in 0..4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
    }
    format!("{:x}", x & 0xffff)
}

#[tauri::command]
pub fn suggestions_list(
    app: tauri::AppHandle,
    conversation_id: String,
) -> Result<Vec<Suggestion>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, conversation_id, label, prompt, status, created_at
             FROM suggestions WHERE conversation_id = ?1
             ORDER BY created_at DESC LIMIT 50",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([conversation_id], |row| {
            Ok(Suggestion {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                label: row.get(2)?,
                prompt: row.get(3)?,
                status: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Recent labels (any status) so generation can avoid repeats. Newest first.
#[tauri::command]
pub fn suggestions_recent_labels(
    app: tauri::AppHandle,
    conversation_id: String,
    limit: i64,
) -> Result<Vec<String>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT label FROM suggestions WHERE conversation_id = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![conversation_id, limit], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[tauri::command]
pub fn suggestions_add(
    app: tauri::AppHandle,
    conversation_id: String,
    label: String,
    prompt: String,
) -> Result<Suggestion, String> {
    let label = label.trim().to_string();
    let prompt = prompt.trim().to_string();
    if label.is_empty() || prompt.is_empty() {
        return Err("label and prompt are required".to_string());
    }
    if label.chars().count() > 140 || prompt.chars().count() > 2000 {
        return Err("label (140) or prompt (2000) too long".to_string());
    }
    let conn = db_conn(&app)?;
    let s = Suggestion {
        id: new_id(),
        conversation_id,
        label,
        prompt,
        status: "pending".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    conn.execute(
        "INSERT INTO suggestions (id, conversation_id, label, prompt, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![s.id, s.conversation_id, s.label, s.prompt, s.status, s.created_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(s)
}

fn set_status(app: &tauri::AppHandle, id: &str, status: &str) -> Result<(), String> {
    let conn = db_conn(app)?;
    let n = conn
        .execute(
            "UPDATE suggestions SET status = ?1 WHERE id = ?2",
            rusqlite::params![status, id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("suggestion not found".to_string());
    }
    Ok(())
}

/// The user tapped it: it ran as a normal user message. Kept for history.
#[tauri::command]
pub fn suggestions_accept(app: tauri::AppHandle, id: String) -> Result<(), String> {
    set_status(&app, &id, "accepted")
}

/// The user dismissed it: generation avoids repeating this pattern.
#[tauri::command]
pub fn suggestions_dismiss(app: tauri::AppHandle, id: String) -> Result<(), String> {
    set_status(&app, &id, "dismissed")
}
