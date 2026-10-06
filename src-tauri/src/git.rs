// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: Git integration.
//
// All operations shell out to the system `git` binary via
// tokio::process::Command with the working directory forced inside the
// project sandbox. If `git` is not installed, every command returns a clear
// error. Push is NEVER automatic: `git_push` refuses to run unless
// `confirmed` is true; the UI only sets it after a confirmation modal that
// shows remote, branch, commits, changed files, and a diff summary.
//
// NOT VERIFIED: this file has not been compiled (no Rust toolchain in this
// build environment).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn canonical_root_checked(root: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(root).map_err(|e| format!("project root is not accessible: {}", e))
}

async fn ensure_git() -> Result<(), String> {
    let out = Command::new("git")
        .arg("--version")
        .output()
        .await
        .map_err(|e| {
            format!(
                "git is not installed or not on PATH ({}); Git features are unavailable",
                e
            )
        })?;
    if !out.status.success() {
        return Err("git --version failed; Git features are unavailable".to_string());
    }
    Ok(())
}

struct GitOut {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
}

/// Run `git <args>` in `root` with a 120 s timeout. Returns an Err carrying
/// git's stderr on failure.
async fn run_git(root: &Path, args: &[&str]) -> Result<GitOut, String> {
    ensure_git().await?;
    let root = canonical_root_checked(root)?;
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(&root)
        // Never prompt for credentials interactively; fail fast instead.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "echo");
    let out = tokio::time::timeout(Duration::from_secs(120), cmd.output())
        .await
        .map_err(|_| "git command timed out after 120s".to_string())?
        .map_err(|e| format!("cannot run git: {}", e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim().chars().take(2000).collect::<String>()
        ));
    }
    Ok(GitOut { stdout, stderr })
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct GitFileStatus {
    path: String,
    staged: String,   // porcelain index status char
    worktree: String, // porcelain worktree status char
}

#[derive(Serialize)]
struct GitCommit {
    hash: String,
    short: String,
    author: String,
    date: String,
    message: String,
}

#[derive(Serialize)]
struct GitBranch {
    name: String,
    current: bool,
    upstream: String,
}

#[derive(Serialize)]
struct GitRemote {
    name: String,
    url: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn git_status(
    project_root: String,
) -> Result<Vec<GitFileStatus>, String> {
    let out = run_git(
        Path::new(&project_root),
        &["status", "--porcelain=v1", "-uall", "--no-renames"],
    )
    .await?;
    let mut files = Vec::new();
    for line in out.stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let mut chars = line.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let path = line[3..].trim().to_string();
        files.push(GitFileStatus {
            path,
            staged: x.to_string(),
            worktree: y.to_string(),
        });
    }
    Ok(files)
}

#[tauri::command]
pub async fn git_diff(
    project_root: String,
    staged: bool,
    path: Option<String>,
) -> Result<String, String> {
    let mut args: Vec<&str> = vec!["diff", "--no-color"];
    if staged {
        args.push("--staged");
    }
    let path_owned;
    if let Some(p) = path.as_ref() {
        // `p` is validated as sandbox-relative by the caller (Phase 6 tools);
        // here we only refuse absolute paths and traversal.
        if p.starts_with('/') || p.contains("..") {
            return Err("invalid path for git diff".to_string());
        }
        args.push("--");
        path_owned = p.clone();
        args.push(&path_owned);
    }
    let out = run_git(Path::new(&project_root), &args).await?;
    Ok(out.stdout)
}

#[tauri::command]
pub async fn git_log(
    project_root: String,
    limit: Option<u32>,
) -> Result<Vec<GitCommit>, String> {
    let n = limit.unwrap_or(30).clamp(1, 200).to_string();
    let out = run_git(
        Path::new(&project_root),
        &[
            "log",
            &format!("-n{}", n),
            "--pretty=format:%H%x1f%h%x1f%an%x1f%ad%x1f%s",
            "--date=iso",
        ],
    )
    .await?;
    let mut commits = Vec::new();
    for line in out.stdout.lines() {
        let parts: Vec<&str> = line.split('\x1f').collect();
        if parts.len() != 5 {
            continue;
        }
        commits.push(GitCommit {
            hash: parts[0].to_string(),
            short: parts[1].to_string(),
            author: parts[2].to_string(),
            date: parts[3].to_string(),
            message: parts[4].to_string(),
        });
    }
    Ok(commits)
}

