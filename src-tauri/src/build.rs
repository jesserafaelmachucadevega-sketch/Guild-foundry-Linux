// SPDX-License-Identifier: Apache-2.0
// Phase 8 — Build/test/fix engine (master spec sections 31, 32, 63, 64).
//
// Owns the build-fix loop as a background tokio task emitting `build://log`
// Tauri events, auto-detected test runs, embedded project templates, and the
// artifact/release store. Rust layer owns ALL privileged operations here:
// subprocesses, filesystem, hashing.
//
// Frozen Phase 6 API (signatures must not change):
//   pub(crate) fn build_start_impl(root: &std::path::Path, target: &str) -> Result<String, String>
//   pub(crate) fn run_tests_impl(root: &std::path::Path) -> Result<String, String>
//
// NOT VERIFIED: this crate has never been compiled in this environment (no
// Rust toolchain / WebKitGTK dev headers here). Verify with `cargo check` on
// the Linux target before wiring the handlers into main.rs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::Emitter;
use tauri::Manager;
use uuid::Uuid;

/// Migration fragment (Phase 8). The v1 schema in db.rs already provides
/// `builds`, `tests` and `artifacts`; this fragment adds `releases`.
/// Apply in db.rs `apply_migrations` as schema version 2 (see
/// .integration/phase-8.md for the exact wiring).
pub(crate) const MIGRATION_BUILD_V2: &str = r#"
CREATE TABLE IF NOT EXISTS releases (
    id          TEXT PRIMARY KEY,
    build_id    TEXT REFERENCES builds(id),
    version     TEXT NOT NULL,
    notes       TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
);
"#;

/// Max rebuild attempts before the loop gives up on its own.
const MAX_ATTEMPTS: u32 = 10;
/// Identical normalized error signatures before the loop HALTs.
const MAX_IDENTICAL_SIGNATURES: u32 = 3;
/// Lines of log tail kept per build in memory.
const LOG_TAIL_CAP: usize = 200;

// ---------------------------------------------------------------------------
// Global engine state
// ---------------------------------------------------------------------------

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static DB_PATH: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildPhase {
    Queued,
    Building,
    Fixing,
    Validating,
    Completed,
    Failed,
    Halted,
    Cancelled,
}

impl BuildPhase {
    fn as_str(self) -> &'static str {
        match self {
            BuildPhase::Queued => "queued",
            BuildPhase::Building => "building",
            BuildPhase::Fixing => "fixing",
            BuildPhase::Validating => "validating",
            BuildPhase::Completed => "completed",
            BuildPhase::Failed => "failed",
            BuildPhase::Halted => "halted",
            BuildPhase::Cancelled => "cancelled",
        }
    }
}

struct BuildState {
    build_id: String,
    root: PathBuf,
    target: String,
    phase: BuildPhase,
    attempts: u32,
    log_tail: Vec<String>,
    error_signature: Option<String>,
    sig_counts: HashMap<String, u32>,
    failure_report_path: Option<String>,
    cancel: Arc<AtomicBool>,
    log_file: PathBuf,
}

static BUILDS: OnceLock<Arc<Mutex<HashMap<String, BuildState>>>> = OnceLock::new();

fn registry() -> Arc<Mutex<HashMap<String, BuildState>>> {
    BUILDS
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// Called once from each Phase 8 command (and by Phase 6 tools via
/// `build::init_build_engine(&app)`) before any engine work.
pub(crate) fn init_build_engine(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir unavailable: {e}"))?;
    let db_path = dir.join("guild-foundry-ai.db");
    let _ = APP.set(app.clone());
    let _ = DB_PATH.set(db_path);
    Ok(())
}

fn app_handle() -> Result<tauri::AppHandle, String> {
    APP.get()
        .cloned()
        .ok_or_else(|| "build engine not initialized (init_build_engine)".to_string())
}

fn db_conn() -> Result<rusqlite::Connection, String> {
    let path = DB_PATH
        .get()
        .ok_or_else(|| "build engine not initialized (init_build_engine)".to_string())?;
    rusqlite::Connection::open(path).map_err(|e| format!("db open failed: {e}"))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Resolve (or create) the project row for a root path so `builds.project_id`
/// always satisfies its NOT NULL / FK constraint.
fn resolve_project_id(conn: &rusqlite::Connection, root: &Path) -> Result<String, String> {
    let root_str = root.to_string_lossy().to_string();
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM projects WHERE root_path = ?1",
            rusqlite::params![root_str.as_str()],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = existing {
        return Ok(id);
    }
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    let id = Uuid::new_v4().to_string();
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO projects (id, name, root_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, name, root_str, now, now],
    )
    .map_err(|e| format!("project insert failed: {e}"))?;
    Ok(id)
}

fn artifact_store_dir() -> Result<PathBuf, String> {
    let base = DB_PATH
        .get()
        .and_then(|p| p.parent())
        .ok_or_else(|| "build engine not initialized".to_string())?
        .join("artifacts");
    std::fs::create_dir_all(&base).map_err(|e| format!("artifact dir failed: {e}"))?;
    Ok(base)
}

fn build_logs_dir() -> Result<PathBuf, String> {
    let base = DB_PATH
        .get()
        .and_then(|p| p.parent())
        .ok_or_else(|| "build engine not initialized".to_string())?
        .join("build-logs");
    std::fs::create_dir_all(&base).map_err(|e| format!("build log dir failed: {e}"))?;
    Ok(base)
}

// ---------------------------------------------------------------------------
// Events + logging
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct BuildLogEvent {
    build_id: String,
    line: String,
    /// "stdout" | "stderr" | "stage"
    stream: String,
}

fn emit_log(app: &tauri::AppHandle, build_id: &str, line: &str, stream: &str) {
    let ev = BuildLogEvent {
        build_id: build_id.to_string(),
        line: line.to_string(),
        stream: stream.to_string(),
    };
    let _ = app.emit("build://log", ev);
}

fn push_log(state: &mut BuildState, app: &tauri::AppHandle, line: &str, stream: &str) {
    if state.log_tail.len() >= LOG_TAIL_CAP {
        state.log_tail.remove(0);
    }
    state.log_tail.push(line.to_string());
    emit_log(app, &state.build_id, line, stream);
    // Best-effort durable log file.
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.log_file)
    {
        let _ = writeln!(f, "[{stream}] {line}");
    }
}

// ---------------------------------------------------------------------------
// Toolchain detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BuildCmd {
    program: String,
    args: Vec<String>,
    label: String,
}

fn read_package_json_scripts(root: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(root.join("package.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&raw).ok()
}

/// Detect the project's build command from well-known manifests.
/// Returns an error when nothing recognizable is found (honest: no guessing).
fn detect_build_command(root: &Path, target: &str) -> Result<BuildCmd, String> {
    if let Some(pkg) = read_package_json_scripts(root) {
        let scripts = pkg.get("scripts");
        let has = |name: &str| {
            scripts
                .and_then(|s| s.get(name))
                .and_then(|v| v.as_str())
                .is_some()
        };
        if has("build") {
            return Ok(BuildCmd {
                program: "npm".into(),
                args: vec!["run".into(), "build".into()],
                label: "npm run build".into(),
            });
        }
        if root.join("tsconfig.json").exists() {
            return Ok(BuildCmd {
                program: "npx".into(),
                args: vec!["--yes".into(), "tsc".into(), "--noEmit".into()],
                label: "tsc --noEmit (typecheck fallback)".into(),
            });
        }
    }
    if root.join("Cargo.toml").exists() {
        let mut args = vec!["build".to_string()];
        if target.eq_ignore_ascii_case("release") {
            args.push("--release".to_string());
        }
        return Ok(BuildCmd {
            program: "cargo".into(),
            args,
            label: "cargo build".into(),
        });
    }
    if root.join("go.mod").exists() {
        return Ok(BuildCmd {
            program: "go".into(),
            args: vec!["build".into(), "./...".into()],
            label: "go build ./...".into(),
        });
    }
    if root.join("pom.xml").exists() {
        return Ok(BuildCmd {
            program: "mvn".into(),
            args: vec!["-q".into(), "-DskipTests".into(), "package".into()],
            label: "mvn package".into(),
        });
    }
    if root.join("build.gradle").exists() || root.join("build.gradle.kts").exists() {
        let gradle = if root.join("gradlew").exists() {
            "./gradlew"
        } else {
            "gradle"
        };
        return Ok(BuildCmd {
            program: gradle.into(),
            args: vec!["build".into(), "-x".into(), "test".into()],
            label: format!("{gradle} build"),
        });
    }
    if root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("setup.cfg").exists()
    {
        return Ok(BuildCmd {
            program: "python3".into(),
            args: vec!["-m".into(), "compileall".into(), "-q".into(), ".".into()],
            label: "python3 -m compileall (syntax check fallback)".into(),
        });
    }
    Err(format!(
        "no recognized build tooling in {} (looked for package.json, Cargo.toml, go.mod, pom.xml, build.gradle, pyproject.toml/setup.py)",
        root.display()
    ))
}

async fn run_command(
    cmd: &BuildCmd,
    cwd: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(bool, Vec<(String, String)>), String> {
    let mut child = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn `{}`: {e}", cmd.label))?;
    let out = tokio::time::timeout(std::time::Duration::from_secs(20 * 60), child.wait_with_output())
        .await
        .map_err(|_| format!("`{}` timed out after 20 minutes", cmd.label))?
        .map_err(|e| format!("`{}` failed while running: {e}", cmd.label))?;
    let _ = cancel; // flag is honored between stages, not mid-process
    let ok = out.status.success();
    let mut lines = Vec::new();
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        lines.push(("stdout".to_string(), l.to_string()));
    }
    for l in String::from_utf8_lossy(&out.stderr).lines() {
        lines.push(("stderr".to_string(), l.to_string()));
    }
    Ok((ok, lines))
}

// ---------------------------------------------------------------------------
// CLASSIFY -> LOCATE -> ASSIGN -> PATCH (rule-based, honest)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorCategory {
    Compile,
    Test,
    Lint,
    Deps,
    Config,
}

impl ErrorCategory {
    fn as_str(self) -> &'static str {
        match self {
            ErrorCategory::Compile => "compile",
            ErrorCategory::Test => "test",
            ErrorCategory::Lint => "lint",
            ErrorCategory::Deps => "deps",
            ErrorCategory::Config => "config",
        }
    }
}

