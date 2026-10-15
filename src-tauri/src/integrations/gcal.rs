// SPDX-License-Identifier: Apache-2.0
// gcal.rs — Google Calendar via OAuth 2.0 (authorization code + PKCE,
// loopback redirect).
//
// The user registers a free "Desktop app" OAuth client in Google Cloud and
// pastes its client id/secret into Connections; everything else happens here:
// PKCE, the 127.0.0.1 loopback callback, code exchange, keychain storage and
// refresh-token renewal. No middleman service — tokens go straight to Google,
// the refresh token lives in the OS keychain, and only the account owner ever
// sees the client secret.
//
// The loopback listener is bound BEFORE the browser opens, so a fast redirect
// can never race the app; the connection window is 5 minutes.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::Rng as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SETTINGS_KEY: &str = "calendar.google";
pub const SECRET_KEY: &str = "gfa-gcal:client-secret";
pub const REFRESH_KEY: &str = "gfa-gcal:refresh";

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const SCOPES: &str = "https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.readonly";
const CONNECT_WINDOW: Duration = Duration::from_secs(300);

const CLOSE_PAGE: &str = "<!doctype html><html><body style=\"font-family:system-ui;background:#0e1420;color:#e8eef8;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><h2>Google Calendar connected</h2><p>You can close this tab and return to Guild Foundry AI.</p></div></body></html>";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Idle,
    Waiting,
    Connected,
    Failed(String),
}

#[derive(Clone, Debug)]
struct ConnectStatus {
    phase: Phase,
    calendars: usize,
}

fn status_slot() -> &'static Mutex<ConnectStatus> {
    static S: OnceLock<Mutex<ConnectStatus>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(ConnectStatus {
            phase: Phase::Idle,
            calendars: 0,
        })
    })
}

/// Cached access token: (token, expires_at).
fn access_slot() -> &'static Mutex<Option<(String, Instant)>> {
    static S: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

// ---------------------------------------------------------------------------
// PKCE + URL building (pure — covered by tests)
// ---------------------------------------------------------------------------

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn pkce_pair() -> Pkce {
    let verifier: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();
    let challenge = challenge_for(&verifier);
    Pkce { verifier, challenge }
}

/// RFC 7636 S256: challenge = BASE64URL-NOPAD(SHA256(verifier)).
pub fn challenge_for(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

pub fn build_auth_url(client_id: &str, redirect_uri: &str, state: &str, challenge: &str) -> String {
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent&include_granted_scopes=true",
        AUTH_URL,
        pct(client_id),
        pct(redirect_uri),
        pct(SCOPES),
        pct(challenge),
        pct(state),
    )
}

