// Guild Foundry AI — Phase 4: sandboxed project filesystem.
//
// Every filesystem operation is confined to a project root. Paths supplied by
// the frontend are treated as untrusted: they are resolved under the root,
// canonicalized, and verified to still start with the root before any access.
// Symlink escapes, `..` traversal, and absolute paths outside the root are
// rejected. A small deny-list blocks credential-bearing component names
// (.ssh, .gnupg, ...) even when they would otherwise resolve inside the root.
//
// Destructive operations (delete, overwrite, patch, restore) automatically
// create a checkpoint first so they are always recoverable. Every mutation is
// appended to a JSONL audit log at `<root>/.gf/audit.log`.
//
// NOT VERIFIED: this file has not been compiled (no Rust toolchain in this
// build environment). All APIs used (std::fs, walkdir 2, chrono, uuid v4,
// serde/serde_json, tauri 2) were checked against documented signatures.

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use uuid::Uuid;
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Sandbox constants
// ---------------------------------------------------------------------------

/// Hidden directory inside each project root holding checkpoints + audit log.
const GF_DIR: &str = ".gf";
const CHECKPOINTS_DIR: &str = "checkpoints";
const AUDIT_FILE: &str = "audit.log";

/// Maximum file size accepted by read_file / write_file (8 MiB).
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Maximum content-search results returned by search_files.
const MAX_SEARCH_RESULTS: usize = 200;

/// Component names that are never addressable, even inside the sandbox.
/// Protects SSH keys, GPG material, cloud credentials, browser profiles.
const DENIED_COMPONENTS: &[&str] = &[
    ".ssh", ".gnupg", ".aws", ".pki", ".mozilla", "id_rsa", "id_ed25519",
];

// ---------------------------------------------------------------------------
// Frozen helpers (Phase 6 tool executor calls these directly — do not change)
// ---------------------------------------------------------------------------

/// Resolve an untrusted relative path under `root`, rejecting traversal,
/// absolute paths, denied components, and symlink escapes.
///
/// Resolution strategy:
/// 1. `root` itself is canonicalized (so the anchor is trustworthy).
/// 2. Only `Normal` / `CurDir` components are accepted from `rel`.
/// 3. The longest existing prefix of `root/rel` is canonicalized; the
///    remaining (nonexistent) suffix is re-appended. The canonicalized prefix
///    must still start with the canonical root — this defeats symlink
///    escapes for existing paths, and nonexistent suffix components cannot
///    be symlinks.
pub(crate) fn resolve_sandboxed(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let root = canonical_root(root)?;

    if rel.is_empty() {
        return Ok(root);
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err("absolute paths are not allowed inside the project sandbox".to_string());
    }
    for component in rel_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if DENIED_COMPONENTS.contains(&name.as_ref()) {
                    return Err(format!("path component '{}' is denied by sandbox policy", name));
                }
            }
            _ => {
                return Err(format!(
                    "path traversal is not allowed: {:?}",
                    rel_path
                ));
            }
        }
    }

    let joined = root.join(rel_path);

    // Canonicalize the longest existing prefix to resolve symlinks, then
    // re-append the (nonexistent) remainder.
    let mut probe = joined.clone();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    while !probe.exists() {
        match probe.file_name() {
            Some(name) => {
                suffix.push(name.to_os_string());
                probe.pop();
            }
            None => break,
        }
    }
    let base = fs::canonicalize(&probe)
        .map_err(|e| format!("cannot canonicalize path: {}", e))?;
    if !base.starts_with(&root) {
        return Err("path escapes the project root (symlink or traversal)".to_string());
    }
    let mut result = base;
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    if !result.starts_with(&root) {
        return Err("path escapes the project root".to_string());
    }
    Ok(result)
}

/// Read a UTF-8 text file inside the sandbox.
pub(crate) fn read_file(root: &Path, rel: &str) -> Result<String, String> {
    let path = resolve_sandboxed(root, rel)?;
    let meta = fs::metadata(&path).map_err(|e| format!("cannot stat file: {}", e))?;
    if !meta.is_file() {
        return Err("path is not a regular file".to_string());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file too large to read ({} bytes, limit {} bytes)",
            meta.len(),
            MAX_FILE_BYTES
        ));
    }
    fs::read_to_string(&path)
        .map_err(|e| format!("cannot read file (not valid UTF-8?): {}", e))
}

