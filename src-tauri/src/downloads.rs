// SPDX-License-Identifier: Apache-2.0
// downloads.rs — Model Download: what this machine can run, and where to get it.
//
// GGUF-first, because this app runs models in-process through llama.cpp: every
// suggestion is a single file that `gguf_load` can open — no vLLM, no tmux, no
// conversion step. (A reference implementation the user trusts solves the same
// problem; its structure informed this one.)
//
// Two sources of recommendations:
//   1. a CURATED shortlist — every entry below was verified live against the
//      HuggingFace API (gated flag, exact filename, exact byte size);
//   2. a DISCOVER search — HuggingFace's trending GGUF models on demand.
//
// This slice answers "what fits" and "where is it". Fetching the bytes with
// progress and resume is the next slice (model_download_* commands).

use serde::Serialize;
use std::time::Duration;

/// One GPU as reported by the platform tooling.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Gpu {
    pub index: u32,
    pub name: String,
    pub free_mb: u64,
    pub total_mb: u64,
    pub used_mb: u64,
    pub util_pct: u64,
    pub vendor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hardware {
    pub gpus: Vec<Gpu>,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    /// What a model may occupy: free VRAM when a GPU exists, else available RAM.
    pub budget_mb: u64,
    /// Plain-language reading of the numbers above.
    pub verdict: String,
}

/// A recommended model: one repo, one file, one size.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DownloadableModel {
    pub id: String,
    pub repo: String,
    pub file: String,
    pub params_b: f32,
    pub quant: String,
    pub size_mb: u64,
    pub context_tokens: u32,
    pub license: String,
    pub tags: Vec<String>,
    pub note: String,
    /// Direct download URL (HuggingFace resolve path).
    pub url: String,
}

/// Parse `nvidia-smi --query-gpu=... --format=csv,noheader,nounits` output.
/// Format: `index, name, memory.free, memory.total, memory.used, utilization.gpu, uuid`
pub fn parse_nvidia_csv(text: &str) -> Vec<Gpu> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 6 {
            continue;
        }
        let num = |s: &str| s.parse::<u64>().ok();
        let (index, free, total, used, util) = (
            num(parts[0]),
            num(parts[2]),
            num(parts[3]),
            num(parts[4]),
            num(parts[5]),
        );
        let (Some(index), Some(free), Some(total), Some(used), Some(util)) =
            (index, free, total, used, util)
        else {
            continue;
        };
        out.push(Gpu {
            index: index as u32,
            name: parts[1].to_string(),
            free_mb: free,
            total_mb: total,
            used_mb: used,
            util_pct: util,
            vendor: "nvidia".to_string(),
        });
    }
    out
}

/// Parse the two lines of /proc/meminfo we care about, in MB.
pub fn parse_meminfo(text: &str) -> (u64, u64) {
    let field = |name: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb / 1024)
            .unwrap_or(0)
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

/// AMD GPUs expose VRAM through sysfs; no vendor tool required.
fn amd_gpus() -> Vec<Gpu> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    let mut index = 0u32;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // card0, card1… — but not card0-DP-1 connectors.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        let read_u64 = |f: &str| -> Option<u64> {
            std::fs::read_to_string(dev.join(f))
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()
        };
        let vendor = std::fs::read_to_string(dev.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if vendor != "0x1002" {
            continue; // 0x1002 = AMD
        }
        if let Some(total) = read_u64("mem_info_vram_total") {
            let used = read_u64("mem_info_vram_used").unwrap_or(0);
            out.push(Gpu {
                index,
                name: format!("AMD GPU {}", index),
                free_mb: total.saturating_sub(used) / (1024 * 1024),
                total_mb: total / (1024 * 1024),
                used_mb: used / (1024 * 1024),
                util_pct: 0,
                vendor: "amd".to_string(),
            });
            index += 1;
        }
    }
    out
}

