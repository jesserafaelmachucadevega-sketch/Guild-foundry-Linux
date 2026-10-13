// SPDX-License-Identifier: Apache-2.0
// Integrations — external services and file formats the agent can reach.
//
// Submodules:
//   web.rs      — web.fetch (HTTP fetch + readable text/markdown extraction)
//   document.rs — document.extract_text/compare/convert/ocr + upload intake
//
// This module also owns:
//   1. `integration_handshake`: live key check against each service's real
//      endpoint, returning HTTP status + plain-language verdict for the
//      Connections status badges. Keys never travel here in logs; the
//      caller passes a key explicitly or the keyring copy is used.
//   2. Direct REST tools (`web.tavily_search`, `web.exa_search`,
//      `github.get_user`, `hf.whoami`) dispatched from `tools.rs`, so the
//      Personal Agent and Builder can search/research without an MCP server.
//      Keys come from the OS keyring only (`secret_set` in the UI).
//
// Keyring convention: `gfa-integration:<service>` — except Fal, which reuses
// the existing media key (`gfa-media-fal-key`, see `crate::media`).

pub mod calendar;
pub mod cloud;
pub mod document;
pub mod email;
pub mod twilio;
pub mod web;

use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;

/// Shared result shape for the Connections "Test" buttons of the native
/// account integrations (email, calendar, phone).
#[derive(Debug, Clone, Serialize)]
pub struct AccountTest {
    pub ok: bool,
    pub layman: String,
    pub detail: String,
    pub latency_ms: u64,
}

pub const KEY_PREFIX: &str = "gfa-integration:";

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("guild-foundry-ai/0.1.0")
            .build()
            .expect("failed to build integrations HTTP client")
    })
}

fn keyring_key(service: &str) -> String {
    if service == "fal" {
        crate::media::FAL_KEYRING_KEY.to_string()
    } else {
        format!("{}{}", KEY_PREFIX, service)
    }
}

fn stored_key(service: &str) -> Option<String> {
    crate::secrets::keyring_get(&keyring_key(service))
}

fn resolve_key(service: &str, api_key: Option<String>) -> Option<String> {
    match api_key {
        Some(k) if !k.trim().is_empty() => Some(k.trim().to_string()),
        _ => stored_key(service),
    }
}

