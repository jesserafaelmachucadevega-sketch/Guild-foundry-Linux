// SPDX-License-Identifier: Apache-2.0
// Phase 14 — Scheduled agent runs. Cron-like tasks that run a tool-calling
// agent headlessly on a schedule (interval or daily at a local time) and
// notify the user with the result. This is the "works while you sleep" half
// of the agent: watches, digests, reminders.
//
// Design: Rust owns persistence + ticking. When a task is due, Rust emits a
// `scheduler-task-due` event; the frontend runs the agent loop headlessly
// (it owns runToolLoop) and reports back via `scheduler_complete`.
// No new crates: schedules are interval-minutes or daily-at-HH:MM.

use chrono::{Local, NaiveTime, TimeZone};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    task_prompt TEXT NOT NULL,
    provider_id TEXT NOT NULL DEFAULT '',
    model_id TEXT NOT NULL DEFAULT '',
    schedule_kind TEXT NOT NULL DEFAULT 'interval',
    interval_minutes INTEGER NOT NULL DEFAULT 60,
    daily_at TEXT NOT NULL DEFAULT '08:00',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run_at TEXT,
    next_run_at TEXT,
    last_status TEXT,
    last_summary TEXT,
    auto_approve INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);";
// Idempotent column add for DBs created before auto_approve existed.
const MIGRATION_AUTO_APPROVE: &str =
    "ALTER TABLE scheduled_tasks ADD COLUMN auto_approve INTEGER NOT NULL DEFAULT 0;";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub task_prompt: String,
    pub provider_id: String,
    pub model_id: String,
    pub schedule_kind: String, // "interval" | "daily"
    pub interval_minutes: i64,
    pub daily_at: String, // "HH:MM" local
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub last_status: Option<String>,
    pub last_summary: Option<String>,
    pub auto_approve: bool,
}

/// In-memory session grants: session_id → task_id. A grant means the user
/// marked that scheduled task as auto-approve, so its headless run may
/// execute tools without the interactive approval modal. Grants live only
/// for the run's session and never persist. An explicit Deny still wins.
static SESSION_GRANTS: std::sync::OnceLock<Mutex<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn grants() -> &'static Mutex<std::collections::HashMap<String, String>> {
    SESSION_GRANTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Returns true if this session is a granted auto-approve scheduled run.
pub fn session_granted(session_id: &str) -> bool {
    grants()
        .lock()
        .map(|g| g.contains_key(session_id))
        .unwrap_or(false)
}

/// Revoke a session grant (called when the headless run ends).
pub fn revoke_grant(session_id: &str) {
    if let Ok(mut g) = grants().lock() {
        g.remove(session_id);
    }
}

