// Phase 1 — Runtime OS / toolchain capability detection.
//
// Master spec sections 1 + 90: detect the operating environment at runtime and
// adapt, rather than hard-coding distribution paths or assuming software
// exists. Every field is best-effort; undetectable values are reported as
// "unknown" rather than invented.

use serde::Serialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Serialize, Debug)]
pub struct EnvironmentInfo {
    pub distribution: String,
    pub distribution_version: String,
    pub kernel_version: String,
    pub architecture: String,
    pub desktop_environment: String,
    pub session_type: String,
    pub cpu_model: String,
    pub cpu_cores: usize,
    pub ram_total_mb: u64,
    pub gpu: String,
    pub shell: String,
    pub tools: HashMap<String, bool>,
}

fn os_release() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(text) = fs::read_to_string("/etc/os-release") {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.to_string(), v.trim_matches('"').to_string());
            }
        }
    }
    map
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

fn which(tool: &str) -> bool {
    let path_var = env::var("PATH").unwrap_or_default();
    for dir in env::split_paths(&path_var) {
        if is_executable(&dir.join(tool)) {
            return true;
        }
    }
    false
}

fn cpu_info() -> (String, usize) {
    let mut model = String::from("unknown");
    let mut cores = 0usize;
    if let Ok(text) = fs::read_to_string("/proc/cpuinfo") {
        for line in text.lines() {
            if line.starts_with("model name") {
                if let Some((_, v)) = line.split_once(':') {
                    model = v.trim().to_string();
                }
            }
            if line.starts_with("processor") {
                cores += 1;
            }
        }
    }
    (model, cores)
}

fn ram_total_mb() -> u64 {
    if let Ok(text) = fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                return kb / 1024;
            }
        }
    }
    0
}

fn gpu_info() -> String {
    // Best-effort: parse `lspci` for VGA / 3D controllers when present.
    let probe = ["/usr/bin/lspci", "/sbin/lspci"];
    for bin in probe {
        if Path::new(bin).exists() {
            if let Ok(out) = std::process::Command::new(bin).output() {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout);
                    let mut found = Vec::new();
                    for line in text.lines() {
                        let lower = line.to_lowercase();
                        if lower.contains(" vga ") || lower.contains("3d controller") {
                            if let Some(desc) = line.splitn(2, ':').nth(1) {
                                found.push(desc.trim().to_string());
                            }
                        }
                    }
                    if !found.is_empty() {
                        return found.join(" | ");
                    }
                }
            }
        }
    }
    String::from("unknown")
}

#[tauri::command]
pub fn detect_environment() -> EnvironmentInfo {
    let release = os_release();
    let (cpu_model, cpu_cores) = cpu_info();

    let session_type = if env::var("WAYLAND_DISPLAY").is_ok() {
        "wayland"
    } else if env::var("DISPLAY").is_ok() {
        "x11"
    } else {
        "unknown"
    }
    .to_string();

    let tool_names = [
        "gcc", "git", "docker", "podman", "node", "python3", "cargo", "rustc", "java", "flatpak",
        "snap", "apt",
    ];
    let mut tools = HashMap::new();
    for t in tool_names {
        tools.insert(t.to_string(), which(t));
    }

    EnvironmentInfo {
        distribution: release
            .get("NAME")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        distribution_version: release
            .get("VERSION_ID")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        kernel_version: std::process::Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        architecture: env::consts::ARCH.to_string(),
        desktop_environment: env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_else(|_| "unknown".to_string()),
        session_type,
        cpu_model,
        cpu_cores,
        ram_total_mb: ram_total_mb(),
        gpu: gpu_info(),
        shell: env::var("SHELL").unwrap_or_else(|_| "unknown".to_string()),
        tools,
    }
}