/// Write (create or overwrite) a UTF-8 text file inside the sandbox,
/// creating parent directories as needed.
pub(crate) fn write_file(root: &Path, rel: &str, content: &str) -> Result<(), String> {
    let path = resolve_sandboxed(root, rel)?;
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err(format!(
            "content too large ({} bytes, limit {} bytes)",
            content.len(),
            MAX_FILE_BYTES
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create parent dirs: {}", e))?;
    }
    fs::write(&path, content).map_err(|e| format!("cannot write file: {}", e))
}

/// Create a file (empty) or directory inside the sandbox.
pub(crate) fn create_entry(root: &Path, rel: &str, is_dir: bool) -> Result<(), String> {
    let path = resolve_sandboxed(root, rel)?;
    if is_dir {
        fs::create_dir_all(&path).map_err(|e| format!("cannot create directory: {}", e))
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create parent dirs: {}", e))?;
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map(|_| ())
            .map_err(|e| format!("cannot create file (may already exist): {}", e))
    }
}

/// Full-text + filename search inside the sandbox (case-insensitive).
/// Skips `.git`, the internal `.gf` dir, and files larger than 1 MiB.
pub(crate) fn search_files(root: &Path, query: &str) -> Result<Vec<String>, String> {
    let root = canonical_root(root)?;
    if query.trim().is_empty() {
        return Err("search query is empty".to_string());
    }
    let pattern = regex::RegexBuilder::new(&regex::escape(query.trim()))
        .case_insensitive(true)
        .build()
        .map_err(|e| format!("invalid search pattern: {}", e))?;

    let mut hits: HashSet<String> = HashSet::new();
    for entry in WalkDir::new(&root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| !is_internal_dir(e.path(), &root))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let rel = match entry.path().strip_prefix(&root) {
            Ok(r) => r.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        // Filename match.
        if let Some(name) = entry.file_name().to_str() {
            if pattern.is_match(name) {
                hits.insert(rel.clone());
                if hits.len() >= MAX_SEARCH_RESULTS {
                    break;
                }
                continue;
            }
        }
        // Content match for regular files under 1 MiB.
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                if meta.len() <= 1024 * 1024 {
                    if let Ok(text) = fs::read_to_string(entry.path()) {
                        if pattern.is_match(&text) {
                            hits.insert(rel);
                        }
                    }
                }
            }
        }
        if hits.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }
    let mut out: Vec<String> = hits.into_iter().collect();
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(root).map_err(|e| format!("project root is not accessible: {}", e))
}

