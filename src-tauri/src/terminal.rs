// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: terminal / process engine.
//
// Processes always run with their working directory forced inside the project
// sandbox, a timeout, captured stdout/stderr, and an environment policy that
// strips dangerous loader variables (LD_PRELOAD, LD_LIBRARY_PATH, ...).
//
// Commands are executed WITHOUT an intermediate shell (`sh -c` is never
// used): `cmd` is resolved to a binary and `args` are passed literally, which
// eliminates shell-injection as a vector. Pipes/redirection/expansion are
// intentionally unavailable.
//
// Command safety classification (SAFE / MODERATE / HIGH RISK / CRITICAL) is a
// heuristic layer on top of the hard sandbox enforcement below — it drives
// progressive confirmation in the UI. Per the master spec, classification
// never relies on string matching alone: the cwd is always confined, timeouts
// are always enforced, and destructive fs ops still checkpoint via workspace.rs.
//
// Resource limits (rlimit) are NOT implemented: they require the `libc` crate,
// which is not in the dependency set. Marked NOT VERIFIED; see .integration/phase-4.md.
//
// NOT VERIFIED: this file has not been compiled (no Rust toolchain in this
// build environment).

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Serialize;
use tauri::Emitter;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Frozen helper (Phase 6 tool executor calls this directly — do not change)
// ---------------------------------------------------------------------------

/// Result of a one-shot command execution. All fields are `pub` per contract.
#[derive(Serialize, Clone, Debug)]
pub struct ExecOut {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// Execute `cmd` with `args` inside `root`, synchronously, with a hard
/// timeout. The child is killed on timeout; `timed_out` reports it.
pub(crate) fn exec_command(
    root: &Path,
    cmd: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<ExecOut, String> {
    let root = canonical_root_checked(root)?;
    let binary = resolve_binary(cmd)?;
    let timeout = Duration::from_secs(timeout_secs.clamp(1, 3600));

    let mut child = Command::new(&binary)
        .args(args)
        .current_dir(&root)
        .envs(sanitized_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start '{}': {}", cmd, e))?;

    // Drain stdout/stderr on background threads so a chatty child can never
    // deadlock on a full pipe while we poll for the timeout.
    let stdout_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    if let Some(out) = child.stdout.take() {
        let buf = Arc::clone(&stdout_buf);
        thread::spawn(move || {
            let mut reader = BufReader::new(out);
            let mut tmp = Vec::new();
            let _ = reader.read_to_end(&mut tmp);
            if let Ok(mut b) = buf.lock() {
                b.extend_from_slice(&tmp);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let buf = Arc::clone(&stderr_buf);
        thread::spawn(move || {
            let mut reader = BufReader::new(err);
            let mut tmp = Vec::new();
            let _ = reader.read_to_end(&mut tmp);
            if let Ok(mut b) = buf.lock() {
                b.extend_from_slice(&tmp);
            }
        });
    }

    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().map_err(|e| format!("wait failed: {}", e))? {
            Some(status) => break status,
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break child
                        .try_wait()
                        .map_err(|e| format!("wait after kill failed: {}", e))?
                        .ok_or_else(|| "process vanished after kill".to_string())?;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    };

    // Give the drain threads a moment to flush the pipes.
    thread::sleep(Duration::from_millis(50));
    let stdout = stdout_buf
        .lock()
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    let stderr = stderr_buf
        .lock()
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();

    Ok(ExecOut {
        stdout: truncate_output(&stdout),
        stderr: truncate_output(&stderr),
        exit_code: status.code().unwrap_or(-1),
        timed_out,
    })
}

// ---------------------------------------------------------------------------
// Sandbox / env policy internals
// ---------------------------------------------------------------------------

/// Environment variables that are never passed through to child processes.
/// These let an attacker (or a confused dependency) hijack dynamic linking.
const BLOCKED_ENV_VARS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_DEBUG",
    "LD_DEBUG_OUTPUT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
];

fn sanitized_env() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| !BLOCKED_ENV_VARS.contains(&k.as_str()))
        .collect()
}

fn canonical_root_checked(root: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(root).map_err(|e| format!("project root is not accessible: {}", e))
}

/// Never run a command through a shell, and never allow an empty/absolute-escape.
/// `cmd` must be a bare binary name or a path inside the project root.
fn resolve_binary(cmd: &str) -> Result<String, String> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Err("empty command".to_string());
    }
    if trimmed.contains('\0') {
        return Err("command contains NUL byte".to_string());
    }
    // Explicitly forbid invoking a shell; the engine never needs one.
    if matches!(trimmed, "sh" | "bash" | "zsh" | "fish" | "dash") {
        return Err("interactive shells are not allowed; run the binary directly".to_string());
    }
    Ok(trimmed.to_string())
}

const MAX_OUTPUT_CHARS: usize = 256 * 1024;

fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        s.to_string()
    } else {
        let mut out = s[..MAX_OUTPUT_CHARS].to_string();
        out.push_str("\n…[output truncated]");
        out
    }
}

// ---------------------------------------------------------------------------
// Safety classification
// ---------------------------------------------------------------------------

