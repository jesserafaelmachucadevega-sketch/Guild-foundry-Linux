// SPDX-License-Identifier: Apache-2.0
// Provider request rate limiting + daily quota guard.
//
// OpenRouter's `:free` tier enforces 20 requests/minute and 50 requests/day —
// 1,000/day once the account has $10+ in lifetime credits. An agent loop can
// burn that budget in minutes, so every model request passes through here
// before hitting the network:
//
//   - Token bucket per provider: smooths bursts to `per_minute`, waiting
//     (bounded) for a token instead of firing into a 429.
//   - Daily counter per provider (SQLite, keyed by local date): hard stop at
//     `per_day` with a clear, actionable error instead of hammering the API.
//
// The guard applies ONLY to free models (OpenRouter `:free` suffix). Paid
// models run unrestricted — the user is paying per request, so no artificial
// cap. Local providers (Ollama etc.) are never throttled either.
//
// Limits are settings-overridable (store keys):
//   ratelimit.<provider_id>.per_minute  (default 20)
//   ratelimit.<provider_id>.per_day     (default 1000)
//   ratelimit.<provider_id>.enabled     (default true)

use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

fn open_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let conn =
        Connection::open(crate::secrets::db_path(app)?).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rate_usage (
            provider_id TEXT NOT NULL,
            day TEXT NOT NULL,
            count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (provider_id, day)
        );",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn get_setting<T: serde::de::DeserializeOwned>(app: &tauri::AppHandle, key: &str) -> Option<T> {
    use tauri_plugin_store::StoreExt;
    let store = app.store("settings.json").ok()?;
    let v = store.get(key)?;
    serde_json::from_value(v).ok()
}

struct Limits {
    per_minute: u32,
    per_day: u32,
    enabled: bool,
}

fn is_local(provider_id: &str) -> bool {
    let id = provider_id.to_lowercase();
    id.contains("ollama") || id.contains("local") || id.contains("llamacpp")
}

/// OpenRouter marks free-tier models with a `:free` suffix. Only those models
/// are subject to the platform's request quotas; paid models are billed per
/// request and run unrestricted.
fn is_free_model(model_id: &str) -> bool {
    model_id.to_lowercase().ends_with(":free")
}

fn load_limits(app: &tauri::AppHandle, provider_id: &str) -> Limits {
    // Only free models reach this point (see acquire), so the defaults match
    // OpenRouter's enhanced free tier: 20 req/min, 1,000 req/day.
    let key = |s: &str| format!("ratelimit.{}.{}", provider_id, s);
    Limits {
        per_minute: get_setting(app, &key("per_minute")).unwrap_or(20),
        per_day: get_setting(app, &key("per_day")).unwrap_or(1000),
        enabled: get_setting(app, &key("enabled")).unwrap_or(true),
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn daily_count(app: &tauri::AppHandle, provider_id: &str) -> Result<u32, String> {
    let conn = open_db(app)?;
    let day = today();
    let n: Option<u32> = conn
        .query_row(
            "SELECT count FROM rate_usage WHERE provider_id = ?1 AND day = ?2",
            rusqlite::params![provider_id, day],
            |r| r.get(0),
        )
        .ok();
    Ok(n.unwrap_or(0))
}

fn bump_daily(app: &tauri::AppHandle, provider_id: &str) -> Result<(), String> {
    let conn = open_db(app)?;
    let day = today();
    conn.execute(
        "INSERT INTO rate_usage (provider_id, day, count) VALUES (?1, ?2, 1)
         ON CONFLICT(provider_id, day) DO UPDATE SET count = count + 1",
        rusqlite::params![provider_id, day],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

static BUCKETS: LazyLock<Mutex<HashMap<String, Bucket>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Enforce rate + quota policy for one model request. Call before any network
/// call in the provider layer. On success the request may proceed; on error
/// the caller must surface the message to the user instead of retrying blindly.
///
/// The guard applies ONLY to free models (`:free` suffix). Paid models and
/// local providers bypass entirely.
pub async fn acquire(
    app: &tauri::AppHandle,
    provider_id: &str,
    model_id: &str,
) -> Result<(), String> {
    // Paid models run free; local providers never had quotas.
    if is_local(provider_id) || !is_free_model(model_id) {
        return Ok(());
    }
    let limits = load_limits(app, provider_id);
    if !limits.enabled {
        return Ok(());
    }

    // Daily quota: fail fast with an actionable message.
    let used = daily_count(app, provider_id)?;
    if used >= limits.per_day {
        return Err(format!(
            "Daily request quota exhausted for provider '{}' ({}/day). \
             The agent stopped instead of burning into rate-limit errors. \
             Raise it in settings (ratelimit.{}.per_day) or switch to a local model.",
            provider_id, limits.per_day, provider_id
        ));
    }

    // Token bucket: smooth bursts to per_minute, waiting boundedly for a token.
    let wait: Duration = {
        let mut map = BUCKETS.lock().map_err(|e| e.to_string())?;
        let bucket = map.entry(provider_id.to_string()).or_insert(Bucket {
            tokens: limits.per_minute as f64,
            last_refill: Instant::now(),
        });
        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * (limits.per_minute as f64) / 60.0)
            .min(limits.per_minute as f64);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Duration::ZERO
        } else {
            let deficit = 1.0 - bucket.tokens;
            let secs = deficit * 60.0 / (limits.per_minute as f64);
            bucket.tokens = 0.0;
            Duration::from_secs_f64(secs)
        }
    };
    if !wait.is_zero() {
        // Bound the wait: if we'd sleep longer than 2 minutes something is
        // misconfigured — fail loudly instead of hanging the agent.
        if wait > Duration::from_secs(120) {
            return Err(format!(
                "Rate limiter would delay provider '{}' by {:?} — refusing to hang. \
                 Check ratelimit.{}.per_minute in settings.",
                provider_id, wait, provider_id
            ));
        }
        tokio::time::sleep(wait).await;
    }

    bump_daily(app, provider_id)?;
    Ok(())
}

/// Current usage snapshot for UI/diagnostics: (used_today, per_day).
#[tauri::command]
pub fn ratelimit_status(
    app: tauri::AppHandle,
    provider_id: String,
) -> Result<(u32, u32), String> {
    let limits = load_limits(&app, &provider_id);
    let used = daily_count(&app, &provider_id)?;
    Ok((used, limits.per_day))
}

/// Quota-awareness notice for the agent's system prompt. Returns `Some` only
/// when the model is a free-tier model subject to quotas, so the model itself
/// knows its budget and spends turns wisely. Paid and local models get `None`
/// — no notice, no constraint.
#[tauri::command]
pub fn ratelimit_quota_notice(
    app: tauri::AppHandle,
    provider_id: String,
    model_id: String,
) -> Result<Option<String>, String> {
    if is_local(&provider_id) || !is_free_model(&model_id) {
        return Ok(None);
    }
    let limits = load_limits(&app, &provider_id);
    if !limits.enabled {
        return Ok(None);
    }
    Ok(Some(format!(
        "QUOTA NOTICE: you are running on a FREE-tier model. Provider limits are \
         ~{pm} requests/minute and ~{pd} requests/day, shared across all sessions. \
         Every one of your turns counts as one request. Spend the budget wisely:\n\
         - Batch independent tool calls into as few turns as possible.\n\
         - Do not re-verify what a tool result already told you.\n\
         - Prefer decisive action over exploratory loops; check your session notes \
         before spending another request.\n\
         - If the harness reports quota exhaustion, stop and summarize for the \
         user instead of retrying.",
        pm = limits.per_minute,
        pd = limits.per_day,
    )))
}
