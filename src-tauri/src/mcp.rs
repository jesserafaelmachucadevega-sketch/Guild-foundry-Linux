// SPDX-License-Identifier: Apache-2.0
// Phase 7 — MCP Client Gateway + Permission Center (Rust backend).
//
// The MCP gateway lives in Rust (master spec section 35): a hand-rolled
// JSON-RPC 2.0 client over reqwest supporting Streamable HTTP and legacy
// HTTP+SSE transports, Bearer <redacted> and OAuth 2.1 + PKCE. Discovered tools are
// normalized into the Phase 1 `tools` registry table.
//
// RUST NOT COMPILED IN THIS ENVIRONMENT — written with extra care; NOT VERIFIED.
//
// Schema note: reuses Phase 1 tables `mcp_servers`, `tools`, `permissions`
// from db.rs (db.rs is never edited). Phase 7-only fields live in two additive
// tables created lazily here:
//   mcp_server_meta(server_id PK, auth_type, oauth_*, allowed_tools, denied_tools, bearer_present)
//   mcp_tool_meta(tool_name PK, input_schema, output_schema, permission_requirements, audit_policy)
// Global (non-agent) permission levels reuse `permissions` with the sentinel
// key (agent_id = '', tool = '*').

use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::Rng;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Manager;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Service name used for ALL keyring entries written by Phase 7. The Phase 2
/// `secret_set`/`secret_get` commands must use this same service so frontend
/// writes and Rust reads share entries.
pub const KEYRING_SERVICE: &str = "guild-foundry-ai";

const OAUTH_STATE_PREFIX: &str = "gfa-mcp-oauth:";
const OAUTH_TOKEN_PREFIX: &str = "gfa-mcp-token:";
const BEARER_TOKEN_PREFIX: &str = "gfa-mcp-bearer:";
const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:18793/oauth/callback";
const PROTOCOL_VERSION: &str = "2025-06-18";
const CLIENT_NAME: &str = "guild-foundry-ai";
const CLIENT_VERSION: &str = "0.1.0";

const PERM_DOMAINS: [&str; 8] = [
    "filesystem",
    "network",
    "shell",
    "git",
    "mcp",
    "browser",
    "credentials",
    "external_apis",
];
const PERM_LEVELS: [&str; 5] = [
    "always_allow",
    "ask_every_time",
    "allow_project",
    "allow_session",
    "deny",
];
const DEFAULT_PERM_LEVEL: &str = "ask_every_time"; // secure default: least privilege

// ---------------------------------------------------------------------------
// Database helpers (same DB file as db.rs; additive Phase 7 tables only)
// ---------------------------------------------------------------------------

fn db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("guild-foundry-ai.db"))
}

fn ensure_phase7_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_server_meta (
            server_id               TEXT PRIMARY KEY,
            auth_type               TEXT NOT NULL DEFAULT 'none',
            oauth_client_id         TEXT NOT NULL DEFAULT '',
            oauth_authorization_url TEXT NOT NULL DEFAULT '',
            oauth_token_url         TEXT NOT NULL DEFAULT '',
            oauth_scopes            TEXT NOT NULL DEFAULT '',
            oauth_redirect_uri      TEXT NOT NULL DEFAULT '',
            allowed_tools           TEXT NOT NULL DEFAULT '[]',
            denied_tools            TEXT NOT NULL DEFAULT '[]',
            bearer_present          INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS mcp_tool_meta (
            tool_name               TEXT PRIMARY KEY,
            input_schema            TEXT NOT NULL DEFAULT '{}',
            output_schema           TEXT NOT NULL DEFAULT '{}',
            permission_requirements TEXT NOT NULL DEFAULT '[]',
            audit_policy            TEXT NOT NULL DEFAULT 'always'
        );",
    )
    .map_err(|e| e.to_string())
}

fn open_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let conn = Connection::open(db_path(app)?).map_err(|e| e.to_string())?;
    ensure_phase7_schema(&conn)?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Keyring helpers (keyring crate; same service Phase 2 secret_* must use)
// ---------------------------------------------------------------------------

fn kr_set(key: &str, value: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| e.to_string())?;
    entry.set_password(value).map_err(|e| e.to_string())
}

fn kr_get(key: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, key)
        .ok()?
        .get_password()
        .ok()
}

fn kr_del(key: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, key) {
        let _ = entry.delete_credential();
    }
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

/// Percent-encode for query strings (no extra deps).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    to_hex(&h.finalize())
}