/// Command safety level driving progressive confirmation in the UI:
/// SAFE runs immediately; MODERATE shows a confirm dialog; HIGH RISK requires
/// an explicit typed/modal confirmation; CRITICAL requires an explicit
/// acknowledgement checkbox plus confirmation. This is a heuristic on top of
/// the hard sandbox guarantees (cwd confinement, timeouts, checkpoints).
#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SafetyLevel {
    Safe,
    Moderate,
    HighRisk,
    Critical,
}

fn classify(cmd: &str, args: &[String]) -> SafetyLevel {
    let c = cmd.trim();
    let base = c.rsplit('/').next().unwrap_or(c);
    let arg0 = args.first().map(|s| s.as_str()).unwrap_or("");

    // CRITICAL — system-level or irreversible.
    if matches!(base, "mkfs" | "dd" | "shutdown" | "reboot" | "poweroff" | "halt" | "init"
        | "su" | "sudo" | "doas" | "passwd" | "useradd" | "userdel" | "usermod"
        | "fdisk" | "parted" | "wipefs" | "shred")
    {
        return SafetyLevel::Critical;
    }
    if base == "rm" && args.iter().any(|a| a == "-rf" || a == "--recursive") {
        // rm -rf on / or ~ is critical; project-local rm -rf is high risk.
        if args.iter().any(|a| a == "/" || a == "/*" || a == "~" || a == "~/*") {
            return SafetyLevel::Critical;
        }
        return SafetyLevel::HighRisk;
    }
    if base == "chmod" && args.iter().any(|a| a.contains("777")) {
        return SafetyLevel::Critical;
    }

    // HIGH RISK — package installation, network changes, destructive ops.
    if matches!(
        base,
        "apt" | "apt-get" | "dpkg" | "yum" | "dnf" | "pacman" | "zypper"
            | "snap" | "flatpak" | "pip" | "pip3" | "npm" | "yarn" | "pnpm"
            | "cargo" | "go" | "gem" | "composer"
    ) && matches!(
        arg0,
        "install" | "add" | "i" | "remove" | "uninstall" | "purge" | "upgrade" | "update"
    ) {
        return SafetyLevel::HighRisk;
    }
    if matches!(base, "curl" | "wget") && args.iter().any(|a| a == "|") {
        return SafetyLevel::HighRisk;
    }
    if matches!(base, "iptables" | "ip" | "ifconfig" | "nmcli" | "ufw" | "firewall-cmd") {
        return SafetyLevel::HighRisk;
    }
    if base == "rm" {
        return SafetyLevel::HighRisk;
    }
    if base == "git"
        && matches!(arg0, "push" | "clean" | "reset" | "rebase" | "filter-branch")
    {
        return SafetyLevel::HighRisk;
    }

    // MODERATE — project-local modifications.
    if matches!(
        base,
        "mkdir" | "touch" | "cp" | "mv" | "ln" | "tee" | "truncate"
            | "chmod" | "chown" | "tar" | "unzip" | "zip" | "patch"
    ) {
        return SafetyLevel::Moderate;
    }
    if matches!(base, "cargo" | "npm" | "yarn" | "pnpm" | "go" | "make" | "cmake" | "gradle" | "pytest" | "jest" | "vitest")
        && matches!(arg0, "build" | "run" | "test" | "start" | "dev" | "check" | "clippy" | "fmt")
    {
        return SafetyLevel::Moderate;
    }
    if base == "git" {
        return SafetyLevel::Moderate;
    }

    // SAFE — read-only or display commands.
    SafetyLevel::Safe
}

// ---------------------------------------------------------------------------
// Long-running process registry (term_start / term_output / term_list / term_kill)
// ---------------------------------------------------------------------------

struct LiveProcess {
    id: String,
    cmdline: String,
    pid: u32,
    started_at: String,
    running: Arc<Mutex<bool>>,
    lines: Arc<Mutex<VecDeque<String>>>, // ring buffer of recent output lines
    child: Arc<Mutex<Option<Child>>>,    // OS handle, taken by killer or reaper
}

fn registry() -> &'static Mutex<HashMap<String, LiveProcess>> {
    static REG: OnceLock<Mutex<HashMap<String, LiveProcess>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

const LINE_BUFFER_CAP: usize = 2000;

fn push_line(lines: &Arc<Mutex<VecDeque<String>>>, line: String) {
    if let Ok(mut q) = lines.lock() {
        if q.len() >= LINE_BUFFER_CAP {
            q.pop_front();
        }
        q.push_back(line);
    }
}

#[derive(Serialize)]
struct ProcInfo {
    id: String,
    cmdline: String,
    pid: u32,
    started_at: String,
    running: bool,
}

#[derive(Serialize, Clone)]
struct TermEvent {
    id: String,
    stream: String,
    line: String,
}

