// SPDX-License-Identifier: Apache-2.0
// twilio.rs — phone calls + SMS via a Twilio account (phone.* tools).
//
// Config lives in the settings store as `phone.twilio` (account_sid,
// from_number, label); the auth token lives ONLY in the OS keyring under
// `gfa-twilio:auth`. Every send/call costs real money and is approval-gated
// upstream — the tool descriptions say so.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Manager as _;

use super::AccountTest;

pub const SETTINGS_KEY: &str = "phone.twilio";
pub const KEYRING_KEY: &str = "gfa-twilio:auth";

pub fn setup_message() -> String {
    "No phone provider configured. Add your Twilio account SID, auth token, and phone number under System & Settings → Tools & Registry → Connections (Phone card), then retry.".to_string()
}

fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn load_config(app: &tauri::AppHandle) -> Result<(String, String), String> {
    let v = crate::settings::settings_get(app.clone(), SETTINGS_KEY.to_string())
        .map_err(|e| e.to_string())?;
    let v = match v {
        Some(v) if !v.is_null() => v,
        _ => return Err(setup_message()),
    };
    let sid = field(&v, "account_sid");
    let from = field(&v, "from_number");
    if sid.is_empty() || from.is_empty() {
        return Err(
            "Phone provider config is incomplete (missing account SID or phone number). Fix it under Connections."
                .to_string(),
        );
    }
    Ok((sid, from))
}

fn load_token() -> Result<String, String> {
    crate::secrets::keyring_get(KEYRING_KEY).ok_or_else(|| {
        "Twilio auth token missing — save it again in Connections (stored in your OS keychain)."
            .to_string()
    })
}

fn twilio_error(code: u16, body: &str) -> String {
    let msg = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| body.chars().take(200).collect());
    match code {
        401 => format!(
            "Twilio rejected the credentials (401). Check the Account SID and auth token in Connections. {}",
            msg
        ),
        400 => format!("Twilio rejected the request (400): {}", msg),
        429 => "Twilio rate-limited the account (429). Wait and retry.".to_string(),
        s => format!("Twilio returned HTTP {}: {}", s, msg),
    }
}

/// One pooled client for every Twilio call: a fresh client per call rebuilt
/// the whole connection pool and TLS state each time, and `Client::new()`
/// has no timeout at all — a silent network black hole would hang the call
/// forever. The static client bounds both connect and total request time.
fn http() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build twilio HTTP client")
    })
}

async fn post_form(
    sid: &str,
    token: &str,
    url: &str,
    form: Vec<(String, String)>,
) -> Result<(u16, String), String> {
    let resp = http()
        .post(url)
        .basic_auth(sid, Some(token.to_string()))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Twilio request failed: {}", e))?;
    let code = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((code, text))
}

