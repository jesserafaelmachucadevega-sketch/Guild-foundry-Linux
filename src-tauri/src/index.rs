// SPDX-License-Identifier: Apache-2.0
// Phase 10 — Project indexing (master spec: memory/indexing/research).
//
// What is indexed per project: filenames, code symbols (fn/def/class via
// regex), text/doc/code content snippets, and Git metadata (best-effort
// `git log --oneline -5`). Search is full-text (LIKE) + filename + symbol
// ranking. A `notify` 8 file watcher detects external changes and emits
// `index://changed` events ({path, kind}) with 500ms per-path debounce.
//
// NOT VERIFIED / honest limitations:
//   - Rust cannot compile in this build environment; all of this is
//     NOT VERIFIED until `cargo build` runs.
//   - Symbol extraction is regex-based, not a real parser; exotic syntax,
//     macros, and nested definitions can be missed or mislabeled.
//   - "Semantic" search from the master spec is NOT implemented: there is no
//     offline embedding model in this build, so search is keyword-based.
//   - The watcher uses std mpsc + a tokio task bridge; recursive watching of
//     huge trees (node_modules etc.) is skipped by directory filter.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager};

use crate::memory::open_db;

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Additive Phase 10 tables for the code index. Idempotent.
fn migrate_index(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS index_entries (
            id              TEXT PRIMARY KEY,
            project_root    TEXT NOT NULL,
            path            TEXT NOT NULL,
            kind            TEXT NOT NULL,  -- filename | symbol | text | git
            name            TEXT NOT NULL,
            snippet         TEXT NOT NULL DEFAULT '',
            lang            TEXT NOT NULL DEFAULT '',
            indexed_at      TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_index_entries_root ON index_entries(project_root);
        CREATE INDEX IF NOT EXISTS idx_index_entries_name ON index_entries(name);
        CREATE INDEX IF NOT EXISTS idx_index_entries_kind ON index_entries(kind);
        CREATE TABLE IF NOT EXISTS index_projects (
            project_root    TEXT PRIMARY KEY,
            file_count      INTEGER NOT NULL DEFAULT 0,
            symbol_count    INTEGER NOT NULL DEFAULT 0,
            git_head        TEXT NOT NULL DEFAULT '',
            indexed_at      TEXT NOT NULL
        );",
    )
    .map_err(|e| e.to_string())
}

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", ".next", "__pycache__",
    ".venv", "venv", "out", "coverage", ".cache", ".idea", ".vscode",
];

fn lang_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "java" | "kt" | "scala" => Some("jvm"),
        "c" | "h" | "cpp" | "hpp" | "cc" => Some("c"),
        "cs" => Some("csharp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "swift" => Some("swift"),
        "md" | "markdown" | "txt" | "rst" | "toml" | "yaml" | "yml" | "json" => Some("text"),
        _ => None,
    }
}

fn is_text_ext(ext: &str) -> bool {
    lang_for_ext(ext).is_some()
}

/// Best-effort: first 5 commits. Returns (head_hash, oneline_log).
fn git_metadata(root: &str) -> (String, String) {
    let out = std::process::Command::new("git")
        .args(["-C", root, "log", "--oneline", "-5"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let log = String::from_utf8_lossy(&o.stdout).to_string();
            let head = log.lines().next().unwrap_or("").to_string();
            (head, log)
        }
        _ => (String::new(), String::new()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub project_root: String,
    pub files: i64,
    pub symbols: i64,
    pub entries: i64,
    pub git_head: String,
    pub indexed_at: String,
}

struct CollectedEntry {
    id: String,
    path: String,
    kind: &'static str,
    name: String,
    snippet: String,
    lang: String,
}

fn collect_entries(root: &str) -> Result<(Vec<CollectedEntry>, i64, i64), String> {
    let symbol_re = regex::Regex::new(
        r"(?m)^\s*(?:pub\s+(?:async\s+)?)?(?:fn|def|class)\s+([A-Za-z_][A-Za-z0-9_]*)",
    )
    .map_err(|e| e.to_string())?;
    let mut entries: Vec<CollectedEntry> = Vec::new();
    let mut file_count: i64 = 0;
    let mut symbol_count: i64 = 0;

    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                !SKIP_DIRS.iter().any(|d| name == *d)
            } else {
                true
            }
        });

    for dent in walker {
        let dent = match dent {
            Ok(d) => d,
            Err(_) => continue,
        };
        if !dent.file_type().is_file() {
            continue;
        }
        let path = dent.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // 1. Filename entry (always).
        entries.push(CollectedEntry {
            id: uuid::Uuid::new_v4().to_string(),
            path: rel.clone(),
            kind: "filename",
            name: file_name.clone(),
            snippet: rel.clone(),
            lang: String::new(),
        });

        // 2./3. Symbols + text snippet for text/code files under 1 MiB.
        let meta = match std::fs::metadata(path) {
            Ok(m) if m.len() <= 1_048_576 => m,
            _ => continue,
        };
        let _ = meta;
        if !is_text_ext(&ext) {
            continue;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        file_count += 1;
        let lang = lang_for_ext(&ext).unwrap_or("text").to_string();

        for cap in symbol_re.captures_iter(&content) {
            let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            symbol_count += 1;
            entries.push(CollectedEntry {
                id: uuid::Uuid::new_v4().to_string(),
                path: rel.clone(),
                kind: "symbol",
                name,
                snippet: rel.clone(),
                lang: lang.clone(),
            });
        }

        let head: String = content.chars().take(300).collect();
        entries.push(CollectedEntry {
            id: uuid::Uuid::new_v4().to_string(),
            path: rel.clone(),
            kind: "text",
            name: file_name,
            snippet: head.replace('\n', " ").trim().to_string(),
            lang,
        });
    }
    Ok((entries, file_count, symbol_count))
}

