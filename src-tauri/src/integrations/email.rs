// SPDX-License-Identifier: Apache-2.0
// email.rs — native IMAP/SMTP email for the agent (email.list / read / send).
//
// Account configuration lives in the settings store as `email.account`
// (hosts/ports/user — never the password). The password lives ONLY in the OS
// keyring under `gfa-email:default`. Presets are provided in the Connections
// UI (Gmail app passwords, Outlook, custom IMAP).

use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use imap::Session;
use imap_proto::types::Address;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::{json, Value};

use super::AccountTest;

/// Implicit-TLS stream used for IMAP (pure Rust — no OpenSSL anywhere).
type TlsStream = StreamOwned<ClientConnection, TcpStream>;

pub const SETTINGS_KEY: &str = "email.account";
pub const KEYRING_KEY: &str = "gfa-email:default";

const READ_CAP: usize = 100_000; // chars returned by email.read

pub fn setup_message() -> String {
    "No email account configured. Connect one under System & Settings → Tools & Registry → Connections (Email card), then retry.".to_string()
}

#[derive(Debug, Clone)]
pub struct EmailAccount {
    pub label: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub user: String,
    pub from_name: String,
    pub security: String, // "tls" (implicit, port 993) or "starttls"
}

fn field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn port_of(v: &Value, key: &str, default: u16) -> u16 {
    v.get(key)
        .and_then(|x| x.as_u64())
        .map(|n| n.clamp(1, 65535) as u16)
        .unwrap_or(default)
}

pub fn load_account(app: &tauri::AppHandle) -> Result<EmailAccount, String> {
    let v = crate::settings::settings_get(app.clone(), SETTINGS_KEY.to_string())
        .map_err(|e| e.to_string())?;
    let v = match v {
        Some(v) if !v.is_null() => v,
        _ => return Err(setup_message()),
    };
    let acc = EmailAccount {
        label: field(&v, "label"),
        imap_host: field(&v, "imap_host"),
        imap_port: port_of(&v, "imap_port", 993),
        smtp_host: field(&v, "smtp_host"),
        smtp_port: port_of(&v, "smtp_port", 587),
        user: field(&v, "user"),
        from_name: field(&v, "from_name"),
        security: {
            let s = field(&v, "security");
            if s.is_empty() {
                "tls".to_string()
            } else {
                s
            }
        },
    };
    if acc.imap_host.is_empty() || acc.user.is_empty() {
        return Err(
            "Email account is incomplete (missing IMAP host or username). Fix it under Connections."
                .to_string(),
        );
    }
    Ok(acc)
}

fn load_password() -> Result<String, String> {
    crate::secrets::keyring_get(KEYRING_KEY).ok_or_else(|| {
        "Email password missing — save it again in Connections (it is stored in your OS keychain, never in the app database)."
            .to_string()
    })
}

fn rustls_config() -> &'static Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    })
}

fn with_session<T>(
    acc: &EmailAccount,
    f: impl FnOnce(&mut Session<TlsStream>) -> Result<T, String>,
) -> Result<T, String> {
    let pass = load_password()?;
    if acc.security == "starttls" {
        return Err(
            "STARTTLS is not supported for IMAP yet — use the implicit-TLS port instead (usually 993)."
                .to_string(),
        );
    }
    let server_name = ServerName::try_from(acc.imap_host.clone())
        .map_err(|e| format!("bad IMAP host name '{}': {}", acc.imap_host, e))?;
    let tcp = TcpStream::connect((acc.imap_host.as_str(), acc.imap_port)).map_err(|e| {
        format!(
            "IMAP connect to {}:{} failed: {}",
            acc.imap_host, acc.imap_port, e
        )
    })?;
    // Bound every socket operation. Without these, one stalled IMAP server
    // hangs the tool call and pins its blocking-pool thread indefinitely.
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .map_err(|e| format!("failed to set IMAP read timeout: {}", e))?;
    tcp.set_write_timeout(Some(std::time::Duration::from_secs(30)))
        .map_err(|e| format!("failed to set IMAP write timeout: {}", e))?;
    let conn = ClientConnection::new(rustls_config().clone(), server_name)
        .map_err(|e| format!("TLS init failed: {}", e))?;
    let stream = StreamOwned::new(conn, tcp);
    let client = imap::Client::new(stream);
    let mut session = client.login(&acc.user, &pass).map_err(|(e, _client)| {
        let msg = e.to_string();
        let lower = msg.to_lowercase();
        if lower.contains("authentication") || lower.contains("login") || lower.contains("credentials") {
            format!(
                "IMAP login rejected for {} — for most providers use an app password, not your main password.",
                acc.user
            )
        } else {
            format!("IMAP login failed: {}", msg)
        }
    })?;
    let result = f(&mut session);
    let _ = session.logout();
    result
}

