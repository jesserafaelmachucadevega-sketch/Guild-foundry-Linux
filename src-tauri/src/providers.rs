// SPDX-License-Identifier: Apache-2.0
// Phase 2 — Provider abstraction layer, handshake, model catalog, streaming chat.
//
// Master spec sections 10-14: providers are never hard-coded into the UI; all
// privileged work (HTTP, credentials, streaming) happens here in Rust. The
// frontend talks to these commands only through the frozen contracts in
// src/lib/providers.ts and src/lib/chat.ts (snake_case channels).
//
// Supported kinds: openai, openai-compatible, anthropic, google, openrouter,
// ollama, custom. Streaming parses real SSE / NDJSON per provider family.
// Pricing is NEVER inferred from a model name — only explicit signals
// (OpenRouter `:free` suffix or zero pricing fields) mark a model free.
//
// NOTE: this module cannot be compile-checked in this environment (no Rust
// toolchain); it is written carefully but is NOT VERIFIED — see
// .integration/phase-2.md.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

// ---------------------------------------------------------------------------
// Diagnostics counters (Phase 11 reads these).
// ---------------------------------------------------------------------------

pub(crate) static TOKENS_IN: AtomicU64 = AtomicU64::new(0);
pub(crate) static TOKENS_OUT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record_tokens(prompt: u64, completion: u64) {
    TOKENS_IN.fetch_add(prompt, Ordering::Relaxed);
    TOKENS_OUT.fetch_add(completion, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Shared state.
// ---------------------------------------------------------------------------

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

fn http() -> &'static reqwest::Client {
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .expect("failed to build HTTP client")
    })
}

/// Stream cancellation flags, keyed by stream_id.
static CANCEL: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn cancel_map() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    CANCEL.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Wire types (must match src/lib/providers.ts / src/lib/chat.ts exactly).
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProviderSummary {
    id: String,
    name: String,
    kind: String,
    base_url: String,
    models_path: String,
    has_key: bool,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_check_at: Option<i64>,
}

#[derive(Serialize)]
struct ModelInfo {
    id: String,
    name: String,
    provider_id: String,
    pricing_verified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_length: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_reasoning: Option<bool>,
}

#[derive(Serialize)]
struct Capabilities {
    tool_calling: bool,
    vision: bool,
    structured_output: bool,
}

#[derive(Serialize)]
struct HandshakeReport {
    provider_id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    raw_code: String,
    layman: String,
    detail: String,
    latency_ms: u64,
    models_found: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    capabilities: Option<Capabilities>,
}

#[derive(Serialize)]
struct RouterSuggestion {
    provider_id: String,
    model_id: String,
    reason: String,
}

#[derive(Serialize)]
struct CompleteResult {
    content: String,
    latency_ms: u64,
}

#[derive(Deserialize)]
struct ChatMsg {
    role: String,
    content: String,
}

/// camelCase: these keys come from the TypeScript ChatParams contract.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ChatParams {
    system_prompt: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    reasoning_effort: Option<String>,
}

#[derive(Serialize)]
struct ChunkPayload {
    stream_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion_tokens: Option<u64>,
}

fn emit_chunk(app: &tauri::AppHandle, p: &ChunkPayload) {
    let _ = app.emit("provider://chat-chunk", p);
}

// ---------------------------------------------------------------------------
// Internal types.
// ---------------------------------------------------------------------------

struct ProviderCfg {
    id: String,
    name: String,
    kind: String,
    base_url: String,
    models_path: String,
}

/// Normalized model parsed from a provider's model listing.
struct RawModel {
    id: String,
    name: String,
    context_length: Option<u64>,
    pricing: &'static str, // "free" | "paid" | "unknown"
    tools: bool,
    vision: bool,
    reasoning: bool,
    structured: bool,
}

const KNOWN_KINDS: &[&str] = &[
    "openai",
    "openai-compatible",
    "anthropic",
    "google",
    "openrouter",
    "ollama",
    "custom",
];

fn default_base_url(kind: &str) -> &'static str {
    match kind {
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        "openrouter" => "https://openrouter.ai/api/v1",
        "ollama" => "http://localhost:11434",
        _ => "",
    }
}

fn default_models_path(kind: &str) -> &'static str {
    match kind {
        "anthropic" => "/v1/models",
        "ollama" => "/api/tags",
        _ => "/models",
    }
}

fn provider_key_name(id: &str) -> String {
    format!("gfa-provider:{}", id)
}

// ---------------------------------------------------------------------------
// Database helpers (reuse db.rs tables; additive columns only).
// ---------------------------------------------------------------------------

fn open_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let conn =
        Connection::open(crate::secrets::db_path(app)?).map_err(|e| e.to_string())?;
    ensure_provider_columns(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Phase 1's `providers` table predates `models_path`/status tracking.
/// These columns are added lazily and additively — never destructively.
fn ensure_provider_columns(conn: &Connection) -> rusqlite::Result<()> {
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(providers)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut add = |name: &str, ddl: &str| -> rusqlite::Result<()> {
        if !cols.iter().any(|c| c == name) {
            conn.execute_batch(&format!("ALTER TABLE providers ADD COLUMN {}", ddl))?;
        }
        Ok(())
    };
    add("models_path", "models_path TEXT NOT NULL DEFAULT ''")?;
    add("status", "status TEXT NOT NULL DEFAULT 'idle'")?;
    add("last_error_code", "last_error_code TEXT")?;
    add("last_check_at", "last_check_at INTEGER")?;
    Ok(())
}

fn load_provider(app: &tauri::AppHandle, id: &str) -> Result<ProviderCfg, String> {
    let conn = open_db(app)?;
    conn.query_row(
        "SELECT id, name, kind, base_url, COALESCE(models_path,'') FROM providers WHERE id = ?1",
        params![id],
        |row| {
            Ok(ProviderCfg {
                id: row.get(0)?,
                name: row.get(1)?,
                kind: row.get(2)?,
                base_url: row.get(3)?,
                models_path: row.get(4)?,
            })
        },
    )
    .map_err(|_| format!("unknown provider: {}", id))
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn with_query(base: &str, pairs: &[(&str, &str)]) -> Result<String, String> {
    let mut url =
        reqwest::Url::parse(base).map_err(|e| format!("invalid URL '{}': {}", base, e))?;
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
    }
    Ok(url.to_string())
}

// ---------------------------------------------------------------------------
// Error mapping: ALWAYS a raw code + a layman explanation.
// ---------------------------------------------------------------------------

/// Extract the most specific message the server itself provided.
fn server_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    // OpenAI-style {"error":{"message":"..."}} / Google {"error":{"message":...}}
    if let Some(m) = v.pointer("/error/message").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            return Some(m.to_string());
        }
    }
    // Anthropic {"type":"error","error":{"type":"...","message":"..."}} /
    // Ollama {"error":"..."}
    if let Some(m) = v.pointer("/error").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            return Some(m.to_string());
        }
    }
    if let Some(m) = v.get("message").and_then(|x| x.as_str()) {
        if !m.is_empty() {
            return Some(m.to_string());
        }
    }
    None
}

