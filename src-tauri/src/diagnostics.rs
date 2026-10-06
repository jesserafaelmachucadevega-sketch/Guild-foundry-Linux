// SPDX-License-Identifier: Apache-2.0
// Phase 11 — Diagnostics dashboard backend + crash recovery persistence.
//
// Master spec sections 54 (offline-first: network state is explicit, never
// ambiguous) and 55 (crash recovery: preserve state/tasks/history/artifacts,
// offer Resume / Inspect / Roll Back / Discard — never auto-resume destructive
// ops).
//
// Honesty rules (spec section 0): undetectable values are reported as absent,
// never invented. That is why there is NO VRAM field: sysinfo cannot read GPU
// memory on Linux, so we read PCI ids best-effort from /sys/class/drm and
// mark the GPU block as "limited" instead of fabricating numbers.
//
// NOT VERIFIED: no Rust toolchain in this environment — none of this has
// compiled. Requires `cargo check` on the Zorin OS target. sysinfo 0.36 APIs
// used: System::new, refresh_cpu_all, global_cpu_usage, refresh_memory,
// total_memory/used_memory (bytes), get_current_pid, ProcessesToUpdate::Some,
// ProcessRefreshKind::nothing().with_memory(), Disks::new_with_refreshed_list,
// Disk::mount_point/available_space. If any of these signatures shifted, fix
// against the vendored sysinfo docs, not against memory.

use serde::Serialize;
use std::time::Duration;
use sysinfo::{Disks, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::Manager;

const NETWORK_CHECK_TIMEOUT: Duration = Duration::from_secs(3);
const CRASH_STATE_FILE: &str = "crash-state.json";

#[derive(Serialize, Debug)]
pub struct GpuInfo {
    pub info: Option<String>,
    /// Always "limited" on this platform path — VRAM cannot be read here.
    pub note: String,
}

#[derive(Serialize, Debug)]
pub struct DiagSnapshot {
    pub cpu_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub disk_free_gb: f64,
    pub network_up: bool,
    pub app_memory_mb: u64,
    pub gpu: GpuInfo,
}

#[derive(Serialize, Debug)]
pub struct TokenStats {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Serialize, Debug)]
pub struct BuildWorkerState {
    pub workers: Vec<serde_json::Value>,
    pub note: String,
}