fn address_string(a: &Address<'_>) -> String {
    let mailbox = a
        .mailbox
        .as_ref()
        .map(|m| String::from_utf8_lossy(m).to_string())
        .unwrap_or_default();
    let host = a
        .host
        .as_ref()
        .map(|h| String::from_utf8_lossy(h).to_string())
        .unwrap_or_default();
    if mailbox.is_empty() && host.is_empty() {
        String::new()
    } else {
        format!("{}@{}", mailbox, host)
    }
}

/// email.list — search + list message summaries (newest first).
pub fn list(
    app: &tauri::AppHandle,
    folder: Option<&str>,
    max_results: Option<u64>,
    unread_only: bool,
    query: Option<&str>,
) -> Result<Value, String> {
    let acc = load_account(app)?;
    let n = max_results.unwrap_or(20).clamp(1, 100) as usize;
    let folder = folder.unwrap_or("INBOX");
    with_session(&acc, |s| {
        s.select(folder)
            .map_err(|e| format!("select {} failed: {}", folder, e))?;
        let search = match query {
            Some(q) if !q.trim().is_empty() => q.trim().to_string(),
            _ => {
                if unread_only {
                    "UNSEEN".to_string()
                } else {
                    "ALL".to_string()
                }
            }
        };
        let found = s
            .uid_search(&search)
            .map_err(|e| format!("search failed (must be a raw IMAP query): {}", e))?;
        let mut uids: Vec<u32> = found.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(n);
        if uids.is_empty() {
            return Ok(json!({ "folder": folder, "account": acc.label, "count": 0, "messages": [] }));
        }
        let set = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let fetches = s
            .uid_fetch(&set, "(UID ENVELOPE FLAGS RFC822.SIZE INTERNALDATE)")
            .map_err(|e| format!("fetch failed: {}", e))?;
        let mut messages = Vec::new();
        for f in fetches.iter() {
            let env = f.envelope();
            let subject = env
                .and_then(|e| e.subject.as_ref())
                .map(|s| String::from_utf8_lossy(s).to_string())
                .unwrap_or_default();
            let from = env
                .and_then(|e| e.from.as_ref())
                .and_then(|list| list.first())
                .map(address_string)
                .unwrap_or_default();
            let date = f
                .internal_date()
                .map(|d| d.to_rfc3339())
                .or_else(|| {
                    env.and_then(|e| e.date.as_ref())
                        .map(|d| String::from_utf8_lossy(d).to_string())
                })
                .unwrap_or_default();
            let seen = f
                .flags()
                .iter()
                .any(|fl| matches!(fl, imap::types::Flag::Seen));
            messages.push(json!({
                "uid": f.uid.unwrap_or(0),
                "subject": subject,
                "from": from,
                "date": date,
                "seen": seen,
                "size": f.size,
            }));
        }
        Ok(json!({ "folder": folder, "account": acc.label, "count": messages.len(), "messages": messages }))
    })
}