/// True when `path` is the internal `.gf` directory or anything under it.
fn is_gf_path(path: &Path, root: &Path) -> bool {
    match path.strip_prefix(root) {
        Ok(rel) => rel
            .components()
            .next()
            .map(|c| c == Component::Normal(GF_DIR.as_ref()))
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn is_internal_dir(path: &Path, root: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    match path.strip_prefix(root) {
        Ok(rel) => {
            let name = rel
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            name == ".git" || name == GF_DIR
        }
        Err(_) => false,
    }
}

fn gf_dir(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join(GF_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {} dir: {}", GF_DIR, e))?;
    Ok(dir)
}

/// Append a JSON audit record for a mutation.
fn audit(root: &Path, op: &str, rel: &str, detail: &str) {
    let Ok(dir) = gf_dir(root) else { return };
    let record = serde_json::json!({
        "ts": Utc::now().to_rfc3339(),
        "op": op,
        "path": rel,
        "detail": detail,
    });
    let line = format!("{}\n", record);
    let log_path = dir.join(AUDIT_FILE);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Best-effort SHA-256 of a file for snapshot manifests.
fn sha256_of(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    match fs::read(path) {
        Ok(bytes) => hex::encode(Sha256::digest(&bytes)),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Checkpoints (independent of Git)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct SnapshotManifest {
    id: String,
    label: String,
    created_at: String,
    file_count: usize,
}

#[derive(Serialize)]
pub struct SnapshotInfo {
    id: String,
    label: String,
    created_at: String,
    file_count: usize,
}

fn checkpoints_root(root: &Path) -> Result<PathBuf, String> {
    let dir = gf_dir(root)?.join(CHECKPOINTS_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create checkpoints dir: {}", e))?;
    Ok(dir)
}

fn snapshot_create(root: &Path, label: &str) -> Result<SnapshotManifest, String> {
    let canonical = canonical_root(root)?;
    let id = Uuid::new_v4().to_string();
    let dest = checkpoints_root(&canonical)?.join(&id);
    fs::create_dir_all(&dest).map_err(|e| format!("cannot create snapshot dir: {}", e))?;

    let mut file_count = 0usize;
    for entry in WalkDir::new(&canonical)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if is_gf_path(path, &canonical) {
            continue;
        }
        let rel = path
            .strip_prefix(&canonical)
            .map_err(|e| format!("cannot relativize path: {}", e))?;
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("snapshot mkdir failed: {}", e))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("snapshot mkdir failed: {}", e))?;
            }
            fs::copy(path, &target).map_err(|e| format!("snapshot copy failed: {}", e))?;
            file_count += 1;
        }
    }

    let manifest = SnapshotManifest {
        id: id.clone(),
        label: label.to_string(),
        created_at: Utc::now().to_rfc3339(),
        file_count,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("manifest serialize: {}", e))?;
    fs::write(dest.join("manifest.json"), manifest_json)
        .map_err(|e| format!("manifest write failed: {}", e))?;
    audit(
        &canonical,
        "snapshot",
        "",
        &format!("{} ({})", manifest.id, manifest.label),
    );
    Ok(manifest)
}

fn snapshot_list(root: &Path) -> Result<Vec<SnapshotInfo>, String> {
    let canonical = canonical_root(root)?;
    let dir = checkpoints_root(&canonical)?;
    let mut out: Vec<SnapshotInfo> = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|e| format!("cannot list checkpoints: {}", e))?;
    for entry in entries.filter_map(|e| e.ok()) {
        let manifest_path = entry.path().join("manifest.json");
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            if let Ok(m) = serde_json::from_str::<SnapshotManifest>(&text) {
                out.push(SnapshotInfo {
                    id: m.id,
                    label: m.label,
                    created_at: m.created_at,
                    file_count: m.file_count,
                });
            }
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

fn snapshot_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains('/') || id.contains("..") {
        return Err("invalid snapshot id".to_string());
    }
    let canonical = canonical_root(root)?;
    let dir = checkpoints_root(&canonical)?.join(id);
    if !dir.is_dir() {
        return Err("snapshot not found".to_string());
    }
    Ok(dir)
}

/// Restore `root` to the content of snapshot `id`. Creates an automatic
/// pre-restore checkpoint of the current state first.
fn snapshot_restore(root: &Path, id: &str) -> Result<SnapshotManifest, String> {
    let canonical = canonical_root(root)?;
    let snap = snapshot_dir(&canonical, id)?;

    // Auto-checkpoint current state before destroying anything.
    snapshot_create(&canonical, "auto: pre-restore")?;

    // Remove everything except `.gf`.
    for entry in fs::read_dir(&canonical).map_err(|e| format!("cannot read root: {}", e))? {
        let entry = entry.map_err(|e| format!("dir read failed: {}", e))?;
        let path = entry.path();
        if path
            .file_name()
            .map(|n| n == GF_DIR)
            .unwrap_or(false)
        {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| format!("cannot remove dir: {}", e))?;
        } else {
            fs::remove_file(&path).map_err(|e| format!("cannot remove file: {}", e))?;
        }
    }

    // Copy snapshot files back.
    let mut restored = 0usize;
    for entry in WalkDir::new(&snap)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.file_name().map(|n| n == "manifest.json").unwrap_or(false) {
            continue;
        }
        let rel = path
            .strip_prefix(&snap)
            .map_err(|e| format!("cannot relativize snapshot path: {}", e))?;
        let target = canonical.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("restore mkdir failed: {}", e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("restore mkdir failed: {}", e))?;
            }
            fs::copy(path, &target).map_err(|e| format!("restore copy failed: {}", e))?;
            restored += 1;
        }
    }
    audit(
        &canonical,
        "restore",
        "",
        &format!("restored snapshot {} ({} files)", id, restored),
    );
    snapshot_list(&canonical)?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| "snapshot vanished during restore".to_string())
        .map(|s| SnapshotManifest {
            id: s.id,
            label: s.label,
            created_at: s.created_at,
            file_count: s.file_count,
        })
}

// ---------------------------------------------------------------------------
// Unified diff (hand-rolled line LCS) + patch application
// ---------------------------------------------------------------------------