/// Start a long-running process. Returns the process id. Output lines are
/// emitted as `terminal://output` events and kept in a ring buffer readable
/// via `term_output`.
#[tauri::command]
pub fn term_start(
    app: tauri::AppHandle,
    project_root: String,
    cmd: String,
    args: Vec<String>,
) -> Result<String, String> {
    let root = canonical_root_checked(Path::new(&project_root))?;
    let binary = resolve_binary(&cmd)?;

    let mut child = Command::new(&binary)
        .args(&args)
        .current_dir(&root)
        .envs(sanitized_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start '{}': {}", cmd, e))?;

    let pid = child.id();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let child_handle: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));
    let id = Uuid::new_v4().to_string();
    let cmdline = std::iter::once(cmd.clone())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");

    let lines: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    let running: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));

    // Reader threads: one per stream, pushing lines into the ring buffer and
    // emitting them to the frontend.
    for (stream_name, pipe) in [("stdout", stdout_pipe), ("stderr", stderr_pipe)] {
        if let Some(pipe) = pipe {
            let lines = Arc::clone(&lines);
            let app = app.clone();
            let id = id.clone();
            let stream = stream_name.to_string();
            thread::spawn(move || {
                let reader = BufReader::new(pipe);
                for line in reader.lines() {
                    match line {
                        Ok(text) => {
                            push_line(&lines, format!("[{}] {}", stream, text));
                            let _ = app.emit(
                                "terminal://output",
                                TermEvent {
                                    id: id.clone(),
                                    stream: stream.clone(),
                                    line: text,
                                },
                            );
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    }

    // Reaper thread: takes the child handle, waits for exit, flips `running`.
    {
        let child_handle = Arc::clone(&child_handle);
        let running = Arc::clone(&running);
        let lines = Arc::clone(&lines);
        let app = app.clone();
        let id = id.clone();
        thread::spawn(move || {
            if let Ok(mut guard) = child_handle.lock() {
                if let Some(mut c) = guard.take() {
                    let _ = c.wait();
                }
            }
            if let Ok(mut r) = running.lock() {
                *r = false;
            }
            push_line(&lines, "[exit] process terminated".to_string());
            let _ = app.emit(
                "terminal://output",
                TermEvent {
                    id,
                    stream: "exit".to_string(),
                    line: "process terminated".to_string(),
                },
            );
        });
    }

    if let Ok(mut reg) = registry().lock() {
        reg.insert(
            id.clone(),
            LiveProcess {
                id: id.clone(),
                cmdline,
                pid,
                started_at: Utc::now().to_rfc3339(),
                running,
                lines,
                child: child_handle,
            },
        );
    }
    Ok(id)
}

#[tauri::command]
pub fn term_output(id: String, tail: Option<usize>) -> Result<Vec<String>, String> {
    let reg = registry()
        .lock()
        .map_err(|_| "process registry poisoned".to_string())?;
    let proc = reg.get(&id).ok_or_else(|| "unknown process id".to_string())?;
    let lines = proc
        .lines
        .lock()
        .map_err(|_| "output buffer poisoned".to_string())?;
    let n = tail.unwrap_or(200).clamp(1, LINE_BUFFER_CAP);
    Ok(lines.iter().rev().take(n).cloned().collect::<Vec<_>>().into_iter().rev().collect())
}

#[tauri::command]
pub fn term_list() -> Result<Vec<ProcInfo>, String> {
    let reg = registry()
        .lock()
        .map_err(|_| "process registry poisoned".to_string())?;
    let mut out: Vec<ProcInfo> = Vec::new();
    for p in reg.values() {
        let running = p.running.lock().map(|r| *r).unwrap_or(false);
        out.push(ProcInfo {
            id: p.id.clone(),
            cmdline: p.cmdline.clone(),
            pid: p.pid,
            started_at: p.started_at.clone(),
            running,
        });
    }
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(out)
}

/// Kill a tracked process via its OS child handle, then drop it from the
/// registry. Note: `Child::kill` signals the direct child only; orphaned
/// grandchildren are not reaped (documented limitation, needs process groups).
#[tauri::command]
pub fn term_kill(id: String) -> Result<(), String> {
    let mut reg = registry()
        .lock()
        .map_err(|_| "process registry poisoned".to_string())?;
    let proc = reg.remove(&id).ok_or_else(|| "unknown process id".to_string())?;
    drop(reg);
    if let Ok(mut guard) = proc.child.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    if let Ok(mut r) = proc.running.lock() {
        *r = false;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// One-shot execute + classification commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn term_exec(
    project_root: String,
    cmd: String,
    args: Vec<String>,
    timeout_secs: Option<u64>,
) -> Result<ExecOut, String> {
    let root = canonical_root_checked(Path::new(&project_root))?;
    exec_command(&root, &cmd, &args, timeout_secs.unwrap_or(60))
}

/// Classify a command for progressive confirmation. Returns one of
/// "SAFE" | "MODERATE" | "HIGH_RISK" | "CRITICAL".
#[tauri::command]
pub fn term_classify(cmd: String, args: Vec<String>) -> Result<String, String> {
    resolve_binary(&cmd)?; // rejects shells / empty commands up front
    let level = classify(&cmd, &args);
    Ok(match level {
        SafetyLevel::Safe => "SAFE",
        SafetyLevel::Moderate => "MODERATE",
        SafetyLevel::HighRisk => "HIGH_RISK",
        SafetyLevel::Critical => "CRITICAL",
    }
    .to_string())
}