/// Regex classify, most-specific first. Never string-matches secrets: only
/// the normalized category is persisted.
fn classify_error_line(line: &str) -> Option<ErrorCategory> {
    // Deps
    for pat in [
        r"(?i)could not resolve",
        r"(?i)npm ERR!",
        r"(?i)failed to download",
        r"(?i)cannot find module",
        r"(?i)ModuleNotFoundError",
        r"(?i)no matching package",
        r"(?i)failed to resolve",
    ] {
        if Regex::new(pat).map(|re| re.is_match(line)).unwrap_or(false) {
            return Some(ErrorCategory::Deps);
        }
    }
    // Config
    for pat in [
        r"(?i)missing field",
        r"(?i)unknown field",
        r"(?i)invalid configuration",
        r"(?i)failed to parse",
        r"(?i)malformed",
    ] {
        if Regex::new(pat).map(|re| re.is_match(line)).unwrap_or(false) {
            return Some(ErrorCategory::Config);
        }
    }
    // Compile
    for pat in [
        r"error\[",
        r"(?i)^error:",
        r"(?i): error:",
        r"error TS\d+",
        r"(?i)failed to compile",
        r"(?i)undefined reference",
        r"(?i)expected .* found",
        r"(?i)cannot borrow",
        r"(?i)type mismatch",
    ] {
        if Regex::new(pat).map(|re| re.is_match(line)).unwrap_or(false) {
            return Some(ErrorCategory::Compile);
        }
    }
    // Test
    for pat in [
        r"(?i)test result: FAILED",
        r"(?i)\bFAILED\b",
        r"(?i)AssertionError",
        r"(?i)test .* panicked",
        r"(?i)\d+ (tests? )?failed",
    ] {
        if Regex::new(pat).map(|re| re.is_match(line)).unwrap_or(false) {
            return Some(ErrorCategory::Test);
        }
    }
    // Lint
    for pat in [
        r"(?i)warning:",
        r"(?i)clippy",
        r"(?i)eslint",
        r"(?i)pylint",
        r"(?i)flake8",
    ] {
        if Regex::new(pat).map(|re| re.is_match(line)).unwrap_or(false) {
            return Some(ErrorCategory::Lint);
        }
    }
    None
}

/// Extract `file:line` from a compiler/test output line.
fn locate_error(line: &str) -> Option<(String, u32)> {
    let re = Regex::new(
        r"(?P<file>[A-Za-z0-9_\-./\\]+\.(?:rs|ts|tsx|js|jsx|mjs|cjs|py|go|java|c|h|cpp|cc|toml|json|yaml|yml))(?::|\()(?P<line>\d+)",
    )
    .ok()?;
    let caps = re.captures(line)?;
    let file = caps.name("file")?.as_str().to_string();
    let line_no = caps.name("line")?.as_str().parse::<u32>().ok()?;
    Some((file, line_no))
}

/// Map an error category to the agent role that owns the fix.
fn assign_role(category: ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Compile => "compiler-fixer",
        ErrorCategory::Test => "qa",
        ErrorCategory::Lint => "linter",
        ErrorCategory::Deps => "dependency-manager",
        ErrorCategory::Config => "config-fixer",
    }
}

/// Normalize an error signature so the loop detector can spot repeats:
/// strip numbers and filesystem paths.
fn normalize_signature(line: &str) -> String {
    let re_num = Regex::new(r"\d+").unwrap_or_else(|_| Regex::new("$^").unwrap());
    let re_path =
        Regex::new(r"[/\\][\w.\-/\\]*").unwrap_or_else(|_| Regex::new("$^").unwrap());
    let s = re_num.replace_all(line, "#");
    re_path.replace_all(&s, "<path>").trim().to_string()
}

struct PatchSuggestion {
    /// Unified-diff-style suggestion text (rule-based, heuristic).
    diff_text: String,
    /// Whether the engine may attempt to apply it automatically.
    actionable: bool,
    /// Concrete file write to perform when actionable.
    apply_path: Option<PathBuf>,
    apply_content: Option<String>,
    /// Human-readable explanation.
    explanation: String,
}

