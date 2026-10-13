// SPDX-License-Identifier: Apache-2.0
// harness.rs — model-agnostic persistence for the Personal Agent's chat thread.
//
// The thread is stored per conversation ('main' for now) as ordered message
// JSON rows keyed by the frontend message id. Every model that runs in the
// harness reads and writes the SAME thread — swapping models keeps the whole
// conversation, exactly like a person who was there all along. Schema v4;
// the table is also created lazily here (idempotent, additive, never
// destructive).

use rusqlite::Connection;
use serde_json::Value;
use tauri::Manager;

pub const MIGRATION_HARNESS_V4: &str = "
CREATE TABLE IF NOT EXISTS agent_thread_messages (
    conversation_id TEXT NOT NULL DEFAULT 'main',
    seq             INTEGER NOT NULL,
    message_id      TEXT NOT NULL,
    data_json       TEXT NOT NULL,
    PRIMARY KEY (conversation_id, seq)
);
";

pub const CONVERSATION: &str = "main";

fn open_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let conn =
        Connection::open(dir.join("guild-foundry-ai.db")).map_err(|e| e.to_string())?;
    conn.execute_batch(MIGRATION_HARNESS_V4)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Load the persisted thread in order (empty when nothing was saved yet).
#[tauri::command(rename_all = "snake_case")]
pub fn harness_thread_load(
    app: tauri::AppHandle,
    conversation_id: Option<String>,
) -> Result<Vec<Value>, String> {
    let conv = conversation_id.unwrap_or_else(|| CONVERSATION.to_string());
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT data_json FROM agent_thread_messages WHERE conversation_id = ?1 ORDER BY seq ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([conv], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Ok(v) = serde_json::from_str::<Value>(&row) {
            out.push(v);
        }
    }
    Ok(out)
}

/// Replace the persisted thread with `messages` (transactional; an empty
/// array persists a cleared history).
#[tauri::command(rename_all = "snake_case")]
pub fn harness_thread_save(
    app: tauri::AppHandle,
    messages: Vec<Value>,
    conversation_id: Option<String>,
) -> Result<usize, String> {
    let conv = conversation_id.unwrap_or_else(|| CONVERSATION.to_string());
    let mut conn = open_db(&app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM agent_thread_messages WHERE conversation_id = ?1",
        [conv.as_str()],
    )
    .map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO agent_thread_messages (conversation_id, seq, message_id, data_json)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(|e| e.to_string())?;
        for (i, m) in messages.iter().enumerate() {
            let id = m
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            stmt.execute(rusqlite::params![
                conv.as_str(),
                i as i64,
                id,
                m.to_string()
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(messages.len())
}

/// Clear the persisted thread.
#[tauri::command(rename_all = "snake_case")]
pub fn harness_thread_clear(
    app: tauri::AppHandle,
    conversation_id: Option<String>,
) -> Result<(), String> {
    let conv = conversation_id.unwrap_or_else(|| CONVERSATION.to_string());
    let conn = open_db(&app)?;
    conn.execute(
        "DELETE FROM agent_thread_messages WHERE conversation_id = ?1",
        [conv.as_str()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