/// Truncate free-form text for token economy (mirrors research caps).
fn snip(s: &str, max: usize) -> String {
    let t = s.trim().replace(|c: char| c.is_control() && c != '\n', " ");
    if t.len() <= max {
        return t;
    }
    format!("{}…", &t[..max])
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationStatus {
    pub service: String,
    pub ok: bool,
    pub http_status: Option<u16>,
    pub raw_code: String,
    pub layman: String,
    pub detail: String,
}

fn status(
    service: &str,
    ok: bool,
    http_status: Option<u16>,
    raw_code: &str,
    layman: &str,
    detail: String,
) -> IntegrationStatus {
    IntegrationStatus {
        service: service.to_string(),
        ok,
        http_status,
        raw_code: raw_code.to_string(),
        layman: layman.to_string(),
        detail,
    }
}

fn no_key(service: &str) -> IntegrationStatus {
    status(
        service,
        false,
        None,
        "NO_KEY",
        "API key missing",
        "Paste an API key and Save it first, then Test.".to_string(),
    )
}

fn offline(service: &str, e: String) -> IntegrationStatus {
    status(
        service,
        false,
        None,
        "OFFLINE",
        "Network unreachable",
        format!("The test request failed before getting an HTTP status: {}", snip(&e, 300)),
    )
}

/// Map a probe HTTP status to a verdict. `ok_detail` describes the success.
fn map_status(service: &str, http: u16, body: &str, ok_detail: String) -> IntegrationStatus {
    match http {
        200 | 201 => status(service, true, Some(http), "200", "Ready", ok_detail),
        401 | 403 => status(
            service,
            false,
            Some(http),
            "401",
            "Invalid API key",
            "The service rejected the key. Check for typos or revoked keys, then Save and Test again.".to_string(),
        ),
        402 => status(
            service,
            false,
            Some(http),
            "402",
            "Out of credits",
            "The key is valid but the account needs credits or a paid plan.".to_string(),
        ),
        429 => status(
            service,
            false,
            Some(http),
            "429",
            "Rate limited",
            "Too many requests — wait a minute and Test again.".to_string(),
        ),
        s if s >= 500 => status(
            service,
            false,
            Some(http),
            &s.to_string(),
            "Service error",
            format!("The service errored ({}). Retry shortly: {}", s, snip(body, 200)),
        ),
        s => status(
            service,
            false,
            Some(http),
            &s.to_string(),
            "Unexpected response",
            format!("HTTP {}: {}", s, snip(body, 200)),
        ),
    }
}

async fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> Result<(u16, String), String> {
    let mut req = http().post(url).json(&body);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((code, text))
}

async fn get(url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), String> {
    let mut req = http().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let code = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((code, text))
}

/// Live handshake for one service. `api_key`: explicit key to test (unsaved
/// keys can be probed without storing); falls back to the keyring copy.
#[tauri::command]
pub async fn integration_handshake(
    service: String,
    api_key: Option<String>,
) -> Result<IntegrationStatus, String> {
    let svc = service.trim().to_lowercase();
    let key = match resolve_key(&svc, api_key) {
        Some(k) => k,
        None => return Ok(no_key(&svc)),
    };
    let res: Result<IntegrationStatus, String> = async {
        match svc.as_str() {
            "tavily" => {
                // Minimal paid probe: 1 basic credit.
                let (code, body) = post_json(
                    "https://api.tavily.com/search",
                    &[("Authorization", &format!("Bearer {}", key))],
                    serde_json::json!({
                        "query": "connectivity probe",
                        "max_results": 1,
                        "include_answer": false,
                        "search_depth": "basic",
                    }),
                )
                .await?;
                Ok(map_status(&svc, code, &body, "Tavily key accepted — search is ready.".to_string()))
            }
            "exa" => {
                let (code, body) = post_json(
                    "https://api.exa.ai/search",
                    &[("x-api-key", &key)],
                    serde_json::json!({ "query": "connectivity probe" }),
                )
                .await?;
                Ok(map_status(&svc, code, &body, "Exa key accepted — search is ready.".to_string()))
            }
            "context7" => {
                // Context7's native protocol is MCP over Streamable HTTP; a
                // minimal JSON-RPC initialize doubles as the key check
                // (bad keys get 401, good keys get a 200 session response).
                let (code, body) = post_json(
                    "https://mcp.context7.com/mcp",
                    &[
                        ("Content-Type", "application/json"),
                        ("Accept", "application/json, text/event-stream"),
                        ("CONTEXT7_API_KEY", &key),
                    ],
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "clientInfo": { "name": "guild-foundry-ai", "version": "0.1.0" },
                        },
                    }),
                )
                .await?;
                Ok(map_status(&svc, code, &body, "Context7 key accepted — docs lookup is ready.".to_string()))
            }
            "github" => {
                let (code, body) = get(
                    "https://api.github.com/user",
                    &[
                        ("Authorization", &format!("Bearer {}", key)),
                        ("Accept", "application/vnd.github+json"),
                        ("X-GitHub-Api-Version", "2022-11-28"),
                    ],
                )
                .await?;
                if code == 200 {
                    let login = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("login").and_then(|l| l.as_str()).map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown".to_string());
                    return Ok(map_status(&svc, code, &body, format!("GitHub token accepted — signed in as @{}.", login)));
                }
                // 403 from GitHub usually means rate-limit abuse detection or
                // missing scope rather than a bad key; say so honestly.
                if code == 403 {
                    return Ok(status(&svc, false, Some(code), "403", "Forbidden or rate limited",
                        "GitHub refused the call — the token may lack scope, or the IP is temporarily rate limited.".to_string()));
                }
                Ok(map_status(&svc, code, &body, "GitHub token accepted.".to_string()))
            }
            "huggingface" => {
                let (code, body) = get(
                    "https://huggingface.co/api/whoami-v2",
                    &[("Authorization", &format!("Bearer {}", key))],
                )
                .await?;
                if code == 200 {
                    let name = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown".to_string());
                    return Ok(map_status(&svc, code, &body, format!("Hugging Face token accepted — signed in as {}.", name)));
                }
                Ok(map_status(&svc, code, &body, "Hugging Face token accepted.".to_string()))
            }
            "fal" => {
                // Fal has no free key-check endpoint; probe the queue gateway.
                // The gateway authenticates before routing, so 401/403 means
                // a bad key while any other HTTP status means the key was
                // accepted at the gateway (nothing is generated or billed).
                let (code, body) = get(
                    "https://queue.fal.run/",
                    &[("Authorization", &format!("Key {}", key))],
                )
                .await?;
                if code == 401 || code == 403 {
                    return Ok(map_status(&svc, code, &body, String::new()));
                }
                Ok(status(&svc, true, Some(code), "200", "Ready",
                    "Fal gateway accepted the key (probe only — nothing generated or billed).".to_string()))
            }
            other => Err(format!("unknown integration service: {}", other)),
        }
    }
    .await;
    match res {
        Ok(s) => Ok(s),
        Err(e) => {
            if e.starts_with("unknown integration") {
                return Err(e);
            }
            Ok(offline(&svc, e))
        }
    }
}