/// Grant a session for a headless scheduled run. Only succeeds if the task
/// exists and the user enabled auto-approve for it.
#[tauri::command]
pub fn scheduler_grant_session(app: AppHandle, session_id: String, task_id: String) -> Result<(), String> {
    let auto: Option<i64> = open(&app)?
        .query_row(
            "SELECT auto_approve FROM scheduled_tasks WHERE id=? AND enabled=1",
            params![task_id],
            |r| r.get(0),
        )
        .ok();
    if auto != Some(1) {
        return Err("task not found, disabled, or auto-approve not enabled".to_string());
    }
    grants()
        .lock()
        .map(|mut g| {
            g.insert(session_id, task_id);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn db_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("guild-foundry-ai.db"))
}

fn open(app: &AppHandle) -> Result<Connection, String> {
    let conn = Connection::open(db_path(app)?).map_err(|e| e.to_string())?;
    conn.execute_batch(MIGRATION).map_err(|e| e.to_string())?;
    // Best-effort: older DBs lack the column; ignore if it already exists.
    let _ = conn.execute_batch(MIGRATION_AUTO_APPROVE);
    Ok(conn)
}

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<ScheduledTask> {
    Ok(ScheduledTask {
        id: row.get(0)?,
        name: row.get(1)?,
        task_prompt: row.get(2)?,
        provider_id: row.get(3)?,
        model_id: row.get(4)?,
        schedule_kind: row.get(5)?,
        interval_minutes: row.get(6)?,
        daily_at: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        last_run_at: row.get(9)?,
        next_run_at: row.get(10)?,
        last_status: row.get(11)?,
        last_summary: row.get(12)?,
        auto_approve: row.get::<_, i64>(13).unwrap_or(0) != 0,
    })
}

fn compute_next_run(kind: &str, interval_minutes: i64, daily_at: &str) -> String {
    let now = Local::now();
    if kind == "daily" {
        let t = NaiveTime::parse_from_str(daily_at, "%H:%M").unwrap_or(
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        );
        let today = now.date_naive().and_time(t);
        let next = Local
            .from_local_datetime(&today)
            .single()
            .map(|dt| {
                if dt <= now {
                    dt + chrono::Duration::days(1)
                } else {
                    dt
                }
            })
            .unwrap_or(now + chrono::Duration::days(1));
        next.to_rfc3339()
    } else {
        let mins = interval_minutes.clamp(5, 60 * 24 * 30);
        (now + chrono::Duration::minutes(mins)).to_rfc3339()
    }
}

#[tauri::command]
pub fn scheduler_list(app: AppHandle) -> Result<Vec<ScheduledTask>, String> {
    let conn = open(&app)?;
    let mut stmt = conn
        .prepare("SELECT id,name,task_prompt,provider_id,model_id,schedule_kind,interval_minutes,daily_at,enabled,last_run_at,next_run_at,last_status,last_summary,auto_approve FROM scheduled_tasks ORDER BY created_at")
        .map_err(|e| e.to_string())?;
    let tasks = stmt
        .query_map([], row_to_task)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(tasks)
}

#[derive(Debug, Deserialize)]
pub struct SchedulerCreate {
    pub auto_approve: bool,
    pub name: String,
    pub task_prompt: String,
    pub provider_id: String,
    pub model_id: String,
    pub schedule_kind: String,
    pub interval_minutes: i64,
    pub daily_at: String,
}

#[tauri::command]
pub fn scheduler_create(app: AppHandle, input: SchedulerCreate) -> Result<ScheduledTask, String> {
    if input.name.trim().is_empty() {
        return Err("name is required".to_string());
    }
    if input.task_prompt.trim().is_empty() {
        return Err("task prompt is required".to_string());
    }
    let kind = if input.schedule_kind == "daily" {
        "daily"
    } else {
        "interval"
    };
    let id = uuid::Uuid::new_v4().to_string();
    let next = compute_next_run(kind, input.interval_minutes, &input.daily_at);
    let conn = open(&app)?;
    conn.execute(
        "INSERT INTO scheduled_tasks (id,name,task_prompt,provider_id,model_id,schedule_kind,interval_minutes,daily_at,enabled,next_run_at,auto_approve) VALUES (?,?,?,?,?,?,?,?,1,?,?)",
        params![id, input.name.trim(), input.task_prompt.trim(), input.provider_id.trim(), input.model_id.trim(), kind, input.interval_minutes.clamp(5, 43200), input.daily_at.trim(), next, if input.auto_approve { 1 } else { 0 }],
    ).map_err(|e| e.to_string())?;
    scheduler_list(app.clone())?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| "create failed".to_string())
}

#[tauri::command]
pub fn scheduler_update(
    app: AppHandle,
    id: String,
    input: SchedulerCreate,
) -> Result<ScheduledTask, String> {
    let kind = if input.schedule_kind == "daily" {
        "daily"
    } else {
        "interval"
    };
    let next = compute_next_run(kind, input.interval_minutes, &input.daily_at);
    let conn = open(&app)?;
    let n = conn
        .execute(
            "UPDATE scheduled_tasks SET name=?,task_prompt=?,provider_id=?,model_id=?,schedule_kind=?,interval_minutes=?,daily_at=?,next_run_at=?,auto_approve=? WHERE id=?",
            params![input.name.trim(), input.task_prompt.trim(), input.provider_id.trim(), input.model_id.trim(), kind, input.interval_minutes.clamp(5, 43200), input.daily_at.trim(), next, if input.auto_approve { 1 } else { 0 }, id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("task not found".to_string());
    }
    scheduler_list(app.clone())?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| "update failed".to_string())
}