fn json_string_list(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Server model (mcp_servers + mcp_server_meta)
// ---------------------------------------------------------------------------

struct ServerRow {
    id: String,
    name: String,
    transport: String,
    url: String,
    enabled: bool,
}

struct ServerMeta {
    auth_type: String,
    oauth_client_id: String,
    oauth_authorization_url: String,
    oauth_token_url: String,
    oauth_scopes: String,
    oauth_redirect_uri: String,
    allowed_tools: Vec<String>,
    denied_tools: Vec<String>,
    bearer_present: bool,
}

fn load_server(app: &tauri::AppHandle, server_id: &str) -> Result<(ServerRow, ServerMeta), String> {
    let conn = open_db(app)?;
    let row = conn
        .query_row(
            "SELECT id, name, transport, url, enabled FROM mcp_servers WHERE id = ?1",
            [server_id],
            |r| {
                Ok(ServerRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    transport: r.get(2)?,
                    url: r.get(3)?,
                    enabled: r.get::<_, i64>(4)? != 0,
                })
            },
        )
        .map_err(|_| format!("MCP server not found: {}", server_id))?;
    let meta = conn
        .query_row(
            "SELECT auth_type, oauth_client_id, oauth_authorization_url, oauth_token_url,
                    oauth_scopes, oauth_redirect_uri, allowed_tools, denied_tools, bearer_present
             FROM mcp_server_meta WHERE server_id = ?1",
            [server_id],
            |r| {
                let allowed: String = r.get(6)?;
                let denied: String = r.get(7)?;
                Ok(ServerMeta {
                    auth_type: r.get(0)?,
                    oauth_client_id: r.get(1)?,
                    oauth_authorization_url: r.get(2)?,
                    oauth_token_url: r.get(3)?,
                    oauth_scopes: r.get(4)?,
                    oauth_redirect_uri: r.get(5)?,
                    allowed_tools: json_string_list(&allowed),
                    denied_tools: json_string_list(&denied),
                    bearer_present: r.get::<_, i64>(8)? != 0,
                })
            },
        )
        .unwrap_or(ServerMeta {
            auth_type: "none".to_string(),
            oauth_client_id: String::new(),
            oauth_authorization_url: String::new(),
            oauth_token_url: String::new(),
            oauth_scopes: String::new(),
            oauth_redirect_uri: String::new(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            bearer_present: false,
        });
    Ok((row, meta))
}

fn validate_server_input(input: &McpServerInput) -> Result<(), String> {
    if input.name.trim().is_empty() {
        return Err("server name is required".to_string());
    }
    if !matches!(input.transport.as_str(), "streamable_http" | "sse") {
        return Err("transport must be streamable_http or sse".to_string());
    }
    if !(input.url.starts_with("http://") || input.url.starts_with("https://")) {
        return Err("url must start with http:// or https://".to_string());
    }
    if !matches!(input.auth_type.as_str(), "none" | "bearer" | "oauth") {
        return Err("auth_type must be none, bearer, or oauth".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Permission helpers (global domain levels; sentinel agent_id='' / tool='*')
// ---------------------------------------------------------------------------

fn perm_get_level(conn: &Connection, domain: &str) -> String {
    conn.query_row(
        "SELECT mode FROM permissions WHERE agent_id = '' AND tool = '*' AND domain = ?1",
        [domain],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| DEFAULT_PERM_LEVEL.to_string())
}

fn validate_perm(domain: &str, level: &str) -> Result<(), String> {
    if !PERM_DOMAINS.contains(&domain) {
        return Err(format!("unknown permission domain: {}", domain));
    }
    if !PERM_LEVELS.contains(&level) {
        return Err(format!("unknown permission level: {}", level));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// MCP wire client: JSON-RPC 2.0 over Streamable HTTP or legacy HTTP+SSE
// ---------------------------------------------------------------------------

struct McpClient {
    http: reqwest::Client,
    url: String,
    transport: String,
    bearer: Option<String>,
    session_id: Option<String>,
    next_id: u64,
}

impl McpClient {
    fn new(url: String, transport: String, bearer: Option<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            url,
            transport,
            bearer,
            session_id: None,
            next_id: 0,
        })
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.bearer {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    /// Full request/response JSON-RPC call. Returns the `result` value.
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let raw = if self.transport == "sse" {
            self.rpc_legacy_sse(&payload, id, true)
                .await?
                .ok_or_else(|| "MCP server gave no response".to_string())?
        } else {
            self.rpc_streamable(&payload, id).await?
        };
        if let Some(err) = raw.get("error") {
            return Err(format!("MCP error: {}", err));
        }
        raw.get("result")
            .cloned()
            .ok_or_else(|| "MCP response had no result".to_string())
    }

    /// JSON-RPC notification (no id, no response expected).
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let payload = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if self.transport == "sse" {
            self.rpc_legacy_sse(&payload, 0, false).await?;
        } else {
            let mut req = self
                .http
                .post(&self.url)
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .json(&payload);
            req = self.apply_auth(req);
            if let Some(s) = &self.session_id {
                req = req.header("Mcp-Session-Id", s);
            }
            let resp = req.send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() && resp.status().as_u16() != 202 {
                return Err(format!("MCP notify failed: HTTP {}", resp.status()));
            }
        }
        Ok(())
    }

    /// Streamable HTTP: single POST; response is JSON or an SSE stream.
    async fn rpc_streamable(&mut self, payload: &Value, want_id: u64) -> Result<Value, String> {
        let mut req = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(payload);
        req = self.apply_auth(req);
        if let Some(s) = &self.session_id {
            req = req.header("Mcp-Session-Id", s);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(s.to_string());
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let head: String = body.chars().take(300).collect();
            return Err(format!("MCP HTTP {}: {}", status, head));
        }
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if ctype.contains("text/event-stream") {
            let mut reader = SseReader::new(resp);
            let deadline = Duration::from_secs(25);
            loop {
                match reader.next_event(deadline).await? {
                    Some((_etype, data)) => {
                        if data.trim() == "[DONE]" {
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<Value>(&data) {
                            if v.get("id").and_then(|i| i.as_u64()) == Some(want_id) {
                                return Ok(v);
                            }
                        }
                    }
                    None => return Err("MCP SSE stream closed without a response".to_string()),
                }
            }
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if v.get("id").and_then(|i| i.as_u64()) == Some(want_id) {
            Ok(v)
        } else {
            Err("MCP response id mismatch".to_string())
        }
    }

    /// Legacy HTTP+SSE (2024-11-05): GET opens the SSE stream, server sends an
    /// `endpoint` event, client POSTs JSON-RPC there, responses arrive as
    /// `message` events on the GET stream.
    async fn rpc_legacy_sse(
        &self,
        payload: &Value,
        want_id: u64,
        expect_response: bool,
    ) -> Result<Option<Value>, String> {
        let mut req = self.http.get(&self.url).header("Accept", "text/event-stream");
        req = self.apply_auth(req);
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("MCP SSE handshake failed: HTTP {}", resp.status()));
        }
        let mut reader = SseReader::new(resp);
        let deadline = Duration::from_secs(25);
        // 1. Wait for the `endpoint` event.
        let endpoint: String = loop {
            match reader.next_event(deadline).await? {
                Some((etype, data)) if etype == "endpoint" => break data.trim().to_string(),
                Some(_) => continue,
                None => return Err("MCP SSE stream closed before endpoint event".to_string()),
            }
        };
        if endpoint.is_empty() {
            return Err("MCP server sent an empty endpoint".to_string());
        }
        let endpoint_url = resolve_url(&self.url, &endpoint)?;
        // 2. POST the JSON-RPC message.
        let mut preq = self
            .http
            .post(&endpoint_url)
            .header("Content-Type", "application/json")
            .json(payload);
        preq = self.apply_auth(preq);
        let presp = preq.send().await.map_err(|e| e.to_string())?;
        if !presp.status().is_success() && presp.status().as_u16() != 202 {
            let body = presp.text().await.unwrap_or_default();
            let head: String = body.chars().take(300).collect();
            return Err(format!("MCP POST {}: {}", presp.status(), head));
        }
        if !expect_response {
            return Ok(None);
        }
        // 3. Read `message` events until the matching response id arrives.
        loop {
            match reader.next_event(deadline).await? {
                Some((etype, data)) => {
                    if etype != "message" || data.trim() == "[DONE]" {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(&data) {
                        if v.get("id").and_then(|i| i.as_u64()) == Some(want_id) {
                            return Ok(Some(v));
                        }
                    }
                }
                None => return Err("MCP SSE stream closed without a response".to_string()),
            }
        }
    }

    /// initialize -> notifications/initialized. Returns (server_info, protocol_version).
    async fn handshake(&mut self) -> Result<(Value, String), String> {
        let result = self
            .call(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": CLIENT_NAME, "version": CLIENT_VERSION }
                }),
            )
            .await?;
        let server_info = result.get("serverInfo").cloned().unwrap_or(json!({}));
        let protocol = result
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let _ = self.notify("notifications/initialized", json!({})).await;
        Ok((server_info, protocol))
    }
}

/// Incremental SSE event reader over a reqwest streaming response.
struct SseReader {
    resp: reqwest::Response,
    buf: String,
}

impl SseReader {
    fn new(resp: reqwest::Response) -> Self {
        Self { resp, buf: String::new() }
    }

    /// Returns the next `(event_type, data)` pair, or None on clean EOF.
    async fn next_event(&mut self, timeout: Duration) -> Result<Option<(String, String)>, String> {
        loop {
            if let Some(pos) = self.buf.find("\n\n") {
                let raw = self.buf[..pos].to_string();
                self.buf = self.buf[pos + 2..].to_string();
                return Ok(Some(parse_sse_event(&raw)));
            }
            let chunk = tokio::time::timeout(timeout, self.resp.chunk())
                .await
                .map_err(|_| "timed out waiting for MCP SSE event".to_string())?
                .map_err(|e| e.to_string())?;
            match chunk {
                Some(bytes) => self.buf.push_str(&String::from_utf8_lossy(&bytes)),
                None => {
                    let rest = self.buf.trim().to_string();
                    self.buf.clear();
                    if rest.is_empty() {
                        return Ok(None);
                    }
                    return Ok(Some(parse_sse_event(&rest)));
                }
            }
        }
    }
}

fn parse_sse_event(raw: &str) -> (String, String) {
    let mut etype = "message".to_string();
    let mut data_lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        if line.starts_with(':') {
            continue; // comment / keep-alive
        } else if let Some(v) = line.strip_prefix("event:") {
            etype = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("data:") {
            data_lines.push(v.strip_prefix(' ').unwrap_or(v).to_string());
        }
    }
    (etype, data_lines.join("\n"))
}

fn resolve_url(base: &str, endpoint: &str) -> Result<String, String> {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return Ok(endpoint.to_string());
    }
    let base_url = reqwest::Url::parse(base).map_err(|e| e.to_string())?;
    base_url.join(endpoint).map(|u| u.to_string()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Auth resolution + client construction
// ---------------------------------------------------------------------------

/// Resolve the credential for a server: explicit Bearer <redacted> first, then OAuth token.
fn resolve_bearer(server_id: &str, meta: &ServerMeta) -> Option<String> {
    match meta.auth_type.as_str() {
        "oauth" => oauth_access_token(server_id).or_else(|| kr_get(&format!("{}{}", BEARER_TOKEN_PREFIX, server_id))),
        "bearer" => kr_get(&format!("{}{}", BEARER_TOKEN_PREFIX, server_id)),
        _ => oauth_access_token(server_id).or_else(|| kr_get(&format!("{}{}", BEARER_TOKEN_PREFIX, server_id))),
    }
}

fn oauth_access_token(server_id: &str) -> Option<String> {
    let raw = kr_get(&format!("{}{}", OAUTH_TOKEN_PREFIX, server_id))?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v.get("access_token").and_then(|t| t.as_str()).map(|s| s.to_string())
}

fn build_client(row: &ServerRow, meta: &ServerMeta) -> Result<McpClient, String> {
    if !row.enabled {
        return Err("MCP server is disabled".to_string());
    }
    McpClient::new(row.url.clone(), row.transport.clone(), resolve_bearer(&row.id, meta))
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct McpServerInput {
    pub name: String,
    pub transport: String, // streamable_http | sse
    pub url: String,
    pub auth_type: String, // none | bearer | oauth
    pub oauth_client_id: Option<String>,
    pub oauth_authorization_url: Option<String>,
    pub oauth_token_url: Option<String>,
    pub oauth_scopes: Option<String>,
    pub oauth_redirect_uri: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub bearer_present: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub url: String,
    pub enabled: bool,
    pub auth_type: String,
    pub bearer_present: bool,
    pub oauth_configured: bool,
    pub oauth_connected: bool,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub tool_count: i64,
}

#[derive(Debug, Serialize)]
pub struct McpTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub tool_count: usize,
    pub server_name: String,
    pub protocol_version: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub risk_class: String,
    pub enabled: bool,
    pub server_id: String,
    pub server_name: String,
}

#[derive(Debug, Serialize)]
pub struct McpConnectResult {
    pub ok: bool,
    pub server_name: String,
    pub protocol_version: String,
    pub latency_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct McpCallAudit {
    pub at: String,
    pub server_id: String,
    pub tool: String,
    pub args_sha256: String,
    pub decision: String, // allowed | denied | approval_required
    pub policy: String,   // permission level that decided
}

#[derive(Debug, Serialize)]
pub struct McpCallResult {
    pub ok: bool,
    pub result: Value,
    pub is_error: bool,
    pub latency_ms: u64,
    pub approval_required: bool,
    pub audit: McpCallAudit,
}

#[derive(Debug, Serialize)]
pub struct OAuthStart {
    pub auth_url: String,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct OAuthCallbackResult {
    pub ok: bool,
    pub expires_in: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OAuthRevokeResult {
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct PermEntry {
    pub domain: String,
    pub level: String,
}

#[derive(Debug, Serialize)]
pub struct RegistryTool {
    pub name: String,
    pub description: String,
    pub provider: Option<String>,
    pub mcp_server: Option<String>,
    pub risk_class: String,
    pub enabled: bool,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permission_requirements: Vec<String>,
    pub audit_policy: String,
}

// ---------------------------------------------------------------------------
// Server management commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn mcp_server_add(app: tauri::AppHandle, input: McpServerInput) -> Result<String, String> {
    validate_server_input(&input)?;
    let conn = open_db(&app)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    conn.execute(
        "INSERT INTO mcp_servers (id, name, transport, url, enabled) VALUES (?1, ?2, ?3, ?4, 1)",
        params![id, input.name.trim(), input.transport, input.url.trim()],
    )
    .map_err(|e| e.to_string())?;
    upsert_meta(&conn, &id, &input)?;
    Ok(id)
}

#[tauri::command]
pub async fn mcp_server_update(
    app: tauri::AppHandle,
    server_id: String,
    input: McpServerInput,
) -> Result<(), String> {
    validate_server_input(&input)?;
    let conn = open_db(&app)?;
    let changed = conn
        .execute(
            "UPDATE mcp_servers SET name = ?1, transport = ?2, url = ?3 WHERE id = ?4",
            params![input.name.trim(), input.transport, input.url.trim(), server_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("MCP server not found: {}", server_id));
    }
    upsert_meta(&conn, &server_id, &input)?;
    Ok(())
}

fn upsert_meta(conn: &Connection, server_id: &str, input: &McpServerInput) -> Result<(), String> {
    let none = String::new();
    conn.execute(
        "INSERT INTO mcp_server_meta
            (server_id, auth_type, oauth_client_id, oauth_authorization_url, oauth_token_url,
             oauth_scopes, oauth_redirect_uri, allowed_tools, denied_tools, bearer_present)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(server_id) DO UPDATE SET
            auth_type = excluded.auth_type,
            oauth_client_id = excluded.oauth_client_id,
            oauth_authorization_url = excluded.oauth_authorization_url,
            oauth_token_url = excluded.oauth_token_url,
            oauth_scopes = excluded.oauth_scopes,
            oauth_redirect_uri = excluded.oauth_redirect_uri,
            allowed_tools = excluded.allowed_tools,
            denied_tools = excluded.denied_tools,
            bearer_present = excluded.bearer_present",
        params![
            server_id,
            input.auth_type,
            input.oauth_client_id.as_ref().unwrap_or(&none),
            input.oauth_authorization_url.as_ref().unwrap_or(&none),
            input.oauth_token_url.as_ref().unwrap_or(&none),
            input.oauth_scopes.as_ref().unwrap_or(&none),
            input.oauth_redirect_uri.as_ref().unwrap_or(&none),
            serde_json::to_string(input.allowed_tools.as_ref().unwrap_or(&Vec::new()))
                .unwrap_or_else(|_| "[]".to_string()),
            serde_json::to_string(input.denied_tools.as_ref().unwrap_or(&Vec::new()))
                .unwrap_or_else(|_| "[]".to_string()),
            input.bearer_present.unwrap_or(false) as i64,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn mcp_server_remove(app: tauri::AppHandle, server_id: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    conn.execute("DELETE FROM mcp_servers WHERE id = ?1", [server_id.as_str()])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM mcp_server_meta WHERE server_id = ?1", [server_id.as_str()])
        .map_err(|e| e.to_string())?;
    // Drop any credentials so a removed server leaves nothing behind.
    kr_del(&format!("{}{}", BEARER_TOKEN_PREFIX, server_id));
    kr_del(&format!("{}{}", OAUTH_TOKEN_PREFIX, server_id));
    Ok(())
}

#[tauri::command]
pub async fn mcp_server_list(app: tauri::AppHandle) -> Result<Vec<McpServerView>, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare("SELECT id, name, transport, url, enabled FROM mcp_servers ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? != 0,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, transport, url, enabled) = row.map_err(|e| e.to_string())?;
        let meta = conn
            .query_row(
                "SELECT auth_type, oauth_client_id, oauth_authorization_url, oauth_token_url,
                        allowed_tools, denied_tools, bearer_present
                 FROM mcp_server_meta WHERE server_id = ?1",
                [&id],
                |r| {
                    let allowed: String = r.get(4)?;
                    let denied: String = r.get(5)?;
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        json_string_list(&allowed),
                        json_string_list(&denied),
                        r.get::<_, i64>(6)? != 0,
                    ))
                },
            )
            .unwrap_or((
                "none".to_string(),
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
                Vec::new(),
                false,
            ));
        let (auth_type, client_id, auth_url, token_url, allowed, denied, bearer_present) = meta;
        let tool_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tools WHERE mcp_server = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        out.push(McpServerView {
            id: id.clone(),
            name,
            transport,
            url,
            enabled,
            auth_type: auth_type.clone(),
            bearer_present,
            oauth_configured: !client_id.is_empty() && !auth_url.is_empty() && !token_url.is_empty(),
            oauth_connected: kr_get(&format!("{}{}", OAUTH_TOKEN_PREFIX, id)).is_some(),
            allowed_tools: allowed,
            denied_tools: denied,
            tool_count,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn mcp_server_enable(app: tauri::AppHandle, server_id: String) -> Result<(), String> {
    set_enabled(&app, &server_id, true)
}

#[tauri::command]
pub async fn mcp_server_disable(app: tauri::AppHandle, server_id: String) -> Result<(), String> {
    set_enabled(&app, &server_id, false)
}

fn set_enabled(app: &tauri::AppHandle, server_id: &str, enabled: bool) -> Result<(), String> {
    let conn = open_db(app)?;
    let changed = conn
        .execute(
            "UPDATE mcp_servers SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, server_id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("MCP server not found: {}", server_id));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tool discovery, registry normalization, test/connect
// ---------------------------------------------------------------------------

/// Classify MCP tool risk from its annotations (spec section 37).
fn classify_risk(tool: &Value) -> String {
    let ann = tool.get("annotations");
    let flag = |k: &str| ann.and_then(|a| a.get(k)).and_then(|v| v.as_bool()).unwrap_or(false);
    if flag("destructiveHint") {
        "high".to_string()
    } else if flag("readOnlyHint") {
        "low".to_string()
    } else {
        "moderate".to_string()
    }
}

fn tool_from_json(server_id: &str, server_name: &str, t: &Value, enabled: bool) -> McpToolInfo {
    McpToolInfo {
        name: t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        input_schema: t.get("inputSchema").cloned().unwrap_or(json!({})),
        output_schema: t.get("outputSchema").cloned().unwrap_or(json!({})),
        risk_class: classify_risk(t),
        enabled,
        server_id: server_id.to_string(),
        server_name: server_name.to_string(),
    }
}

/// Normalize discovered tools into the Phase 1 `tools` registry table plus
/// Phase 7 `mcp_tool_meta` (schemas, permission requirements, audit policy).
fn upsert_registry(conn: &Connection, server_id: &str, tools: &[McpToolInfo]) -> Result<(), String> {
    for t in tools {
        conn.execute(
            "INSERT INTO tools (name, description, provider, mcp_server, risk_class, enabled)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET
                description = excluded.description,
                mcp_server = excluded.mcp_server,
                risk_class = excluded.risk_class",
            params![t.name, t.description, server_id, t.risk_class, t.enabled as i64],
        )
        .map_err(|e| e.to_string())?;
        let reqs = permission_requirements_for(&t.risk_class);
        conn.execute(
            "INSERT INTO mcp_tool_meta (tool_name, input_schema, output_schema, permission_requirements, audit_policy)
             VALUES (?1, ?2, ?3, ?4, 'always')
             ON CONFLICT(tool_name) DO UPDATE SET
                input_schema = excluded.input_schema,
                output_schema = excluded.output_schema,
                permission_requirements = excluded.permission_requirements",
            params![
                t.name,
                serde_json::to_string(&t.input_schema).unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&t.output_schema).unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string(&reqs).unwrap_or_else(|_| "[]".to_string()),
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn permission_requirements_for(risk_class: &str) -> Vec<String> {
    match risk_class {
        "high" => vec!["mcp:execute".to_string(), "ask_every_time".to_string()],
        "moderate" => vec!["mcp:execute".to_string()],
        _ => vec!["mcp:read".to_string()],
    }
}

fn enabled_map(conn: &Connection) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT name, enabled FROM tools WHERE mcp_server IS NOT NULL") {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))) {
            for row in rows.flatten() {
                map.insert(row.0, row.1);
            }
        }
    }
    map
}

async fn discover_tools(
    row: &ServerRow,
    meta: &ServerMeta,
) -> Result<(Vec<McpToolInfo>, Value, String, u64), String> {
    let started = Instant::now();
    let mut client = build_client(row, meta)?;
    let (server_info, protocol) = client.handshake().await?;
    let result = client.call("tools/list", json!({})).await?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let arr = result.get("tools").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let tools: Vec<McpToolInfo> = arr
        .iter()
        .map(|t| tool_from_json(&row.id, &row.name, t, true)) // enabled resolved from registry by caller
        .collect();
    Ok((tools, server_info, protocol, latency_ms))
}

#[tauri::command]
pub async fn mcp_connect(app: tauri::AppHandle, server_id: String) -> Result<McpConnectResult, String> {
    let (row, meta) = load_server(&app, &server_id)?;
    let started = Instant::now();
    let mut client = build_client(&row, &meta)?;
    let (_info, protocol) = client.handshake().await?;
    Ok(McpConnectResult {
        ok: true,
        server_name: row.name,
        protocol_version: protocol,
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

#[tauri::command]
pub async fn mcp_server_test(app: tauri::AppHandle, server_id: String) -> Result<McpTestResult, String> {
    let (row, meta) = load_server(&app, &server_id)?;
    let started = Instant::now();
    let outcome: Result<(usize, String), String> = async {
        let mut client = build_client(&row, &meta)?;
        let (_info, protocol) = client.handshake().await?;
        let result = client.call("tools/list", json!({})).await?;
        let count = result.get("tools").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        Ok((count, protocol))
    }
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok((tool_count, protocol_version)) => Ok(McpTestResult {
            ok: true,
            latency_ms,
            tool_count,
            server_name: row.name,
            protocol_version,
            error: None,
        }),
        Err(e) => Ok(McpTestResult {
            ok: false,
            latency_ms,
            tool_count: 0,
            server_name: row.name,
            protocol_version: String::new(),
            error: Some(e),
        }),
    }
}

#[tauri::command]
pub async fn mcp_list_tools(app: tauri::AppHandle, server_id: String) -> Result<Vec<McpToolInfo>, String> {
    let (row, meta) = load_server(&app, &server_id)?;
    let (mut tools, _info, _protocol, _lat) = discover_tools(&row, &meta).await?;
    let conn = open_db(&app)?;
    let emap = enabled_map(&conn);
    for t in tools.iter_mut() {
        if let Some(e) = emap.get(&t.name) {
            t.enabled = *e;
        }
    }
    upsert_registry(&conn, &row.id, &tools)?;
    Ok(tools)
}

#[tauri::command]
pub async fn mcp_list_resources(app: tauri::AppHandle, server_id: String) -> Result<Value, String> {
    let (row, meta) = load_server(&app, &server_id)?;
    let mut client = build_client(&row, &meta)?;
    let _ = client.handshake().await?;
    client.call("resources/list", json!({})).await
}

#[tauri::command]
pub async fn mcp_list_prompts(app: tauri::AppHandle, server_id: String) -> Result<Value, String> {
    let (row, meta) = load_server(&app, &server_id)?;
    let mut client = build_client(&row, &meta)?;
    let _ = client.handshake().await?;
    client.call("prompts/list", json!({})).await
}

// ---------------------------------------------------------------------------
// Tool registry (read-only view + enabled toggle)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn mcp_tool_registry_list(app: tauri::AppHandle) -> Result<Vec<RegistryTool>, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare(
            "SELECT t.name, t.description, t.provider, t.mcp_server, t.risk_class, t.enabled,
                    m.input_schema, m.output_schema, m.permission_requirements, m.audit_policy
             FROM tools t LEFT JOIN mcp_tool_meta m ON m.tool_name = t.name
             WHERE t.mcp_server IS NOT NULL
             ORDER BY t.mcp_server, t.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)? != 0,
                r.get::<_, Option<String>>(6)?.unwrap_or_else(|| "{}".to_string()),
                r.get::<_, Option<String>>(7)?.unwrap_or_else(|| "{}".to_string()),
                r.get::<_, Option<String>>(8)?.unwrap_or_else(|| "[]".to_string()),
                r.get::<_, Option<String>>(9)?.unwrap_or_else(|| "always".to_string()),
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (name, description, provider, mcp_server, risk_class, enabled, input_schema, output_schema, reqs, audit_policy) =
            row.map_err(|e| e.to_string())?;
        out.push(RegistryTool {
            name,
            description,
            provider,
            mcp_server,
            risk_class,
            enabled,
            input_schema: serde_json::from_str(&input_schema).unwrap_or(json!({})),
            output_schema: serde_json::from_str(&output_schema).unwrap_or(json!({})),
            permission_requirements: serde_json::from_str(&reqs).unwrap_or_default(),
            audit_policy,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn mcp_tool_set_enabled(app: tauri::AppHandle, tool_name: String, enabled: bool) -> Result<(), String> {
    let conn = open_db(&app)?;
    let changed = conn
        .execute(
            "UPDATE tools SET enabled = ?1 WHERE name = ?2",
            params![enabled as i64, tool_name],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("tool not found: {}", tool_name));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// mcp_call_tool — permission boundary + global policy enforcement
// ---------------------------------------------------------------------------

fn audit_entry(server_id: &str, tool: &str, args_json: &str, decision: &str, policy: &str) -> McpCallAudit {
    McpCallAudit {
        at: chrono::Utc::now().to_rfc3339(),
        server_id: server_id.to_string(),
        tool: tool.to_string(),
        args_sha256: sha256_hex(args_json),
        decision: decision.to_string(),
        policy: policy.to_string(),
    }
}

#[tauri::command]
pub async fn mcp_call_tool(
    app: tauri::AppHandle,
    server_id: String,
    tool_name: String,
    args_json: String,
) -> Result<McpCallResult, String> {
    let started = Instant::now();
    let latency = || started.elapsed().as_millis() as u64;
    let (row, meta) = load_server(&app, &server_id)?;

    // 1. Per-server permission boundary (master spec section 36).
    if meta.denied_tools.iter().any(|t| t == &tool_name) {
        let audit = audit_entry(&server_id, &tool_name, &args_json, "denied", "server:denied_tools");
        return Ok(McpCallResult {
            ok: false,
            result: json!({ "error": "tool is denied by this server's permission boundary" }),
            is_error: true,
            latency_ms: latency(),
            approval_required: false,
            audit,
        });
    }
    if !meta.allowed_tools.is_empty() && !meta.allowed_tools.iter().any(|t| t == &tool_name) {
        let audit = audit_entry(&server_id, &tool_name, &args_json, "denied", "server:allowed_tools");
        return Ok(McpCallResult {
            ok: false,
            result: json!({ "error": "tool is not in this server's allowed-tools boundary" }),
            is_error: true,
            latency_ms: latency(),
            approval_required: false,
            audit,
        });
    }

    // 2. Global MCP domain policy from the permissions table.
    let conn = open_db(&app)?;
    let level = perm_get_level(&conn, "mcp");
    if level == "deny" {
        let audit = audit_entry(&server_id, &tool_name, &args_json, "denied", "domain:mcp=deny");
        return Ok(McpCallResult {
            ok: false,
            result: json!({ "error": "MCP tool calls are denied by the permission center" }),
            is_error: true,
            latency_ms: latency(),
            approval_required: false,
            audit,
        });
    }
    if level == "ask_every_time" {
        // The agent host / UI presents the approval UX; Phase 7 only records the policy decision.
        let audit = audit_entry(&server_id, &tool_name, &args_json, "approval_required", "domain:mcp=ask_every_time");
        return Ok(McpCallResult {
            ok: false,
            result: json!({ "status": "approval_required" }),
            is_error: false,
            latency_ms: latency(),
            approval_required: true,
            audit,
        });
    }

    // 3. Execute. (Phase 9 owns the persistent audit log; the audit entry is
    //    returned in the result payload so the caller can record it.)
    let args: Value = serde_json::from_str(&args_json)
        .map_err(|e| format!("args_json is not valid JSON: {}", e))?;
    let mut client = build_client(&row, &meta)?;
    let _ = client.handshake().await?;
    let result = client
        .call("tools/call", json!({ "name": tool_name, "arguments": args }))
        .await;
    let audit = audit_entry(&server_id, &tool_name, &args_json, "allowed", &format!("domain:mcp={}", level));
    match result {
        Ok(v) => {
            let is_error = v.get("isError").and_then(|b| b.as_bool()).unwrap_or(false);
            Ok(McpCallResult {
                ok: !is_error,
                result: v,
                is_error,
                latency_ms: latency(),
                approval_required: false,
                audit,
            })
        }
        Err(e) => Ok(McpCallResult {
            ok: false,
            result: json!({ "error": e }),
            is_error: true,
            latency_ms: latency(),
            approval_required: false,
            audit,
        }),
    }
}

// ---------------------------------------------------------------------------
// OAuth 2.1 + PKCE (RFC 7636 / RFC 8414 style discovery)
// ---------------------------------------------------------------------------

fn redirect_uri_for(meta: &ServerMeta) -> String {
    if meta.oauth_redirect_uri.trim().is_empty() {
        DEFAULT_REDIRECT_URI.to_string()
    } else {
        meta.oauth_redirect_uri.trim().to_string()
    }
}

/// Try RFC 8414 well-known discovery against the MCP server origin when the
/// server record has no explicit OAuth endpoints configured.
async fn discover_oauth_metadata(server_url: &str) -> Option<Value> {
    let parsed = reqwest::Url::parse(server_url).ok()?;
    let host = parsed.host_str()?;
    let origin = format!("{}://{}", parsed.scheme(), host);
    let port_part = parsed.port().map(|p| format!(":{}", p)).unwrap_or_default();
    let well_known = format!("{}{}/.well-known/oauth-authorization-server", origin, port_part);
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().ok()?;
    http.get(&well_known).send().await.ok()?.json::<Value>().await.ok()
}

#[tauri::command]
pub async fn mcp_oauth_start(app: tauri::AppHandle, server_id: String) -> Result<OAuthStart, String> {
    let (row, meta) = load_server(&app, &server_id)?;
    if meta.auth_type != "oauth" {
        return Err("server is not configured for OAuth".to_string());
    }
    let mut auth_url = meta.oauth_authorization_url.clone();
    let mut token_url = meta.oauth_token_url.clone();
    if auth_url.is_empty() || token_url.is_empty() {
        if let Some(doc) = discover_oauth_metadata(&row.url).await {
            if auth_url.is_empty() {
                auth_url = doc.get("authorization_endpoint").and_then(|v| v.as_str()).unwrap_or("").to_string();
            }
            if token_url.is_empty() {
                token_url = doc.get("token_endpoint").and_then(|v| v.as_str()).unwrap_or("").to_string();
            }
        }
    }
    if meta.oauth_client_id.is_empty() || auth_url.is_empty() || token_url.is_empty() {
        return Err("OAuth is not fully configured for this server (need client id, authorization and token endpoints)".to_string());
    }

    // PKCE: 64-char verifier, challenge = BASE64URL(SHA256(verifier)).
    let verifier: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());
    let state = uuid::Uuid::new_v4().simple().to_string();

    // Persist verifier in the keyring under the spec key; the payload also
    // carries the server id so the callback only needs (state, code).
    let payload = serde_json::to_string(&json!({ "verifier": verifier, "server_id": server_id }))
        .map_err(|e| e.to_string())?;
    kr_set(&format!("{}{}", OAUTH_STATE_PREFIX, state), &payload)?;

    let redirect = redirect_uri_for(&meta);
    let mut url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}",
        auth_url,
        pct(&meta.oauth_client_id),
        pct(&redirect),
        pct(&challenge),
        pct(&state),
    );
    if !meta.oauth_scopes.trim().is_empty() {
        url.push_str(&format!("&scope={}", pct(meta.oauth_scopes.trim())));
    }
    // RFC 8707 resource indicator: the MCP server the token is for.
    url.push_str(&format!("&resource={}", pct(&row.url)));
    Ok(OAuthStart { auth_url: url, state })
}

#[tauri::command]
pub async fn mcp_oauth_callback(
    app: tauri::AppHandle,
    state: String,
    code: String,
) -> Result<OAuthCallbackResult, String> {
    let key = format!("{}{}", OAUTH_STATE_PREFIX, state);
    let raw = kr_get(&key).ok_or_else(|| "unknown or expired OAuth state — restart authentication".to_string())?;
    kr_del(&key); // single-use
    let saved: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let verifier = saved.get("verifier").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let server_id = saved.get("server_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if verifier.is_empty() || server_id.is_empty() {
        return Err("corrupt OAuth state — restart authentication".to_string());
    }
    let (_row, meta) = load_server(&app, &server_id)?;
    let redirect = redirect_uri_for(&meta);
    let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let resp = http
        .post(&meta.oauth_token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("client_id", meta.oauth_client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = body.get("error") {
        let desc = body.get("error_description").and_then(|v| v.as_str()).unwrap_or("");
        return Err(format!("token exchange failed: {} {}", err, desc).trim().to_string());
    }
    let access = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "token endpoint returned no access_token".to_string())?;
    let expires_in = body.get("expires_in").and_then(|v| v.as_i64());
    let stored = json!({
        "access_token": access,
        "refresh_token": body.get("refresh_token").and_then(|v| v.as_str()),
        "token_type": body.get("token_type").and_then(|v| v.as_str()).unwrap_or("Bearer"),
        "expires_at": expires_in.map(|s| chrono::Utc::now().timestamp() + s),
    });
    kr_set(
        &format!("{}{}", OAUTH_TOKEN_PREFIX, server_id),
        &serde_json::to_string(&stored).map_err(|e| e.to_string())?,
    )?;
    Ok(OAuthCallbackResult { ok: true, expires_in })
}

#[tauri::command]
pub async fn mcp_oauth_revoke(app: tauri::AppHandle, server_id: String) -> Result<OAuthRevokeResult, String> {
    let key = format!("{}{}", OAUTH_TOKEN_PREFIX, server_id);
    let raw = kr_get(&key);
    let mut detail = String::from("no token stored");
    if let Some(raw) = raw {
        let v: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
        let access = v.get("access_token").and_then(|t| t.as_str()).unwrap_or("").to_string();
        // Best-effort RFC 7009 revocation: prefer the revocation_endpoint from
        // well-known metadata, else try token_url with "token" -> "revoke".
        let (_row, meta) = load_server(&app, &server_id)?;
        let mut revoke_url: Option<String> = None;
        if let Some(doc) = discover_oauth_metadata(&_row.url).await {
            revoke_url = doc.get("revocation_endpoint").and_then(|e| e.as_str()).map(|s| s.to_string());
        }
        if revoke_url.is_none() && meta.oauth_token_url.contains("/token") {
            revoke_url = Some(meta.oauth_token_url.replacen("/token", "/revoke", 1));
        }
        if let (Some(url), true) = (revoke_url, !access.is_empty()) {
            let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
            let resp = http
                .post(&url)
                .form(&[
                    ("token", access.as_str()),
                    ("token_type_hint", "access_token"),
                    ("client_id", meta.oauth_client_id.as_str()),
                ])
                .send()
                .await;
            detail = match resp {
                Ok(r) if r.status().is_success() => format!("server revocation accepted ({})", url),
                Ok(r) => format!("server revocation returned HTTP {}; local token cleared", r.status()),
                Err(e) => format!("server revocation unreachable ({}); local token cleared", e),
            };
        } else {
            detail = "no revocation endpoint known; local token cleared".to_string();
        }
        kr_del(&key);
    }
    Ok(OAuthRevokeResult { ok: true, detail })
}

// ---------------------------------------------------------------------------
// Permission Center commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn perm_get(app: tauri::AppHandle, domain: String) -> Result<String, String> {
    validate_perm(&domain, DEFAULT_PERM_LEVEL)?;
    let conn = open_db(&app)?;
    Ok(perm_get_level(&conn, &domain))
}

#[tauri::command]
pub async fn perm_set(app: tauri::AppHandle, domain: String, level: String) -> Result<(), String> {
    validate_perm(&domain, &level)?;
    let conn = open_db(&app)?;
    conn.execute(
        "INSERT INTO permissions (agent_id, tool, domain, mode) VALUES ('', '*', ?1, ?2)
         ON CONFLICT(agent_id, tool, domain) DO UPDATE SET mode = excluded.mode",
        params![domain, level],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn perm_list(app: tauri::AppHandle) -> Result<Vec<PermEntry>, String> {
    let conn = open_db(&app)?;
    Ok(PERM_DOMAINS
        .iter()
        .map(|d| PermEntry {
            domain: d.to_string(),
            level: perm_get_level(&conn, d),
        })
        .collect())
}