async fn get_json(sid: &str, token: &str, url: &str) -> Result<(u16, String), String> {
    let resp = http()
        .get(url)
        .basic_auth(sid, Some(token.to_string()))
        .send()
        .await
        .map_err(|e| format!("Twilio request failed: {}", e))?;
    let code = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((code, text))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn sms_send(app: &tauri::AppHandle, to: &str, body: &str) -> Result<Value, String> {
    let (sid, from) = load_config(app)?;
    let token = load_token()?;
    if body.chars().count() > 1600 {
        return Err("SMS body too long (max 1600 characters)".to_string());
    }
    let url = format!("https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json", sid);
    let (code, text) = tauri::async_runtime::block_on(post_form(
        &sid,
        &token,
        &url,
        vec![
            ("To".to_string(), to.to_string()),
            ("From".to_string(), from.clone()),
            ("Body".to_string(), body.to_string()),
        ],
    ))?;
    if code == 201 {
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Ok(json!({
            "sent": true,
            "sid": v.get("sid").cloned().unwrap_or(Value::Null),
            "status": v.get("status").cloned().unwrap_or(Value::Null),
            "to": to,
            "from": from,
        }));
    }
    Err(twilio_error(code, &text))
}

pub fn call_start(
    app: &tauri::AppHandle,
    to: &str,
    url: Option<&str>,
    message: Option<&str>,
) -> Result<Value, String> {
    let (sid, from) = load_config(app)?;
    let token = load_token()?;
    let twiml = match (url, message) {
        (Some(u), _) if !u.trim().is_empty() => None.or(Some(("Url".to_string(), u.trim().to_string()))),
        (_, Some(m)) if !m.trim().is_empty() => Some((
            "Twiml".to_string(),
            format!(
                "<Response><Say voice=\"Polly.Joanna\">{}</Say></Response>",
                xml_escape(m.trim())
            ),
        )),
        _ => {
            return Err(
                "phone.call_start needs either 'message' (read aloud when answered) or 'url' (a TwiML document)"
                    .to_string(),
            )
        }
    };
    let call_url = format!("https://api.twilio.com/2010-04-01/Accounts/{}/Calls.json", sid);
    let mut form = vec![
        ("To".to_string(), to.to_string()),
        ("From".to_string(), from.clone()),
    ];
    if let Some(pair) = twiml {
        form.push(pair);
    }
    // Recording is opt-in (settings key phone.record_calls) — recording laws
    // vary by jurisdiction; the UI states that plainly.
    if record_enabled(app) {
        form.push(("Record".to_string(), "true".to_string()));
    }
    let (code, text) = tauri::async_runtime::block_on(post_form(&sid, &token, &call_url, form))?;
    if code == 201 {
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Ok(json!({
            "started": true,
            "sid": v.get("sid").cloned().unwrap_or(Value::Null),
            "status": v.get("status").cloned().unwrap_or(Value::Null),
            "to": to,
            "from": from,
        }));
    }
    Err(twilio_error(code, &text))
}

pub fn call_status(app: &tauri::AppHandle, call_sid: Option<&str>) -> Result<Value, String> {
    let (sid, _from) = load_config(app)?;
    let token = load_token()?;
    match call_sid {
        Some(s) if !s.trim().is_empty() => {
            let url = format!(
                "https://api.twilio.com/2010-04-01/Accounts/{}/Calls/{}.json",
                sid,
                s.trim()
            );
            let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &url))?;
            if code == 200 {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                return Ok(json!({
                    "sid": v.get("sid"), "status": v.get("status"),
                    "to": v.get("to"), "from": v.get("from"),
                    "duration": v.get("duration"), "start_time": v.get("start_time"),
                }));
            }
            Err(twilio_error(code, &text))
        }
        _ => {
            let url = format!(
                "https://api.twilio.com/2010-04-01/Accounts/{}/Calls.json?PageSize=10",
                sid
            );
            let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &url))?;
            if code == 200 {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let calls: Vec<Value> = v
                    .get("calls")
                    .and_then(|c| c.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|c| {
                                json!({
                                    "sid": c.get("sid"), "status": c.get("status"),
                                    "to": c.get("to"), "from": c.get("from"),
                                    "duration": c.get("duration"), "start_time": c.get("start_time"),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                return Ok(json!({ "count": calls.len(), "calls": calls }));
            }
            Err(twilio_error(code, &text))
        }
    }
}

/// Connections "Test" button — verifies Twilio credentials.
#[tauri::command]
pub fn twilio_test_connection(app: tauri::AppHandle) -> Result<AccountTest, String> {
    let started = Instant::now();
    let result = (|| -> Result<String, String> {
        let (sid, from) = load_config(&app)?;
        let token = load_token()?;
        let url = format!("https://api.twilio.com/2010-04-01/Accounts/{}.json", sid);
        let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &url))?;
        if code != 200 {
            return Err(twilio_error(code, &text));
        }
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let name = v
            .get("friendly_name")
            .and_then(|n| n.as_str())
            .unwrap_or("account");
        Ok(format!("Twilio account '{}' verified (sending from {})", name, from))
    })();
    Ok(match result {
        Ok(detail) => AccountTest {
            ok: true,
            layman: "Ready".to_string(),
            detail,
            latency_ms: started.elapsed().as_millis() as u64,
        },
        Err(e) => AccountTest {
            ok: false,
            layman: "Not connected".to_string(),
            detail: e,
            latency_ms: started.elapsed().as_millis() as u64,
        },
    })
}

// ---------------------------------------------------------------------------
// Call recordings → offline transcription → memory
// ---------------------------------------------------------------------------

pub const RECORD_KEY: &str = "phone.record_calls";