fn split_lines(s: &str) -> Vec<&str> {
    s.lines().collect()
}

/// Longest-common-subsequence diff on lines, rendered as a unified diff with
/// 3 lines of context. Falls back to a whole-file hunk for very large inputs
/// (keeps the O(n*m) DP bounded).
fn unified_diff(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
    let a = split_lines(old);
    let b = split_lines(new);

    // Op list from LCS.
    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        Same,
        Del,
        Ins,
    }
    let mut ops: Vec<(Op, &str)> = Vec::new();
    if (a.len() as u64) * (b.len() as u64) <= 4_000_000 {
        let n = a.len();
        let m = b.len();
        let mut dp = vec![vec![0usize; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i][j] = if a[i] == b[j] {
                    dp[i + 1][j + 1] + 1
                } else {
                    dp[i + 1][j].max(dp[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0usize, 0usize);
        while i < n && j < m {
            if a[i] == b[j] {
                ops.push((Op::Same, a[i]));
                i += 1;
                j += 1;
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                ops.push((Op::Del, a[i]));
                i += 1;
            } else {
                ops.push((Op::Ins, b[j]));
                j += 1;
            }
        }
        while i < n {
            ops.push((Op::Del, a[i]));
            i += 1;
        }
        while j < m {
            ops.push((Op::Ins, b[j]));
            j += 1;
        }
    } else {
        // Large-file fallback: treat the whole file as replaced.
        for line in &a {
            ops.push((Op::Del, line));
        }
        for line in &b {
            ops.push((Op::Ins, line));
        }
    }

    let mut out = format!("--- {}\n+++ {}\n", old_label, new_label);
    // Group into hunks with 3 lines of context.
    let context = 3usize;
    let mut hunks: Vec<(usize, usize)> = Vec::new(); // (start, end) in ops
    let mut hunk_start: Option<usize> = None;
    let mut last_change = 0usize;
    for (idx, (op, _)) in ops.iter().enumerate() {
        if *op != Op::Same {
            if hunk_start.is_none() {
                hunk_start = Some(idx.saturating_sub(context));
            }
            last_change = idx;
        } else if let Some(start) = hunk_start {
            if idx - last_change > context * 2 {
                hunks.push((start, last_change + context + 1));
                hunk_start = None;
            }
        }
    }
    if let Some(start) = hunk_start {
        hunks.push((start, (last_change + context + 1).min(ops.len())));
    }

    for (start, end) in hunks {
        let end = end.min(ops.len());
        let mut old_count = 0usize;
        let mut new_count = 0usize;
        // Compute 1-based line numbers for the hunk header.
        let mut o = 0usize;
        let mut nn = 0usize;
        for (op, _) in ops.iter().take(start) {
            match op {
                Op::Same => {
                    o += 1;
                    nn += 1;
                }
                Op::Del => o += 1,
                Op::Ins => nn += 1,
            }
        }
        let old_start = o + 1;
        let new_start = nn + 1;
        for (op, _) in &ops[start..end] {
            match op {
                Op::Same => {
                    old_count += 1;
                    new_count += 1;
                }
                Op::Del => old_count += 1,
                Op::Ins => new_count += 1,
            }
        }
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_count, new_start, new_count
        ));
        for (op, line) in &ops[start..end] {
            match op {
                Op::Same => out.push_str(&format!(" {}\n", line)),
                Op::Del => out.push_str(&format!("-{}\n", line)),
                Op::Ins => out.push_str(&format!("+{}\n", line)),
            }
        }
    }
    out
}

struct PatchHunk {
    old_start: usize, // 1-based
    lines: Vec<(char, String)>,
}

fn parse_unified_patch(patch: &str) -> Result<Vec<PatchHunk>, String> {
    let mut hunks: Vec<PatchHunk> = Vec::new();
    let mut current: Option<PatchHunk> = None;
    for raw in patch.lines() {
        if raw.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            // Parse "@@ -old_start,old_count +new_start,new_count @@"
            let inner = raw.trim_start_matches('@').trim_start();
            let old_part = inner
                .split_whitespace()
                .next()
                .ok_or("malformed hunk header")?;
            let old_start: usize = old_part
                .trim_start_matches('-')
                .split(',')
                .next()
                .ok_or("malformed hunk header")?
                .parse()
                .map_err(|_| "malformed hunk header")?;
            current = Some(PatchHunk {
                old_start,
                lines: Vec::new(),
            });
        } else if let Some(h) = current.as_mut() {
            if raw.is_empty() {
                continue;
            }
            let sig = raw.chars().next().unwrap_or(' ');
            let text = raw.get(1..).unwrap_or("");
            match sig {
                ' ' | '-' | '+' => h.lines.push((sig, text.to_string())),
                _ => return Err(format!("malformed patch line: {:?}", raw)),
            }
        } else if raw.starts_with("---") || raw.starts_with("+++") {
            continue;
        } else if !raw.trim().is_empty() {
            return Err("patch content outside a hunk".to_string());
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return Err("patch contains no hunks".to_string());
    }
    Ok(hunks)
}