/// Google's token endpoint shape → (access, refresh?, expires_in).
pub fn tokens_from_response(body: &Value) -> Result<(String, Option<String>, u64), String> {
    if let Some(err) = body.get("error") {
        let desc = body
            .get("error_description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        return Err(format!("Google rejected the token request: {} {}", err, desc)
            .trim()
            .to_string());
    }
    let access = body
        .get("access_token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "token response had no access_token".to_string())?
        .to_string();
    let refresh = body
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    let expires = body.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(3600);
    Ok((access, refresh, expires))
}

/// Event create body: `dateTime` when the string carries a time, `date`
/// otherwise (all-day). Pure — covered by tests.
pub fn event_body(
    summary: &str,
    start: &str,
    end: &str,
    description: Option<&str>,
    location: Option<&str>,
) -> Value {
    fn side(s: &str) -> Value {
        if s.contains('T') {
            json!({ "dateTime": s })
        } else {
            json!({ "date": s })
        }
    }
    let mut body = json!({ "summary": summary, "start": side(start), "end": side(end) });
    if let Some(d) = description.filter(|d| !d.trim().is_empty()) {
        body["description"] = json!(d);
    }
    if let Some(l) = location.filter(|l| !l.trim().is_empty()) {
        body["location"] = json!(l);
    }
    body
}

fn event_time(v: Option<&Value>) -> Option<String> {
    let v = v?;
    v.get("dateTime")
        .or_else(|| v.get("date"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

fn client_id(app: &tauri::AppHandle) -> Option<String> {
    crate::settings::settings_get(app.clone(), SETTINGS_KEY.to_string())
        .ok()
        .flatten()
        .and_then(|v| {
            v.get("client_id")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.trim().is_empty())
}

fn client_creds(app: &tauri::AppHandle) -> Result<(String, String), String> {
    let id = client_id(app).ok_or_else(|| {
        "Google client ID missing — add it under Connections → Google Calendar".to_string()
    })?;
    let secret = crate::secrets::secret_get(SECRET_KEY.to_string())?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            "Google client secret missing — save it again on the Google Calendar card".to_string()
        })?;
    Ok((id, secret))
}

pub fn connected() -> bool {
    matches!(
        crate::secrets::secret_get(REFRESH_KEY.to_string()),
        Ok(Some(v)) if !v.trim().is_empty()
    )
}

fn open_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ---------------------------------------------------------------------------
// OAuth flow
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct CallbackCtx {
    tx: tokio::sync::mpsc::Sender<Result<String, String>>,
    want_state: String,
}

async fn callback_handler(
    axum::extract::State(cb): axum::extract::State<CallbackCtx>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> axum::response::Html<&'static str> {
    let outcome = if q.get("state").map(|s| s.as_str()) != Some(cb.want_state.as_str()) {
        Err("state mismatch — this callback did not come from this login attempt".to_string())
    } else if let Some(err) = q.get("error") {
        Err(format!("Google returned: {}", err))
    } else if let Some(code) = q.get("code").filter(|c| !c.trim().is_empty()) {
        Ok(code.clone())
    } else {
        Err("callback had neither code nor error".to_string())
    };
    let _ = cb.tx.try_send(outcome);
    axum::response::Html(CLOSE_PAGE)
}

async fn wait_for_code(
    listener: std::net::TcpListener,
    want_state: &str,
) -> Result<String, String> {
    use axum::routing::get;
    use axum::Router;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<String, String>>(1);
    let router = Router::new()
        .route("/", get(callback_handler))
        .with_state(CallbackCtx {
            tx,
            want_state: want_state.to_string(),
        });
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
    let serve = tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let outcome = tokio::time::timeout(CONNECT_WINDOW, rx.recv()).await;
    serve.abort();
    match outcome {
        Ok(Some(res)) => res,
        Ok(None) => Err("the browser callback never arrived".to_string()),
        Err(_) => Err(format!(
            "timed out waiting for the browser ({} minutes) — press Connect again",
            CONNECT_WINDOW.as_secs() / 60
        )),
    }
}

async fn post_token_form(form: Vec<(&str, &str)>) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Google token request failed: {}", e))?;
    let status = resp.status().as_u16();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("token response was not JSON: {}", e))?;
    if status >= 400 {
        let msg = tokens_from_response(&body).err().unwrap_or_default();
        return Err(format!("{} (HTTP {})", msg, status));
    }
    Ok(body)
}

async fn exchange_code(
    app: &tauri::AppHandle,
    client_id: &str,
    verifier: &str,
    redirect: &str,
    code: &str,
) -> Result<(), String> {
    let (_, secret) = client_creds(app)?;
    let body = post_token_form(vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("client_id", client_id),
        ("client_secret", secret.as_str()),
        ("code_verifier", verifier),
    ])
    .await?;
    let (access, refresh, expires) = tokens_from_response(&body)?;
    let refresh = refresh.ok_or_else(|| {
        "Google did not return a refresh token — remove Guild Foundry AI from your Google Account's third-party access, then Connect again".to_string()
    })?;
    crate::secrets::secret_set(app.clone(), REFRESH_KEY.to_string(), refresh)?;
    if let Ok(mut slot) = access_slot().lock() {
        *slot = Some((
            access,
            Instant::now() + Duration::from_secs(expires.saturating_sub(60)),
        ));
    }
    Ok(())
}

pub async fn access_token(app: &tauri::AppHandle) -> Result<String, String> {
    if let Ok(slot) = access_slot().lock() {
        if let Some((tok, until)) = slot.as_ref() {
            if Instant::now() < *until {
                return Ok(tok.clone());
            }
        }
    }
    let (id, secret) = client_creds(app)?;
    let refresh = crate::secrets::secret_get(REFRESH_KEY.to_string())?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            "Google Calendar is not connected — press Connect on the Google Calendar card"
                .to_string()
        })?;
    let body = post_token_form(vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh.as_str()),
        ("client_id", id.as_str()),
        ("client_secret", secret.as_str()),
    ])
    .await?;
    let (access, _r, expires) = tokens_from_response(&body)?;
    if let Ok(mut slot) = access_slot().lock() {
        *slot = Some((
            access.clone(),
            Instant::now() + Duration::from_secs(expires.saturating_sub(60)),
        ));
    }
    Ok(access)
}

// ---------------------------------------------------------------------------
// Calendar API
// ---------------------------------------------------------------------------

fn google_error(status: u16, body: &Value) -> String {
    let msg = body
        .pointer("/error/message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error");
    format!("Google Calendar API error ({}): {}", status, msg)
}