/// Rule-based patch generation. Honest: heuristics only — a configured model
/// provider (Phase 2) is required for real LLM-generated patches.
fn suggest_patch(
    category: ErrorCategory,
    line: &str,
    location: &Option<(String, u32)>,
    root: &Path,
) -> PatchSuggestion {
    let role = assign_role(category);
    let header = format!(
        "# category: {} | assigned: {}\n# rule-based suggestion (heuristic; configure a model provider for LLM patches)\n",
        category.as_str(),
        role
    );
    match category {
        ErrorCategory::Deps => PatchSuggestion {
            diff_text: format!(
                "{header}# error: {line}\n# suggestion: install the missing dependency with the project's package manager\n#   node : npm install <package>\n#   rust : cargo add <crate>\n#   python: pip install <package>  (or add to pyproject.toml)\n#   go   : go get <module>\n"
            ),
            actionable: false,
            apply_path: None,
            apply_content: None,
            explanation: "missing dependency — requires a package-manager install, not a file patch".into(),
        },
        ErrorCategory::Config => PatchSuggestion {
            diff_text: format!(
                "{header}# error: {line}\n# suggestion: open the manifest/config at the reported location and fix the\n# offending field (missing/unknown/invalid key). No safe automatic rewrite.\n"
            ),
            actionable: false,
            apply_path: None,
            apply_content: None,
            explanation: "config error — needs a human or LLM to edit the manifest".into(),
        },
        ErrorCategory::Test => PatchSuggestion {
            diff_text: format!(
                "{header}# failing test output: {line}\n# suggestion: fix the implementation or the assertion at the located file.\n# No automatic rewrite — test semantics need a human or LLM.\n"
            ),
            actionable: false,
            apply_path: None,
            apply_content: None,
            explanation: "test failure — implementation/assertion fix required".into(),
        },
        ErrorCategory::Lint => {
            // Actionable rule: TS/JS unused variable reported as
            // "'x' is declared but its value is never read" -> prefix with _.
            if let Some((file, _)) = location {
                let re = Regex::new(r"'([^']+)' is declared but (?:its value is )?never read")
                    .ok()
                    .and_then(|re| re.captures(line))
                    .and_then(|c| c.get(1).map(|m| m.as_str().to_string()));
                if let (Some(var), true) =
                    (re, file.ends_with(".ts") || file.ends_with(".tsx"))
                {
                    let path = root.join(file);
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let new_content = content.replacen(&var, &format!("_{var}"), 1);
                        if new_content != content {
                            return PatchSuggestion {
                                diff_text: format!(
                                    "{header}--- a/{file}\n+++ b/{file}\n@@\n- {var}\n+ _{var}\n# unused variable renamed with underscore prefix\n"
                                ),
                                actionable: true,
                                apply_path: Some(path),
                                apply_content: Some(new_content),
                                explanation: format!(
                                    "unused variable '{var}' renamed to '_{var}'"
                                ),
                            };
                        }
                    }
                }
            }
            PatchSuggestion {
                diff_text: format!(
                    "{header}# lint output: {line}\n# suggestion: address the reported warning at the located file.\n"
                ),
                actionable: false,
                apply_path: None,
                apply_content: None,
                explanation: "lint warning — manual fix suggested".into(),
            }
        }
        ErrorCategory::Compile => {
            // Actionable rule: TS "Property 'x' does not exist" — no safe auto-fix.
            // Actionable rule: missing semicolon hints ("expected ';'").
            if line.contains("expected ';'") || line.contains("Expected ';'") {
                if let Some((file, line_no)) = location {
                    let path = root.join(file);
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let mut lines: Vec<String> =
                            content.lines().map(|l| l.to_string()).collect();
                        let idx = (*line_no as usize).saturating_sub(1);
                        if idx < lines.len() && !lines[idx].trim_end().ends_with(';') {
                            lines[idx] = format!("{};", lines[idx].trim_end());
                            return PatchSuggestion {
                                diff_text: format!(
                                    "{header}--- a/{}\n+++ b/{}\n@@ -{line_no},1 +{line_no},1 @@\n-{}\n+{};\n# appended missing semicolon\n",
                                    path.display(), path.display(), lines[idx].trim_end_matches(';'), lines[idx]
                                ),
                                actionable: true,
                                apply_path: Some(path),
                                apply_content: Some(lines.join("\n")),
                                explanation: "appended missing semicolon".into(),
                            };
                        }
                    }
                }
            }
            PatchSuggestion {
                diff_text: format!(
                    "{header}# compile error: {line}\n# suggestion: fix the reported type/syntax error at the located file.\n# Configure a model provider for an LLM-generated patch.\n"
                ),
                actionable: false,
                apply_path: None,
                apply_content: None,
                explanation: "compile error — manual/LLM fix required".into(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Build-fix loop: BUILD -> CAPTURE -> CLASSIFY -> LOCATE -> ASSIGN -> PATCH
// -> APPLY -> TARGETED TEST -> REBUILD -> FULL VALIDATION.
// ---------------------------------------------------------------------------

fn update_build_status(build_id: &str, status: &str) {
    if let Ok(conn) = db_conn() {
        let _ = conn.execute(
            "UPDATE builds SET status = ?1 WHERE id = ?2",
            rusqlite::params![status, build_id],
        );
    }
}

/// Frozen Phase 6 entry point. Spawns the build-fix loop as a background
/// tokio task and returns the build id.
pub(crate) fn build_start_impl(root: &std::path::Path, target: &str) -> Result<String, String> {
    let app = app_handle()?;
    let build_id = Uuid::new_v4().to_string();
    let root = root.to_path_buf();
    let target = target.to_string();

    let log_file = build_logs_dir()?.join(format!("{build_id}.log"));

    // Persist the build row up-front so `builds`/`tests` tables stay truthful.
    let conn = db_conn()?;
    let project_id = resolve_project_id(&conn, &root)?;
    conn.execute(
        "INSERT INTO builds (id, project_id, status, log_path, created_at) VALUES (?1, ?2, 'queued', ?3, ?4)",
        rusqlite::params![build_id, project_id, log_file.to_string_lossy().to_string(), now_rfc3339()],
    )
    .map_err(|e| format!("build insert failed: {e}"))?;
    drop(conn);

    let state = BuildState {
        build_id: build_id.clone(),
        root: root.clone(),
        target: target.clone(),
        phase: BuildPhase::Queued,
        attempts: 0,
        log_tail: Vec::new(),
        error_signature: None,
        sig_counts: HashMap::new(),
        failure_report_path: None,
        cancel: Arc::new(AtomicBool::new(false)),
        log_file,
    };
    registry()
        .lock()
        .map_err(|e| format!("build registry poisoned: {e}"))?
        .insert(build_id.clone(), state);

    let app_clone = app.clone();
    let bid = build_id.clone();
    tokio::spawn(async move {
        build_loop(app_clone, bid).await;
    });
    Ok(build_id)
}

fn get_state(build_id: &str) -> Result<BuildState, String> {
    registry()
        .lock()
        .map_err(|e| format!("build registry poisoned: {e}"))?
        .get(build_id)
        .ok_or_else(|| format!("unknown build id: {build_id}"))
        .map(clone_state)
}

fn clone_state(s: &BuildState) -> BuildState {
    BuildState {
        build_id: s.build_id.clone(),
        root: s.root.clone(),
        target: s.target.clone(),
        phase: s.phase,
        attempts: s.attempts,
        log_tail: s.log_tail.clone(),
        error_signature: s.error_signature.clone(),
        sig_counts: s.sig_counts.clone(),
        failure_report_path: s.failure_report_path.clone(),
        cancel: Arc::clone(&s.cancel),
        log_file: s.log_file.clone(),
    }
}

fn with_state<F>(build_id: &str, f: F) -> Result<(), String>
where
    F: FnOnce(&mut BuildState),
{
    let reg = registry();
    let mut guard = reg
        .lock()
        .map_err(|e| format!("build registry poisoned: {e}"))?;
    let state = guard
        .get_mut(build_id)
        .ok_or_else(|| format!("unknown build id: {build_id}"))?;
    f(state);
    Ok(())
}

fn cancelled(build_id: &str) -> bool {
    get_state(build_id).map(|s| s.cancel.load(Ordering::SeqCst)).unwrap_or(true)
}

async fn build_loop(app: tauri::AppHandle, build_id: String) {
    let snapshot = match get_state(&build_id) {
        Ok(s) => s,
        Err(e) => {
            emit_log(&app, &build_id, &format!("build loop abort: {e}"), "stage");
            return;
        }
    };
    let root = snapshot.root.clone();
    let target = snapshot.target.clone();

    let stage = |id: &str, app: &tauri::AppHandle, msg: &str| {
        let line = format!("[{msg}]");
        let _ = with_state(id, |s| push_log(s, app, &line, "stage"));
    };
    let say = |id: &str, app: &tauri::AppHandle, line: &str, stream: &str| {
        let _ = with_state(id, |s| push_log(s, app, line, stream));
    };

    // BUILD: detect the toolchain command.
    stage(&build_id, &app, "BUILD");
    let cmd = match detect_build_command(&root, &target) {
        Ok(c) => {
            say(&build_id, &app, &format!("detected: {}", c.label), "stage");
            c
        }
        Err(e) => {
            say(&build_id, &app, &format!("no build tooling: {e}"), "stderr");
            let _ = with_state(&build_id, |s| s.phase = BuildPhase::Failed);
            update_build_status(&build_id, "failed");
            return;
        }
    };

    let _ = with_state(&build_id, |s| s.phase = BuildPhase::Building);
    update_build_status(&build_id, "running");

    loop {
        if cancelled(&build_id) {
            stage(&build_id, &app, "CANCELLED");
            let _ = with_state(&build_id, |s| s.phase = BuildPhase::Cancelled);
            update_build_status(&build_id, "cancelled");
            return;
        }
        let attempt_no = get_state(&build_id).map(|s| s.attempts + 1).unwrap_or(1);
        let _ = with_state(&build_id, |s| s.attempts = attempt_no);
        say(
            &build_id,
            &app,
            &format!("attempt {attempt_no}/{MAX_ATTEMPTS}: {}", cmd.label),
            "stage",
        );

        // BUILD -> CAPTURE
        let cancel_flag = get_state(&build_id)
            .map(|s| Arc::clone(&s.cancel))
            .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)));
        let (ok, lines) = match run_command(&cmd, &root, &cancel_flag).await {
            Ok(r) => r,
            Err(e) => {
                say(&build_id, &app, &e, "stderr");
                let _ = with_state(&build_id, |s| s.phase = BuildPhase::Failed);
                update_build_status(&build_id, "failed");
                return;
            }
        };
        for (stream, line) in &lines {
            say(&build_id, &app, line, stream);
        }

        if ok {
            // FULL VALIDATION: run the whole detected test suite.
            stage(&build_id, &app, "FULL VALIDATION");
            let _ = with_state(&build_id, |s| s.phase = BuildPhase::Validating);
            match run_tests_impl(&root) {
                Ok(summary) => {
                    for sl in summary.lines() {
                        say(&build_id, &app, sl, "stdout");
                    }
                    if summary_contains_failure(&summary) {
                        say(&build_id, &app, "validation failed: tests red after green build", "stderr");
                        let _ = with_state(&build_id, |s| s.phase = BuildPhase::Failed);
                        update_build_status(&build_id, "failed");
                    } else {
                        stage(&build_id, &app, "COMPLETED");
                        let _ = with_state(&build_id, |s| s.phase = BuildPhase::Completed);
                        update_build_status(&build_id, "completed");
                    }
                }
                Err(e) => {
                    say(&build_id, &app, &format!("validation error: {e}"), "stderr");
                    let _ = with_state(&build_id, |s| s.phase = BuildPhase::Failed);
                    update_build_status(&build_id, "failed");
                }
            }
            return;
        }

        // --- Fix path: CLASSIFY -> LOCATE -> ASSIGN -> PATCH ---
        stage(&build_id, &app, "CLASSIFY");
        let _ = with_state(&build_id, |s| s.phase = BuildPhase::Fixing);

        let error_lines: Vec<String> = lines
            .iter()
            .filter(|(_, l)| classify_error_line(l).is_some())
            .map(|(_, l)| l.clone())
            .collect();
        if error_lines.is_empty() {
            say(
                &build_id,
                &app,
                "build failed with no classifiable error lines; halting",
                "stderr",
            );
            halt_with_report(&app, &build_id, "unclassified build failure", &lines);
            return;
        }

        let first = &error_lines[0];
        let category = classify_error_line(first).unwrap_or(ErrorCategory::Compile);
        stage(&build_id, &app, "LOCATE");
        let location = locate_error(first);
        stage(&build_id, &app, "ASSIGN");
        let role = assign_role(category);
        say(
            &build_id,
            &app,
            &format!("category={} role={}", category.as_str(), role),
            "stage",
        );
        if let Some((ref f, ln)) = location {
            say(&build_id, &app, &format!("located: {f}:{ln}"), "stage");
        }

        // Loop detector: normalized signature of the first error line.
        let sig = normalize_signature(first);
        let count = {
            let mut c = 0u32;
            let _ = with_state(&build_id, |s| {
                let e = s.sig_counts.entry(sig.clone()).or_insert(0);
                *e += 1;
                c = *e;
                s.error_signature = Some(sig.clone());
            });
            c
        };
        say(
            &build_id,
            &app,
            &format!("error signature: {sig} (seen {count}x)"),
            "stage",
        );
        if count >= MAX_IDENTICAL_SIGNATURES {
            stage(&build_id, &app, "HALT (3 identical signatures)");
            halt_with_report(&app, &build_id, &sig, &lines);
            return;
        }
        if attempt_no >= MAX_ATTEMPTS {
            stage(&build_id, &app, "HALT (attempt budget exhausted)");
            halt_with_report(&app, &build_id, &sig, &lines);
            return;
        }

        stage(&build_id, &app, "PATCH");
        let suggestion = suggest_patch(category, first, &location, &root);
        for dl in suggestion.diff_text.lines() {
            say(&build_id, &app, dl, "stdout");
        }
        say(&build_id, &app, &suggestion.explanation, "stage");

        if suggestion.actionable {
            // APPLY via Phase 4's sandboxed workspace writer.
            stage(&build_id, &app, "APPLY");
            let path = suggestion.apply_path.clone().unwrap();
            let content = suggestion.apply_content.clone().unwrap();
            // write_file takes (root, rel, content); derive the
            // sandbox-relative path from the absolute apply path.
            let rel = path
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string_lossy().into_owned());
            match crate::workspace::write_file(&root, &rel, &content) {
                Ok(()) => {
                    say(&build_id, &app, &format!("applied patch to {}", path.display()), "stage");
                    // TARGETED TEST
                    stage(&build_id, &app, "TARGETED TEST");
                    match targeted_test(&root, &location).await {
                        Ok(t) => say(&build_id, &app, &t, "stdout"),
                        Err(e) => say(&build_id, &app, &format!("targeted test: {e}"), "stderr"),
                    }
                }
                Err(e) => {
                    say(
                        &build_id,
                        &app,
                        &format!("patch apply failed: {e:?}"),
                        "stderr",
                    );
                }
            }
        } else {
            say(
                &build_id,
                &app,
                "patch is advisory only (no safe automatic rewrite); configure a model provider for LLM patches",
                "stage",
            );
        }

        // REBUILD: loop continues.
        stage(&build_id, &app, "REBUILD");
    }
}