/// Apply a unified patch to `original`, returning the patched text.
fn apply_patch_text(original: &str, patch: &str) -> Result<String, String> {
    let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
    let hunks = parse_unified_patch(patch)?;
    // Apply hunks in reverse order so earlier line numbers stay valid.
    let mut sorted: Vec<PatchHunk> = hunks;
    sorted.sort_by_key(|h| std::cmp::Reverse(h.old_start));
    for hunk in sorted {
        let mut idx = hunk.old_start.saturating_sub(1); // 0-based
        let mut replacement: Vec<String> = Vec::new();
        for (sig, text) in &hunk.lines {
            match sig {
                ' ' => {
                    let actual = lines
                        .get(idx)
                        .ok_or_else(|| "patch context exceeds file".to_string())?;
                    if actual != text {
                        return Err(format!(
                            "patch context mismatch at line {}: expected {:?}, found {:?}",
                            idx + 1,
                            text,
                            actual
                        ));
                    }
                    replacement.push(actual.clone());
                    idx += 1;
                }
                '-' => {
                    let actual = lines
                        .get(idx)
                        .ok_or_else(|| "patch deletion exceeds file".to_string())?;
                    if actual != text {
                        return Err(format!(
                            "patch deletion mismatch at line {}: expected {:?}, found {:?}",
                            idx + 1,
                            text,
                            actual
                        ));
                    }
                    idx += 1;
                }
                '+' => replacement.push(text.clone()),
                _ => unreachable!(),
            }
        }
        let start = hunk.old_start.saturating_sub(1);
        if start > lines.len() || idx > lines.len() {
            return Err("patch hunk out of file bounds".to_string());
        }
        lines.splice(start..idx, replacement);
    }
    let mut out = lines.join("\n");
    if original.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct FsEntry {
    name: String,
    rel: String,
    is_dir: bool,
    size: u64,
    modified: Option<String>,
}

/// Resolve the effective project root. When `root` is empty, fall back to a
/// `projects` directory under the Tauri app-data dir.
fn effective_root(app: &tauri::AppHandle, root: &str) -> Result<PathBuf, String> {
    if !root.trim().is_empty() {
        let p = PathBuf::from(root.trim());
        if p.is_dir() {
            return Ok(p);
        }
        fs::create_dir_all(&p).map_err(|e| format!("cannot create project root: {}", e))?;
        return Ok(p);
    }
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot locate app data dir: {}", e))?;
    let projects = data_dir.join("projects");
    fs::create_dir_all(&projects).map_err(|e| format!("cannot create projects dir: {}", e))?;
    Ok(projects)
}

#[tauri::command]
pub fn ws_project_root(app: tauri::AppHandle, root: Option<String>) -> Result<String, String> {
    let r = effective_root(&app, root.as_deref().unwrap_or(""))?;
    Ok(r.to_string_lossy().to_string())
}

#[tauri::command]
pub fn ws_list(
    app: tauri::AppHandle,
    project_root: String,
    rel: Option<String>,
) -> Result<Vec<FsEntry>, String> {
    let root = effective_root(&app, &project_root)?;
    let dir = resolve_sandboxed(&root, rel.as_deref().unwrap_or(""))?;
    if !dir.is_dir() {
        return Err("path is not a directory".to_string());
    }
    let canonical = canonical_root(&root)?;
    let mut out: Vec<FsEntry> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("cannot list directory: {}", e))? {
        let entry = entry.map_err(|e| format!("dir entry failed: {}", e))?;
        let path = entry.path();
        if is_gf_path(&path, &canonical) {
            continue;
        }
        let meta = entry.metadata().map_err(|e| format!("cannot stat entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let rel_path = path
            .strip_prefix(&canonical)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(name.clone());
        out.push(FsEntry {
            name,
            rel: rel_path,
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified: meta.modified().ok().map(|t| {
                chrono::DateTime::<Utc>::from(t)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            }),
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

#[tauri::command]
pub fn ws_read(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
) -> Result<String, String> {
    let root = effective_root(&app, &project_root)?;
    read_file(&root, &rel)
}

#[tauri::command]
pub fn ws_write(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
    content: String,
) -> Result<(), String> {
    let root = effective_root(&app, &project_root)?;
    // Checkpoint before overwriting an existing file.
    if resolve_sandboxed(&root, &rel)
        .map(|p| p.is_file())
        .unwrap_or(false)
    {
        snapshot_create(&root, &format!("auto: before write {}", rel))?;
    }
    write_file(&root, &rel, &content)?;
    audit(&root, "write", &rel, &format!("{} bytes", content.len()));
    Ok(())
}

#[tauri::command]
pub fn ws_create(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
    is_dir: bool,
) -> Result<(), String> {
    let root = effective_root(&app, &project_root)?;
    create_entry(&root, &rel, is_dir)?;
    audit(&root, if is_dir { "mkdir" } else { "create" }, &rel, "");
    Ok(())
}

#[tauri::command]
pub fn ws_delete(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
) -> Result<(), String> {
    let root = effective_root(&app, &project_root)?;
    let path = resolve_sandboxed(&root, &rel)?;
    if !path.exists() {
        return Err("path does not exist".to_string());
    }
    // Auto-checkpoint before any destructive delete.
    snapshot_create(&root, &format!("auto: before delete {}", rel))?;
    if path.is_dir() {
        fs::remove_dir_all(&path).map_err(|e| format!("cannot delete directory: {}", e))?;
    } else {
        fs::remove_file(&path).map_err(|e| format!("cannot delete file: {}", e))?;
    }
    audit(&root, "delete", &rel, "");
    Ok(())
}

#[tauri::command]
pub fn ws_rename(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
    new_name: String,
) -> Result<(), String> {
    let root = effective_root(&app, &project_root)?;
    let src = resolve_sandboxed(&root, &rel)?;
    if new_name.contains('/') || new_name.contains('\\') || new_name.is_empty() {
        return Err("new name must be a plain file name".to_string());
    }
    let parent = src.parent().ok_or("cannot determine parent")?.to_path_buf();
    let dst = parent.join(&new_name);
    // Verify the destination is still inside the sandbox.
    let canonical = canonical_root(&root)?;
    if !dst.starts_with(&canonical) {
        return Err("destination escapes the project root".to_string());
    }
    if dst.exists() {
        return Err("a file with the new name already exists".to_string());
    }
    fs::rename(&src, &dst).map_err(|e| format!("cannot rename: {}", e))?;
    audit(&root, "rename", &rel, &format!("-> {}", new_name));
    Ok(())
}

#[tauri::command]
pub fn ws_move(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
    dest: String,
) -> Result<(), String> {
    let root = effective_root(&app, &project_root)?;
    let src = resolve_sandboxed(&root, &rel)?;
    let dst = resolve_sandboxed(&root, &dest)?;
    if dst.exists() {
        return Err("destination already exists".to_string());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create dest dirs: {}", e))?;
    }
    fs::rename(&src, &dst).map_err(|e| format!("cannot move: {}", e))?;
    audit(&root, "move", &rel, &format!("-> {}", dest));
    Ok(())
}

#[tauri::command]
pub fn ws_search(
    app: tauri::AppHandle,
    project_root: String,
    query: String,
) -> Result<Vec<String>, String> {
    let root = effective_root(&app, &project_root)?;
    search_files(&root, &query)
}

#[tauri::command]
pub fn ws_diff(
    app: tauri::AppHandle,
    project_root: String,
    rel_a: String,
    rel_b: String,
) -> Result<String, String> {
    let root = effective_root(&app, &project_root)?;
    let a = read_file(&root, &rel_a)?;
    let b = read_file(&root, &rel_b)?;
    Ok(unified_diff(&a, &b, &rel_a, &rel_b))
}

#[tauri::command]
pub fn ws_patch(
    app: tauri::AppHandle,
    project_root: String,
    rel: String,
    patch: String,
) -> Result<(), String> {
    let root = effective_root(&app, &project_root)?;
    let original = read_file(&root, &rel)?;
    let patched = apply_patch_text(&original, &patch)?;
    // Checkpoint before applying the patch (destructive).
    snapshot_create(&root, &format!("auto: before patch {}", rel))?;
    write_file(&root, &rel, &patched)?;
    audit(&root, "patch", &rel, &format!("{} bytes", patch.len()));
    Ok(())
}

#[tauri::command]
pub fn ws_snapshot(
    app: tauri::AppHandle,
    project_root: String,
    label: String,
) -> Result<SnapshotInfo, String> {
    let root = effective_root(&app, &project_root)?;
    let m = snapshot_create(&root, &label)?;
    Ok(SnapshotInfo {
        id: m.id,
        label: m.label,
        created_at: m.created_at,
        file_count: m.file_count,
    })
}

#[tauri::command]
pub fn ws_checkpoints(
    app: tauri::AppHandle,
    project_root: String,
) -> Result<Vec<SnapshotInfo>, String> {
    let root = effective_root(&app, &project_root)?;
    snapshot_list(&root)
}

#[derive(Serialize)]
pub struct SnapshotFile {
    rel: String,
    size: u64,
    sha256: String,
}

/// Preview: list files inside a snapshot with sizes and hashes.
#[tauri::command]
pub fn ws_checkpoint_preview(
    app: tauri::AppHandle,
    project_root: String,
    snapshot_id: String,
) -> Result<Vec<SnapshotFile>, String> {
    let root = effective_root(&app, &project_root)?;
    let snap = snapshot_dir(&root, &snapshot_id)?;
    let mut out: Vec<SnapshotFile> = Vec::new();
    for entry in WalkDir::new(&snap)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        if path.file_name().map(|n| n == "manifest.json").unwrap_or(false) {
            continue;
        }
        let rel = path
            .strip_prefix(&snap)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push(SnapshotFile {
            size: entry.metadata().map(|m| m.len()).unwrap_or(0),
            sha256: sha256_of(path),
            rel,
        });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

#[derive(Serialize)]
pub struct SnapshotChange {
    rel: String,
    kind: String, // "added" | "removed" | "modified"
}

/// Compare: diff a snapshot against the current working tree.
#[tauri::command]
pub fn ws_checkpoint_diff(
    app: tauri::AppHandle,
    project_root: String,
    snapshot_id: String,
) -> Result<Vec<SnapshotChange>, String> {
    let root = effective_root(&app, &project_root)?;
    let canonical = canonical_root(&root)?;
    let snap = snapshot_dir(&root, &snapshot_id)?;

    // rel -> sha256 for snapshot.
    let mut snap_files: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for entry in WalkDir::new(&snap)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !entry.file_type().is_file()
            || path.file_name().map(|n| n == "manifest.json").unwrap_or(false)
        {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(&snap) {
            snap_files.insert(rel.to_string_lossy().to_string(), sha256_of(path));
        }
    }
    // rel -> sha256 for current tree.
    let mut live_files: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for entry in WalkDir::new(&canonical)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !entry.file_type().is_file() || is_gf_path(path, &canonical) {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(&canonical) {
            live_files.insert(rel.to_string_lossy().to_string(), sha256_of(path));
        }
    }

    let mut changes: Vec<SnapshotChange> = Vec::new();
    for (rel, live_hash) in &live_files {
        match snap_files.get(rel) {
            None => changes.push(SnapshotChange {
                rel: rel.clone(),
                kind: "added".to_string(),
            }),
            Some(snap_hash) if snap_hash != live_hash => changes.push(SnapshotChange {
                rel: rel.clone(),
                kind: "modified".to_string(),
            }),
            _ => {}
        }
    }
    for rel in snap_files.keys() {
        if !live_files.contains_key(rel) {
            changes.push(SnapshotChange {
                rel: rel.clone(),
                kind: "removed".to_string(),
            });
        }
    }
    changes.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(changes)
}

#[tauri::command]
pub fn ws_restore(
    app: tauri::AppHandle,
    project_root: String,
    snapshot_id: String,
) -> Result<SnapshotInfo, String> {
    let root = effective_root(&app, &project_root)?;
    let m = snapshot_restore(&root, &snapshot_id)?;
    Ok(SnapshotInfo {
        id: m.id,
        label: m.label,
        created_at: m.created_at,
        file_count: m.file_count,
    })
}