#[tauri::command]
pub fn scheduler_delete(app: AppHandle, id: String) -> Result<(), String> {
    let conn = open(&app)?;
    conn.execute("DELETE FROM scheduled_tasks WHERE id=?", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn scheduler_set_enabled(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    let conn = open(&app)?;
    if enabled {
        // Recompute next run from now so a re-enabled task fires on schedule,
        // not immediately on a stale timestamp.
        let task: Option<(String, i64, String)> = conn
            .query_row(
                "SELECT schedule_kind,interval_minutes,daily_at FROM scheduled_tasks WHERE id=?",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let next = task
            .map(|(k, m, d)| compute_next_run(&k, m, &d))
            .unwrap_or_else(|| compute_next_run("interval", 60, "08:00"));
        conn.execute(
            "UPDATE scheduled_tasks SET enabled=1,next_run_at=? WHERE id=?",
            params![next, id],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute("UPDATE scheduled_tasks SET enabled=0 WHERE id=?", params![id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Fire a task immediately (emits the due event; does not change its schedule).
#[tauri::command]
pub fn scheduler_run_now(app: AppHandle, id: String) -> Result<(), String> {
    let task = scheduler_list(app.clone())?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| "task not found".to_string())?;
    app.emit("scheduler-task-due", &task)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Called by the frontend after a headless run finishes. Also revokes any
/// session grant so the auto-approve cannot outlive the run.
#[tauri::command]
pub fn scheduler_complete(
    app: AppHandle,
    id: String,
    status: String,
    summary: String,
    session_id: Option<String>,
) -> Result<(), String> {
    if let Some(sid) = session_id {
        revoke_grant(&sid);
    }
    let conn = open(&app)?;
    let now = Local::now().to_rfc3339();
    let summary: String = summary.chars().take(2000).collect();
    conn.execute(
        "UPDATE scheduled_tasks SET last_run_at=?,last_status=?,last_summary=? WHERE id=?",
        params![now, status, summary, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Desktop notification for a finished scheduled run.
#[tauri::command]
pub fn scheduler_notify(app: AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body.chars().take(300).collect::<String>())
        .show()
        .map_err(|e| e.to_string())
}

/// Advance next_run_at after a due task fires so a slow run can't double-fire.
fn advance_after_fire(app: &AppHandle, task: &ScheduledTask) -> Result<(), String> {
    let conn = open(app)?;
    let next = compute_next_run(&task.schedule_kind, task.interval_minutes, &task.daily_at);
    conn.execute(
        "UPDATE scheduled_tasks SET next_run_at=? WHERE id=?",
        params![next, task.id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Background ticker: every 60s, emit due events for enabled tasks.
pub fn start_ticker(app: AppHandle) {
    let guard: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let _lock = guard.lock().unwrap();
            let due: Vec<ScheduledTask> = (|| -> Result<Vec<ScheduledTask>, String> {
                let conn = open(&app)?;
                let now = Local::now().to_rfc3339();
                let mut stmt = conn
                    .prepare("SELECT id,name,task_prompt,provider_id,model_id,schedule_kind,interval_minutes,daily_at,enabled,last_run_at,next_run_at,last_status,last_summary,auto_approve FROM scheduled_tasks WHERE enabled=1 AND next_run_at IS NOT NULL AND next_run_at <= ?")
                    .map_err(|e| e.to_string())?;
                let tasks = stmt
                    .query_map(params![now], row_to_task)
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                Ok(tasks)
            })()
            .unwrap_or_default();
            for task in due {
                // Advance first: a crashed/slow run must not refire the task.
                if advance_after_fire(&app, &task).is_ok() {
                    let _ = app.emit("scheduler-task-due", &task);
                }
            }
        }
    });
}