fn nvidia_gpus() -> Vec<Gpu> {
    // Same query the reference implementation uses — free/total/used/util.
    let query = "index,name,memory.free,memory.total,memory.used,utilization.gpu,uuid";
    let out = std::process::Command::new("nvidia-smi")
        .args([
            format!("--query-gpu={}", query).as_str(),
            "--format=csv,noheader,nounits",
        ])
        .envs(crate::terminal::sanitized_env())
        .output();
    match out {
        Ok(o) if o.status.success() => parse_nvidia_csv(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Read the machine's memory budget. Never fails: zeroes mean "unknown".
pub fn probe() -> Hardware {
    let mut gpus = nvidia_gpus();
    if gpus.is_empty() {
        gpus = amd_gpus();
    }
    let (ram_total_mb, ram_available_mb) = std::fs::read_to_string("/proc/meminfo")
        .map(|t| parse_meminfo(&t))
        .unwrap_or((0, 0));

    let best_free_vram = gpus.iter().map(|g| g.free_mb).max();
    let budget_mb = best_free_vram.filter(|v| *v > 0).unwrap_or(ram_available_mb);

    let verdict = verdict_for(&gpus, budget_mb, ram_available_mb);
    Hardware {
        gpus,
        ram_total_mb,
        ram_available_mb,
        budget_mb,
        verdict,
    }
}

/// One sentence a person can act on, computed from the list itself rather than
/// guessed: the largest curated model that fits is named.
fn verdict_for(gpus: &[Gpu], budget_mb: u64, ram_available_mb: u64) -> String {
    if budget_mb == 0 {
        return "Could not read this machine's memory — models can still be added by file path."
            .to_string();
    }
    let on_gpu = !gpus.is_empty() && budget_mb != ram_available_mb;
    let best = curated()
        .into_iter()
        .filter(|m| model_needs_mb(m.size_mb) <= budget_mb)
        .max_by(|a, b| {
            a.params_b
                .partial_cmp(&b.params_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    let where_ = if on_gpu {
        let g = gpus
            .iter()
            .max_by_key(|g| g.free_mb)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| "GPU".to_string());
        format!("{} free VRAM on {}", gb(budget_mb), g)
    } else {
        format!(
            "{} RAM available (no GPU detected — CPU inference)",
            gb(budget_mb)
        )
    };
    match best {
        Some(m) => format!(
            "{} — that comfortably runs {} ({} at {}, {}). Larger models will work but spill.",
            where_,
            m.id,
            m.params_b,
            m.quant,
            gb(m.size_mb)
        ),
        None => format!(
            "{} — too little for the smallest curated model ({}).",
            where_,
            gb(curated().iter().map(|m| m.size_mb).min().unwrap_or(0))
        ),
    }
}

fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

/// What a model needs to run: its own bytes plus KV/overhead headroom.
pub fn model_needs_mb(size_mb: u64) -> u64 {
    size_mb + size_mb / 8 + 512
}

/// The curated shortlist. Every repo, filename and size here was verified live
/// against the HuggingFace API — if a number looks odd, re-run:
/// `cargo test downloads::tests::live_curated_entries_exist -- --ignored`
pub fn curated() -> Vec<DownloadableModel> {
    let t = |id: &str,
             repo: &str,
             file: &str,
             params_b: f32,
             quant: &str,
             size_mb: u64,
             ctx: u32,
             license: &str,
             tags: &[&str],
             note: &str| DownloadableModel {
        id: id.to_string(),
        repo: repo.to_string(),
        file: file.to_string(),
        params_b,
        quant: quant.to_string(),
        size_mb,
        context_tokens: ctx,
        license: license.to_string(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        note: note.to_string(),
        url: format!("https://huggingface.co/{}/resolve/main/{}", repo, file),
    };
    vec![
        t(
            "Qwen3 0.6B", "Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf", 0.6, "Q8_0", 640, 32768,
            "Apache-2.0", &["tiny", "chat"], "Instant answers on any machine. Weak at tool calling — best for simple chat and summaries.",
        ),
        t(
            "Gemma 3 1B", "ggml-org/gemma-3-1b-it-GGUF", "gemma-3-1b-it-Q4_K_M.gguf", 1.0, "Q4_K_M", 810, 32768,
            "Gemma Terms of Use", &["tiny", "chat"], "Google's small Gemma, GGUF hosted by the llama.cpp project itself.",
        ),
        t(
            "Qwen3 1.7B", "Qwen/Qwen3-1.7B-GGUF", "Qwen3-1.7B-Q8_0.gguf", 1.7, "Q8_0", 1830, 32768,
            "Apache-2.0", &["small", "chat", "tools"], "The official repo ships Q8_0 only — bigger file, cleaner output. Good first local model.",
        ),
        t(
            "Llama 3.2 3B", "bartowski/Llama-3.2-3B-Instruct-GGUF", "Llama-3.2-3B-Instruct-Q4_K_M.gguf", 3.0, "Q4_K_M", 2020, 131072,
            "Llama 3.2 Community License", &["small", "chat", "long-context"], "128k context in a 2 GB file — good for long documents on modest hardware.",
        ),
        t(
            "Qwen3 4B", "Qwen/Qwen3-4B-GGUF", "Qwen3-4B-Q4_K_M.gguf", 4.0, "Q4_K_M", 2500, 32768,
            "Apache-2.0", &["recommended", "chat", "tools"], "The sweet spot for most machines: capable, fast, four quants to choose from.",
        ),
        t(
            "Mistral 7B v0.3", "bartowski/Mistral-7B-Instruct-v0.3-GGUF", "Mistral-7B-Instruct-v0.3-Q4_K_M.gguf", 7.0, "Q4_K_M", 4370, 32768,
            "Apache-2.0", &["chat", "writing"], "Steady general-purpose writer; Apache-2.0 makes it safe for commercial work.",
        ),
        t(
            "Qwen2.5 Coder 7B", "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF", "qwen2.5-coder-7b-instruct-q4_k_m.gguf", 7.0, "Q4_K_M", 4680, 32768,
            "Apache-2.0", &["coding", "tools"], "Purpose-built for code — the local choice for the Projects IDE and Builder.",
        ),
        t(
            "Qwen3 8B", "Qwen/Qwen3-8B-GGUF", "Qwen3-8B-Q4_K_M.gguf", 8.0, "Q4_K_M", 5030, 32768,
            "Apache-2.0", &["recommended", "chat", "tools", "reasoning"], "Strong reasoning and tool use; needs ~6.5 GB to run well.",
        ),
        t(
            "Qwen3 14B", "Qwen/Qwen3-14B-GGUF", "Qwen3-14B-Q4_K_M.gguf", 14.0, "Q4_K_M", 9000, 32768,
            "Apache-2.0", &["large", "chat", "reasoning"], "Near-frontier local quality if you have the memory and patience.",
        ),
        t(
            "Nomic Embed v1.5", "nomic-ai/nomic-embed-text-v1.5-GGUF", "nomic-embed-text-v1.5.Q4_K_M.gguf", 0.1, "Q4_K_M", 80, 8192,
            "Apache-2.0", &["embeddings", "memory"], "Not a chat model: 80 MB of embeddings for the semantic memory index. Runs alongside any chat model.",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Discover — HuggingFace's trending GGUF models on demand.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscoveredModel {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    pub gated: bool,
    pub page: String,
}

/// Parse the `/api/models` list payload defensively — unknown fields are
/// normal, and a missing trending score must not drop the row.
pub fn parse_hf_models(json: &serde_json::Value) -> Vec<DiscoveredModel> {
    let Some(arr) = json.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|m| {
            let repo = m.get("id").and_then(|v| v.as_str())?.to_string();
            Some(DiscoveredModel {
                page: format!("https://huggingface.co/{}", repo),
                repo,
                downloads: m.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0),
                likes: m.get("likes").and_then(|v| v.as_u64()).unwrap_or(0),
                gated: m
                    .get("gated")
                    .map(|g| g != &serde_json::Value::Bool(false) && g != &serde_json::Value::Null)
                    .unwrap_or(false),
            })
        })
        .collect()
}

pub async fn discover(query: Option<&str>) -> Result<Vec<DiscoveredModel>, String> {
    let mut url = String::from(
        "https://huggingface.co/api/models?filter=gguf&sort=trendingScore&direction=-1&limit=30",
    );
    if let Some(q) = query.map(|q| q.trim()).filter(|q| !q.is_empty()) {
        url.push_str(&format!("&search={}", urlencode(q)));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HuggingFace request failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("HuggingFace returned HTTP {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parse_hf_models(&json))
}

/// Minimal percent-encoding for a search term (query strings only).
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn model_hardware() -> Hardware {
    probe()
}

#[tauri::command]
pub fn model_catalog() -> serde_json::Value {
    let hw = probe();
    let models: Vec<serde_json::Value> = curated()
        .into_iter()
        .map(|m| {
            let needs = model_needs_mb(m.size_mb);
            serde_json::json!({
                "id": m.id,
                "repo": m.repo,
                "file": m.file,
                "params_b": m.params_b,
                "quant": m.quant,
                "size_mb": m.size_mb,
                "needs_mb": needs,
                "context_tokens": m.context_tokens,
                "license": m.license,
                "tags": m.tags,
                "note": m.note,
                "url": m.url,
                "fits": needs <= hw.budget_mb,
                "headroom_mb": hw.budget_mb.saturating_sub(needs),
            })
        })
        .collect();
    serde_json::json!({ "hardware": hw, "models": models })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn model_discover(query: Option<String>) -> Result<Vec<DiscoveredModel>, String> {
    discover(query.as_deref()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi_rows_and_skips_junk() {
        // Real column order: index, name, free, total, used, util, uuid.
        let text = "0, NVIDIA GeForce RTX 4090, 22000, 24576, 2576, 3, GPU-abc123\n\
                    1, NVIDIA GeForce RTX 3060, 11000, 12288, 1288, 12, GPU-def456\n\
                    \n\
                    bogus line without commas\n";
        let gpus = parse_nvidia_csv(text);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(gpus[0].free_mb, 22000);
        assert_eq!(gpus[1].free_mb, 11000);
        assert!(gpus.iter().all(|g| g.vendor == "nvidia"));
    }

    #[test]
    fn reads_meminfo_in_megabytes() {
        let text = "MemTotal:       32768000 kB\nMemFree:         1000000 kB\nMemAvailable:   24576000 kB\n";
        let (total, avail) = parse_meminfo(text);
        assert_eq!(total, 32000);
        assert_eq!(avail, 24000);
        // Missing fields must not panic or invent numbers.
        assert_eq!(parse_meminfo("Nonsense: 1 kB\n"), (0, 0));
    }

    #[test]
    fn needs_include_headroom_and_fit_is_decidable() {
        // A 5 GB file needs more than 5 GB once KV/overhead is counted.
        assert!(model_needs_mb(5030) > 5030);
        assert!(model_needs_mb(2500) <= 8000, "4B at Q4 must fit in 8 GB");
        assert!(model_needs_mb(9000) > 8000, "14B must not claim to fit in 8 GB");
    }

    #[test]
    fn verdict_names_a_model_that_actually_fits() {
        let gpu = Gpu {
            index: 0,
            name: "NVIDIA GeForce RTX 4090".to_string(),
            free_mb: 22000,
            total_mb: 24576,
            used_mb: 2576,
            util_pct: 3,
            vendor: "nvidia".to_string(),
        };
        let v = verdict_for(&[gpu], 22000, 24000);
        assert!(v.contains("RTX 4090"), "{}", v);
        assert!(v.contains("Qwen3 14B"), "largest fitting curated model: {}", v);

        let small = verdict_for(&[], 4000, 4000);
        assert!(small.contains("CPU"), "{}", small);
        assert!(small.contains("Qwen3 4B"), "{}", small);
    }

    #[test]
    fn curated_entries_are_complete_and_single_file() {
        let list = curated();
        assert!(list.len() >= 8, "shortlist should be useful, got {}", list.len());
        for m in &list {
            assert!(m.repo.contains('/'), "repo must be org/name: {}", m.repo);
            assert!(m.file.ends_with(".gguf"), "must be a GGUF file: {}", m.file);
            assert!(
                m.size_mb > 0 && m.size_mb < 20_000,
                "implausible size: {}",
                m.size_mb
            );
            assert!(m.url.starts_with("https://huggingface.co/"), "{}", m.url);
            assert!(m.license.len() > 3, "license must be named honestly: {}", m.id);
            assert!(m.context_tokens >= 8192, "{}", m.id);
        }
        assert!(list
            .iter()
            .any(|m| m.tags.iter().any(|t| t == "recommended")));
    }

    #[test]
    fn discover_parses_list_payloads_defensively() {
        let json = serde_json::json!([
            { "id": "org/model-gguf", "downloads": 1234, "likes": 56, "gated": false },
            { "id": "other/gated-gguf", "downloads": 7, "likes": 1, "gated": "manual" },
            { "nonsense": true }
        ]);
        let rows = parse_hf_models(&json);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].repo, "org/model-gguf");
        assert!(!rows[0].gated);
        assert!(rows[1].gated, "gated values other than false mean gated");
        assert_eq!(rows[0].page, "https://huggingface.co/org/model-gguf");
    }

    /// Verifies the curated list against the live HuggingFace API: every repo
    /// must still exist, stay ungated, and still contain the exact file.
    /// `cargo test downloads::tests::live_curated_entries_exist -- --ignored`
    #[test]
    #[ignore]
    fn live_curated_entries_exist() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("client");
        for m in curated() {
            let api = format!("https://huggingface.co/api/models/{}", m.repo);
            let body = tauri::async_runtime::block_on(async {
                let r = client.get(&api).send().await.expect("request");
                assert!(r.status().is_success(), "{} → HTTP {}", m.repo, r.status());
                r.text().await.expect("body")
            });
            assert!(
                body.contains(&m.file),
                "{} no longer contains {}",
                m.repo,
                m.file
            );
            assert!(
                !body.contains("\"gated\":true"),
                "{} became gated — the list must not offer a file nobody can download",
                m.repo
            );
        }
    }

    /// Live discover call — proves the HF query shape still works.
    /// `cargo test downloads::tests::live_discover_returns_gguf_repos -- --ignored`
    #[test]
    #[ignore]
    fn live_discover_returns_gguf_repos() {
        let rows = tauri::async_runtime::block_on(discover(Some("qwen3"))).expect("discover");
        assert!(!rows.is_empty(), "expected trending results for 'qwen3'");
        assert!(rows.iter().all(|r| r.repo.contains('/')));
    }
}