// ---------------------------------------------------------------------------
// Direct agent tools (dispatched from tools.rs).
// ---------------------------------------------------------------------------

fn integration_key(service: &str) -> Result<String, String> {
    stored_key(service).ok_or_else(|| {
        format!(
            "No {} API key configured. Add one under System & Settings → Tools & Registry → Connections.",
            service
        )
    })
}

fn clamp_results(n: Option<u64>) -> u64 {
    match n {
        Some(v) => v.clamp(1, 8),
        None => 5,
    }
}

/// Web search via Tavily. Returns trimmed results (token-capped).
/// Sync wrapper (the tools.rs dispatcher is sync) over the async worker.
pub fn tavily_search(query: &str, max_results: Option<u64>) -> Result<serde_json::Value, String> {
    tauri::async_runtime::block_on(tavily_search_async(query, max_results))
}

async fn tavily_search_async(
    query: &str,
    max_results: Option<u64>,
) -> Result<serde_json::Value, String> {
    let key = integration_key("tavily")?;
    let n = clamp_results(max_results);
    let (code, body) = post_json(
        "https://api.tavily.com/search",
        &[("Authorization", &format!("Bearer {}", key))],
        serde_json::json!({
            "query": query,
            "max_results": n,
            "include_answer": true,
            "search_depth": "advanced",
        }),
    )
    .await
    .map_err(|e| format!("tavily request failed: {}", snip(&e, 200)))?;
    if code == 401 || code == 403 {
        return Err("Tavily rejected the API key (401). Save a valid key under Connections.".to_string());
    }
    if code == 402 {
        return Err("Tavily account is out of credits (402).".to_string());
    }
    if code == 429 {
        return Err("Tavily rate limited this key (429). Wait and retry.".to_string());
    }
    if code != 200 {
        return Err(format!("Tavily search failed: HTTP {} {}", code, snip(&body, 200)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Tavily returned bad JSON: {}", e))?;
    let answer = v.get("answer").and_then(|a| a.as_str()).map(|s| snip(s, 800));
    let mut results = Vec::new();
    if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
        for r in arr.iter().take(n as usize) {
            let mut item = serde_json::Map::new();
            if let Some(t) = r.get("title").and_then(|t| t.as_str()) {
                item.insert("title".into(), serde_json::Value::String(snip(t, 200)));
            }
            if let Some(u) = r.get("url").and_then(|u| u.as_str()) {
                item.insert("url".into(), serde_json::Value::String(u.to_string()));
            }
            if let Some(c) = r.get("content").and_then(|c| c.as_str()) {
                item.insert("content".into(), serde_json::Value::String(snip(c, 600)));
            }
            if let Some(s) = r.get("score") {
                item.insert("score".into(), s.clone());
            }
            results.push(serde_json::Value::Object(item));
        }
    }
    let mut out = serde_json::Map::new();
    out.insert("query".into(), serde_json::Value::String(query.to_string()));
    if let Some(a) = answer {
        out.insert("answer".into(), serde_json::Value::String(a));
    }
    out.insert("results".into(), serde_json::Value::Array(results));
    Ok(serde_json::Value::Object(out))
}

/// Web search via Exa. Returns trimmed results (token-capped).
/// Sync wrapper (the tools.rs dispatcher is sync) over the async worker.
pub fn exa_search(query: &str, max_results: Option<u64>) -> Result<serde_json::Value, String> {
    tauri::async_runtime::block_on(exa_search_async(query, max_results))
}

async fn exa_search_async(query: &str, max_results: Option<u64>) -> Result<serde_json::Value, String> {
    let key = integration_key("exa")?;
    let n = clamp_results(max_results);
    let (code, body) = post_json(
        "https://api.exa.ai/search",
        &[("x-api-key", &key)],
        serde_json::json!({
            "query": query,
            "numResults": n,
            "contents": { "text": { "maxCharacters": 600 } },
        }),
    )
    .await
    .map_err(|e| format!("exa request failed: {}", snip(&e, 200)))?;
    if code == 401 || code == 403 {
        return Err("Exa rejected the API key (401). Save a valid key under Connections.".to_string());
    }
    if code == 402 {
        return Err("Exa account is out of credits (402).".to_string());
    }
    if code == 429 {
        return Err("Exa rate limited this key (429). Wait and retry.".to_string());
    }
    if code != 200 {
        return Err(format!("Exa search failed: HTTP {} {}", code, snip(&body, 200)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Exa returned bad JSON: {}", e))?;
    let mut results = Vec::new();
    if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
        for r in arr.iter().take(n as usize) {
            let mut item = serde_json::Map::new();
            for field in ["title", "url"] {
                if let Some(s) = r.get(field).and_then(|x| x.as_str()) {
                    item.insert(field.into(), serde_json::Value::String(snip(s, 200)));
                }
            }
            if let Some(t) = r.get("text").and_then(|x| x.as_str()) {
                item.insert("text".into(), serde_json::Value::String(snip(t, 600)));
            }
            results.push(serde_json::Value::Object(item));
        }
    }
    Ok(serde_json::json!({ "query": query, "results": results }))
}

/// GitHub identity check — also serves the handshake detail.
/// Sync wrapper (the tools.rs dispatcher is sync) over the async worker.
pub fn github_get_user() -> Result<serde_json::Value, String> {
    tauri::async_runtime::block_on(github_get_user_async())
}

async fn github_get_user_async() -> Result<serde_json::Value, String> {
    let key = integration_key("github")?;
    let (code, body) = get(
        "https://api.github.com/user",
        &[
            ("Authorization", &format!("Bearer {}", key)),
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
        ],
    )
    .await
    .map_err(|e| format!("github request failed: {}", snip(&e, 200)))?;
    if code == 401 {
        return Err("GitHub rejected the token (401). Save a valid fine-grained PAT under Connections.".to_string());
    }
    if code != 200 {
        return Err(format!("GitHub /user failed: HTTP {} {}", code, snip(&body, 200)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("GitHub returned bad JSON: {}", e))?;
    let mut out = serde_json::Map::new();
    for field in ["login", "name", "company", "public_repos", "followers", "created_at"] {
        if let Some(val) = v.get(field) {
            out.insert(field.into(), val.clone());
        }
    }
    Ok(serde_json::Value::Object(out))
}

/// Hugging Face identity check — also serves the handshake detail.
/// Sync wrapper (the tools.rs dispatcher is sync) over the async worker.
pub fn hf_whoami() -> Result<serde_json::Value, String> {
    tauri::async_runtime::block_on(hf_whoami_async())
}

async fn hf_whoami_async() -> Result<serde_json::Value, String> {
    let key = integration_key("huggingface")?;
    let (code, body) = get(
        "https://huggingface.co/api/whoami-v2",
        &[("Authorization", &format!("Bearer {}", key))],
    )
    .await
    .map_err(|e| format!("hugging face request failed: {}", snip(&e, 200)))?;
    if code == 401 {
        return Err("Hugging Face rejected the token (401). Save a valid token under Connections.".to_string());
    }
    if code != 200 {
        return Err(format!("Hugging Face whoami failed: HTTP {} {}", code, snip(&body, 200)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Hugging Face returned bad JSON: {}", e))?;
    let mut out = serde_json::Map::new();
    for field in ["name", "fullname", "email", "isPro"] {
        if let Some(val) = v.get(field) {
            out.insert(field.into(), serde_json::Value::String(
                val.as_str().map(|s| s.to_string()).unwrap_or_else(|| val.to_string()),
            ));
        }
    }
    Ok(serde_json::Value::Object(out))
}