/// Best-effort GPU summary from /sys/class/drm. Returns None when nothing is
/// detectable; the caller keeps `info: None` and `note: "limited"` rather than
/// inventing a GPU.
fn gpu_summary() -> Option<String> {
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    let mut cards: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // DRM "card" nodes are card0, card1, ... — the "-"-suffixed ones are
        // connectors (card0-HDMI-A-1), not GPUs.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device_dir = entry.path().join("device");
        let vendor = std::fs::read_to_string(device_dir.join("vendor"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let device = std::fs::read_to_string(device_dir.join("device"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let driver = std::fs::read_to_string(device_dir.join("driver"))
            .ok()
            .and_then(|p| {
                std::path::Path::new(p.trim())
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "unknown".to_string());
        cards.push(format!(
            "{}: PCI vendor={} device={} driver={}",
            name, vendor, device, driver
        ));
    }
    if cards.is_empty() {
        None
    } else {
        Some(cards.join("; "))
    }
}

/// Honest, bounded network check: TCP connect to a well-known public host with
/// a 3s timeout. Resolves DNS too, so a DNS failure also reads as offline.
/// Never hangs the UI.
async fn network_reachable() -> bool {
    match tokio::time::timeout(NETWORK_CHECK_TIMEOUT, tokio::net::TcpStream::connect("example.com:80")).await {
        Ok(Ok(_)) => true,
        _ => false,
    }
}

/// Machine + app resource snapshot.
#[tauri::command]
pub async fn diag_snapshot(app: tauri::AppHandle) -> Result<DiagSnapshot, String> {
    let mut sys = System::new();
    sys.refresh_memory();

    // CPU usage is a delta between two samples; take two samples 250ms apart
    // so the reading is real instead of the first-sample zero.
    sys.refresh_cpu_all();
    tokio::time::sleep(Duration::from_millis(250)).await;
    sys.refresh_cpu_all();
    let cpu_pct = sys.global_cpu_usage();

    let ram_total_mb = sys.total_memory() / (1024 * 1024);
    let ram_used_mb = sys.used_memory() / (1024 * 1024);

    // This process's resident memory.
    let pid = sysinfo::get_current_pid().map_err(|e| format!("cannot get current pid: {}", e))?;
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let app_memory_mb = sys.process(pid).map(|p| p.memory() / (1024 * 1024)).unwrap_or(0);

    // Free space on the volume holding the app-data dir (best-effort: longest
    // matching mount point); falls back to the sum across all disks.
    let disks = Disks::new_with_refreshed_list();
    let data_dir = app.path().app_data_dir().ok();
    let mut best: Option<(&sysinfo::Disk, usize)> = None;
    for disk in disks.list() {
        if let Some(dir) = data_dir.as_ref() {
            let mp = disk.mount_point().to_string_lossy();
            let ds = dir.to_string_lossy();
            if ds.starts_with(mp.as_ref()) && mp.len() > best.map(|(_, len)| len).unwrap_or(0) {
                best = Some((disk, mp.len()));
            }
        }
    }
    let disk_free_gb = match best {
        Some((disk, _)) => disk.available_space() as f64 / 1_000_000_000.0,
        None => disks
            .list()
            .iter()
            .map(|d| d.available_space())
            .sum::<u64>() as f64
            / 1_000_000_000.0,
    };

    let network_up = network_reachable().await;

    Ok(DiagSnapshot {
        cpu_pct,
        ram_used_mb,
        ram_total_mb,
        disk_free_gb,
        network_up,
        app_memory_mb,
        gpu: GpuInfo {
            info: gpu_summary(),
            note: "limited".to_string(),
        },
    })
}

/// Standalone network probe the UI can poll cheaply for the ONLINE/OFFLINE
/// badge (offline-first: AI actions show OFFLINE instead of failing
/// ambiguously).
#[tauri::command]
pub async fn diag_network() -> Result<bool, String> {
    Ok(network_reachable().await)
}

/// Cumulative token counters, owned by the Phase 2 provider layer.
/// PENDING DEP: requires Phase 2's providers.rs to expose
/// `pub(crate) static TOKENS_IN: AtomicU64` and `TOKENS_OUT`. If it uses a
/// different shape, this command must be adjusted — it will not compile
/// until that module lands.
#[tauri::command]
pub fn diag_token_stats() -> Result<TokenStats, String> {
    Ok(TokenStats {
        tokens_in: crate::providers::TOKENS_IN.load(std::sync::atomic::Ordering::Relaxed),
        tokens_out: crate::providers::TOKENS_OUT.load(std::sync::atomic::Ordering::Relaxed),
    })
}

/// Build worker state belongs to Phase 8 (build/test/fix engine). The module
/// does not exist yet, so this reports that honestly instead of inventing
/// workers.
#[tauri::command]
pub fn diag_build_workers() -> Result<BuildWorkerState, String> {
    Ok(BuildWorkerState {
        workers: Vec::new(),
        note: "build worker state lands with Phase 8".to_string(),
    })
}

fn crash_state_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot resolve app-data dir: {}", e))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create app-data dir: {}", e))?;
    Ok(dir.join(CRASH_STATE_FILE))
}

/// Persist a crash-recovery snapshot (JSON). Called by the frontend on a
/// heartbeat / before risky operations. The payload is validated as JSON so a
/// corrupt snapshot is rejected at write time, not discovered at recovery.
#[tauri::command]
pub fn crash_state_save(app: tauri::AppHandle, state_json: String) -> Result<(), String> {
    serde_json::from_str::<serde_json::Value>(&state_json)
        .map_err(|e| format!("refusing to save invalid crash-state JSON: {}", e))?;
    let path = crash_state_path(&app)?;
    std::fs::write(&path, state_json).map_err(|e| format!("cannot write crash state: {}", e))?;
    Ok(())
}

/// Load the persisted snapshot, or None when no snapshot exists (clean start).
#[tauri::command]
pub fn crash_state_load(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let path = crash_state_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read crash state: {}", e))?;
    Ok(Some(text))
}

/// Clear the snapshot after the user resumes, inspects, rolls back, or
/// discards it.
#[tauri::command]
pub fn crash_state_clear(app: tauri::AppHandle) -> Result<(), String> {
    let path = crash_state_path(&app)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("cannot clear crash state: {}", e))?;
    }
    Ok(())
}
