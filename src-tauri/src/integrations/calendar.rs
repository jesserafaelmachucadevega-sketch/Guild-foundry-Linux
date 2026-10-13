// SPDX-License-Identifier: Apache-2.0
// calendar.rs — native CalDAV calendars for the agent (calendar.*).
//
// Configuration lives in the settings store as `calendar.account`
// (base URL + username); the password lives ONLY in the OS keyring under
// `gfa-calendar:default`.
//
// CalDAV providers work natively (Nextcloud, Fastmail, Radicale, iCloud with
// an app password, and similar). Google/Outlook calendar accounts require
// OAuth — connect their MCP connectors from the Connections tab and use
// mcp.call_tool for those; this module says so plainly instead of pretending.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::AccountTest;

pub const SETTINGS_KEY: &str = "calendar.account";
pub const KEYRING_KEY: &str = "gfa-calendar:default";

pub fn setup_message() -> String {
    "No calendar account configured. Connect a CalDAV calendar under System & Settings → Tools & Registry → Connections (Calendar card), then retry. Google/Outlook accounts use their MCP connector instead.".to_string()
}

#[derive(Debug, Clone)]
pub struct CalendarAccount {
    pub label: String,
    pub base_url: String,
    pub user: String,
}

fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn load_account(app: &tauri::AppHandle) -> Result<CalendarAccount, String> {
    let v = crate::settings::settings_get(app.clone(), SETTINGS_KEY.to_string())
        .map_err(|e| e.to_string())?;
    let v = match v {
        Some(v) if !v.is_null() => v,
        _ => return Err(setup_message()),
    };
    let acc = CalendarAccount {
        label: field(&v, "label"),
        base_url: field(&v, "base_url"),
        user: field(&v, "user"),
    };
    if acc.base_url.is_empty() || acc.user.is_empty() {
        return Err(
            "Calendar account is incomplete (missing CalDAV URL or username). Fix it under Connections."
                .to_string(),
        );
    }
    Ok(acc)
}

fn load_password() -> Result<String, String> {
    crate::secrets::keyring_get(KEYRING_KEY).ok_or_else(|| {
        "Calendar password missing — save it again in Connections (stored in your OS keychain)."
            .to_string()
    })
}

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("guild-foundry-ai/0.1.0")
            .build()
            .expect("failed to build calendar HTTP client")
    })
}

fn collection_url(base: &str) -> String {
    if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{}/", base)
    }
}

async fn dav_request(
    acc: &CalendarAccount,
    pass: &str,
    method: reqwest::Method,
    url: &str,
    depth: Option<&str>,
    body: Option<&str>,
    content_type: Option<&str>,
) -> Result<(u16, String), String> {
    let mut req = http()
        .request(method, url)
        .basic_auth(acc.user.clone(), Some(pass.to_string()));
    if let Some(d) = depth {
        req = req.header("Depth", d);
    }
    if let Some(ct) = content_type {
        req = req.header("Content-Type", ct);
    }
    if let Some(b) = body {
        req = req.body(b.to_string());
    }
    let resp = req.send().await.map_err(|e| {
        format!(
            "CalDAV request to {} failed before a response: {}",
            url, e
        )
    })?;
    let code = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((code, text))
}

fn check_status(code: u16, body: &str, ok: &[u16]) -> Result<(), String> {
    if ok.contains(&code) {
        return Ok(());
    }
    let snippet: String = body.chars().take(200).collect();
    match code {
        401 | 403 => Err("CalDAV rejected the credentials (401/403). Use your full username and an app password where required, then retry.".to_string()),
        404 => Err(format!("CalDAV URL not found (404). Check the collection URL. {}", snippet)),
        s if s >= 500 => Err(format!("CalDAV server error ({}): {}", s, snippet)),
        s => Err(format!("CalDAV returned HTTP {}: {}", s, snippet)),
    }
}

// ---------------------------------------------------------------------------
// ICS parsing (hand-rolled, unfold + property scan)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct IcsEvent {
    uid: String,
    summary: String,
    description: String,
    location: String,
    start: String,
    end: String,
    rrule: String,
    href: String,
}

