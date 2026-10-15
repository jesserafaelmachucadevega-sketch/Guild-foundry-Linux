// SPDX-License-Identifier: Apache-2.0
// inbox.rs — unified triage feed: unread email + inbound texts in one list.
//
// The data half of "unified inbox triage": a bounded, time-windowed digest
// from every configured source (the IMAP account, the Twilio number), with
// per-source health reported honestly — a source that is unconfigured or
// failing shows up in `sources` with its error instead of silently
// contributing nothing. The agent (or the user) decides what is urgent; no
// classification call is made behind the user's back.

use serde_json::{json, Value};

fn parse_when(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(s)
                .map(|d| d.with_timezone(&chrono::Utc))
                .ok()
        })
}

pub fn fetch(app: &tauri::AppHandle, hours: i64, include_sms: bool) -> Result<Value, String> {
    let hours = hours.clamp(1, 168);
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
    let mut items: Vec<Value> = Vec::new();
    let mut sources: Vec<Value> = Vec::new();

    match crate::integrations::email::list(app, Some("INBOX"), Some(30), true, None) {
        Ok(v) => {
            let mut kept = 0usize;
            if let Some(arr) = v.get("messages").and_then(|m| m.as_array()) {
                for m in arr {
                    let date = m.get("date").and_then(|d| d.as_str()).unwrap_or("");
                    let fresh = parse_when(date).map(|d| d >= cutoff).unwrap_or(true);
                    if !fresh {
                        continue;
                    }
                    kept += 1;
                    items.push(json!({
                        "source": "email",
                        "account": v.get("account").cloned().unwrap_or(Value::Null),
                        "from": m.get("from").cloned().unwrap_or(Value::Null),
                        "subject": m.get("subject").cloned().unwrap_or(Value::Null),
                        "date": date,
                        "preview": Value::Null,
                        "uid": m.get("uid").cloned().unwrap_or(Value::Null),
                    }));
                }
            }
            sources.push(json!({ "source": "email", "ok": true, "items": kept }));
        }
        Err(e) => sources.push(json!({ "source": "email", "ok": false, "error": e })),
    }

    if include_sms {
        match crate::integrations::twilio::messages_recent(app, 25) {
            Ok(v) => {
                let mut kept = 0usize;
                if let Some(arr) = v.get("messages").and_then(|m| m.as_array()) {
                    for m in arr {
                        let date = m.get("date").and_then(|d| d.as_str()).unwrap_or("");
                        let fresh = parse_when(date).map(|d| d >= cutoff).unwrap_or(true);
                        if !fresh {
                            continue;
                        }
                        kept += 1;
                        items.push(json!({
                            "source": "sms",
                            "account": "phone",
                            "from": m.get("from").cloned().unwrap_or(Value::Null),
                            "subject": Value::Null,
                            "date": date,
                            "preview": m.get("body").cloned().unwrap_or(Value::Null),
                            "sid": m.get("sid").cloned().unwrap_or(Value::Null),
                        }));
                    }
                }
                sources.push(json!({ "source": "sms", "ok": true, "items": kept }));
            }
            Err(e) => sources.push(json!({ "source": "sms", "ok": false, "error": e })),
        }
    }

    // Newest first, best-effort across mixed RFC3339 / RFC2822 timestamps.
    items.sort_by(|a, b| {
        let da = a.get("date").and_then(|d| d.as_str()).and_then(parse_when);
        let db = b.get("date").and_then(|d| d.as_str()).and_then(parse_when);
        db.cmp(&da)
    });

    Ok(json!({
        "hours": hours,
        "count": items.len(),
        "sources": sources,
        "items": items,
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn inbox_fetch(
    app: tauri::AppHandle,
    hours: Option<i64>,
    include_sms: Option<bool>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        fetch(&app, hours.unwrap_or(24), include_sms.unwrap_or(true))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::parse_when;

    #[test]
    fn parses_both_timestamp_dialects() {
        assert!(parse_when("2026-10-15T10:00:00Z").is_some());
        // RFC 2822 with the correct weekday (chrono validates it).
        assert!(parse_when("Thu, 15 Oct 2026 10:00:00 +0000").is_some());
        assert!(parse_when("not a date").is_none());
    }
}