/// (raw_code, layman, detail) for an HTTP failure.
fn http_error(status: u16, body: &str) -> (String, String, String) {
    let layman = match status {
        400 => "Bad Request",
        401 => "API Key Incorrect",
        402 => "Out of Funds",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        429 => "Rate Limit Exceeded",
        500 | 502 | 503 => "Server Down",
        504 => "Gateway Timeout",
        _ => "Request Failed",
    };
    let detail = server_message(body).unwrap_or_else(|| layman.to_string());
    (status.to_string(), layman.to_string(), detail)
}

/// (raw_code, layman, detail) for a transport-level failure.
fn net_error(e: &reqwest::Error) -> (String, String, String) {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    let (raw, layman) = if e.is_timeout() {
        ("ETIMEOUT", "Request timed out")
    } else if e.is_connect() {
        if lower.contains("dns") || lower.contains("failed to lookup") || lower.contains("name resolution") {
            ("DNS_ERROR", "Could not resolve the server address")
        } else if lower.contains("refused") {
            ("CONNECTION_REFUSED", "Server refused the connection")
        } else {
            ("CONNECTION_FAILED", "Could not reach the server")
        }
    } else if lower.contains("tls") || lower.contains("certificate") {
        ("TLS_ERROR", "Secure connection failed")
    } else {
        ("NETWORK_ERROR", "Network request failed")
    };
    let detail: String = msg.chars().take(300).collect();
    (raw.to_string(), layman.to_string(), detail)
}

fn err_text(raw: &str, layman: &str, detail: &str) -> String {
    if detail.is_empty() || detail == layman {
        format!("{} [{}]", layman, raw)
    } else {
        format!("{} [{}]: {}", layman, raw, detail)
    }
}