pub fn record_enabled(app: &tauri::AppHandle) -> bool {
    crate::settings::settings_get(app.clone(), RECORD_KEY.to_string())
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// List a call's recordings (empty when the call was not recorded).
pub fn call_recordings(app: &tauri::AppHandle, call_sid: &str) -> Result<Value, String> {
    let (sid, _from) = load_config(app)?;
    let token = load_token()?;
    if call_sid.trim().is_empty() {
        return Err("call_recordings needs a call sid".to_string());
    }
    let url = format!(
        "https://api.twilio.com/2010-04-01/Accounts/{}/Calls/{}/Recordings.json",
        sid,
        call_sid.trim()
    );
    let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &url))?;
    if code != 200 {
        return Err(twilio_error(code, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let items: Vec<Value> = v
        .get("recordings")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    json!({
                        "sid": r.get("sid"),
                        "duration": r.get("duration"),
                        "date_created": r.get("date_created"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "call_sid": call_sid.trim(), "count": items.len(), "recordings": items }))
}

fn latest_call_sid(app: &tauri::AppHandle) -> Result<String, String> {
    let (sid, _from) = load_config(app)?;
    let token = load_token()?;
    let url = format!(
        "https://api.twilio.com/2010-04-01/Accounts/{}/Calls.json?PageSize=1",
        sid
    );
    let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &url))?;
    if code != 200 {
        return Err(twilio_error(code, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    v.get("calls")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("sid"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no calls found on this account".to_string())
}

/// Download the newest recording of a call as WAV into app-data. Returns
/// (recording_sid, path). Uses its own client: recordings can be large.
fn download_recording(
    app: &tauri::AppHandle,
    call_sid: &str,
) -> Result<(String, std::path::PathBuf), String> {
    let (sid, _from) = load_config(app)?;
    let token = load_token()?;
    let list_url = format!(
        "https://api.twilio.com/2010-04-01/Accounts/{}/Calls/{}/Recordings.json",
        sid,
        call_sid
    );
    let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &list_url))?;
    if code != 200 {
        return Err(twilio_error(code, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let newest = v
        .get("recordings")
        .and_then(|r| r.as_array())
        .and_then(|a| a.last())
        .ok_or_else(|| {
            "this call has no recordings — recordings only exist when 'Record calls' was enabled before the call started".to_string()
        })?;
    let rec_sid = newest
        .get("sid")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let uri = newest
        .get("uri")
        .and_then(|u| u.as_str())
        .ok_or_else(|| "recording entry has no uri".to_string())?
        .replace(".json", ".wav");
    let wav_url = format!("https://api.twilio.com{}", uri);

    let bytes = tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| format!("HTTP client failed: {}", e))?;
        let resp = client
            .get(&wav_url)
            .basic_auth(&sid, Some(token))
            .send()
            .await
            .map_err(|e| format!("recording download failed: {}", e))?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(format!("recording download failed: HTTP {}", status));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        Ok::<Vec<u8>, String>(bytes.to_vec())
    })?;
    if bytes.len() < 44 || bytes.len() > 100 * 1024 * 1024 {
        return Err("recording is empty or implausibly large (max 100 MB)".to_string());
    }

    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("media")
        .join("calls");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("call-{}.wav", call_sid));
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok((rec_sid, path))
}

/// Newest call recording → offline transcription → memory. Local Whisper is
/// the preference (no cloud); cloud STT is only the fallback when the local
/// model/helper is missing.
pub fn transcribe_call(app: &tauri::AppHandle, call_sid: Option<&str>) -> Result<Value, String> {
    let call_sid = match call_sid.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => latest_call_sid(app)?,
    };
    let (rec_sid, path) = download_recording(app, &call_sid)?;

    let (text, engine) = match crate::localvoice::transcribe_wav_file(app, &path) {
        Ok(t) => (t, "whisper-local"),
        Err(local_err) => {
            let audio = path.to_string_lossy().to_string();
            match crate::media::transcribe(None, Some(&audio), None, None) {
                Ok(v) => (
                    v.get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                    "cloud",
                ),
                Err(cloud_err) => {
                    return Err(format!(
                        "local transcription failed ({}) and cloud transcription is unavailable ({})",
                        local_err, cloud_err
                    ))
                }
            }
        }
    };
    if text.trim().is_empty() {
        return Err("the recording produced an empty transcript".to_string());
    }

    let memory_key = format!("call-{}", call_sid);
    let metadata = json!({
        "source": "twilio-recording",
        "call_sid": call_sid,
        "recording_sid": rec_sid,
        "audio_path": path.to_string_lossy(),
        "engine": engine,
    });
    let stored = crate::memory::memory_put(
        app.clone(),
        "global".to_string(),
        "phone".to_string(),
        "twilio".to_string(),
        "v1".to_string(),
        None,
        memory_key.clone(),
        text.clone(),
        Some(metadata.to_string()),
        Some("verified_fact".to_string()),
    );

    Ok(json!({
        "call_sid": call_sid,
        "recording_sid": rec_sid,
        "transcript": text,
        "audio_path": path.to_string_lossy(),
        "engine": engine,
        "stored_in_memory": stored.is_ok(),
        "memory_key": memory_key,
        "memory_error": stored.err(),
    }))
}

/// Connections button + agent tool: transcribe the latest (or given) call.
#[tauri::command(rename_all = "snake_case")]
pub fn phone_call_transcribe(
    app: tauri::AppHandle,
    call_sid: Option<String>,
) -> Result<Value, String> {
    transcribe_call(&app, call_sid.as_deref())
}

/// Recent inbound text messages, newest first (for inbox triage).
pub fn messages_recent(app: &tauri::AppHandle, limit: u64) -> Result<Value, String> {
    let (sid, from_number) = load_config(app)?;
    let token = load_token()?;
    let n = limit.clamp(1, 100);
    let url = format!(
        "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json?PageSize={}",
        sid, n
    );
    let (code, text) = tauri::async_runtime::block_on(get_json(&sid, &token, &url))?;
    if code != 200 {
        return Err(twilio_error(code, &text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let items: Vec<Value> = v
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| {
                    // Inbound only: not from our own number, and direction says so.
                    let direction = m.get("direction").and_then(|d| d.as_str()).unwrap_or("");
                    let from = m.get("from").and_then(|f| f.as_str()).unwrap_or("");
                    direction.starts_with("inbound") && from != from_number
                })
                .map(|m| {
                    json!({
                        "sid": m.get("sid"),
                        "from": m.get("from"),
                        "body": m.get("body"),
                        "date": m.get("date_sent"),
                        "status": m.get("status"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({ "count": items.len(), "messages": items }))
}