#[tauri::command]
pub async fn git_branch(
    project_root: String,
) -> Result<Vec<GitBranch>, String> {
    let out = run_git(
        Path::new(&project_root),
        &[
            "branch",
            "--format=%(HEAD)%1f%(refname:short)%1f%(upstream:short)",
        ],
    )
    .await?;
    let mut branches = Vec::new();
    for line in out.stdout.lines() {
        let parts: Vec<&str> = line.split('\x1f').collect();
        if parts.len() != 3 {
            continue;
        }
        branches.push(GitBranch {
            current: parts[0] == "*",
            name: parts[1].to_string(),
            upstream: parts[2].to_string(),
        });
    }
    Ok(branches)
}

#[tauri::command]
pub async fn git_commit(
    project_root: String,
    message: String,
) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err("commit message is empty".to_string());
    }
    run_git(Path::new(&project_root), &["commit", "-m", message.trim()]).await?;
    let out = run_git(Path::new(&project_root), &["rev-parse", "HEAD"]).await?;
    Ok(out.stdout.trim().to_string())
}

#[tauri::command]
pub async fn git_stash(
    project_root: String,
    message: Option<String>,
    include_untracked: bool,
) -> Result<String, String> {
    let mut owned: Vec<String> = vec!["stash".to_string(), "push".to_string()];
    if include_untracked {
        owned.push("-u".to_string());
    }
    if let Some(m) = message {
        if !m.trim().is_empty() {
            owned.push("-m".to_string());
            owned.push(m.trim().to_string());
        }
    }
    let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let out = run_git(Path::new(&project_root), &refs).await?;
    Ok(out.stdout.trim().to_string())
}

#[tauri::command]
pub async fn git_checkout(
    project_root: String,
    target: String,
) -> Result<String, String> {
    if target.trim().is_empty() || target.contains("..") || target.starts_with('-') {
        return Err("invalid checkout target".to_string());
    }
    let out = run_git(Path::new(&project_root), &["checkout", target.trim()]).await?;
    Ok(out.stdout.trim().to_string())
}

#[tauri::command]
pub async fn git_merge(
    project_root: String,
    branch: String,
) -> Result<String, String> {
    if branch.trim().is_empty() || branch.contains("..") || branch.starts_with('-') {
        return Err("invalid merge branch".to_string());
    }
    let out = run_git(
        Path::new(&project_root),
        &["merge", "--no-edit", branch.trim()],
    )
    .await?;
    Ok(format!("{}{}", out.stdout, out.stderr).trim().to_string())
}

#[tauri::command]
pub async fn git_remote(
    project_root: String,
) -> Result<Vec<GitRemote>, String> {
    let out = run_git(Path::new(&project_root), &["remote", "-v"]).await?;
    let mut seen = std::collections::HashSet::new();
    let mut remotes = Vec::new();
    for line in out.stdout.lines() {
        // "origin\tgit@host:repo.git (fetch)"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 || !line.contains("(fetch)") {
            continue;
        }
        if seen.insert(parts[0].to_string()) {
            remotes.push(GitRemote {
                name: parts[0].to_string(),
                url: parts[1].to_string(),
            });
        }
    }
    Ok(remotes)
}

#[tauri::command]
pub async fn git_pull(
    project_root: String,
    remote: String,
    branch: String,
) -> Result<String, String> {
    if remote.trim().is_empty() || branch.trim().is_empty() {
        return Err("remote and branch are required".to_string());
    }
    let out = run_git(
        Path::new(&project_root),
        &["pull", remote.trim(), branch.trim()],
    )
    .await?;
    Ok(format!("{}{}", out.stdout, out.stderr).trim().to_string())
}

/// Push is NEVER automatic. `confirmed` must be true — the frontend only sets
/// it after the user passes the pre-push confirmation modal (remote, branch,
/// commits, changed files, diff summary). Returns the push output on success.
#[tauri::command]
pub async fn git_push(
    project_root: String,
    remote: String,
    branch: String,
    confirmed: bool,
) -> Result<String, String> {
    if !confirmed {
        return Err(
            "push requires explicit confirmation (pre-push modal was not confirmed)".to_string(),
        );
    }
    if remote.trim().is_empty() || branch.trim().is_empty() {
        return Err("remote and branch are required".to_string());
    }
    let out = run_git(
        Path::new(&project_root),
        &["push", remote.trim(), branch.trim()],
    )
    .await?;
    Ok(format!("{}{}", out.stdout, out.stderr).trim().to_string())
}
