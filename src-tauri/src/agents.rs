// Phase 5 — Agent orchestration engine and Builder state machine.
//
// Rust owns: the 14-state builder state machine (transition validation,
// transition log, restart recovery), the DAG task graph store, the human
// approval-gate store (emits `agent://approval` Tauri events), the
// requirements/architecture/ADR store, and the agent roster.
// The frontend (src/components/builder/) owns the orchestration loop and UI.
//
// Reuses the `agents` and `runs` tables from db.rs (Phase 1); adds `tasks`,
// `builder_transitions`, `approvals`, `requirements`, `adrs` tables plus
// `state`/`mode`/`goal` columns on `runs` via an idempotent migration.
//
// NOTE (not verified): this file was written without a Rust toolchain on the
// machine — it has NOT been compiled. See .integration/phase-5.md.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const MIGRATION_PHASE5: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    parent_id       TEXT,
    agent           TEXT NOT NULL,          -- supervisor | architect | frontend | backend | qa | security
    title           TEXT NOT NULL,
    priority        TEXT NOT NULL DEFAULT 'normal',  -- low | normal | high | critical
    status          TEXT NOT NULL DEFAULT 'pending', -- pending | ready | running | blocked | done | failed | skipped
    dependencies    TEXT NOT NULL DEFAULT '[]',      -- JSON array of task ids
    inputs          TEXT NOT NULL DEFAULT '{}',
    outputs         TEXT NOT NULL DEFAULT '{}',
    artifacts       TEXT NOT NULL DEFAULT '[]',
    tool_calls      TEXT NOT NULL DEFAULT '[]',
    errors          TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    started_at      TEXT,
    ended_at        TEXT
);
CREATE INDEX IF NOT EXISTS idx_tasks_run ON tasks(run_id);
CREATE TABLE IF NOT EXISTS builder_transitions (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL,
    from_state  TEXT NOT NULL,
    to_state    TEXT NOT NULL,
    note        TEXT NOT NULL DEFAULT '',
    at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_transitions_run ON builder_transitions(run_id);
CREATE TABLE IF NOT EXISTS approvals (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    kind            TEXT NOT NULL,          -- gate kind or workflow kind ('architecture', 'plan')
    summary         TEXT NOT NULL,
    payload_json    TEXT NOT NULL DEFAULT '{}',
    status          TEXT NOT NULL DEFAULT 'pending',  -- pending | approved | denied | expired
    decision        TEXT,
    reason          TEXT NOT NULL DEFAULT '',
    requested_at    TEXT NOT NULL,
    resolved_at     TEXT
);
CREATE INDEX IF NOT EXISTS idx_approvals_run ON approvals(run_id);
CREATE TABLE IF NOT EXISTS requirements (
    run_id                  TEXT PRIMARY KEY,
    doc_json                TEXT NOT NULL DEFAULT '{}',
    architecture_approved   INTEGER NOT NULL DEFAULT 0,
    updated_at              TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS adrs (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL,
    title       TEXT NOT NULL,
    context     TEXT NOT NULL DEFAULT '',
    decision    TEXT NOT NULL DEFAULT '',
    consequences TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_adrs_run ON adrs(run_id);
"#;

fn db_conn(app: &AppHandle) -> Result<Connection, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let conn = Connection::open(dir.join("guild-foundry-ai.db")).map_err(|e| e.to_string())?;
    conn.execute_batch(MIGRATION_PHASE5)
        .map_err(|e| e.to_string())?;
    // Reuse the Phase 1 `runs` table; add builder columns idempotently.
    let cols: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(runs)")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?
    };
    for (col, ddl) in [
        ("state", "ALTER TABLE runs ADD COLUMN state TEXT NOT NULL DEFAULT 'DISCOVERY'"),
        ("mode", "ALTER TABLE runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'SAFE'"),
        ("goal", "ALTER TABLE runs ADD COLUMN goal TEXT NOT NULL DEFAULT ''"),
    ] {
        if !cols.iter().any(|c| c == col) {
            conn.execute(ddl, []).map_err(|e| e.to_string())?;
        }
    }
    Ok(conn)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

// ---------------------------------------------------------------------------
// Agent roster
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub prompt_key: String,
}

fn agent_roster() -> Vec<AgentInfo> {
    vec![
        AgentInfo {
            id: "supervisor".into(),
            name: "Supervisor".into(),
            kind: "supervisor".into(),
            role: "Owns final coordination: understands requirements, maintains project state, coordinates the other agents, approves state transitions, detects conflicts, enforces architecture.".into(),
            prompt_key: "agent.supervisor".into(),
        },
        AgentInfo {
            id: "architect".into(),
            name: "Architect".into(),
            kind: "architect".into(),
            role: "Designs the system: turns approved requirements into architecture, component boundaries, and ADRs for sign-off.".into(),
            prompt_key: "agent.architect".into(),
        },
        AgentInfo {
            id: "frontend".into(),
            name: "Frontend / Application Engineer".into(),
            kind: "frontend".into(),
            role: "Builds user-facing code: UI, views, styling, and client-side behavior per the architecture.".into(),
            prompt_key: "agent.frontend".into(),
        },
        AgentInfo {
            id: "backend".into(),
            name: "Backend / System Engineer".into(),
            kind: "backend".into(),
            role: "Builds system code: services, data layer, IPC handlers, and native integrations per the architecture.".into(),
            prompt_key: "agent.backend".into(),
        },
        AgentInfo {
            id: "qa".into(),
            name: "QA / Test Engineer".into(),
            kind: "qa".into(),
            role: "Verifies the build: writes and runs tests, reports failures with reproduction detail.".into(),
            prompt_key: "agent.qa".into(),
        },
        AgentInfo {
            id: "security".into(),
            name: "Security Reviewer".into(),
            kind: "security".into(),
            role: "Reviews for risk: scans changes for vulnerabilities, unsafe patterns, and credential exposure; blocks releases on findings.".into(),
            prompt_key: "agent.security".into(),
        },
    ]
}

/// List the six builder agents with role descriptions and prompt keys.
/// Also seeds the Phase 1 `agents` table so other phases can reference them.
#[tauri::command]
pub fn agent_list(app: AppHandle) -> Result<Vec<AgentInfo>, String> {
    let roster = agent_roster();
    let conn = db_conn(&app)?;
    let ts = now();
    for a in &roster {
        conn.execute(
            "INSERT INTO agents (id, name, kind, system_prompt, updated_at)
             VALUES (?1, ?2, ?3, '', ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, kind = excluded.kind, updated_at = excluded.updated_at",
            params![a.id, a.name, a.kind, ts],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(roster)
}

// ---------------------------------------------------------------------------
// Builder state machine
// ---------------------------------------------------------------------------

/// The 14 states in canonical order (frontend rail renders this same order).
pub const BUILDER_STATES: [&str; 14] = [
    "DISCOVERY",
    "REQUIREMENTS_ANALYSIS",
    "ARCHITECTURE",
    "PLANNING",
    "AWAITING_USER_APPROVAL",
    "IMPLEMENTATION",
    "BUILDING",
    "TESTING",
    "SECURITY_REVIEW",
    "FIXING",
    "VALIDATION",
    "READY",
    "PACKAGING",
    "RELEASED",
];

fn allowed_transitions() -> HashMap<&'static str, Vec<&'static str>> {
    let mut m: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    m.insert("DISCOVERY", vec!["REQUIREMENTS_ANALYSIS"]);
    m.insert("REQUIREMENTS_ANALYSIS", vec!["ARCHITECTURE", "DISCOVERY"]);
    m.insert("ARCHITECTURE", vec!["PLANNING", "REQUIREMENTS_ANALYSIS"]);
    m.insert("PLANNING", vec!["AWAITING_USER_APPROVAL", "ARCHITECTURE"]);
    m.insert("AWAITING_USER_APPROVAL", vec!["IMPLEMENTATION", "PLANNING"]);
    m.insert("IMPLEMENTATION", vec!["BUILDING", "PLANNING"]);
    m.insert("BUILDING", vec!["TESTING", "FIXING"]);
    m.insert("TESTING", vec!["SECURITY_REVIEW", "FIXING"]);
    m.insert("SECURITY_REVIEW", vec!["VALIDATION", "FIXING"]);
    m.insert("FIXING", vec!["IMPLEMENTATION", "BUILDING", "TESTING"]);
    m.insert("VALIDATION", vec!["READY", "FIXING"]);
    m.insert("READY", vec!["PACKAGING"]);
    m.insert("PACKAGING", vec!["RELEASED", "FIXING"]);
    m.insert("RELEASED", vec![]);
    m
}

fn validate_mode(mode: &str) -> Result<(), String> {
    match mode {
        "SAFE" | "ASSISTED" | "AUTONOMOUS" => Ok(()),
        _ => Err(format!(
            "invalid mode '{}': expected SAFE | ASSISTED | AUTONOMOUS",
            mode
        )),
    }
}

fn record_transition(
    conn: &Connection,
    run_id: &str,
    from_state: &str,
    to_state: &str,
    note: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO builder_transitions (id, run_id, from_state, to_state, note, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![new_id(), run_id, from_state, to_state, note, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
pub struct RunSummary {
    pub id: String,
    pub kind: String,
    pub goal: String,
    pub mode: String,
    pub state: String,
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: String,
    pub task_counts: HashMap<String, i64>,
    pub pending_approvals: i64,
}

/// Start a builder run. Returns the run id. Always begins in DISCOVERY.
#[tauri::command]
pub fn run_start(app: AppHandle, goal: String, mode: String) -> Result<String, String> {
    validate_mode(&mode)?;
    let goal = goal.trim().to_string();
    if goal.is_empty() {
        return Err("goal must not be empty".to_string());
    }
    let conn = db_conn(&app)?;
    let run_id = new_id();
    conn.execute(
        "INSERT INTO runs (id, kind, goal, mode, state, status, started_at, summary)
         VALUES (?1, 'builder', ?2, ?3, 'DISCOVERY', 'running', ?4, '')",
        params![run_id, goal, mode, now()],
    )
    .map_err(|e| e.to_string())?;
    record_transition(&conn, &run_id, "NONE", "DISCOVERY", "run started")?;
    Ok(run_id)
}

#[derive(Serialize)]
pub struct TransitionRow {
    pub id: String,
    pub from_state: String,
    pub to_state: String,
    pub note: String,
    pub at: String,
}

fn run_row_to_summary(conn: &Connection, run_id: &str) -> Result<RunSummary, String> {
    let (id, kind, goal, mode, state, status, started_at, ended_at, summary): (
        String, String, String, String, String, String, String, Option<String>, String,
    ) = conn
        .query_row(
            "SELECT id, kind, goal, mode, state, status, started_at, ended_at, summary
             FROM runs WHERE id = ?1",
            params![run_id],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?,
                ))
            },
        )
        .map_err(|_| format!("run not found: {}", run_id))?;

    let mut task_counts: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT status, COUNT(*) FROM tasks WHERE run_id = ?1 GROUP BY status")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![run_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            let (s, c) = r.map_err(|e| e.to_string())?;
            task_counts.insert(s, c);
        }
    }
    let pending_approvals: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM approvals WHERE run_id = ?1 AND status = 'pending'",
            params![run_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(RunSummary {
        id, kind, goal, mode, state, status, started_at, ended_at, summary,
        task_counts, pending_approvals,
    })
}

/// Current run state plus task counts and pending approvals.
#[tauri::command]
pub fn run_status(app: AppHandle, run_id: String) -> Result<RunSummary, String> {
    let conn = db_conn(&app)?;
    run_row_to_summary(&conn, &run_id)
}

/// Stop a run. No further transitions or task updates are accepted afterwards.
#[tauri::command]
pub fn run_cancel(app: AppHandle, run_id: String) -> Result<(), String> {
    let conn = db_conn(&app)?;
    let status: String = conn
        .query_row("SELECT status FROM runs WHERE id = ?1", params![run_id], |row| {
            row.get(0)
        })
        .map_err(|_| format!("run not found: {}", run_id))?;
    if status != "running" {
        return Err(format!("run {} is not running (status: {})", run_id, status));
    }
    conn.execute(
        "UPDATE runs SET status = 'stopped', ended_at = ?1 WHERE id = ?2",
        params![now(), run_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Move a run to another state. Illegal transitions are rejected; every legal
/// transition is recorded with a timestamp.
#[tauri::command]
pub fn run_transition(
    app: AppHandle,
    run_id: String,
    to_state: String,
    note: Option<String>,
) -> Result<(), String> {
    if !BUILDER_STATES.contains(&to_state.as_str()) {
        return Err(format!("unknown builder state '{}'", to_state));
    }
    let conn = db_conn(&app)?;
    let (state, status): (String, String) = conn
        .query_row(
            "SELECT state, status FROM runs WHERE id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| format!("run not found: {}", run_id))?;
    if status != "running" {
        return Err(format!(
            "run {} is not running (status: {}); transitions are closed",
            run_id, status
        ));
    }
    let allowed = allowed_transitions();
    let nexts: &[&str] = allowed.get(state.as_str()).map(|v| v.as_slice()).unwrap_or(&[]);
    if !nexts.contains(&to_state.as_str()) {
        return Err(format!(
            "illegal transition: {} -> {} (allowed: {})",
            state,
            to_state,
            nexts.join(", ")
        ));
    }
    let note = note.unwrap_or_default();
    record_transition(&conn, &run_id, &state, &to_state, &note)?;
    if to_state == "RELEASED" {
        conn.execute(
            "UPDATE runs SET state = ?1, status = 'completed', ended_at = ?2 WHERE id = ?3",
            params![to_state, now(), run_id],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "UPDATE runs SET state = ?1 WHERE id = ?2",
            params![to_state, run_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Runs that were still `running` when the app last exited — recoverable after
/// restart. The frontend resumes the orchestration loop from the stored state.
#[tauri::command]
pub fn run_recover(app: AppHandle) -> Result<Vec<RunSummary>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare("SELECT id FROM runs WHERE kind = 'builder' AND status = 'running' ORDER BY started_at DESC")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for id in ids {
        out.push(run_row_to_summary(&conn, &id)?);
    }
    Ok(out)
}

/// Transition log for a run (audit trail).
#[tauri::command]
pub fn run_transitions(app: AppHandle, run_id: String) -> Result<Vec<TransitionRow>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, from_state, to_state, note, at FROM builder_transitions
             WHERE run_id = ?1 ORDER BY at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(TransitionRow {
                id: row.get(0)?,
                from_state: row.get(1)?,
                to_state: row.get(2)?,
                note: row.get(3)?,
                at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}

/// Recent builder runs, newest first.
#[tauri::command]
pub fn run_history_list(app: AppHandle, limit: Option<i64>) -> Result<Vec<RunSummary>, String> {
    let conn = db_conn(&app)?;
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let mut stmt = conn
        .prepare("SELECT id FROM runs WHERE kind = 'builder' ORDER BY started_at DESC LIMIT ?1")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map(params![limit], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for id in ids {
        out.push(run_row_to_summary(&conn, &id)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// DAG task graph
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct TaskRow {
    pub id: String,
    pub run_id: String,
    pub parent_id: Option<String>,
    pub agent: String,
    pub title: String,
    pub priority: String,
    pub status: String,
    pub dependencies: String,
    pub inputs: String,
    pub outputs: String,
    pub artifacts: String,
    pub tool_calls: String,
    pub errors: String,
    pub created_at: String,
    pub updated_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
}

fn assert_run_open(conn: &Connection, run_id: &str) -> Result<(), String> {
    let status: String = conn
        .query_row("SELECT status FROM runs WHERE id = ?1", params![run_id], |row| {
            row.get(0)
        })
        .map_err(|_| format!("run not found: {}", run_id))?;
    if status != "running" {
        return Err(format!("run {} is closed (status: {})", run_id, status));
    }
    Ok(())
}

#[tauri::command]
pub fn agent_task_create(
    app: AppHandle,
    run_id: String,
    agent: String,
    title: String,
    parent_id: Option<String>,
    priority: Option<String>,
    dependencies: Option<String>,
    inputs: Option<String>,
) -> Result<String, String> {
    let conn = db_conn(&app)?;
    assert_run_open(&conn, &run_id)?;
    let valid_agents = ["supervisor", "architect", "frontend", "backend", "qa", "security"];
    if !valid_agents.contains(&agent.as_str()) {
        return Err(format!("unknown agent '{}'", agent));
    }
    let task_id = new_id();
    let ts = now();
    conn.execute(
        "INSERT INTO tasks (id, run_id, parent_id, agent, title, priority, status,
                            dependencies, inputs, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9, ?9)",
        params![
            task_id,
            run_id,
            parent_id,
            agent,
            title,
            priority.unwrap_or_else(|| "normal".into()),
            dependencies.unwrap_or_else(|| "[]".into()),
            inputs.unwrap_or_else(|| "{}".into()),
            ts
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(task_id)
}

#[tauri::command]
pub fn agent_task_update(
    app: AppHandle,
    task_id: String,
    status: Option<String>,
    outputs: Option<String>,
    artifacts: Option<String>,
    tool_calls: Option<String>,
    error: Option<String>,
) -> Result<(), String> {
    let conn = db_conn(&app)?;
    let run_id: String = conn
        .query_row("SELECT run_id FROM tasks WHERE id = ?1", params![task_id], |row| {
            row.get(0)
        })
        .map_err(|_| format!("task not found: {}", task_id))?;
    assert_run_open(&conn, &run_id)?;
    if let Some(s) = &status {
        let valid = ["pending", "ready", "running", "blocked", "done", "failed", "skipped"];
        if !valid.contains(&s.as_str()) {
            return Err(format!("invalid task status '{}'", s));
        }
    }
    let ts = now();
    conn.execute(
        "UPDATE tasks SET
            status = COALESCE(?2, status),
            outputs = COALESCE(?3, outputs),
            artifacts = COALESCE(?4, artifacts),
            tool_calls = COALESCE(?5, tool_calls),
            errors = CASE WHEN ?6 IS NOT NULL
                          THEN errors || ?6 ELSE errors END,
            started_at = CASE WHEN ?2 = 'running' AND started_at IS NULL THEN ?7 ELSE started_at END,
            ended_at = CASE WHEN ?2 IN ('done','failed','skipped') THEN ?7 ELSE ended_at END,
            updated_at = ?7
         WHERE id = ?1",
        params![
            task_id,
            status,
            outputs,
            artifacts,
            tool_calls,
            error.map(|e| format!("[{}] {}\n", ts, e)),
            ts
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn agent_task_get(app: AppHandle, task_id: String) -> Result<TaskRow, String> {
    let conn = db_conn(&app)?;
    task_row_by_id(&conn, &task_id)
}

fn task_row_by_id(conn: &Connection, task_id: &str) -> Result<TaskRow, String> {
    conn.query_row(
        "SELECT id, run_id, parent_id, agent, title, priority, status, dependencies,
                inputs, outputs, artifacts, tool_calls, errors,
                created_at, updated_at, started_at, ended_at
         FROM tasks WHERE id = ?1",
        params![task_id],
        |row| {
            Ok(TaskRow {
                id: row.get(0)?, run_id: row.get(1)?, parent_id: row.get(2)?,
                agent: row.get(3)?, title: row.get(4)?, priority: row.get(5)?,
                status: row.get(6)?, dependencies: row.get(7)?, inputs: row.get(8)?,
                outputs: row.get(9)?, artifacts: row.get(10)?, tool_calls: row.get(11)?,
                errors: row.get(12)?, created_at: row.get(13)?, updated_at: row.get(14)?,
                started_at: row.get(15)?, ended_at: row.get(16)?,
            })
        },
    )
    .map_err(|_| format!("task not found: {}", task_id))
}

#[tauri::command]
pub fn agent_task_list(app: AppHandle, run_id: String) -> Result<Vec<TaskRow>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, parent_id, agent, title, priority, status, dependencies,
                    inputs, outputs, artifacts, tool_calls, errors,
                    created_at, updated_at, started_at, ended_at
             FROM tasks WHERE run_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(TaskRow {
                id: row.get(0)?, run_id: row.get(1)?, parent_id: row.get(2)?,
                agent: row.get(3)?, title: row.get(4)?, priority: row.get(5)?,
                status: row.get(6)?, dependencies: row.get(7)?, inputs: row.get(8)?,
                outputs: row.get(9)?, artifacts: row.get(10)?, tool_calls: row.get(11)?,
                errors: row.get(12)?, created_at: row.get(13)?, updated_at: row.get(14)?,
                started_at: row.get(15)?, ended_at: row.get(16)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Human approval gates
// ---------------------------------------------------------------------------

/// The nine tool-use gate kinds from master spec section 26, plus workflow
/// kinds ('architecture', 'plan') used for builder sign-offs.
const GATE_KINDS: [&str; 9] = [
    "deletes",
    "mass_replaces",
    "system_packages",
    "privileged_commands",
    "credentials",
    "pushes",
    "releases",
    "outside_project_writes",
    "destructive_commands",
];

/// High-risk gates: fire even in ASSISTED mode.
const HIGH_RISK_KINDS: [&str; 6] = [
    "deletes",
    "privileged_commands",
    "credentials",
    "pushes",
    "releases",
    "destructive_commands",
];

/// Whether an approval gate of `kind` fires under `mode`.
/// SAFE: every gate fires. ASSISTED: high-risk gates fire.
/// AUTONOMOUS: only credentials / pushes / releases fire.
/// Workflow kinds ('architecture', 'plan') always require a human decision.
#[tauri::command]
pub fn approval_gate_check(kind: String, mode: String) -> Result<bool, String> {
    validate_mode(&mode)?;
    if kind == "architecture" || kind == "plan" {
        return Ok(true);
    }
    if !GATE_KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown approval kind '{}'", kind));
    }
    Ok(match mode {
        m if m == "SAFE" => true,
        m if m == "ASSISTED" => HIGH_RISK_KINDS.contains(&kind.as_str()),
        _ => matches!(kind.as_str(), "credentials" | "pushes" | "releases"),
    })
}

#[derive(Clone, Serialize)]
pub struct ApprovalPayload {
    pub approval_id: String,
    pub run_id: String,
    pub kind: String,
    pub summary: String,
    pub payload_json: String,
    pub requested_at: String,
}

/// Request human approval. Emits the `agent://approval` Tauri event so the
/// frontend can pop the ApprovalGate modal. Returns the approval id.
#[tauri::command]
pub fn approval_request(
    app: AppHandle,
    run_id: String,
    kind: String,
    summary: String,
    payload_json: Option<String>,
) -> Result<String, String> {
    let conn = db_conn(&app)?;
    assert_run_open(&conn, &run_id)?;
    let approval_id = new_id();
    let requested_at = now();
    let payload_json = payload_json.unwrap_or_else(|| "{}".into());
    conn.execute(
        "INSERT INTO approvals (id, run_id, kind, summary, payload_json, status, requested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
        params![approval_id, run_id, kind, summary, payload_json, requested_at],
    )
    .map_err(|e| e.to_string())?;
    app.emit(
        "agent://approval",
        ApprovalPayload {
            approval_id: approval_id.clone(),
            run_id,
            kind,
            summary,
            payload_json,
            requested_at,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(approval_id)
}

#[derive(Serialize)]
pub struct ApprovalRow {
    pub id: String,
    pub run_id: String,
    pub kind: String,
    pub summary: String,
    pub payload_json: String,
    pub status: String,
    pub decision: Option<String>,
    pub reason: String,
    pub requested_at: String,
    pub resolved_at: Option<String>,
}

/// Resolve a pending approval. Emits `agent://approval-resolved` so any
/// waiting orchestration loop can continue.
#[tauri::command]
pub fn approval_resolve(
    app: AppHandle,
    approval_id: String,
    decision: String,
    reason: Option<String>,
) -> Result<(), String> {
    if decision != "approved" && decision != "denied" {
        return Err("decision must be 'approved' or 'denied'".to_string());
    }
    let conn = db_conn(&app)?;
    let status: String = conn
        .query_row(
            "SELECT status FROM approvals WHERE id = ?1",
            params![approval_id],
            |row| row.get(0),
        )
        .map_err(|_| format!("approval not found: {}", approval_id))?;
    if status != "pending" {
        return Err(format!("approval {} is already {}", approval_id, status));
    }
    conn.execute(
        "UPDATE approvals SET status = ?2, decision = ?2, reason = ?3, resolved_at = ?4 WHERE id = ?1",
        params![approval_id, decision, reason.unwrap_or_default(), now()],
    )
    .map_err(|e| e.to_string())?;
    app.emit(
        "agent://approval-resolved",
        serde_json::json!({ "approval_id": approval_id, "decision": decision }),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn approval_list(app: AppHandle, run_id: String) -> Result<Vec<ApprovalRow>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, kind, summary, payload_json, status, decision, reason,
                    requested_at, resolved_at
             FROM approvals WHERE run_id = ?1 ORDER BY requested_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(ApprovalRow {
                id: row.get(0)?, run_id: row.get(1)?, kind: row.get(2)?,
                summary: row.get(3)?, payload_json: row.get(4)?, status: row.get(5)?,
                decision: row.get(6)?, reason: row.get(7)?,
                requested_at: row.get(8)?, resolved_at: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Requirements engine + architecture approval + ADRs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct RequirementsDoc {
    pub run_id: String,
    pub doc_json: String,
    pub architecture_approved: bool,
    pub updated_at: String,
}

#[tauri::command]
pub fn requirements_get(app: AppHandle, run_id: String) -> Result<RequirementsDoc, String> {
    let conn = db_conn(&app)?;
    conn.query_row(
        "SELECT run_id, doc_json, architecture_approved, updated_at FROM requirements WHERE run_id = ?1",
        params![run_id],
        |row| {
            Ok(RequirementsDoc {
                run_id: row.get(0)?,
                doc_json: row.get(1)?,
                architecture_approved: row.get::<_, i64>(2)? != 0,
                updated_at: row.get(3)?,
            })
        },
    )
    .map_err(|_| format!("no requirements recorded for run {}", run_id))
}

#[tauri::command]
pub fn requirements_upsert(
    app: AppHandle,
    run_id: String,
    doc_json: String,
) -> Result<(), String> {
    let conn = db_conn(&app)?;
    assert_run_open(&conn, &run_id)?;
    // Reject obviously invalid JSON so downstream consumers can parse safely.
    let _: serde_json::Value =
        serde_json::from_str(&doc_json).map_err(|e| format!("doc_json is not valid JSON: {}", e))?;
    conn.execute(
        "INSERT INTO requirements (run_id, doc_json, architecture_approved, updated_at)
         VALUES (?1, ?2, 0, ?3)
         ON CONFLICT(run_id) DO UPDATE SET doc_json = excluded.doc_json,
                                           architecture_approved = 0,
                                           updated_at = excluded.updated_at",
        params![run_id, doc_json, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Record the human architecture sign-off (used by the RequirementsPanel and
/// the orchestration loop before leaving ARCHITECTURE).
#[tauri::command]
pub fn requirements_signoff(app: AppHandle, run_id: String) -> Result<(), String> {
    let conn = db_conn(&app)?;
    assert_run_open(&conn, &run_id)?;
    let changed = conn
        .execute(
            "UPDATE requirements SET architecture_approved = 1, updated_at = ?2 WHERE run_id = ?1",
            params![run_id, now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("no requirements recorded for run {}", run_id));
    }
    Ok(())
}

#[derive(Serialize)]
pub struct AdrRow {
    pub id: String,
    pub run_id: String,
    pub title: String,
    pub context: String,
    pub decision: String,
    pub consequences: String,
    pub created_at: String,
}

#[tauri::command]
pub fn adr_create(
    app: AppHandle,
    run_id: String,
    title: String,
    context: Option<String>,
    decision: Option<String>,
    consequences: Option<String>,
) -> Result<String, String> {
    let conn = db_conn(&app)?;
    assert_run_open(&conn, &run_id)?;
    let id = new_id();
    conn.execute(
        "INSERT INTO adrs (id, run_id, title, context, decision, consequences, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id, run_id, title,
            context.unwrap_or_default(),
            decision.unwrap_or_default(),
            consequences.unwrap_or_default(),
            now()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn adr_list(app: AppHandle, run_id: String) -> Result<Vec<AdrRow>, String> {
    let conn = db_conn(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, run_id, title, context, decision, consequences, created_at
             FROM adrs WHERE run_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![run_id], |row| {
            Ok(AdrRow {
                id: row.get(0)?, run_id: row.get(1)?, title: row.get(2)?,
                context: row.get(3)?, decision: row.get(4)?,
                consequences: row.get(5)?, created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())
}