fn raw_header(raw: &str, name: &str) -> Option<String> {
    let head = raw.split("\r\n\r\n").next().unwrap_or(raw);
    let unfolded = head.replace("\r\n ", " ").replace("\r\n\t", " ");
    for line in unfolded.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn parse_message(raw: &[u8]) -> Result<Value, String> {
    let parsed = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or_else(|| "could not parse the message".to_string())?;
    let subject = parsed.subject().unwrap_or("").to_string();

    let mut text = String::new();
    for i in 0..parsed.text_body_count() {
        if let Some(t) = parsed.body_text(i) {
            text.push_str(t.as_ref());
            text.push_str("\n\n");
        }
    }
    if text.trim().is_empty() {
        for i in 0..parsed.html_body_count() {
            if let Some(h) = parsed.body_html(i) {
                text.push_str(&crate::integrations::web::html_to_text(h.as_ref()));
                text.push('\n');
            }
        }
    }
    let raw_str = String::from_utf8_lossy(raw);
    let (capped, truncated) = crate::integrations::document::cap_text(&text, READ_CAP);
    Ok(json!({
        "subject": subject,
        "from": raw_header(&raw_str, "From").unwrap_or_default(),
        "to": raw_header(&raw_str, "To").unwrap_or_default(),
        "cc": raw_header(&raw_str, "Cc"),
        "date": raw_header(&raw_str, "Date").unwrap_or_default(),
        "attachments": parsed.attachment_count(),
        "chars": text.len(),
        "truncated": truncated,
        "text": capped,
    }))
}

/// email.read — full message by uid.
pub fn read(app: &tauri::AppHandle, uid: u32, folder: Option<&str>) -> Result<Value, String> {
    let acc = load_account(app)?;
    let folder = folder.unwrap_or("INBOX");
    with_session(&acc, |s| {
        s.select(folder)
            .map_err(|e| format!("select {} failed: {}", folder, e))?;
        let fetches = s
            .uid_fetch(uid.to_string(), "(RFC822)")
            .map_err(|e| format!("fetch failed: {}", e))?;
        let raw = fetches
            .iter()
            .next()
            .and_then(|f| f.body())
            .ok_or_else(|| format!("message {} not found in {}", uid, folder))?;
        parse_message(raw)
    })
}

/// email.send — SMTP send with text/html bodies and optional attachments.
pub fn send(
    app: &tauri::AppHandle,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: &str,
    body_text: Option<&str>,
    body_html: Option<&str>,
    attachments: Vec<Value>,
) -> Result<Value, String> {
    use base64::Engine as _;
    use lettre::message::header::ContentType;
    use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{Message, SmtpTransport, Transport};

    let acc = load_account(app)?;
    let pass = load_password()?;
    if acc.smtp_host.is_empty() {
        return Err("Email account has no SMTP host. Add it under Connections.".to_string());
    }

    let from_name = if acc.from_name.is_empty() {
        acc.user.clone()
    } else {
        acc.from_name.clone()
    };
    let from_mailbox: Mailbox = format!("{} <{}>", from_name, acc.user)
        .parse()
        .map_err(|e| format!("bad from address: {}", e))?;

    let mut builder = Message::builder().from(from_mailbox).subject(subject);
    for t in &to {
        builder = builder.to(t.parse::<Mailbox>().map_err(|e| format!("bad recipient '{}': {}", t, e))?);
    }
    for c in &cc {
        builder = builder.cc(c.parse::<Mailbox>().map_err(|e| format!("bad cc '{}': {}", c, e))?);
    }
    for b in &bcc {
        builder = builder.bcc(b.parse::<Mailbox>().map_err(|e| format!("bad bcc '{}': {}", b, e))?);
    }

    let text_part = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(body_text.unwrap_or("").to_string());
    let alternative = match body_html {
        Some(h) => MultiPart::alternative().singlepart(text_part).singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(h.to_string()),
        ),
        None => MultiPart::alternative().singlepart(text_part),
    };

    let mut mixed = MultiPart::mixed().multipart(alternative);
    for att in &attachments {
        let name = att
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("attachment")
            .to_string();
        let b64 = att
            .get("content_base64")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if b64.is_empty() {
            return Err(format!("attachment '{}' is missing content_base64", name));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| format!("attachment '{}' has bad base64: {}", name, e))?;
        let mime: ContentType = att
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream")
            .parse()
            .unwrap_or(ContentType::TEXT_PLAIN);
        mixed = mixed.singlepart(Attachment::new(name).body(bytes, mime));
    }

    let email = builder
        .multipart(mixed)
        .map_err(|e| format!("could not build the email: {}", e))?;

    let transport = SmtpTransport::relay(&acc.smtp_host)
        .map_err(|e| format!("SMTP setup failed for {}: {}", acc.smtp_host, e))?
        .port(acc.smtp_port)
        .credentials(Credentials::new(acc.user.clone(), pass))
        .build();
    let receipt = transport
        .send(&email)
        .map_err(|e| format!("SMTP send failed: {}", e))?;

    Ok(json!({
        "sent": true,
        "to": to,
        "cc": cc,
        "bcc": bcc,
        "subject": subject,
        "server_response": receipt.message().collect::<Vec<_>>().join(" | "),
    }))
}

/// Connections "Test" button — verifies IMAP sign-in and INBOX access.
#[tauri::command]
pub fn email_test_connection(app: tauri::AppHandle) -> Result<AccountTest, String> {
    let started = Instant::now();
    let result = tauri::async_runtime::block_on(async {
        let acc = load_account(&app)?;
        with_session(&acc, |s| {
            s.select("INBOX")
                .map_err(|e| format!("INBOX not accessible: {}", e))?;
            Ok(())
        })?;
        Ok::<String, String>(format!("Signed in as {} ({})", acc.user, acc.imap_host))
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
