// Phase 1 — SQLite schema + migrations (master spec section 53).
//
// Entities: projects, conversations, messages, agents, models, providers,
// tools, mcp_servers, permissions, memories, artifacts, runs, checkpoints,
// builds, tests, settings. Migrations are additive and versioned; the schema
// is never destructively changed without a migration step.

use rusqlite::Connection;
use std::path::PathBuf;
use tauri::Manager;

const CURRENT_SCHEMA_VERSION: i64 = 2;

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    root_path   TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS conversations (
    id              TEXT PRIMARY KEY,
    project_id      TEXT REFERENCES projects(id),
    workspace       TEXT NOT NULL,          -- 'builder' | 'conference'
    title           TEXT NOT NULL DEFAULT '',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    parent_id       TEXT REFERENCES messages(id),
    role            TEXT NOT NULL,          -- user | assistant | system | winner
    content         TEXT NOT NULL DEFAULT '',
    model_id        TEXT,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id);
CREATE TABLE IF NOT EXISTS agents (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,          -- supervisor | architect | frontend | backend | qa | security | conference
    system_prompt   TEXT NOT NULL DEFAULT '',
    provider_id     TEXT,
    model_id        TEXT,
    updated_at      TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS providers (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    base_url    TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS models (
    id              TEXT PRIMARY KEY,       -- provider_id + ':' + model id
    provider_id     TEXT NOT NULL REFERENCES providers(id),
    model_id        TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    pricing_class   TEXT NOT NULL DEFAULT 'unknown',  -- free | paid | unknown
    context_window  INTEGER,
    capabilities    TEXT NOT NULL DEFAULT '{}',
    favorite        INTEGER NOT NULL DEFAULT 0,
    hidden          INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS tools (
    name                TEXT PRIMARY KEY,
    description         TEXT NOT NULL DEFAULT '',
    provider            TEXT,
    mcp_server          TEXT,
    risk_class          TEXT NOT NULL DEFAULT 'moderate',
    enabled             INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS mcp_servers (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    transport   TEXT NOT NULL,              -- streamable_http | sse | stdio
    url         TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS permissions (
    agent_id    TEXT NOT NULL,
    tool        TEXT NOT NULL,
    domain      TEXT NOT NULL,
    mode        TEXT NOT NULL,              -- allow_always | ask | allow_project | allow_session | deny
    PRIMARY KEY (agent_id, tool, domain)
);
CREATE TABLE IF NOT EXISTS memories (
    provider_id TEXT NOT NULL,
    model_id    TEXT NOT NULL,
    version     TEXT NOT NULL DEFAULT '',
    scope       TEXT NOT NULL,              -- conversation | model | agent | project | workspace | global
    scope_id    TEXT,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    importance  REAL NOT NULL DEFAULT 0.5,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (provider_id, model_id, version, scope, key)
);
CREATE TABLE IF NOT EXISTS artifacts (
    id          TEXT PRIMARY KEY,
    project_id  TEXT REFERENCES projects(id),
    kind        TEXT NOT NULL,
    source      TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1,
    path        TEXT,
    sha256      TEXT,
    created_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    id              TEXT PRIMARY KEY,
    kind            TEXT NOT NULL,          -- conversation | builder | tool | build | test
    project_id      TEXT REFERENCES projects(id),
    agent_id        TEXT,
    model_id        TEXT,
    status          TEXT NOT NULL,          -- running | completed | failed | stopped
    started_at      TEXT NOT NULL,
    ended_at        TEXT,
    summary         TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS checkpoints (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id),
    agent_id    TEXT,
    label       TEXT NOT NULL,
    state_json  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS builds (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id),
    status      TEXT NOT NULL,
    log_path    TEXT,
    created_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tests (
    id          TEXT PRIMARY KEY,
    build_id    TEXT REFERENCES builds(id),
    name        TEXT NOT NULL,
    status      TEXT NOT NULL,
    detail      TEXT NOT NULL DEFAULT ''
);
"#;

fn apply_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version     INTEGER PRIMARY KEY,
            applied_at  TEXT NOT NULL
        )",
    )?;
    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if current < 1 {
        conn.execute_batch(MIGRATION_V1)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (1, datetime('now'))",
            [],
        )?;
    }
    // Phase 8: releases table (additive; build.rs also applies it defensively).
    if current < 2 {
        conn.execute_batch(crate::build::MIGRATION_BUILD_V2)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (2, datetime('now'))",
            [],
        )?;
    }
    let _ = CURRENT_SCHEMA_VERSION;
    Ok(())
}

/// Open (creating if needed) the application database and run migrations.
/// Returns the database path. Called once during app setup.
pub fn init_db(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let db_path = dir.join("guild-foundry-ai.db");
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    apply_migrations(&conn).map_err(|e| e.to_string())?;
    Ok(db_path)
}
