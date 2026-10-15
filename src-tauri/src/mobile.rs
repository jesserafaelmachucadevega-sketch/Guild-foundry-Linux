// SPDX-License-Identifier: Apache-2.0
// mobile.rs — LAN mobile companion: a tiny HTTP surface on the local network.
//
// No cloud, no relay: the phone talks straight to this machine over Wi-Fi.
// Everything is gated behind a per-start random token carried in the link,
// because anyone on the LAN holding the link could otherwise talk to the
// assistant. The companion chats on the SAME persisted thread as the Agent
// tab, so the conversation follows the user between desktop and phone.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

#[derive(Clone)]
struct Ctx {
    app: tauri::AppHandle,
    token: String,
}

struct Running {
    url: String,
    token: String,
    port: u16,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

fn state() -> &'static Mutex<Option<Running>> {
    static S: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// First non-loopback IPv4 reachable via the default route. The UDP
/// "connect" is a routing-table lookup only — no packet is ever sent.
fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:80").ok()?; // TEST-NET-1; nothing transmits
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

/// Constant-time-ish token comparison (no early-exit equality leak).
fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn authorized(ctx: &Ctx, q: &HashMap<String, String>) -> bool {
    q.get("k").map(|k| token_eq(k, &ctx.token)).unwrap_or(false)
}

type ApiError = (StatusCode, String);

fn internal<E: std::fmt::Display>(e: E) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn unauthorized() -> ApiError {
    (
        StatusCode::UNAUTHORIZED,
        "missing or wrong token — open the link from the app's Mobile card".to_string(),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "app": "Guild Foundry AI", "api": 1 }))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("mobile_page.html"))
}

fn message_text(m: &Value) -> String {
    for key in ["full", "text", "content"] {
        if let Some(s) = m.get(key).and_then(|v| v.as_str()) {
            if !s.trim().is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

async fn thread(
    State(ctx): State<Ctx>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    if !authorized(&ctx, &q) {
        return Err(unauthorized());
    }
    let msgs = crate::harness::harness_thread_load(ctx.app.clone(), None).map_err(internal)?;
    let compact: Vec<Value> = msgs
        .iter()
        .filter(|m| {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
            role == "user" || role == "assistant"
        })
        .filter_map(|m| {
            let text = message_text(m);
            if text.trim().is_empty() {
                return None;
            }
            Some(json!({
                "role": m.get("role").and_then(|r| r.as_str()).unwrap_or("user"),
                "text": text,
                "model": m.get("modelName").and_then(|v| v.as_str()).unwrap_or(""),
            }))
        })
        .collect();
    let start = compact.len().saturating_sub(40);
    Ok(Json(json!({ "messages": &compact[start..] })))
}

#[derive(serde::Deserialize)]
struct ChatBody {
    message: String,
}

fn active_choice(app: &tauri::AppHandle) -> Result<(String, String), String> {
    let get = |key: &str| -> Option<String> {
        crate::settings::settings_get(app.clone(), key.to_string())
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.trim().is_empty())
    };
    match (get("agent.active_provider"), get("agent.active_model")) {
        (Some(p), Some(m)) => Ok((p, m)),
        _ => Err(
            "no model selected yet — open the Agent tab once on the desktop so a provider and model are chosen"
                .to_string(),
        ),
    }
}

async fn chat(
    State(ctx): State<Ctx>,
    Query(q): Query<HashMap<String, String>>,
    Json(body): Json<ChatBody>,
) -> Result<Json<Value>, ApiError> {
    if !authorized(&ctx, &q) {
        return Err(unauthorized());
    }
    let message = body.message.trim().to_string();
    if message.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty message".to_string()));
    }
    if message.len() > 8000 {
        return Err((StatusCode::BAD_REQUEST, "message too long (max 8000 chars)".to_string()));
    }
    let (provider_id, model_id) =
        active_choice(&ctx.app).map_err(|e| (StatusCode::CONFLICT, e))?;
    let existing = crate::harness::harness_thread_load(ctx.app.clone(), None).unwrap_or_default();

    // Build model history from the shared thread (last 20 turns, newest kept).
    let mut history: Vec<crate::providers::ChatMsg> = Vec::new();
    let recent: Vec<&Value> = existing.iter().rev().take(20).collect();
    for m in recent.into_iter().rev() {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }
        let text = message_text(m);
        if text.trim().is_empty() {
            continue;
        }
        history.push(crate::providers::ChatMsg::new(role, &text));
    }
    history.push(crate::providers::ChatMsg::new("user", &message));

    let result = crate::providers::provider_chat_complete(
        ctx.app.clone(),
        provider_id.clone(),
        model_id.clone(),
        history,
        crate::providers::ChatParams::default(),
    )
    .await
    .map_err(internal)?;
    // Consume CompleteResult through its Serialize contract (fields are
    // private by design; the wire shape is the public API).
    let result_val = serde_json::to_value(&result).map_err(internal)?;
    let reply = result_val
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let latency_ms = result_val
        .get("latency_ms")
        .and_then(|l| l.as_u64())
        .unwrap_or(0);

    // Persist on the same thread the desktop uses, so the conversation
    // continues in both directions.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut updated = existing;
    updated.push(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "role": "user",
        "text": message,
        "full": message,
        "from": "mobile",
        "ts": now_ms,
    }));
    updated.push(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "role": "assistant",
        "text": reply.clone(),
        "full": reply.clone(),
        "modelName": model_id,
        "from": "mobile",
        "ts": now_ms,
    }));
    if let Err(e) = crate::harness::harness_thread_save(ctx.app.clone(), updated, None) {
        // The reply is still valid; surface the persistence failure honestly.
        return Ok(Json(json!({
            "reply": reply,
            "model": model_id,
            "latency_ms": latency_ms,
            "warning": format!("reply could not be saved to the shared thread: {}", e),
        })));
    }
    Ok(Json(json!({
        "reply": reply,
        "model": model_id,
        "latency_ms": latency_ms,
    })))
}