async fn api_call(
    app: &tauri::AppHandle,
    method: reqwest::Method,
    url: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| e.to_string())?;
    for attempt in 0..2 {
        let token = access_token(app).await?;
        let mut req = client.request(method.clone(), url).bearer_auth(&token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("Google Calendar request failed: {}", e))?;
        let status = resp.status().as_u16();
        let value: Value = resp.json().await.unwrap_or(Value::Null);
        if status == 401 && attempt == 0 {
            // Token went stale between calls: drop the cache and retry once.
            if let Ok(mut slot) = access_slot().lock() {
                *slot = None;
            }
            continue;
        }
        if status >= 400 {
            return Err(google_error(status, &value));
        }
        return Ok(value);
    }
    Err("Google Calendar request failed after a token refresh — try Connect again".to_string())
}

pub async fn list_calendars(app: &tauri::AppHandle) -> Result<Value, String> {
    let v = api_call(
        app,
        reqwest::Method::GET,
        &format!("{}/users/me/calendarList", API_BASE),
        None,
    )
    .await?;
    let items: Vec<Value> = v
        .get("items")
        .and_then(|i| i.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    json!({
                        "id": c.get("id"),
                        "summary": c.get("summary"),
                        "primary": c.get("primary").and_then(|p| p.as_bool()).unwrap_or(false),
                        "timezone": c.get("timeZone"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "count": items.len(), "calendars": items }))
}

pub async fn list_events(
    app: &tauri::AppHandle,
    calendar_id: Option<&str>,
    days: Option<u32>,
    max: Option<u32>,
) -> Result<Value, String> {
    let cal = calendar_id
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("primary");
    let days = days.unwrap_or(7).clamp(1, 365);
    let max = max.unwrap_or(50).clamp(1, 250);
    let now = chrono::Utc::now();
    let time_min = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let time_max = (now + chrono::Duration::days(days as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let url = format!(
        "{}/calendars/{}/events?timeMin={}&timeMax={}&maxResults={}&singleEvents=true&orderBy=startTime",
        API_BASE,
        pct(cal),
        pct(&time_min),
        pct(&time_max),
        max
    );
    let v = api_call(app, reqwest::Method::GET, &url, None).await?;
    let items: Vec<Value> = v
        .get("items")
        .and_then(|i| i.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| {
                    json!({
                        "id": e.get("id"),
                        "summary": e.get("summary"),
                        "start": event_time(e.get("start")),
                        "end": event_time(e.get("end")),
                        "location": e.get("location"),
                        "link": e.get("htmlLink"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "calendar": cal, "days": days, "count": items.len(), "events": items }))
}

#[allow(clippy::too_many_arguments)]
pub async fn create_event(
    app: &tauri::AppHandle,
    calendar_id: Option<&str>,
    summary: &str,
    start: &str,
    end: &str,
    description: Option<&str>,
    location: Option<&str>,
) -> Result<Value, String> {
    if summary.trim().is_empty() {
        return Err("event needs a summary".to_string());
    }
    if start.trim().is_empty() || end.trim().is_empty() {
        return Err("event needs start and end (RFC3339 with time, or YYYY-MM-DD for all-day)".to_string());
    }
    let cal = calendar_id
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("primary");
    let url = format!("{}/calendars/{}/events", API_BASE, pct(cal));
    let body = event_body(summary, start, end, description, location);
    let v = api_call(app, reqwest::Method::POST, &url, Some(&body)).await?;
    Ok(json!({
        "created": true,
        "id": v.get("id"),
        "link": v.get("htmlLink"),
        "summary": v.get("summary"),
        "start": event_time(v.get("start")),
        "end": event_time(v.get("end")),
    }))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command(rename_all = "snake_case")]
pub fn gcal_save_client(
    app: tauri::AppHandle,
    client_id: String,
    client_secret: String,
) -> Result<(), String> {
    let id = client_id.trim().to_string();
    if id.is_empty() || !id.ends_with(".apps.googleusercontent.com") {
        return Err(
            "that does not look like a Google OAuth client ID (expected …apps.googleusercontent.com)"
                .to_string(),
        );
    }
    crate::settings::settings_set(app.clone(), SETTINGS_KEY.to_string(), json!({ "client_id": id }))
        .map_err(|e| e.to_string())?;
    let secret = client_secret.trim().to_string();
    if !secret.is_empty() {
        crate::secrets::secret_set(app, SECRET_KEY.to_string(), secret)?;
    }
    Ok(())
}

#[tauri::command]
pub fn gcal_status(app: tauri::AppHandle) -> Result<Value, String> {
    let phase = status_slot().lock().map_err(|e| e.to_string())?.clone();
    Ok(json!({
        "configured": client_id(&app).is_some(),
        "connected": connected(),
        "phase": match phase.phase {
            Phase::Idle => "idle",
            Phase::Waiting => "waiting",
            Phase::Connected => "connected",
            Phase::Failed(_) => "failed",
        },
        "error": match phase.phase {
            Phase::Failed(ref e) => Some(e.clone()),
            _ => None,
        },
        "calendars": phase.calendars,
    }))
}

/// Binds the loopback listener, opens the browser, and completes the exchange
/// in the background; poll `gcal_status` for the outcome.
#[tauri::command]
pub async fn gcal_connect(app: tauri::AppHandle) -> Result<Value, String> {
    let (id, _secret) = client_creds(&app)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("cannot open the loopback listener: {}", e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{}", port);
    let pkce = pkce_pair();
    let state = uuid::Uuid::new_v4().simple().to_string();
    let auth_url = build_auth_url(&id, &redirect, &state, &pkce.challenge);

    {
        let mut st = status_slot().lock().map_err(|e| e.to_string())?;
        *st = ConnectStatus {
            phase: Phase::Waiting,
            calendars: 0,
        };
    }
    open_browser(&auth_url);

    let app2 = app.clone();
    let client_id = id.clone();
    tauri::async_runtime::spawn(async move {
        let result: Result<(), String> = async {
            let code = wait_for_code(listener, &state).await?;
            exchange_code(&app2, &client_id, &pkce.verifier, &redirect, &code).await?;
            let n = list_calendars(&app2)
                .await
                .ok()
                .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
                .unwrap_or(0) as usize;
            if let Ok(mut st) = status_slot().lock() {
                *st = ConnectStatus {
                    phase: Phase::Connected,
                    calendars: n,
                };
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            if let Ok(mut st) = status_slot().lock() {
                *st = ConnectStatus {
                    phase: Phase::Failed(e),
                    calendars: 0,
                };
            }
        }
    });

    Ok(json!({ "auth_url": auth_url, "port": port }))
}

#[tauri::command]
pub fn gcal_disconnect(app: tauri::AppHandle) -> Result<(), String> {
    let _ = crate::secrets::secret_delete(app, REFRESH_KEY.to_string());
    if let Ok(mut slot) = access_slot().lock() {
        *slot = None;
    }
    if let Ok(mut st) = status_slot().lock() {
        *st = ConnectStatus {
            phase: Phase::Idle,
            calendars: 0,
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc7636_test_vector() {
        // RFC 7636 appendix B.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let p = pkce_pair();
        assert!(p.verifier.len() >= 43);
        assert!(!p.challenge.is_empty());
        assert_ne!(p.verifier, p.challenge);
    }

    #[test]
    fn auth_url_carries_every_required_parameter() {
        let url = build_auth_url(
            "123.apps.googleusercontent.com",
            "http://127.0.0.1:4567",
            "state123",
            "challenge456",
        );
        assert!(url.starts_with(AUTH_URL));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A4567"));
        assert!(url.contains("state=state123"));
        assert!(url.contains("123.apps.googleusercontent.com"));
        assert!(url.contains("calendar.events"));
    }

    #[test]
    fn token_response_parsing_covers_all_three_outcomes() {
        let ok = json!({
            "access_token": "ya29.abc",
            "refresh_token": "1//refresh",
            "expires_in": 3599
        });
        let (access, refresh, expires) = tokens_from_response(&ok).expect("ok");
        assert_eq!(access, "ya29.abc");
        assert_eq!(refresh.as_deref(), Some("1//refresh"));
        assert_eq!(expires, 3599);

        // Refresh grant responses carry no new refresh token.
        let refreshed = json!({ "access_token": "ya29.new", "expires_in": 3599 });
        let (_, refresh, _) = tokens_from_response(&refreshed).expect("refresh");
        assert!(refresh.is_none());

        let err = json!({ "error": "invalid_grant", "error_description": "Bad Request" });
        let e = tokens_from_response(&err).unwrap_err();
        assert!(e.contains("invalid_grant"));
        assert!(e.contains("Bad Request"));
    }

    #[test]
    fn event_body_switches_between_timed_and_all_day() {
        let timed = event_body("Standup", "2026-10-16T09:00:00-07:00", "2026-10-16T09:30:00-07:00", None, None);
        assert_eq!(timed["start"]["dateTime"], "2026-10-16T09:00:00-07:00");
        assert!(timed["start"].get("date").is_none());
        assert!(timed.get("description").is_none());

        let all_day = event_body("Mom's birthday", "2026-11-02", "2026-11-03", Some("  "), Some("Home"));
        assert_eq!(all_day["start"]["date"], "2026-11-02");
        assert!(all_day["start"].get("dateTime").is_none());
        assert!(all_day.get("description").is_none(), "blank description must be omitted");
        assert_eq!(all_day["location"], "Home");
    }

    #[test]
    fn percent_encoding_keeps_unreserved_characters() {
        assert_eq!(pct("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(pct("a:b/c d"), "a%3Ab%2Fc%20d");
    }
}