// ---------------------------------------------------------------------------
// Provider commands.
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn providers_any_connected(app: tauri::AppHandle) -> Result<bool, String> {
    let conn = open_db(&app)?;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM providers WHERE status = 'connected'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

#[tauri::command]
pub fn provider_list(app: tauri::AppHandle) -> Result<Vec<ProviderSummary>, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, kind, base_url, COALESCE(models_path,''), \
             COALESCE(status,'idle'), last_error_code, last_check_at \
             FROM providers ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        let (id, name, kind, base_url, models_path, status, last_error_code, last_check_at) =
            r.map_err(|e| e.to_string())?;
        let has_key = crate::secrets::keyring_get(&provider_key_name(&id)).is_some();
        out.push(ProviderSummary {
            id,
            name,
            kind,
            base_url,
            models_path,
            has_key,
            status,
            last_error_code,
            last_check_at,
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn provider_upsert(
    app: tauri::AppHandle,
    id: Option<String>,
    name: String,
    kind: String,
    base_url: String,
    models_path: String,
    api_key: Option<String>,
) -> Result<ProviderSummary, String> {
    if !KNOWN_KINDS.contains(&kind.as_str()) {
        return Err(format!("unknown provider kind: {}", kind));
    }
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("provider name is required".to_string());
    }
    let mut base_url = base_url.trim().to_string();
    if base_url.is_empty() {
        base_url = default_base_url(&kind).to_string();
    }
    if base_url.is_empty() {
        return Err("base_url is required for this provider kind".to_string());
    }
    reqwest::Url::parse(&base_url)
        .map_err(|e| format!("invalid base_url '{}': {}", base_url, e))?;
    let mut models_path = models_path.trim().to_string();
    if models_path.is_empty() {
        models_path = default_models_path(&kind).to_string();
    }
    if !models_path.starts_with('/') {
        models_path = format!("/{}", models_path);
    }

    let id = match id {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => uuid::Uuid::new_v4().to_string(),
    };

    let conn = open_db(&app)?;
    conn.execute(
        "INSERT INTO providers (id, name, kind, base_url, models_path, status, enabled)
         VALUES (?1, ?2, ?3, ?4, ?5, 'idle', 1)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name, kind = excluded.kind,
           base_url = excluded.base_url, models_path = excluded.models_path",
        params![id, name, kind, base_url, models_path],
    )
    .map_err(|e| e.to_string())?;

    // Store/rotate the key in the OS keyring; the key is never returned.
    if let Some(k) = api_key {
        if !k.is_empty() {
            crate::secrets::secret_set(app.clone(), provider_key_name(&id), k)?;
        }
    }

    let has_key = crate::secrets::keyring_get(&provider_key_name(&id)).is_some();
    let (status, last_error_code, last_check_at): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT COALESCE(status,'idle'), last_error_code, last_check_at FROM providers WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    Ok(ProviderSummary {
        id,
        name,
        kind,
        base_url,
        models_path,
        has_key,
        status,
        last_error_code,
        last_check_at,
    })
}

#[tauri::command]
pub fn provider_delete(app: tauri::AppHandle, provider_id: String) -> Result<(), String> {
    // Ensure it exists first so deletes of unknown ids fail loudly.
    load_provider(&app, &provider_id)?;
    let conn = open_db(&app)?;
    conn.execute("DELETE FROM models WHERE provider_id = ?1", params![provider_id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM providers WHERE id = ?1", params![provider_id])
        .map_err(|e| e.to_string())?;
    let _ = crate::secrets::secret_delete(app, provider_key_name(&provider_id));
    Ok(())
}

// ---------------------------------------------------------------------------
// Handshake.
// ---------------------------------------------------------------------------

/// Fetch + parse + store the model catalog. Updates provider status.
/// Returns the parsed models on success, or (raw_code, layman, detail) on failure.
async fn fetch_and_store_models(
    app: &tauri::AppHandle,
    cfg: &ProviderCfg,
) -> Result<Vec<RawModel>, (String, String, String)> {
    let key = crate::secrets::keyring_get(&provider_key_name(&cfg.id));
    let models_url = join_url(&cfg.base_url, &cfg.models_path);
    let models_url = if cfg.kind == "google" {
        let k = key
            .as_deref()
            .ok_or_else(|| {
                (
                    "NO_KEY".to_string(),
                    "API Key Missing".to_string(),
                    "Add an API key for this provider before connecting.".to_string(),
                )
            })?;
        with_query(&models_url, &[("key", k), ("pageSize", "100")])
            .map_err(|e| ("BAD_URL".to_string(), "Invalid endpoint URL".to_string(), e))?
    } else {
        reqwest::Url::parse(&models_url)
            .map_err(|e| {
                (
                    "BAD_URL".to_string(),
                    "Invalid endpoint URL".to_string(),
                    e.to_string(),
                )
            })?
            .to_string()
    };

    let mut req = http().get(&models_url);
    match cfg.kind.as_str() {
        "anthropic" => {
            let k = key.as_deref().ok_or_else(|| {
                (
                    "NO_KEY".to_string(),
                    "API Key Missing".to_string(),
                    "Add an API key for this provider before connecting.".to_string(),
                )
            })?;
            req = req
                .header("x-api-key", k)
                .header("anthropic-version", "2023-06-01");
        }
        "ollama" => {}
        _ => {
            let k = key.as_deref().ok_or_else(|| {
                (
                    "NO_KEY".to_string(),
                    "API Key Missing".to_string(),
                    "Add an API key for this provider before connecting.".to_string(),
                )
            })?;
            req = req.header("Authorization", format!("Bearer {}", k));
        }
    }

    let resp = tokio::time::timeout(Duration::from_secs(30), req.send())
        .await
        .map_err(|_| {
            (
                "ETIMEOUT".to_string(),
                "Request timed out".to_string(),
                "The models endpoint did not answer within 30 seconds.".to_string(),
            )
        })?
        .map_err(net_error)?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let (raw, layman, detail) = http_error(status.as_u16(), &body);
        return Err((raw, layman, detail));
    }
    let body: serde_json::Value = tokio::time::timeout(Duration::from_secs(30), resp.json())
        .await
        .map_err(|_| {
            (
                "ETIMEOUT".to_string(),
                "Request timed out".to_string(),
                "Timed out reading the model list.".to_string(),
            )
        })?
        .map_err(|e| net_error(&e))?;

    let models = parse_models(&cfg.kind, &body);
    store_models(app, cfg, &models).map_err(|e| {
        (
            "DB_ERROR".to_string(),
            "Could not save the model catalog".to_string(),
            e,
        )
    })?;
    Ok(models)
}

fn mark_status(
    app: &tauri::AppHandle,
    cfg: &ProviderCfg,
    ok: bool,
    raw_code: Option<&str>,
) -> Result<(), String> {
    let conn = open_db(app)?;
    let now = chrono::Utc::now().timestamp_millis();
    if ok {
        conn.execute(
            "UPDATE providers SET status='connected', last_error_code=NULL, last_check_at=?1 WHERE id=?2",
            params![now, cfg.id],
        )
    } else {
        conn.execute(
            "UPDATE providers SET status='error', last_error_code=?1, last_check_at=?2 WHERE id=?3",
            params![raw_code, now, cfg.id],
        )
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn provider_handshake(
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<HandshakeReport, String> {
    let started = Instant::now();
    let cfg = load_provider(&app, &provider_id)?;

    // Step 1: validate configuration before any network use.
    if !KNOWN_KINDS.contains(&cfg.kind.as_str()) {
        return Err(format!("unknown provider kind: {}", cfg.kind));
    }
    if reqwest::Url::parse(&cfg.base_url).is_err() {
        return Err(format!("invalid base_url: {}", cfg.base_url));
    }
    if !cfg.models_path.starts_with('/') {
        return Err("models_path must start with '/'".to_string());
    }
    let latency = || started.elapsed().as_millis() as u64;

    // Steps 2-3: verify endpoint + retrieve models. Steps 4-9: parse
    // capabilities, context limits, pricing from the listing.
    match fetch_and_store_models(&app, &cfg).await {
        Ok(models) => {
            let _ = mark_status(&app, &cfg, true, None);
            let capabilities = Capabilities {
                tool_calling: models.iter().any(|m| m.tools),
                vision: models.iter().any(|m| m.vision),
                structured_output: models.iter().any(|m| m.structured),
            };
            Ok(HandshakeReport {
                provider_id: cfg.id,
                ok: true,
                http_status: Some(200),
                raw_code: "200".to_string(),
                layman: "Connected".to_string(),
                detail: format!(
                    "Endpoint verified; {} model{} cataloged.",
                    models.len(),
                    if models.len() == 1 { "" } else { "s" }
                ),
                latency_ms: latency(),
                models_found: models.len(),
                capabilities: Some(capabilities),
            })
        }
        Err((raw, layman, detail)) => {
            let _ = mark_status(&app, &cfg, false, Some(&raw));
            let http_status: Option<u16> = raw.parse().ok();
            Ok(HandshakeReport {
                provider_id: cfg.id,
                ok: false,
                http_status,
                raw_code: raw,
                layman,
                detail,
                latency_ms: latency(),
                models_found: 0,
                capabilities: None,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Model catalog parsing + storage.
// ---------------------------------------------------------------------------

/// Pricing comes ONLY from explicit signals — never from a model name.
fn openrouter_pricing(id: &str, obj: &serde_json::Value) -> &'static str {
    if id.ends_with(":free") {
        return "free";
    }
    let pricing = &obj["pricing"];
    if pricing.is_object() {
        let zero = |v: &serde_json::Value| {
            v.as_str().map(|s| s == "0" || s == "0.0").unwrap_or(false)
                || v.as_f64().map(|f| f == 0.0).unwrap_or(false)
        };
        let fields = ["prompt", "completion", "request", "image"];
        let present: Vec<&serde_json::Value> =
            fields.iter().filter_map(|f| pricing.get(*f)).collect();
        if !present.is_empty() {
            if present.iter().all(|v| zero(v)) {
                return "free";
            }
            return "paid";
        }
    }
    "unknown"
}

fn str_in(arr: &serde_json::Value, needle: &str) -> bool {
    arr.as_array().map(|a| {
        a.iter().any(|v| {
            v.as_str()
                .map(|s| s.eq_ignore_ascii_case(needle))
                .unwrap_or(false)
        })
    })
    .unwrap_or(false)
}

fn parse_models(kind: &str, body: &serde_json::Value) -> Vec<RawModel> {
    let mut out = Vec::new();
    match kind {
        "anthropic" => {
            if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                for m in arr {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    out.push(RawModel {
                        id: id.to_string(),
                        name: m
                            .get("display_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(id)
                            .to_string(),
                        context_length: None,
                        pricing: "unknown",
                        tools: false,
                        vision: false,
                        reasoning: false,
                        structured: false,
                    });
                }
            }
        }
        "google" => {
            if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
                for m in arr {
                    let raw = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let id = raw.strip_prefix("models/").unwrap_or(raw);
                    if id.is_empty() {
                        continue;
                    }
                    out.push(RawModel {
                        id: id.to_string(),
                        name: m
                            .get("displayName")
                            .and_then(|v| v.as_str())
                            .unwrap_or(id)
                            .to_string(),
                        context_length: m
                            .get("inputTokenLimit")
                            .and_then(|v| v.as_u64()),
                        pricing: "unknown",
                        tools: false,
                        vision: false,
                        reasoning: false,
                        structured: false,
                    });
                }
            }
        }
        "ollama" => {
            if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
                for m in arr {
                    let id = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    out.push(RawModel {
                        id: id.to_string(),
                        name: m
                            .get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or(id)
                            .to_string(),
                        context_length: None,
                        pricing: "free", // local models have no per-token cost
                        tools: false,
                        vision: false,
                        reasoning: false,
                        structured: false,
                    });
                }
            }
        }
        _ => {
            // OpenAI-compatible listing (covers openai, openai-compatible,
            // openrouter, custom).
            if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                for m in arr {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let is_openrouter = kind == "openrouter";
                    let pricing = if is_openrouter {
                        openrouter_pricing(id, m)
                    } else {
                        "unknown"
                    };
                    let supported = m.get("supported_parameters").unwrap_or(&serde_json::Value::Null);
                    out.push(RawModel {
                        id: id.to_string(),
                        name: m
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(id)
                            .to_string(),
                        context_length: m
                            .get("context_length")
                            .and_then(|v| v.as_u64())
                            .or_else(|| m.get("context_window").and_then(|v| v.as_u64())),
                        pricing,
                        tools: is_openrouter && str_in(supported, "tools"),
                        vision: false,
                        reasoning: false,
                        structured: is_openrouter
                            && (str_in(supported, "response_format")
                                || str_in(supported, "structured_outputs")),
                    });
                }
            }
        }
    }
    out
}

fn store_models(
    app: &tauri::AppHandle,
    cfg: &ProviderCfg,
    models: &[RawModel],
) -> Result<(), String> {
    let mut conn = open_db(app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM models WHERE provider_id = ?1", params![cfg.id])
        .map_err(|e| e.to_string())?;
    for m in models {
        let row_id = format!("{}:{}", cfg.id, m.id);
        let caps = serde_json::json!({
            "tools": m.tools,
            "vision": m.vision,
            "reasoning": m.reasoning,
            "structured": m.structured,
            "source": "provider_metadata",
        })
        .to_string();
        tx.execute(
            "INSERT OR REPLACE INTO models
             (id, provider_id, model_id, display_name, pricing_class, context_window, capabilities, favorite, hidden)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0)",
            params![
                row_id,
                cfg.id,
                m.id,
                m.name,
                m.pricing,
                m.context_length.map(|v| v as i64),
                caps
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn row_to_model_info(
    id: String,
    model_id: String,
    display_name: String,
    provider_id: String,
    pricing_class: String,
    context_window: Option<i64>,
    capabilities: String,
) -> ModelInfo {
    let caps: serde_json::Value =
        serde_json::from_str(&capabilities).unwrap_or(serde_json::json!({}));
    let flag = |k: &str| caps.get(k).and_then(|v| v.as_bool());
    ModelInfo {
        id,
        name: display_name,
        provider_id,
        pricing_verified: pricing_class,
        context_length: context_window.map(|v| v as u64),
        supports_tools: flag("tools"),
        supports_vision: flag("vision"),
        supports_reasoning: flag("reasoning"),
    }
}

#[tauri::command]
pub fn provider_list_models(
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<Vec<ModelInfo>, String> {
    load_provider(&app, &provider_id)?;
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, model_id, display_name, provider_id, pricing_class, context_window, capabilities
             FROM models WHERE provider_id = ?1 ORDER BY display_name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![provider_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        let (id, model_id, display_name, pid, pricing, ctx, caps) =
            r.map_err(|e| e.to_string())?;
        out.push(row_to_model_info(
            id,
            model_id,
            display_name,
            pid,
            pricing,
            ctx,
            caps,
        ));
    }
    Ok(out)
}

#[tauri::command]
pub async fn provider_refresh_models(
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<Vec<ModelInfo>, String> {
    let cfg = load_provider(&app, &provider_id)?;
    match fetch_and_store_models(&app, &cfg).await {
        Ok(_) => {
            let _ = mark_status(&app, &cfg, true, None);
            provider_list_models(app, provider_id)
        }
        Err((raw, layman, detail)) => {
            let _ = mark_status(&app, &cfg, false, Some(&raw));
            Err(err_text(&raw, &layman, &detail))
        }
    }
}

// ---------------------------------------------------------------------------
// Chat: request building per provider family.
// ---------------------------------------------------------------------------

struct ChatTarget {
    url: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
}

fn openai_messages(messages: &[ChatMsg], params: &ChatParams) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if let Some(sys) = params.system_prompt.as_deref() {
        if !sys.trim().is_empty() {
            out.push(serde_json::json!({"role": "system", "content": sys}));
        }
    }
    for m in messages {
        out.push(serde_json::json!({"role": m.role, "content": m.content}));
    }
    out
}

fn build_chat_target(
    cfg: &ProviderCfg,
    key: Option<&str>,
    model_id: &str,
    messages: &[ChatMsg],
    params: &ChatParams,
    stream: bool,
) -> Result<ChatTarget, String> {
    let no_key = || {
        (
            "NO_KEY".to_string(),
            "API Key Missing".to_string(),
            "Add an API key for this provider before chatting.".to_string(),
        )
    };
    match cfg.kind.as_str() {
        "anthropic" => {
            let k = key.ok_or_else(|| err_text(&no_key().0, &no_key().1, &no_key().2))?;
            let mut system_parts: Vec<String> = Vec::new();
            if let Some(s) = params.system_prompt.as_deref() {
                if !s.trim().is_empty() {
                    system_parts.push(s.to_string());
                }
            }
            let mut msgs = Vec::new();
            for m in messages {
                match m.role.as_str() {
                    "system" => system_parts.push(m.content.clone()),
                    "assistant" => {
                        msgs.push(serde_json::json!({"role": "assistant", "content": m.content}))
                    }
                    _ => msgs.push(serde_json::json!({"role": "user", "content": m.content})),
                }
            }
            let mut body = serde_json::json!({
                "model": model_id,
                "max_tokens": params.max_tokens.unwrap_or(4096),
                "messages": msgs,
                "stream": stream,
            });
            if !system_parts.is_empty() {
                body["system"] = serde_json::json!(system_parts.join("\n\n"));
            }
            if let Some(t) = params.temperature {
                body["temperature"] = serde_json::json!(t);
            }
            Ok(ChatTarget {
                url: join_url(&cfg.base_url, "/v1/messages"),
                headers: vec![
                    ("x-api-key".to_string(), k.to_string()),
                    ("anthropic-version".to_string(), "2023-06-01".to_string()),
                ],
                body,
            })
        }
        "google" => {
            let k = key.ok_or_else(|| {
                let (r, l, d) = no_key();
                err_text(&r, &l, &d)
            })?;
            let model = model_id.strip_prefix("models/").unwrap_or(model_id);
            let mut contents = Vec::new();
            let mut system_parts: Vec<String> = Vec::new();
            if let Some(s) = params.system_prompt.as_deref() {
                if !s.trim().is_empty() {
                    system_parts.push(s.to_string());
                }
            }
            for m in messages {
                match m.role.as_str() {
                    "system" => system_parts.push(m.content.clone()),
                    "assistant" => contents.push(
                        serde_json::json!({"role": "model", "parts": [{"text": m.content}]}),
                    ),
                    _ => contents.push(
                        serde_json::json!({"role": "user", "parts": [{"text": m.content}]}),
                    ),
                }
            }
            let mut body = serde_json::json!({ "contents": contents });
            if !system_parts.is_empty() {
                body["system_instruction"] =
                    serde_json::json!({"parts": [{"text": system_parts.join("\n\n")}]});
            }
            let mut gen = serde_json::json!({});
            if let Some(t) = params.temperature {
                gen["temperature"] = serde_json::json!(t);
            }
            if let Some(n) = params.max_tokens {
                gen["maxOutputTokens"] = serde_json::json!(n);
            }
            body["generationConfig"] = gen;
            let (suffix, extra): (&str, &[(&str, &str)]) = if stream {
                (":streamGenerateContent", &[("alt", "sse")])
            } else {
                (":generateContent", &[])
            };
            let mut qp: Vec<(&str, &str)> = vec![("key", k)];
            qp.extend_from_slice(extra);
            let url = with_query(
                &format!(
                    "{}/models/{}{}",
                    cfg.base_url.trim_end_matches('/'),
                    model,
                    suffix
                ),
                &qp,
            )?;
            Ok(ChatTarget { url, headers: vec![], body })
        }
        "ollama" => {
            let mut options = serde_json::json!({});
            if let Some(t) = params.temperature {
                options["temperature"] = serde_json::json!(t);
            }
            if let Some(n) = params.max_tokens {
                options["num_predict"] = serde_json::json!(n);
            }
            let body = serde_json::json!({
                "model": model_id,
                "messages": openai_messages(messages, params),
                "stream": stream,
                "options": options,
            });
            Ok(ChatTarget {
                url: join_url(&cfg.base_url, "/api/chat"),
                headers: vec![],
                body,
            })
        }
        _ => {
            // openai, openai-compatible, openrouter, custom
            let k = key.ok_or_else(|| {
                let (r, l, d) = no_key();
                err_text(&r, &l, &d)
            })?;
            let mut body = serde_json::json!({
                "model": model_id,
                "messages": openai_messages(messages, params),
                "stream": stream,
            });
            if let Some(t) = params.temperature {
                body["temperature"] = serde_json::json!(t);
            }
            if let Some(n) = params.max_tokens {
                body["max_tokens"] = serde_json::json!(n);
            }
            if cfg.kind == "openai" {
                if let Some(e) = params.reasoning_effort.as_deref() {
                    body["reasoning_effort"] = serde_json::json!(e);
                }
            }
            if stream {
                body["stream_options"] = serde_json::json!({"include_usage": true});
            }
            Ok(ChatTarget {
                url: join_url(&cfg.base_url, "/chat/completions"),
                headers: vec![("Authorization".to_string(), format!("Bearer {}", k))],
                body,
            })
        }
    }
}

fn send_chat(target: &ChatTarget) -> reqwest::RequestBuilder {
    let mut req = http().post(&target.url).json(&target.body);
    for (k, v) in &target.headers {
        req = req.header(k, v);
    }
    req
}

/// Parse a non-streaming completion into (text, prompt_tokens, completion_tokens).
fn parse_complete(kind: &str, body: &serde_json::Value) -> Result<(String, u64, u64), String> {
    if let Some(err) = body.get("error") {
        let msg = err
            .get("message")
            .and_then(|v| v.as_str())
            .or_else(|| err.as_str())
            .unwrap_or("provider returned an error");
        return Err(msg.to_string());
    }
    match kind {
        "anthropic" => {
            let mut text = String::new();
            if let Some(blocks) = body.get("content").and_then(|v| v.as_array()) {
                for b in blocks {
                    if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                            text.push_str(t);
                        }
                    }
                }
            }
            let pt = body.pointer("/usage/input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let ct = body.pointer("/usage/output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok((text, pt, ct))
        }
        "google" => {
            let mut text = String::new();
            if let Some(parts) = body
                .pointer("/candidates/0/content/parts")
                .and_then(|v| v.as_array())
            {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                }
            }
            let pt = body.pointer("/usageMetadata/promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
            let ct = body.pointer("/usageMetadata/candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok((text, pt, ct))
        }
        "ollama" => {
            let text = body
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let pt = body.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let ct = body.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok((text, pt, ct))
        }
        _ => {
            let text = body
                .pointer("/choices/0/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let pt = body.pointer("/usage/prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let ct = body.pointer("/usage/completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            Ok((text, pt, ct))
        }
    }
}

#[tauri::command]
pub async fn provider_chat_complete(
    app: tauri::AppHandle,
    provider_id: String,
    model_id: String,
    messages: Vec<ChatMsg>,
    params: ChatParams,
) -> Result<CompleteResult, String> {
    let started = Instant::now();
    // Rate limit + daily quota guard: fail fast before spending a request.
    crate::ratelimit::acquire(&app, &provider_id).await?;
    let cfg = load_provider(&app, &provider_id)?;
    let key = crate::secrets::keyring_get(&provider_key_name(&cfg.id));
    let target = build_chat_target(&cfg, key.as_deref(), &model_id, &messages, &params, false)?;
    let resp = tokio::time::timeout(Duration::from_secs(180), send_chat(&target).send())
        .await
        .map_err(|_| "Request timed out [ETIMEOUT]: the model did not answer within 3 minutes.".to_string())?
        .map_err(|e| {
            let (raw, layman, detail) = net_error(&e);
            err_text(&raw, &layman, &detail)
        })?;
    let status = resp.status();
    if !status.is_success() {
        // Read the error body as text: failure payloads are not always JSON.
        let body = resp.text().await.unwrap_or_default();
        let (raw, layman, detail) = http_error(status.as_u16(), &body);
        return Err(err_text(&raw, &layman, &detail));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| {
        let (raw, layman, detail) = net_error(&e);
        err_text(&raw, &layman, &detail)
    })?;
    let (content, pt, ct) = parse_complete(&cfg.kind, &body)?;
    record_tokens(pt, ct);
    Ok(CompleteResult {
        content,
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

// ---------------------------------------------------------------------------
// Streaming.
// ---------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
enum StreamKind {
    OpenAI,
    Anthropic,
    Google,
    Ollama,
}

fn stream_kind(kind: &str) -> StreamKind {
    match kind {
        "anthropic" => StreamKind::Anthropic,
        "google" => StreamKind::Google,
        "ollama" => StreamKind::Ollama,
        _ => StreamKind::OpenAI,
    }
}

struct StreamState {
    kind: StreamKind,
    prompt_tokens: u64,
    completion_tokens: u64,
    finished: bool,
}

fn sse_error_text(obj: &serde_json::Value) -> String {
    // OpenAI-style {"error": {...}} or Anthropic {"type":"error","error":{...}}
    let msg = obj
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .or_else(|| obj.pointer("/error").and_then(|v| v.as_str()))
        .or_else(|| obj.get("message").and_then(|v| v.as_str()))
        .unwrap_or("provider stream error");
    msg.to_string()
}

/// Handle one decoded SSE/NDJSON payload. Returns Some(delta) to emit.
fn handle_payload(state: &mut StreamState, data: &str) -> Result<Option<String>, String> {
    let obj: serde_json::Value =
        serde_json::from_str(data).map_err(|e| format!("unparseable stream payload: {}", e))?;
    match state.kind {
        StreamKind::OpenAI => {
            if obj.get("error").is_some() {
                return Err(sse_error_text(&obj));
            }
            if let Some(u) = obj.get("usage") {
                state.prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(state.prompt_tokens);
                state.completion_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(state.completion_tokens);
            }
            let delta = obj
                .pointer("/choices/0/delta/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if obj.pointer("/choices/0/finish_reason").and_then(|v| v.as_str()).is_some() {
                state.finished = true;
            }
            Ok(if delta.is_empty() { None } else { Some(delta) })
        }
        StreamKind::Anthropic => {
            let t = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match t {
                "error" => Err(sse_error_text(&obj)),
                "message_start" => {
                    state.prompt_tokens = obj
                        .pointer("/message/usage/input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    Ok(None)
                }
                "content_block_delta" => {
                    let is_text = obj.pointer("/delta/type").and_then(|v| v.as_str()) == Some("text_delta");
                    let text = if is_text {
                        obj.pointer("/delta/text").and_then(|v| v.as_str()).unwrap_or("")
                    } else {
                        ""
                    };
                    Ok(if text.is_empty() { None } else { Some(text.to_string()) })
                }
                "message_delta" => {
                    state.completion_tokens = obj
                        .pointer("/usage/output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(state.completion_tokens);
                    Ok(None)
                }
                "message_stop" => {
                    state.finished = true;
                    Ok(None)
                }
                _ => Ok(None),
            }
        }
        StreamKind::Google => {
            if obj.get("error").is_some() {
                return Err(sse_error_text(&obj));
            }
            let mut text = String::new();
            if let Some(parts) = obj.pointer("/candidates/0/content/parts").and_then(|v| v.as_array()) {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                }
            }
            state.prompt_tokens = obj.pointer("/usageMetadata/promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(state.prompt_tokens);
            state.completion_tokens = obj.pointer("/usageMetadata/candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(state.completion_tokens);
            Ok(if text.is_empty() { None } else { Some(text) })
        }
        StreamKind::Ollama => {
            if let Some(e) = obj.get("error").and_then(|v| v.as_str()) {
                return Err(e.to_string());
            }
            let text = obj
                .pointer("/message/content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if obj.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                state.prompt_tokens = obj.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
                state.completion_tokens = obj.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
                state.finished = true;
            }
            Ok(if text.is_empty() { None } else { Some(text) })
        }
    }
}

async fn run_stream(
    app: tauri::AppHandle,
    stream_id: &str,
    cfg: &ProviderCfg,
    model_id: &str,
    messages: &[ChatMsg],
    params: &ChatParams,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let started = Instant::now();
    let key = crate::secrets::keyring_get(&provider_key_name(&cfg.id));
    let target = build_chat_target(cfg, key.as_deref(), model_id, messages, params, true)?;

    let mut resp = send_chat(&target).send().await.map_err(|e| {
        let (raw, layman, detail) = net_error(&e);
        err_text(&raw, &layman, &detail)
    })?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let (raw, layman, detail) = http_error(status.as_u16(), &body);
        return Err(err_text(&raw, &layman, &detail));
    }

    let kind = stream_kind(&cfg.kind);
    let mut state = StreamState {
        kind,
        prompt_tokens: 0,
        completion_tokens: 0,
        finished: false,
    };
    let mut buf = String::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Stream cancelled [CANCELLED]".to_string());
        }
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(pos) = buf.find('\n') {
                    let line: String = buf[..pos].trim_end_matches('\r').to_string();
                    buf.drain(..=pos);
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Stream cancelled [CANCELLED]".to_string());
                    }
                    let t = line.trim();
                    if t.is_empty() || t.starts_with(':') {
                        continue;
                    }
                    let payload: Option<&str> = match state.kind {
                        StreamKind::Ollama => Some(t),
                        _ => t.strip_prefix("data:").map(|s| s.trim()),
                    };
                    let data = match payload {
                        Some(d) => d,
                        None => continue,
                    };
                    if data == "[DONE]" {
                        state.finished = true;
                        break;
                    }
                    match handle_payload(&mut state, data) {
                        Ok(Some(delta)) => emit_chunk(
                            &app,
                            &ChunkPayload {
                                stream_id: stream_id.to_string(),
                                delta: Some(delta),
                                done: None,
                                error: None,
                                latency_ms: None,
                                prompt_tokens: None,
                                completion_tokens: None,
                            },
                        ),
                        Ok(None) => {}
                        Err(e) => return Err(e),
                    }
                    if state.finished {
                        break;
                    }
                }
                if state.finished {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let (raw, layman, detail) = net_error(&e);
                return Err(err_text(&raw, &layman, &detail));
            }
        }
    }

    record_tokens(state.prompt_tokens, state.completion_tokens);
    emit_chunk(
        &app,
        &ChunkPayload {
            stream_id: stream_id.to_string(),
            delta: None,
            done: Some(true),
            error: None,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            prompt_tokens: Some(state.prompt_tokens),
            completion_tokens: Some(state.completion_tokens),
        },
    );
    Ok(())
}

#[tauri::command]
pub fn provider_chat_stream(
    app: tauri::AppHandle,
    stream_id: String,
    provider_id: String,
    model_id: String,
    messages: Vec<ChatMsg>,
    params: ChatParams,
) -> Result<(), String> {
    let cfg = load_provider(&app, &provider_id)?;
    let flag = Arc::new(AtomicBool::new(false));
    cancel_map()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(stream_id.clone(), Arc::clone(&flag));
    // The command returns immediately; the stream runs on the async runtime and
    // reports through the `provider://chat-chunk` event.
    tauri::async_runtime::spawn(async move {
        // Rate limit + daily quota guard before any network call.
        if let Err(e) = crate::ratelimit::acquire(&app, &provider_id).await {
            emit_chunk(
                &app,
                &ChunkPayload {
                    stream_id: stream_id.clone(),
                    delta: None,
                    done: None,
                    error: Some(e),
                    latency_ms: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                },
            );
            if let Ok(mut map) = cancel_map().lock() {
                map.remove(&stream_id);
            }
            return;
        }
        let res = run_stream(app.clone(), &stream_id, &cfg, &model_id, &messages, &params, &flag).await;
        if let Ok(mut map) = cancel_map().lock() {
            map.remove(&stream_id);
        }
        if let Err(e) = res {
            emit_chunk(
                &app,
                &ChunkPayload {
                    stream_id: stream_id.clone(),
                    delta: None,
                    done: None,
                    error: Some(e),
                    latency_ms: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                },
            );
        }
    });
    Ok(())
}

#[tauri::command]
pub fn provider_chat_cancel(stream_id: String) -> Result<(), String> {
    if let Ok(map) = cancel_map().lock() {
        if let Some(flag) = map.get(&stream_id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Model router (spec section 14). Disable-able; always explains its choice.
// ---------------------------------------------------------------------------

fn get_setting<T: serde::de::DeserializeOwned>(app: &tauri::AppHandle, key: &str) -> Option<T> {
    use tauri_plugin_store::StoreExt;
    let store = app.store("settings.json").ok()?;
    let v = store.get(key)?;
    serde_json::from_value(v).ok()
}

struct Candidate {
    row_id: String,
    model_id: String,
    name: String,
    provider_id: String,
    provider_name: String,
    pricing: String,
    context_length: Option<u64>,
    tools: bool,
    vision: bool,
    reasoning: bool,
}

fn pricing_rank(p: &str) -> u8 {
    match p {
        "free" => 0,
        "unknown" => 1,
        _ => 2,
    }
}

#[tauri::command]
pub fn provider_router_suggest(
    app: tauri::AppHandle,
    task: String,
) -> Result<RouterSuggestion, String> {
    let enabled: bool = get_setting(&app, "router.enabled").unwrap_or(true);
    if !enabled {
        let def: Option<serde_json::Value> = get_setting(&app, "router.default");
        if let Some(d) = def {
            let provider_id = d.get("provider_id").and_then(|v| v.as_str()).unwrap_or("");
            let model_id = d.get("model_id").and_then(|v| v.as_str()).unwrap_or("");
            if !provider_id.is_empty() && !model_id.is_empty() {
                return Ok(RouterSuggestion {
                    provider_id: provider_id.to_string(),
                    model_id: model_id.to_string(),
                    reason: "Automatic routing is off — using your configured default model.".to_string(),
                });
            }
        }
        return Err(
            "Automatic routing is disabled and no default model is configured.".to_string(),
        );
    }

    let hidden: Vec<String> = get_setting(&app, "models.hidden").unwrap_or_default();
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT m.id, m.model_id, m.display_name, m.provider_id, p.name,
                    m.pricing_class, m.context_window, m.capabilities
             FROM models m JOIN providers p ON p.id = m.provider_id
             WHERE p.status = 'connected' ORDER BY p.name, m.display_name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut cands: Vec<Candidate> = Vec::new();
    for r in rows {
        let (row_id, model_id, name, provider_id, provider_name, pricing, ctx, caps) =
            r.map_err(|e| e.to_string())?;
        if hidden.iter().any(|h| h == &row_id) {
            continue;
        }
        let caps_v: serde_json::Value =
            serde_json::from_str(&caps).unwrap_or(serde_json::json!({}));
        let flag = |k: &str| caps_v.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        cands.push(Candidate {
            row_id,
            model_id,
            name,
            provider_id,
            provider_name,
            pricing,
            context_length: ctx.map(|v| v as u64),
            tools: flag("tools"),
            vision: flag("vision"),
            reasoning: flag("reasoning"),
        });
    }
    if cands.is_empty() {
        return Err("No connected providers with cataloged models. Connect a provider first.".to_string());
    }

    // Task signals (plain keyword matching over the user's own description).
    let t = task.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    let wants_vision = has(&["image", "photo", "picture", "screenshot", "vision", "diagram", "ocr", "describe this"]);
    let wants_reason = has(&["reason", "math", "proof", "logic", "plan", "strategy", "step-by-step", "step by step", "complex", "think", "analyze"]);
    let wants_long = has(&["long", "large", "document", "codebase", "repo", "repository", "whole file", "book", "paper", "transcript"]);
    let wants_code = has(&["code", "debug", "refactor", "function", "python", "typescript", "javascript", "rust", "compile", "test"]);

    // Hard filters for signaled hard requirements; relax if nothing survives.
    let mut pool: Vec<&Candidate> = cands.iter().collect();
    let filtered: Vec<&Candidate> = pool
        .iter()
        .filter(|c| (!wants_vision || c.vision) && (!wants_reason || c.reasoning))
        .copied()
        .collect();
    if !filtered.is_empty() {
        pool = filtered;
    }

    pool.sort_by(|a, b| {
        let score = |c: &Candidate| -> (u8, i64, i64) {
            let mut match_pts: i64 = 0;
            if wants_vision && c.vision {
                match_pts += 40;
            }
            if wants_reason && c.reasoning {
                match_pts += 30;
            }
            if wants_long {
                if let Some(ctx) = c.context_length {
                    match_pts += (ctx.min(1_000_000) / 10_000) as i64;
                }
            }
            if wants_code && c.tools {
                match_pts += 10;
            }
            (
                pricing_rank(&c.pricing),
                -match_pts,
                -(c.context_length.unwrap_or(0) as i64),
            )
        };
        score(a).cmp(&score(b))
    });

    let best = pool[0];
    let mut why: Vec<String> = Vec::new();
    match best.pricing.as_str() {
        "free" => why.push("it is VERIFIED FREE".to_string()),
        "paid" => why.push("it is a verified paid model".to_string()),
        _ => why.push("its pricing is unverified".to_string()),
    }
    if wants_vision && best.vision {
        why.push("it supports vision".to_string());
    }
    if wants_reason && best.reasoning {
        why.push("it supports reasoning".to_string());
    }
    if wants_long {
        if let Some(ctx) = best.context_length {
            why.push(format!("it offers a {}k context window", ctx / 1000));
        }
    }
    if wants_code && best.tools {
        why.push("it supports tool calling".to_string());
    }
    why.push(format!("it is on a connected provider ({})", best.provider_name));

    Ok(RouterSuggestion {
        provider_id: best.provider_id.clone(),
        model_id: best.model_id.clone(),
        reason: format!("Selected '{}' because {}.", best.name, why.join(", ")),
    })
}