fn unfold_ics(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.replace("\r\n", "\n").split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !lines.is_empty() {
            let last = lines.last_mut().unwrap();
            last.push_str(&line[1..]);
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn prop(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let name = line[..colon].split(';').next()?;
    Some((name, &line[colon + 1..]))
}

fn parse_events(ics: &str) -> Vec<IcsEvent> {
    let mut events = Vec::new();
    let mut cur: Option<IcsEvent> = None;
    for line in unfold_ics(ics) {
        match line.as_str() {
            "BEGIN:VEVENT" => {
                cur = Some(IcsEvent::default());
                continue;
            }
            "END:VEVENT" => {
                if let Some(c) = cur.take() {
                    events.push(c);
                }
                continue;
            }
            _ => {}
        }
        if let Some(c) = cur.as_mut() {
            if let Some((name, value)) = prop(&line) {
                match name {
                    "UID" => c.uid = value.trim().to_string(),
                    "SUMMARY" => c.summary = unescape_text(value),
                    "DESCRIPTION" => c.description = unescape_text(value),
                    "LOCATION" => c.location = unescape_text(value),
                    "DTSTART" => c.start = value.trim().to_string(),
                    "DTEND" => c.end = value.trim().to_string(),
                    "RRULE" => c.rrule = value.trim().to_string(),
                    _ => {}
                }
            }
        }
    }
    events
}

fn regexes() -> &'static (regex::Regex, regex::Regex, regex::Regex) {
    static R: OnceLock<(regex::Regex, regex::Regex, regex::Regex)> = OnceLock::new();
    R.get_or_init(|| {
        (
            regex::Regex::new(r"(?is)<(?:[A-Za-z]+:)?calendar-data[^>]*>(.*?)</(?:[A-Za-z]+:)?calendar-data\s*>").unwrap(),
            regex::Regex::new(r"(?is)<(?:[A-Za-z]+:)?href[^>]*>(.*?)</(?:[A-Za-z]+:)?href\s*>").unwrap(),
            regex::Regex::new(r"&#(?:13|10);").unwrap(),
        )
    })
}

fn xml_unescape(s: &str) -> String {
    let s = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    regexes().2.replace_all(&s, "\n").to_string()
}

/// Extract (href, ICS text) pairs from a CalDAV multistatus response.
fn parse_multistatus(body: &str) -> Vec<(String, String)> {
    let (ics_re, href_re, _) = regexes();
    let hrefs: Vec<String> = href_re
        .captures_iter(body)
        .filter_map(|c| c.get(1).map(|m| xml_unescape(m.as_str()).trim().to_string()))
        .collect();
    let ics: Vec<String> = ics_re
        .captures_iter(body)
        .filter_map(|c| c.get(1).map(|m| xml_unescape(m.as_str())))
        .collect();
    ics.into_iter()
        .enumerate()
        .map(|(i, data)| (hrefs.get(i).cloned().unwrap_or_default(), data))
        .collect()
}

// ---------------------------------------------------------------------------
// ICS building
// ---------------------------------------------------------------------------

fn ics_time(rfc3339: &str) -> Result<String, String> {
    let dt = chrono::DateTime::parse_from_rfc3339(rfc3339.trim())
        .map_err(|e| format!("'{}' is not an RFC3339 time (e.g. 2026-10-14T09:00:00-06:00): {}", rfc3339, e))?;
    Ok(dt
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string())
}

#[allow(clippy::too_many_arguments)]
fn build_ics(
    uid: &str,
    summary: &str,
    description: &str,
    location: &str,
    start_ics: &str,
    end_ics: &str,
    attendees: &[String],
    rrule: &str,
) -> String {
    let mut out = String::new();
    out.push_str("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Guild Foundry//Calendar//EN\r\nBEGIN:VEVENT\r\n");
    out.push_str(&format!("UID:{}\r\n", uid));
    out.push_str(&format!(
        "DTSTAMP:{}\r\n",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    out.push_str(&format!("SUMMARY:{}\r\n", escape_text(summary)));
    if !description.is_empty() {
        out.push_str(&format!("DESCRIPTION:{}\r\n", escape_text(description)));
    }
    if !location.is_empty() {
        out.push_str(&format!("LOCATION:{}\r\n", escape_text(location)));
    }
    out.push_str(&format!("DTSTART:{}\r\n", start_ics));
    out.push_str(&format!("DTEND:{}\r\n", end_ics));
    if !rrule.is_empty() {
        out.push_str(&format!("RRULE:{}\r\n", rrule));
    }
    for a in attendees {
        out.push_str(&format!(
            "ATTENDEE;CN={};PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{}\r\n",
            escape_text(a),
            a
        ));
    }
    out.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    out
}

fn event_json(e: &IcsEvent) -> Value {
    let (desc, truncated) = crate::integrations::document::cap_text(&e.description, 2_000);
    json!({
        "uid": e.uid,
        "href": e.href,
        "summary": e.summary,
        "start": e.start,
        "end": e.end,
        "location": e.location,
        "description": desc,
        "description_truncated": truncated,
        "recurrence": if e.rrule.is_empty() { Value::Null } else { json!(e.rrule) },
    })
}

/// calendar.list_events — REPORT calendar-query over the configured range.
pub fn list_events(
    app: &tauri::AppHandle,
    time_min: Option<&str>,
    time_max: Option<&str>,
    max_results: Option<u64>,
) -> Result<Value, String> {
    let acc = load_account(app)?;
    let pass = load_password()?;
    let n = max_results.unwrap_or(50).clamp(1, 100) as usize;
    let start = match time_min {
        Some(t) if !t.trim().is_empty() => ics_time(t)?,
        _ => chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
    };
    let end = match time_max {
        Some(t) if !t.trim().is_empty() => ics_time(t)?,
        _ => (chrono::Utc::now() + chrono::Duration::days(30))
            .format("%Y%m%dT%H%M%SZ")
            .to_string(),
    };
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" ?>\
         <C:calendar-query xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\">\
         <D:prop><D:getetag/><C:calendar-data/></D:prop>\
         <C:filter><C:comp-filter name=\"VCALENDAR\"><C:comp-filter name=\"VEVENT\">\
         <C:time-range start=\"{}\" end=\"{}\"/>\
         </C:comp-filter></C:comp-filter></C:filter></C:calendar-query>",
        start, end
    );
    let url = collection_url(&acc.base_url);
    let (code, resp) = tauri::async_runtime::block_on(dav_request(
        &acc,
        &pass,
        reqwest::Method::from_bytes(b"REPORT").map_err(|e| e.to_string())?,
        &url,
        Some("1"),
        Some(&body),
        Some("application/xml; charset=utf-8"),
    ))?;
    check_status(code, &resp, &[207])?;

    let mut events: Vec<Value> = Vec::new();
    for (href, ics) in parse_multistatus(&resp) {
        for mut e in parse_events(&ics) {
            e.href = href.clone();
            events.push(event_json(&e));
            if events.len() >= n {
                break;
            }
        }
        if events.len() >= n {
            break;
        }
    }
    Ok(json!({
        "account": acc.label,
        "time_min": start,
        "time_max": end,
        "count": events.len(),
        "events": events,
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn create_event(
    app: &tauri::AppHandle,
    summary: &str,
    start_time: &str,
    end_time: &str,
    description: Option<&str>,
    location: Option<&str>,
    attendees: Vec<String>,
    recurrence: Option<&str>,
) -> Result<Value, String> {
    let acc = load_account(app)?;
    let pass = load_password()?;
    let uid = format!("{}@guild-foundry", uuid::Uuid::new_v4());
    let ics = build_ics(
        &uid,
        summary,
        description.unwrap_or(""),
        location.unwrap_or(""),
        &ics_time(start_time)?,
        &ics_time(end_time)?,
        &attendees,
        recurrence.unwrap_or(""),
    );
    let url = format!("{}{}.ics", collection_url(&acc.base_url), uid);
    let (code, resp) = tauri::async_runtime::block_on(dav_request_raw(
        &acc,
        &pass,
        reqwest::Method::PUT,
        &url,
        Some(&ics),
        Some("text/calendar; charset=utf-8"),
        Some(("If-None-Match", "*")),
    ))?;
    check_status(code, &resp, &[200, 201, 204])?;
    Ok(json!({ "created": true, "uid": uid, "href": url, "summary": summary, "start": start_time, "end": end_time }))
}

#[allow(clippy::too_many_arguments)]
pub fn update_event(
    app: &tauri::AppHandle,
    event_id: &str,
    summary: Option<&str>,
    description: Option<&str>,
    start_time: Option<&str>,
    end_time: Option<&str>,
) -> Result<Value, String> {
    let acc = load_account(app)?;
    let pass = load_password()?;
    let href = resolve_href(&acc, event_id);
    let (get_code, existing) = tauri::async_runtime::block_on(dav_request(
        &acc,
        &pass,
        reqwest::Method::GET,
        &href,
        None,
        None,
        None,
    ))?;
    check_status(get_code, &existing, &[200])?;
    let cur = parse_events(&existing)
        .into_iter()
        .next()
        .ok_or_else(|| format!("no VEVENT found at {}", href))?;
    let new_ics = build_ics(
        if cur.uid.is_empty() { event_id } else { &cur.uid },
        summary.unwrap_or(&cur.summary),
        description.unwrap_or(&cur.description),
        &cur.location,
        &match start_time {
            Some(t) => ics_time(t)?,
            None => cur.start.clone(),
        },
        &match end_time {
            Some(t) => ics_time(t)?,
            None => cur.end.clone(),
        },
        &[],
        &cur.rrule,
    );
    let (code, resp) = tauri::async_runtime::block_on(dav_request_raw(
        &acc,
        &pass,
        reqwest::Method::PUT,
        &href,
        Some(&new_ics),
        Some("text/calendar; charset=utf-8"),
        Some(("If-Match", "*")),
    ))?;
    check_status(code, &resp, &[200, 201, 204])?;
    Ok(json!({ "updated": true, "href": href }))
}

pub fn delete_event(app: &tauri::AppHandle, event_id: &str) -> Result<Value, String> {
    let acc = load_account(app)?;
    let pass = load_password()?;
    let href = resolve_href(&acc, event_id);
    let (code, resp) = tauri::async_runtime::block_on(dav_request(
        &acc,
        &pass,
        reqwest::Method::DELETE,
        &href,
        None,
        None,
        None,
    ))?;
    check_status(code, &resp, &[200, 204, 404])?;
    Ok(json!({ "deleted": true, "href": href }))
}

fn resolve_href(acc: &CalendarAccount, event_id: &str) -> String {
    if event_id.starts_with("http://") || event_id.starts_with("https://") {
        event_id.to_string()
    } else {
        format!("{}{}.ics", collection_url(&acc.base_url), event_id.trim_end_matches(".ics"))
    }
}

#[allow(clippy::too_many_arguments)]
async fn dav_request_raw(
    acc: &CalendarAccount,
    pass: &str,
    method: reqwest::Method,
    url: &str,
    body: Option<&str>,
    content_type: Option<&str>,
    extra_header: Option<(&str, &str)>,
) -> Result<(u16, String), String> {
    let mut req = http()
        .request(method, url)
        .basic_auth(acc.user.clone(), Some(pass.to_string()));
    if let Some(ct) = content_type {
        req = req.header("Content-Type", ct);
    }
    if let Some((k, v)) = extra_header {
        req = req.header(k, v);
    }
    if let Some(b) = body {
        req = req.body(b.to_string());
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("CalDAV request to {} failed: {}", url, e))?;
    let code = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    Ok((code, text))
}

/// Connections "Test" button — verifies CalDAV sign-in on the collection URL.
#[tauri::command]
pub fn calendar_test_connection(app: tauri::AppHandle) -> Result<AccountTest, String> {
    let started = Instant::now();
    let result = tauri::async_runtime::block_on(async {
        let acc = load_account(&app)?;
        let pass = load_password()?;
        let url = collection_url(&acc.base_url);
        let body = "<D:propfind xmlns:D=\"DAV:\"><D:prop><D:resourcetype/></D:prop></D:propfind>";
        let (code, resp) = dav_request(
            &acc,
            &pass,
            reqwest::Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?,
            &url,
            Some("0"),
            Some(body),
            Some("application/xml; charset=utf-8"),
        )
        .await?;
        check_status(code, &resp, &[207])?;
        Ok::<String, String>(format!("CalDAV reachable at {} as {}", acc.base_url, acc.user))
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolds_and_parses_events() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:abc@example\r\nSUMMARY:Team \r\n standup\r\nDTSTART:20261014T150000Z\r\nDTEND:20261014T153000Z\r\nDESCRIPTION:Line one\\nLine two\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse_events(ics);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, "abc@example");
        assert_eq!(events[0].summary, "Team standup");
        assert_eq!(events[0].start, "20261014T150000Z");
        assert!(events[0].description.contains("Line two"));
    }

    #[test]
    fn builds_valid_ics() {
        let ics = build_ics(
            "uid-1",
            "Dentist, cleaning; bring form",
            "note",
            "",
            "20261014T150000Z",
            "20261014T160000Z",
            &["a@example.com".to_string()],
            "",
        );
        assert!(ics.contains("UID:uid-1"));
        assert!(ics.contains("SUMMARY:Dentist\\, cleaning\\; bring form"));
        assert!(ics.contains("ATTENDEE;CN=a@example.com"));
        assert!(ics.starts_with("BEGIN:VCALENDAR"));
        assert!(ics.trim_end().ends_with("END:VCALENDAR"));
    }

    #[test]
    fn rfc3339_to_ics_utc() {
        assert_eq!(ics_time("2026-10-14T09:00:00-06:00").unwrap(), "20261014T150000Z");
        assert!(ics_time("not-a-time").is_err());
    }
}