fn summary_contains_failure(summary: &str) -> bool {
    let lower = summary.to_lowercase();
    lower.contains("fail") && !lower.contains("0 fail")
}

/// Failure report artifact + halted state (master spec section 31).
fn halt_with_report(
    app: &tauri::AppHandle,
    build_id: &str,
    signature: &str,
    lines: &[(String, String)],
) {
    let report = format!(
        "# Build failure report\n\n\
         - build_id: {build_id}\n\
         - created: {}\n\
         - error signature: {signature}\n\n\
         ## Attempted fixes\n\
         See the build log; rule-based patch suggestions were emitted as\n\
         `PATCH` stage entries. No automatic rewrite was safe for this\n\
         signature (or the signature repeated 3 times).\n\n\
         ## Relevant files\n\
         Extracted from LOCATE stage entries in the build log.\n\n\
         ## Logs (tail)\n\
         ```\n{}\n```\n\n\
         ## Likely root cause\n\
         The normalized signature above repeated without progress; the\n\
         underlying cause needs a human or an LLM-generated patch\n\
         (configure a model provider).\n\n\
         ## Recommended action\n\
         1. Inspect the located file(s).\n\
         2. Apply the suggested fix manually or enable a model provider.\n\
         3. Re-run the build.\n",
        now_rfc3339(),
        lines
            .iter()
            .rev()
            .take(80)
            .rev()
            .map(|(s, l)| format!("[{s}] {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let path = match build_logs_dir() {
        Ok(d) => d.join(format!("{build_id}-failure-report.md")),
        Err(_) => std::env::temp_dir().join(format!("{build_id}-failure-report.md")),
    };
    let report_path = if std::fs::write(&path, &report).is_ok() {
        // Record as an artifact row (best effort).
        if let (Ok(conn), Ok(store)) = (db_conn(), artifact_store_dir()) {
            if let Ok(stored) = store_artifact_file(&store, build_id, &path) {
                let project_id: Option<String> = conn
                    .query_row(
                        "SELECT project_id FROM builds WHERE id = ?1",
                        rusqlite::params![build_id],
                        |r| r.get(0),
                    )
                    .ok();
                let aid = Uuid::new_v4().to_string();
                let _ = conn.execute(
                    "INSERT INTO artifacts (id, project_id, kind, source, version, path, sha256, created_at) VALUES (?1, ?2, 'failure-report', 'build-engine', 1, ?3, ?4, ?5)",
                    rusqlite::params![
                        aid,
                        project_id,
                        stored.to_string_lossy().to_string(),
                        sha256_of_file(&stored).unwrap_or_default(),
                        now_rfc3339()
                    ],
                );
            }
        }
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };
    let _ = with_state(build_id, |s| {
        s.phase = BuildPhase::Halted;
        s.failure_report_path = report_path.clone();
    });
    update_build_status(build_id, "halted");
    let _ = with_state(build_id, |s| {
        push_log(
            s,
            app,
            "loop halted — failure report written (see build log / artifacts)",
            "stage",
        )
    });
}

/// Run a test command scoped to the located file when the ecosystem supports
/// it; otherwise report that no targeted runner exists.
async fn targeted_test(
    root: &Path,
    location: &Option<(String, u32)>,
) -> Result<String, String> {
    let (file, _) = match location {
        Some(l) => l,
        None => return Err("no located file; skipping targeted test".into()),
    };
    let stem = file
        .rsplit('/')
        .next()
        .unwrap_or(file)
        .split('.')
        .next()
        .unwrap_or("");
    let cmd = if root.join("package.json").exists() && file.contains(".test.") {
        Some(BuildCmd {
            program: "npx".into(),
            args: vec!["--yes".into(), "jest".into(), file.clone()],
            label: format!("jest {file}"),
        })
    } else if root.join("Cargo.toml").exists() && file.ends_with(".rs") {
        Some(BuildCmd {
            program: "cargo".into(),
            args: vec!["test".into(), stem.into()],
            label: format!("cargo test {stem}"),
        })
    } else if (root.join("pytest.ini").exists()
        || root.join("pyproject.toml").exists()
        || root.join("setup.cfg").exists())
        && file.ends_with(".py")
    {
        Some(BuildCmd {
            program: "python3".into(),
            args: vec!["-m".into(), "pytest".into(), "-q".into(), file.clone()],
            label: format!("pytest {file}"),
        })
    } else if root.join("go.mod").exists() && file.ends_with(".go") {
        Some(BuildCmd {
            program: "go".into(),
            args: vec!["test".into(), "-run".into(), stem.into(), "./...".into()],
            label: format!("go test -run {stem}"),
        })
    } else {
        None
    };
    match cmd {
        Some(c) => {
            let cancel = Arc::new(AtomicBool::new(false));
            let (ok, lines) = run_command(&c, root, &cancel).await?;
            let tail: Vec<String> = lines
                .iter()
                .rev()
                .take(10)
                .rev()
                .map(|(s, l)| format!("[{s}] {l}"))
                .collect();
            Ok(format!(
                "targeted test `{}`: {}\n{}",
                c.label,
                if ok { "PASS" } else { "FAIL" },
                tail.join("\n")
            ))
        }
        None => Err("no targeted runner for this file/ecosystem; proceeding to rebuild".into()),
    }
}

// ---------------------------------------------------------------------------
// Testing: auto-detect project tooling and run the full matrix
// (unit/integration/e2e/lint/format/typecheck/static analysis/dependency
// audit/security). Returns a human-readable summary.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestSuite {
    /// Display name, e.g. "npm test".
    name: String,
    /// "unit" | "lint" | "format" | "typecheck" | "static-analysis" | "audit" | "security" | "e2e"
    kind: String,
    cmd: BuildCmd,
}

fn detect_test_suites(root: &Path) -> Vec<TestSuite> {
    let mut suites = Vec::new();
    let has = |p: &str| root.join(p).exists();

    if has("package.json") {
        let scripts = read_package_json_scripts(root);
        let script = |name: &str| {
            scripts
                .as_ref()
                .and_then(|s| s.get("scripts"))
                .and_then(|s| s.get(name))
                .and_then(|v| v.as_str())
                .is_some()
        };
        if script("test") {
            suites.push(TestSuite {
                name: "npm test".into(),
                kind: "unit".into(),
                cmd: BuildCmd {
                    program: "npm".into(),
                    args: vec!["test".into()],
                    label: "npm test".into(),
                },
            });
        }
        if has("tsconfig.json") {
            suites.push(TestSuite {
                name: "tsc --noEmit".into(),
                kind: "typecheck".into(),
                cmd: BuildCmd {
                    program: "npx".into(),
                    args: vec!["--yes".into(), "tsc".into(), "--noEmit".into()],
                    label: "tsc --noEmit".into(),
                },
            });
        }
        for (cfg, tool) in [
            (".eslintrc", "eslint"),
            (".eslintrc.json", "eslint"),
            (".eslintrc.js", "eslint"),
            ("eslint.config.js", "eslint"),
            ("eslint.config.mjs", "eslint"),
        ] {
            if has(cfg) {
                suites.push(TestSuite {
                    name: tool.into(),
                    kind: "lint".into(),
                    cmd: BuildCmd {
                        program: "npx".into(),
                        args: vec!["--yes".into(), tool.into(), ".".into()],
                        label: format!("{tool} ."),
                    },
                });
                break;
            }
        }
        for (cfg, tool) in [
            (".prettierrc", "prettier"),
            (".prettierrc.json", "prettier"),
            ("prettier.config.js", "prettier"),
        ] {
            if has(cfg) {
                suites.push(TestSuite {
                    name: "prettier --check".into(),
                    kind: "format".into(),
                    cmd: BuildCmd {
                        program: "npx".into(),
                        args: vec!["--yes".into(), "prettier".into(), "--check".into(), ".".into()],
                        label: "prettier --check .".into(),
                    },
                });
                break;
            }
        }
        if has("package-lock.json") {
            suites.push(TestSuite {
                name: "npm audit".into(),
                kind: "audit".into(),
                cmd: BuildCmd {
                    program: "npm".into(),
                    args: vec!["audit".into(), "--audit-level=moderate".into()],
                    label: "npm audit".into(),
                },
            });
        }
    }

    if has("Cargo.toml") {
        suites.push(TestSuite {
            name: "cargo test".into(),
            kind: "unit".into(),
            cmd: BuildCmd {
                program: "cargo".into(),
                args: vec!["test".into()],
                label: "cargo test".into(),
            },
        });
        suites.push(TestSuite {
            name: "cargo clippy".into(),
            kind: "static-analysis".into(),
            cmd: BuildCmd {
                program: "cargo".into(),
                args: vec!["clippy".into(), "--".into(), "-D".into(), "warnings".into()],
                label: "cargo clippy".into(),
            },
        });
        suites.push(TestSuite {
            name: "cargo fmt --check".into(),
            kind: "format".into(),
            cmd: BuildCmd {
                program: "cargo".into(),
                args: vec!["fmt".into(), "--check".into()],
                label: "cargo fmt --check".into(),
            },
        });
        if has("Cargo.lock") {
            suites.push(TestSuite {
                name: "cargo audit".into(),
                kind: "audit".into(),
                cmd: BuildCmd {
                    program: "cargo".into(),
                    args: vec!["audit".into()],
                    label: "cargo audit (needs cargo-audit installed)".into(),
                },
            });
        }
    }

    let py = has("pytest.ini") || has("pyproject.toml") || has("setup.cfg") || has("setup.py");
    if py {
        suites.push(TestSuite {
            name: "pytest".into(),
            kind: "unit".into(),
            cmd: BuildCmd {
                program: "python3".into(),
                args: vec!["-m".into(), "pytest".into(), "-q".into()],
                label: "pytest".into(),
            },
        });
        suites.push(TestSuite {
            name: "ruff check".into(),
            kind: "lint".into(),
            cmd: BuildCmd {
                program: "python3".into(),
                args: vec!["-m".into(), "ruff".into(), "check".into(), ".".into()],
                label: "ruff check . (needs ruff installed)".into(),
            },
        });
        suites.push(TestSuite {
            name: "bandit".into(),
            kind: "security".into(),
            cmd: BuildCmd {
                program: "python3".into(),
                args: vec!["-m".into(), "bandit".into(), "-q".into(), "-r".into(), ".".into()],
                label: "bandit -r . (needs bandit installed)".into(),
            },
        });
    }

    if has("go.mod") {
        suites.push(TestSuite {
            name: "go test".into(),
            kind: "unit".into(),
            cmd: BuildCmd {
                program: "go".into(),
                args: vec!["test".into(), "./...".into()],
                label: "go test ./...".into(),
            },
        });
        suites.push(TestSuite {
            name: "go vet".into(),
            kind: "static-analysis".into(),
            cmd: BuildCmd {
                program: "go".into(),
                args: vec!["vet".into(), "./...".into()],
                label: "go vet ./...".into(),
            },
        });
        suites.push(TestSuite {
            name: "gofmt -l".into(),
            kind: "format".into(),
            cmd: BuildCmd {
                program: "gofmt".into(),
                args: vec!["-l".into(), ".".into()],
                label: "gofmt -l . (empty output = clean)".into(),
            },
        });
    }

    if has("pom.xml") {
        suites.push(TestSuite {
            name: "mvn test".into(),
            kind: "unit".into(),
            cmd: BuildCmd {
                program: "mvn".into(),
                args: vec!["-q".into(), "test".into()],
                label: "mvn test".into(),
            },
        });
    }
    if has("build.gradle") || has("build.gradle.kts") {
        let gradle = if has("gradlew") { "./gradlew" } else { "gradle" };
        suites.push(TestSuite {
            name: "gradle test".into(),
            kind: "unit".into(),
            cmd: BuildCmd {
                program: gradle.into(),
                args: vec!["test".into()],
                label: format!("{gradle} test"),
            },
        });
    }

    // e2e: Playwright / Cypress configs detected, run only when present.
    if has("playwright.config.ts") || has("playwright.config.js") {
        suites.push(TestSuite {
            name: "playwright test".into(),
            kind: "e2e".into(),
            cmd: BuildCmd {
                program: "npx".into(),
                args: vec!["--yes".into(), "playwright".into(), "test".into()],
                label: "playwright test".into(),
            },
        });
    }
    if has("cypress.config.ts") || has("cypress.config.js") {
        suites.push(TestSuite {
            name: "cypress run".into(),
            kind: "e2e".into(),
            cmd: BuildCmd {
                program: "npx".into(),
                args: vec!["--yes".into(), "cypress".into(), "run".into()],
                label: "cypress run".into(),
            },
        });
    }

    suites
}

fn run_sync(cmd: &BuildCmd, cwd: &Path) -> Result<(bool, String), String> {
    let out = std::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("failed to spawn `{}`: {e}", cmd.label))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// Frozen Phase 6 entry point. Runs every auto-detected test suite and
/// returns a human-readable summary. Results are also recorded in the `tests`
/// table (build_id left NULL for standalone runs).
pub(crate) fn run_tests_impl(root: &std::path::Path) -> Result<String, String> {
    let suites = detect_test_suites(root);
    if suites.is_empty() {
        return Err(format!(
            "no test tooling detected in {} (looked for package.json scripts, Cargo.toml, pytest config, go.mod, pom.xml/gradle, playwright/cypress configs)",
            root.display()
        ));
    }
    let mut parts: Vec<String> = Vec::new();
    let mut passed = 0u32;
    let mut failed = 0u32;
    for suite in &suites {
        let (ok, output) = match run_sync(&suite.cmd, root) {
            Ok(r) => r,
            Err(e) => {
                failed += 1;
                record_test_row(None, &suite.name, "error", &e);
                parts.push(format!("{} [{}]: ERROR — {e}", suite.name, suite.kind));
                continue;
            }
        };
        let tail: String = output.lines().rev().take(6).collect::<Vec<_>>().iter().rev().cloned().collect::<Vec<_>>().join("\n");
        let status = if ok { "PASS" } else { "FAIL" };
        if ok { passed += 1; } else { failed += 1; }
        record_test_row(None, &suite.name, if ok { "passed" } else { "failed" }, &tail);
        parts.push(format!("{} [{}]: {status}", suite.name, suite.kind));
    }
    Ok(format!(
        "test run: {passed} passed, {failed} failed ({} suites)\n{}",
        suites.len(),
        parts.join("\n")
    ))
}

fn record_test_row(build_id: Option<&str>, name: &str, status: &str, detail: &str) {
    if let Ok(conn) = db_conn() {
        let id = Uuid::new_v4().to_string();
        let detail: String = detail.chars().take(4000).collect();
        let _ = conn.execute(
            "INSERT INTO tests (id, build_id, name, status, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, build_id, name, status, detail],
        );
    }
}

// ---------------------------------------------------------------------------
// Project templates (master spec section 63). File contents are embedded so
// `template_instantiate` works offline. Each template is minimal but real:
// the generated project builds with its ecosystem's standard toolchain.
// `{{APP_NAME}}` is replaced with the sanitized app name,
// `{{APP_SNAKE}}` with the snake_case variant.
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct TemplateMeta {
    id: String,
    name: String,
    stack: String,
    description: String,
    file_count: usize,
}

fn template_files(id: &str) -> Option<Vec<(&'static str, &'static str)>> {
    match id {
        "react-vite-ts" => Some(vec![
            ("package.json", r#"{
  "name": "{{APP_NAME}}",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  },
  "devDependencies": {
    "@vitejs/plugin-react": "^4.0.0",
    "typescript": "^5.0.0",
    "vite": "^6.0.0"
  }
}
"#),
            ("vite.config.ts", r#"import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({ plugins: [react()] });
"#),
            ("tsconfig.json", r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "skipLibCheck": true,
    "noEmit": true,
    "types": ["vite/client"]
  },
  "include": ["src"]
}
"#),
            ("index.html", r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>{{APP_NAME}}</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#),
            ("src/main.tsx", r#"import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './index.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
"#),
            ("src/App.tsx", r#"import { useState } from 'react';

export default function App() {
  const [count, setCount] = useState(0);
  return (
    <main style={{ fontFamily: 'system-ui', padding: 32 }}>
      <h1>{{APP_NAME}}</h1>
      <p>Scaffolded by Guild Foundry AI.</p>
      <button onClick={() => setCount((c) => c + 1)}>count: {count}</button>
    </main>
  );
}
"#),
            ("src/index.css", r#"body { margin: 0; }
button { font-size: 1rem; padding: 0.5rem 1rem; cursor: pointer; }
"#),
        ]),
        "ts-library" => Some(vec![
            ("package.json", r#"{
  "name": "{{APP_NAME}}",
  "version": "0.1.0",
  "type": "module",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "scripts": {
    "build": "tsc",
    "test": "node --test dist/index.test.js"
  },
  "devDependencies": { "typescript": "^5.0.0" }
}
"#),
            ("tsconfig.json", r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "strict": true,
    "declaration": true,
    "outDir": "dist",
    "skipLibCheck": true
  },
  "include": ["src"]
}
"#),
            ("src/index.ts", r#"export function greet(name: string): string {
  return `Hello, ${name}!`;
}

export function add(a: number, b: number): number {
  return a + b;
}
"#),
            ("README.md", "# {{APP_NAME}}\n\nTypeScript library scaffolded by Guild Foundry AI.\n\n```sh\nnpm install\nnpm run build\n```\n"),
        ]),
        "python-cli" => Some(vec![
            ("pyproject.toml", r#"[project]
name = "{{APP_SNAKE}}"
version = "0.1.0"
description = "CLI scaffolded by Guild Foundry AI"
requires-python = ">=3.9"

[project.scripts]
{{APP_SNAKE}} = "app.main:main"
"#),
            ("src/app/main.py", r#"import argparse


def main() -> None:
    parser = argparse.ArgumentParser(prog="{{APP_SNAKE}}")
    parser.add_argument("--name", default="world", help="name to greet")
    args = parser.parse_args()
    print(f"Hello, {args.name}!")


if __name__ == "__main__":
    main()
"#),
            ("README.md", "# {{APP_NAME}}\n\nPython CLI scaffolded by Guild Foundry AI.\n\n```sh\npip install -e .\n{{APP_SNAKE}} --name Ada\n```\n"),
        ]),
        "rust-binary" => Some(vec![
            ("Cargo.toml", r#"[package]
name = "{{APP_SNAKE}}"
version = "0.1.0"
edition = "2021"

[profile.release]
opt-level = "s"
"#),
            ("src/main.rs", r#"use std::env;

fn main() {
    let name = env::args().nth(1).unwrap_or_else(|| "world".to_string());
    println!("Hello, {name}!");
}
"#),
        ]),
        "node-api" => Some(vec![
            ("package.json", r#"{
  "name": "{{APP_NAME}}",
  "version": "0.1.0",
  "type": "module",
  "scripts": { "start": "node src/index.js" }
}
"#),
            ("src/index.js", r#"import http from 'node:http';

const PORT = Number(process.env.PORT ?? 3000);

const server = http.createServer((req, res) => {
  res.setHeader('content-type', 'application/json');
  if (req.url === '/health') {
    res.end(JSON.stringify({ ok: true, app: '{{APP_NAME}}' }));
    return;
  }
  if (req.url === '/echo' && req.method === 'POST') {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => res.end(JSON.stringify({ echo: body })));
    return;
  }
  res.statusCode = 404;
  res.end(JSON.stringify({ error: 'not found' }));
});

server.listen(PORT, () => console.log(`{{APP_NAME}} listening on :${PORT}`));
"#),
        ]),
        "go-cli" => Some(vec![
            ("go.mod", "module {{APP_SNAKE}}\n\ngo 1.21\n"),
            ("main.go", r#"package main

import (
	"flag"
	"fmt"
)

func main() {
	name := flag.String("name", "world", "name to greet")
	flag.Parse()
	fmt.Printf("Hello, %s!\n", *name)
}
"#),
        ]),
        "java-app" => Some(vec![
            ("pom.xml", r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>app</groupId>
  <artifactId>{{APP_NAME}}</artifactId>
  <version>0.1.0</version>
  <properties>
    <maven.compiler.source>17</maven.compiler.source>
    <maven.compiler.target>17</maven.compiler.target>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
  </properties>
  <build>
    <plugins>
      <plugin>
        <groupId>org.codehaus.mojo</groupId>
        <artifactId>exec-maven-plugin</artifactId>
        <version>3.1.0</version>
        <configuration><mainClass>app.App</mainClass></configuration>
      </plugin>
    </plugins>
  </build>
</project>
"#),
            ("src/main/java/app/App.java", r#"package app;

public class App {
    public static void main(String[] args) {
        String name = args.length > 0 ? args[0] : "world";
        System.out.println("Hello, " + name + "!");
    }
}
"#),
        ]),
        "tauri-desktop" => Some(vec![
            ("package.json", r#"{
  "name": "{{APP_NAME}}",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "tauri": "tauri"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "typescript": "^5.0.0",
    "vite": "^6.0.0"
  }
}
"#),
            ("index.html", r#"<!doctype html>
<html lang="en">
  <head><meta charset="UTF-8" /><title>{{APP_NAME}}</title></head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
"#),
            ("src/main.ts", r#"document.getElementById('root')!.innerHTML =
  `<main style="font-family: system-ui; padding: 32px;">
     <h1>{{APP_NAME}}</h1>
     <p>Tauri desktop scaffold — run with <code>tauri dev</code>.</p>
   </main>`;
"#),
            ("src-tauri/Cargo.toml", r#"[package]
name = "{{APP_SNAKE}}"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = "2"

[dependencies]
tauri = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[features]
default = ["custom-protocol"]
custom-protocol = ["tauri/custom-protocol"]
"#),
            ("src-tauri/tauri.conf.json", r#"{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "{{APP_NAME}}",
  "version": "0.1.0",
  "identifier": "com.guildfoundry.{{APP_SNAKE}}",
  "build": {
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "app": { "windows": [{ "title": "{{APP_NAME}}", "width": 1100, "height": 750 }] },
  "bundle": { "active": true, "targets": "all" }
}
"#),
            ("src-tauri/src/main.rs", r#"// SPDX-License-Identifier: Apache-2.0
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run {{APP_NAME}}");
}
"#),
        ]),
        "python-fastapi" => Some(vec![
            ("pyproject.toml", r#"[project]
name = "{{APP_SNAKE}}"
version = "0.1.0"
description = "FastAPI scaffolded by Guild Foundry AI"
requires-python = ">=3.9"
dependencies = ["fastapi>=0.110", "uvicorn[standard]>=0.29"]
"#),
            ("main.py", r#"from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI(title="{{APP_NAME}}")


class Echo(BaseModel):
    message: str


@app.get("/health")
def health():
    return {"ok": True, "app": "{{APP_NAME}}"}


@app.post("/echo")
def echo(payload: Echo):
    return {"echo": payload.message}
"#),
        ]),
        "ai-agent" => Some(vec![
            ("pyproject.toml", r#"[project]
name = "{{APP_SNAKE}}"
version = "0.1.0"
description = "AI agent scaffolded by Guild Foundry AI"
requires-python = ">=3.9"
"#),
            ("tools.py", r#""""Tool registry for the agent scaffold."""
import datetime


def tool_echo(text: str) -> str:
    return text


def tool_time() -> str:
    return datetime.datetime.now().isoformat()


TOOLS = {
    "echo": (tool_echo, "Echo text back. Args: text"),
    "time": (tool_time, "Current local time. Args: none"),
}
"#),
            ("agent.py", r#""""Minimal AI agent scaffold.

Reads goals from stdin, dispatches to registered tools, prints results.
Wire a model provider into `respond()` to make it a real agent.
"""
from tools import TOOLS

SYSTEM_PROMPT = "You are {{APP_NAME}}, a helpful agent scaffold."


def respond(user_input: str) -> str:
    parts = user_input.strip().split(maxsplit=1)
    cmd, arg = parts[0], (parts[1] if len(parts) > 1 else "")
    if cmd in TOOLS:
        fn, _ = TOOLS[cmd]
        try:
            return str(fn(arg) if arg else fn())
        except TypeError:
            return str(fn())
    return f"Unknown tool '{cmd}'. Available: {', '.join(TOOLS)}"


def main() -> None:
    print(SYSTEM_PROMPT)
    print("Type '<tool> [args]', or 'quit'.")
    while True:
        try:
            line = input("> ")
        except EOFError:
            break
        if line.strip() in ("quit", "exit"):
            break
        print(respond(line))


if __name__ == "__main__":
    main()
"#),
            ("README.md", "# {{APP_NAME}}\n\nAI agent scaffolded by Guild Foundry AI.\n\n```sh\npython3 agent.py\n> time\n> echo hello\n```\n\nRegister tools in `tools.py`; wire a model provider into `respond()` in `agent.py`.\n"),
        ]),
        _ => None,
    }
}

fn template_meta(id: &str, name: &str, stack: &str, description: &str) -> Option<TemplateMeta> {
    template_files(id).map(|files| TemplateMeta {
        id: id.to_string(),
        name: name.to_string(),
        stack: stack.to_string(),
        description: description.to_string(),
        file_count: files.len(),
    })
}

fn all_templates() -> Vec<TemplateMeta> {
    [
        ("react-vite-ts", "React + Vite + TS", "React 19 / Vite / TypeScript", "Single-page web app with hot reload and typed components."),
        ("ts-library", "TypeScript Library", "TypeScript / Node", "Publishable npm library with declarations and a build script."),
        ("python-cli", "Python CLI", "Python / argparse", "Command-line tool with an installable console entry point."),
        ("rust-binary", "Rust Binary", "Rust / Cargo", "Native CLI binary; `cargo build --release` for a stripped artifact."),
        ("node-api", "Node.js API", "Node.js (no deps)", "Zero-dependency HTTP JSON API with /health and /echo routes."),
        ("go-cli", "Go CLI", "Go", "Single-binary command-line tool with flag parsing."),
        ("java-app", "Java App", "Java 17 / Maven", "Maven project with an executable main class."),
        ("tauri-desktop", "Tauri Desktop", "Tauri 2 / Vite / TS", "Native desktop app shell; `tauri dev` / `tauri build`."),
        ("python-fastapi", "Python API (FastAPI)", "Python / FastAPI", "Typed HTTP API; run with `uvicorn main:app`."),
        ("ai-agent", "AI Agent Scaffold", "Python", "REPL agent with a tool registry; wire in a model provider."),
    ]
    .iter()
    .filter_map(|(id, name, stack, desc)| template_meta(id, name, stack, desc))
    .collect()
}

fn sanitize_app_name(name: &str) -> Result<String, String> {
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if clean.is_empty() || clean.len() > 64 {
        return Err("app_name must be 1-64 chars of [A-Za-z0-9_-]".to_string());
    }
    Ok(clean)
}

fn instantiate_template(id: &str, dest_root: &Path, app_name: &str) -> Result<PathBuf, String> {
    let files = template_files(id).ok_or_else(|| format!("unknown template: {id}"))?;
    if !dest_root.is_dir() {
        return Err(format!("dest_root is not a directory: {}", dest_root.display()));
    }
    let app_name = sanitize_app_name(app_name)?;
    let snake = app_name.to_lowercase().replace('-', "_");
    let target = dest_root.join(&app_name);
    if target.exists() {
        return Err(format!("target already exists: {}", target.display()));
    }
    for (rel, content) in &files {
        let rendered = content
            .replace("{{APP_NAME}}", &app_name)
            .replace("{{APP_SNAKE}}", &snake);
        let path = target.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
        }
        std::fs::write(&path, rendered).map_err(|e| format!("write failed: {e}"))?;
    }
    Ok(target)
}

// ---------------------------------------------------------------------------
// Artifacts + releases
// ---------------------------------------------------------------------------

fn sha256_of_file(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("read failed: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hex::encode(hasher.finalize()))
}

/// Copy a file into the artifact store under `<store>/<build_id>/<name>`.
fn store_artifact_file(store: &Path, build_id: &str, src: &Path) -> Result<PathBuf, String> {
    let name = src
        .file_name()
        .ok_or_else(|| "artifact path has no file name".to_string())?;
    let dir = store.join(build_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("artifact dir failed: {e}"))?;
    let dest = dir.join(name);
    std::fs::copy(src, &dest).map_err(|e| format!("artifact copy failed: {e}"))?;
    Ok(dest)
}

#[derive(Serialize, Clone)]
pub struct ArtifactMeta {
    id: String,
    build_id: Option<String>,
    kind: String,
    path: Option<String>,
    sha256: Option<String>,
    created_at: String,
}

fn artifact_create_impl(build_id: &str, kind: &str, path: &Path) -> Result<String, String> {
    let store = artifact_store_dir()?;
    let sha = sha256_of_file(path)?;
    let stored = store_artifact_file(&store, build_id, path)?;
    let conn = db_conn()?;
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM builds WHERE id = ?1",
            rusqlite::params![build_id],
            |r| r.get(0),
        )
        .ok();
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO artifacts (id, project_id, kind, source, version, path, sha256, created_at) VALUES (?1, ?2, ?3, 'build-engine', 1, ?4, ?5, ?6)",
        rusqlite::params![
            id,
            project_id,
            kind,
            stored.to_string_lossy().to_string(),
            sha,
            now_rfc3339()
        ],
    )
    .map_err(|e| format!("artifact insert failed: {e}"))?;
    Ok(id)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Minimal ZIP writer (stored, no compression) for a single file. Pure Rust —
/// no extra crate needed.
fn write_minimal_zip(zip_path: &Path, entry_name: &str, data: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut f =
        std::fs::File::create(zip_path).map_err(|e| format!("zip create failed: {e}"))?;
    let name = entry_name.as_bytes();
    let crc = crc32(data);
    let len = data.len() as u32;
    let w = |f: &mut std::fs::File, b: &[u8]| f.write_all(b).map_err(|e| e.to_string());

    // Local file header
    w(&mut f, &0x0403_4B50u32.to_le_bytes())?;
    w(&mut f, &20u16.to_le_bytes())?; // version needed
    w(&mut f, &0u16.to_le_bytes())?; // flags
    w(&mut f, &0u16.to_le_bytes())?; // method: stored
    w(&mut f, &0u16.to_le_bytes())?; // mod time
    w(&mut f, &0x21u16.to_le_bytes())?; // mod date (1980-01-01)
    w(&mut f, &crc.to_le_bytes())?;
    w(&mut f, &len.to_le_bytes())?;
    w(&mut f, &len.to_le_bytes())?;
    w(&mut f, &(name.len() as u16).to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?; // extra len
    w(&mut f, name)?;
    w(&mut f, data)?;
    let central_offset = 30 + name.len() as u64 + data.len() as u64;

    // Central directory entry
    w(&mut f, &0x0201_4B50u32.to_le_bytes())?;
    w(&mut f, &20u16.to_le_bytes())?; // version made by
    w(&mut f, &20u16.to_le_bytes())?; // version needed
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0x21u16.to_le_bytes())?;
    w(&mut f, &crc.to_le_bytes())?;
    w(&mut f, &len.to_le_bytes())?;
    w(&mut f, &len.to_le_bytes())?;
    w(&mut f, &(name.len() as u16).to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u32.to_le_bytes())?; // external attrs
    w(&mut f, &0u32.to_le_bytes())?; // local header offset
    w(&mut f, name)?;

    // End of central directory
    let central_size = (46 + name.len()) as u32;
    w(&mut f, &0x0605_4B50u32.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    w(&mut f, &1u16.to_le_bytes())?;
    w(&mut f, &1u16.to_le_bytes())?;
    w(&mut f, &central_size.to_le_bytes())?;
    w(&mut f, &(central_offset as u32).to_le_bytes())?;
    w(&mut f, &0u16.to_le_bytes())?;
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct ReleaseMeta {
    id: String,
    build_id: String,
    version: String,
    notes: String,
    created_at: String,
}

// ---------------------------------------------------------------------------
// Tauri commands (Phase 8 IPC surface)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct BuildStatus {
    state: String,
    log_tail: Vec<String>,
    attempts: u32,
    error_signature: Option<String>,
    failure_report_path: Option<String>,
}

#[tauri::command]
pub(crate) fn build_start(
    app: tauri::AppHandle,
    project_root: String,
    target: String,
) -> Result<String, String> {
    init_build_engine(&app)?;
    build_start_impl(Path::new(&project_root), &target)
}

#[tauri::command]
pub(crate) fn build_status(
    app: tauri::AppHandle,
    build_id: String,
) -> Result<BuildStatus, String> {
    init_build_engine(&app)?;
    let s = get_state(&build_id)?;
    Ok(BuildStatus {
        state: s.phase.as_str().to_string(),
        log_tail: s.log_tail,
        attempts: s.attempts,
        error_signature: s.error_signature,
        failure_report_path: s.failure_report_path,
    })
}

#[tauri::command]
pub(crate) fn build_cancel(app: tauri::AppHandle, build_id: String) -> Result<(), String> {
    init_build_engine(&app)?;
    let s = get_state(&build_id)?;
    s.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub(crate) async fn test_run(
    app: tauri::AppHandle,
    project_root: String,
) -> Result<String, String> {
    init_build_engine(&app)?;
    let root = PathBuf::from(project_root);
    tokio::task::spawn_blocking(move || run_tests_impl(&root))
        .await
        .map_err(|e| format!("test task failed: {e}"))?
}

#[tauri::command]
pub(crate) fn template_list() -> Vec<TemplateMeta> {
    all_templates()
}

#[tauri::command]
pub(crate) fn template_instantiate(
    template_id: String,
    dest_root: String,
    app_name: String,
) -> Result<String, String> {
    let target = instantiate_template(&template_id, Path::new(&dest_root), &app_name)?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
pub(crate) fn artifact_create(
    app: tauri::AppHandle,
    build_id: String,
    kind: String,
    path: String,
) -> Result<String, String> {
    init_build_engine(&app)?;
    artifact_create_impl(&build_id, &kind, Path::new(&path))
}

#[tauri::command]
pub(crate) fn artifact_list(
    app: tauri::AppHandle,
    build_id: Option<String>,
) -> Result<Vec<ArtifactMeta>, String> {
    init_build_engine(&app)?;
    let conn = db_conn()?;
    let rows: Vec<ArtifactMeta> = if let Some(bid) = build_id {
        let like = format!("%artifacts/{bid}/%");
        let collected = {
            let mut stmt = conn
                .prepare("SELECT id, kind, path, sha256, created_at FROM artifacts WHERE path LIKE ?1 ORDER BY created_at DESC")
                .map_err(|e| format!("artifact query failed: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![like], |r| {
                    Ok(ArtifactMeta {
                        id: r.get(0)?,
                        build_id: Some(bid.clone()),
                        kind: r.get(1)?,
                        path: r.get(2)?,
                        sha256: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                })
                .map_err(|e| format!("artifact rows failed: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("artifact rows failed: {e}"))?;
            rows
        };
        collected
    } else {
        let collected = {
            let mut stmt = conn
                .prepare("SELECT id, kind, path, sha256, created_at FROM artifacts ORDER BY created_at DESC LIMIT 200")
                .map_err(|e| format!("artifact query failed: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![], |r| {
                    Ok(ArtifactMeta {
                        id: r.get(0)?,
                        build_id: None,
                        kind: r.get(1)?,
                        path: r.get(2)?,
                        sha256: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                })
                .map_err(|e| format!("artifact rows failed: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("artifact rows failed: {e}"))?;
            rows
        };
        collected
    };
    Ok(rows)
}

#[derive(Serialize)]
pub struct ArtifactDetail {
    id: String,
    kind: String,
    path: Option<String>,
    sha256: Option<String>,
    created_at: String,
}

#[tauri::command]
pub(crate) fn artifact_get(
    app: tauri::AppHandle,
    artifact_id: String,
) -> Result<ArtifactDetail, String> {
    init_build_engine(&app)?;
    let conn = db_conn()?;
    conn.query_row(
        "SELECT id, kind, path, sha256, created_at FROM artifacts WHERE id = ?1",
        rusqlite::params![artifact_id.as_str()],
        |r| {
            Ok(ArtifactDetail {
                id: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                sha256: r.get(3)?,
                created_at: r.get(4)?,
            })
        },
    )
    .map_err(|e| format!("artifact not found: {e}"))
}

#[derive(Serialize)]
pub struct VerifyResult {
    ok: bool,
    expected: Option<String>,
    actual: Option<String>,
}

#[tauri::command]
pub(crate) fn artifact_verify(
    app: tauri::AppHandle,
    artifact_id: String,
) -> Result<VerifyResult, String> {
    init_build_engine(&app)?;
    let detail = artifact_get(app, artifact_id)?;
    let path = detail
        .path
        .ok_or_else(|| "artifact has no stored path".to_string())?;
    let actual = sha256_of_file(Path::new(&path)).ok();
    Ok(VerifyResult {
        ok: actual.is_some() && actual == detail.sha256,
        expected: detail.sha256,
        actual,
    })
}

#[derive(Serialize)]
pub struct ExportResult {
    path: String,
}

#[tauri::command]
pub(crate) fn artifact_export(
    app: tauri::AppHandle,
    artifact_id: String,
) -> Result<ExportResult, String> {
    init_build_engine(&app)?;
    let detail = artifact_get(app.clone(), artifact_id.clone())?;
    let src = detail
        .path
        .clone()
        .ok_or_else(|| "artifact has no stored path".to_string())?;
    let data = std::fs::read(&src).map_err(|e| format!("read failed: {e}"))?;
    let entry_name = Path::new(&src)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{artifact_id}.bin"));
    let store = artifact_store_dir()?;
    let zip_path = store.join(format!("{artifact_id}.zip"));
    write_minimal_zip(&zip_path, &entry_name, &data)?;
    Ok(ExportResult {
        path: zip_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub(crate) fn release_create(
    app: tauri::AppHandle,
    build_id: String,
    version: String,
    notes: String,
) -> Result<ReleaseMeta, String> {
    init_build_engine(&app)?;
    if version.trim().is_empty() {
        return Err("version must not be empty".to_string());
    }
    let conn = db_conn()?;
    // `releases` comes from MIGRATION_BUILD_V2; create defensively in case the
    // migration has not been wired into db.rs yet.
    conn.execute_batch(MIGRATION_BUILD_V2)
        .map_err(|e| format!("releases table unavailable: {e}"))?;
    let id = Uuid::new_v4().to_string();
    let created = now_rfc3339();
    conn.execute(
        "INSERT INTO releases (id, build_id, version, notes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, build_id, version, notes, created],
    )
    .map_err(|e| format!("release insert failed: {e}"))?;
    Ok(ReleaseMeta {
        id,
        build_id,
        version,
        notes,
        created_at: created,
    })
}