pub fn start(app: &tauri::AppHandle, port: u16) -> Result<Value, String> {
    let mut guard = state().lock().map_err(|e| e.to_string())?;
    if let Some(r) = guard.as_ref() {
        return Ok(json!({
            "running": true,
            "already_running": true,
            "url": r.url,
            "token": r.token,
            "port": r.port,
        }));
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    let ctx = Ctx {
        app: app.clone(),
        token: token.clone(),
    };
    let router = Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/api/thread", get(thread))
        .route("/api/chat", post(chat))
        .with_state(ctx);

    // Bind synchronously so a busy port fails here (and is reported to the
    // UI), instead of silently inside the background task.
    let addr: std::net::SocketAddr = format!("0.0.0.0:{}", port)
        .parse()
        .map_err(|e| format!("bad port {}: {}", port, e))?;
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|e| format!("cannot listen on port {}: {}", port, e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(_) => return,
        };
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });

    let ip = lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let url = format!("http://{}:{}/?k={}", ip, port, token);
    *guard = Some(Running {
        url: url.clone(),
        token: token.clone(),
        port,
        shutdown: Some(tx),
    });
    Ok(json!({ "running": true, "url": url, "token": token, "port": port, "lan_ip": ip }))
}

/// Stop without a Result (used by the app-exit hook).
pub fn shutdown() {
    if let Ok(mut guard) = state().lock() {
        if let Some(mut r) = guard.take() {
            if let Some(tx) = r.shutdown.take() {
                let _ = tx.send(());
            }
        }
    }
}

#[tauri::command(rename_all = "snake_case")]
pub fn mobile_start(app: tauri::AppHandle, port: Option<u16>) -> Result<Value, String> {
    start(&app, port.unwrap_or(8787).clamp(1024, 65535))
}

#[tauri::command]
pub fn mobile_stop() -> Result<(), String> {
    let mut guard = state().lock().map_err(|e| e.to_string())?;
    if let Some(mut r) = guard.take() {
        if let Some(tx) = r.shutdown.take() {
            let _ = tx.send(());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn mobile_status() -> Result<Value, String> {
    let guard = state().lock().map_err(|e| e.to_string())?;
    Ok(match guard.as_ref() {
        Some(r) => json!({
            "running": true,
            "url": r.url,
            "token": r.token,
            "port": r.port,
            "lan_ip": lan_ip(),
        }),
        None => json!({ "running": false, "port": 8787, "lan_ip": lan_ip() }),
    })
}

#[cfg(test)]
mod tests {
    use super::token_eq;

    #[test]
    fn token_comparison_is_exact() {
        assert!(token_eq("abc123", "abc123"));
        assert!(!token_eq("abc123", "abc124"));
        assert!(!token_eq("abc", "abc123"));
        assert!(!token_eq("", "x"));
        assert!(token_eq("", ""));
    }
}