/// Rebuild the index for a project root. Runs the walk on a blocking thread;
/// the command itself is async so the UI stays responsive.
#[tauri::command]
pub async fn index_rebuild(
    app: tauri::AppHandle,
    project_root: String,
) -> Result<IndexStats, String> {
    let root: PathBuf = project_root.clone().into();
    if !root.is_dir() {
        return Err(format!("refused: '{}' is not a directory", project_root));
    }
    let root_str = root.to_string_lossy().to_string();
    let (entries, file_count, symbol_count) =
        tokio::task::spawn_blocking(move || collect_entries(&root_str))
            .await
            .map_err(|e| e.to_string())??;
    let (git_head, git_log) = tokio::task::spawn_blocking({
        let r = project_root.clone();
        move || git_metadata(&r)
    })
    .await
    .map_err(|e| e.to_string())?;

    let conn = open_db(&app)?;
    migrate_index(&conn)?;
    let now = now_iso();
    conn.execute(
        "DELETE FROM index_entries WHERE project_root = ?1",
        params![project_root],
    )
    .map_err(|e| e.to_string())?;
    {
        let mut stmt = conn
            .prepare(
                "INSERT INTO index_entries
                    (id, project_root, path, kind, name, snippet, lang, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|e| e.to_string())?;
        for e in &entries {
            let _ = stmt.execute(params![
                e.id, project_root, e.path, e.kind, e.name, e.snippet, e.lang, now
            ]);
        }
    }
    if !git_head.is_empty() {
        let _ = conn.execute(
            "INSERT INTO index_entries
                (id, project_root, path, kind, name, snippet, lang, indexed_at)
             VALUES (?1, ?2, '.git', 'git', ?3, ?4, '', ?5)",
            params![
                uuid::Uuid::new_v4().to_string(),
                project_root,
                format!("git:{}", git_head),
                git_log,
                now
            ],
        );
    }
    let entry_count = entries.len() as i64;
    conn.execute(
        "INSERT INTO index_projects
            (project_root, file_count, symbol_count, git_head, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(project_root) DO UPDATE SET
            file_count = excluded.file_count,
            symbol_count = excluded.symbol_count,
            git_head = excluded.git_head,
            indexed_at = excluded.indexed_at",
        params![project_root, file_count, symbol_count, git_head, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(IndexStats {
        project_root,
        files: file_count,
        symbols: symbol_count,
        entries: entry_count,
        git_head,
        indexed_at: now,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexHit {
    pub path: String,
    pub kind: String,
    pub name: String,
    pub snippet: String,
    pub score: f64,
}

/// Ranked search: exact filename > symbol > text content. `project_root` is
/// optional; when omitted, all indexed projects are searched.
#[tauri::command]
pub fn index_search(
    app: tauri::AppHandle,
    query: String,
    project_root: Option<String>,
) -> Result<Vec<IndexHit>, String> {
    let conn = open_db(&app)?;
    migrate_index(&conn)?;
    let q = query.trim().to_lowercase();
    if q.len() < 2 {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let sql = if project_root.is_some() {
        "SELECT path, kind, name, snippet FROM index_entries
         WHERE project_root = ?2 AND (LOWER(name) LIKE ?1 ESCAPE '\\'
            OR LOWER(snippet) LIKE ?1 ESCAPE '\\') LIMIT 300"
    } else {
        "SELECT path, kind, name, snippet FROM index_entries
         WHERE LOWER(name) LIKE ?1 ESCAPE '\\' OR LOWER(snippet) LIKE ?1 ESCAPE '\\'
         LIMIT 300"
    };
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<(String, String, String, String)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    };
    let rows: Vec<(String, String, String, String)> = match project_root {
        Some(root) => stmt
            .query_map(params![like, root], map_row)
            .map_err(|e| e.to_string())?,
        None => stmt
            .query_map(params![like], map_row)
            .map_err(|e| e.to_string())?,
    }
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| e.to_string())?;

    let mut hits: Vec<IndexHit> = Vec::new();
    for (path, kind, name, snippet) in rows {
        let lname = name.to_lowercase();
        let score = if lname == q {
            100.0
        } else if kind == "symbol" && lname.starts_with(&q) {
            80.0
        } else if kind == "filename" && lname.contains(&q) {
            70.0
        } else if kind == "symbol" {
            50.0
        } else if kind == "git" {
            20.0
        } else {
            30.0
        };
        hits.push(IndexHit {
            path,
            kind,
            name,
            snippet,
            score,
        });
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(60);
    Ok(hits)
}

// ---------------------------------------------------------------------------
// File watcher (notify 8): external-change detection with 500ms debounce.
// ---------------------------------------------------------------------------

static WATCHERS: OnceLock<Mutex<HashMap<String, notify::RecommendedWatcher>>> = OnceLock::new();

fn watchers() -> &'static Mutex<HashMap<String, notify::RecommendedWatcher>> {
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize)]
struct IndexChangeEvent {
    path: String,
    kind: String,
}

/// Start watching a project root. Emits `index://changed` {path, kind}
/// whenever a file is created/modified/removed by an EXTERNAL process.
/// Debounced: at most one event per path per 500ms.
#[tauri::command]
pub fn index_watch_start(
    app: tauri::AppHandle,
    project_root: String,
) -> Result<(), String> {
    if watchers().lock().map_err(|e| e.to_string())?.contains_key(&project_root) {
        return Ok(()); // already watching
    }
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher =
        notify::RecommendedWatcher::new(tx, notify::Config::default()).map_err(|e| e.to_string())?;
    watcher
        .watch(
            std::path::Path::new(&project_root),
            notify::RecursiveMode::Recursive,
        )
        .map_err(|e| e.to_string())?;

    let app_handle = app.clone();
    // Bridge the blocking notify receiver into an async emitter:
    // a dedicated OS thread owns `rx`, debounces, and forwards events
    // through a tokio channel. The thread ends when the app exits.
    let (fwd_tx, mut fwd_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    std::thread::spawn(move || {
        let mut last_emit: HashMap<String, std::time::Instant> = HashMap::new();
        while let Ok(evt) = rx.recv() {
            let evt = match evt {
                Ok(e) => e,
                Err(_) => continue,
            };
            use notify::EventKind::*;
            let kind = match evt.kind {
                Create(_) => "created",
                Modify(_) => "modified",
                Remove(_) => "removed",
                _ => continue,
            };
            for p in evt.paths {
                let ps = p.to_string_lossy().to_string();
                let now = std::time::Instant::now();
                if let Some(last) = last_emit.get(&ps) {
                    if now.duration_since(*last).as_millis() < 500 {
                        continue; // debounced
                    }
                }
                last_emit.insert(ps.clone(), now);
                if fwd_tx.send((ps, kind.to_string())).is_err() {
                    return; // receiver gone: stop the thread
                }
            }
        }
    });
    tokio::spawn(async move {
        while let Some((path, kind)) = fwd_rx.recv().await {
            let _ = app_handle.emit(
                "index://changed",
                IndexChangeEvent {
                    path,
                    kind,
                },
            );
        }
    });

    watchers()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(project_root, watcher);
    Ok(())
}

/// Stop watching a project root. Dropping the watcher ends notifications.
#[tauri::command]
pub fn index_watch_stop(project_root: String) -> Result<(), String> {
    watchers()
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&project_root);
    Ok(())
}
