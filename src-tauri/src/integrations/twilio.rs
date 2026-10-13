// SPDX-License-Identifier: Apache-2.0
// twilio.rs — phone calls + SMS via a Twilio account (phone.* tools).
//
// Config lives in the settings store as `phone.twilio` (account_sid,
// from_number, label); the auth token lives ONLY in the OS keyring under
// `gfa-twilio:auth`. Every send/call costs real money and is approval-gated
// upstream — the tool descriptions say so.

use std::time::Instant;

use serde_json::{json, Value};

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

async fn post_form(
    sid: &str,
    token: &str,
    url: &str,
    form: Vec<(String, String)>,
) -> Result<(u16, String), String> {
    let resp = reqwest::Client::new()
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
    let resp = reqwest::Client::new()
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
