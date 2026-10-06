// Phase 11 — Update system.
//
// Master spec section 71: check for updates / view release notes / download /
// install / postpone. NEVER silently installs an update without explicit user
// confirmation. Every code path returns an explicit, user-facing result —
// "up-to-date", "no feed configured", "unreachable" — never an ambiguous
// failure.
//
// Design: the update feed is a GitHub repo slug stored in settings.json under
// the key "update_repo" (e.g. "owner/repo"). The default is empty, which means
// updates are disabled until the operator configures a feed — honest rather
// than guessing. Postponed versions are recorded in the same store under
// "update_postponed_version" + "update_postponed_at".
//
// NOT VERIFIED: no Rust toolchain in this environment, so none of this has
// compiled. Needs `cargo check` on the Zorin OS target before shipping.
// NOT VERIFIED: real download+install has never run against a signed feed; the
// download step below deliberately stops at "downloaded for review" and NEVER
// executes an installer — the final install command requires explicit user
// confirmation from the UI AND must run natively.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_store::StoreExt;

const SETTINGS_STORE: &str = "settings.json";
const REPO_KEY: &str = "update_repo";
const POSTPONED_VERSION_KEY: &str = "update_postponed_version";
const POSTPONED_AT_KEY: &str = "update_postponed_at";
const FEED_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize, Debug)]
pub struct UpdateCheckResult {
    pub available: bool,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub reason: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct PostponeResult {
    pub postponed_version: String,
    pub postponed_at: String,
}

#[derive(Serialize, Debug)]
pub struct DownloadResult {
    /// Always "awaiting-user-confirmation": this function never installs.
    pub status: String,
    pub version: String,
    pub asset_name: String,
    pub downloaded_to: String,
    pub size_bytes: u64,
    pub note: String,
}

#[derive(Deserialize, Debug, Clone)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: Option<String>,
    assets: Option<Vec<GithubAsset>>,
}

#[derive(Deserialize, Debug, Clone)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: Option<u64>,
}

/// Internal settings read (same store Phase 1's settings.rs uses).
fn store_get(app: &tauri::AppHandle, key: &str) -> Result<Option<serde_json::Value>, String> {
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    Ok(store.get(key))
}

fn update_repo(app: &tauri::AppHandle) -> Result<Option<String>, String> {
    match store_get(app, REPO_KEY)? {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        _ => Ok(None),
    }
}

/// Best-effort semver compare on the numeric segments ("1.2.10" > "1.2.9").
/// Returns None when either side is unparseable (caller then treats it as
/// "no usable version info").
fn is_newer(remote: &str, current: &str) -> Option<bool> {
    let parse = |v: &str| -> Option<Vec<u64>> {
        v.trim()
            .trim_start_matches('v')
            .split(|c: char| c == '.' || c == '-')
            .map(|seg| seg.parse::<u64>().ok())
            .collect::<Option<Vec<u64>>>()
    };
    let (r, c) = (parse(remote)?, parse(current)?);
    let len = r.len().max(c.len());
    for i in 0..len {
        let (a, b) = (
            *r.get(i).unwrap_or(&0),
            *c.get(i).unwrap_or(&0),
        );
        if a != b {
            return Some(a > b);
        }
    }
    Some(false)
}

async fn latest_release(repo: &str) -> Result<GithubRelease, String> {
    let client = reqwest::Client::builder()
        .timeout(FEED_TIMEOUT)
        .user_agent("guild-foundry-ai-updater")
        .build()
        .map_err(|e| format!("http client error: {}", e))?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "update feed unreachable: request timed out".to_string()
            } else if e.is_connect() {
                "update feed unreachable: connection failed (offline?)".to_string()
            } else {
                format!("update feed unreachable: {}", e)
            }
        })?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("no releases found for the configured repo (404)".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("update feed returned HTTP {}", resp.status()));
    }
    resp.json::<GithubRelease>()
        .await
        .map_err(|e| format!("malformed release payload: {}", e))
}

/// Check for updates. Never throws an ambiguous error: every outcome is a
/// plain-language reason the UI can show directly.
#[tauri::command]
pub async fn updater_check(app: tauri::AppHandle) -> Result<UpdateCheckResult, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let repo = match update_repo(&app)? {
        Some(r) => r,
        None => {
            return Ok(UpdateCheckResult {
                available: false,
                current_version: current,
                version: None,
                notes: None,
                url: None,
                reason: Some(
                    "no-update-feed-configured: set update_repo in Settings (e.g. \"owner/repo\")"
                        .to_string(),
                ),
            })
        }
    };
    let release = match latest_release(&repo).await {
        Ok(r) => r,
        Err(reason) => {
            return Ok(UpdateCheckResult {
                available: false,
                current_version: current,
                version: None,
                notes: None,
                url: None,
                reason: Some(reason),
            })
        }
    };
    // A postponed version is treated as "not available" so the user is not
    // nagged again; the UI shows "postponed" state from this reason string.
    if let Some(serde_json::Value::String(pv)) = store_get(&app, POSTPONED_VERSION_KEY)? {
        if pv == release.tag_name {
            return Ok(UpdateCheckResult {
                available: false,
                current_version: current,
                version: Some(release.tag_name),
                notes: release.body,
                url: release.html_url,
                reason: Some(format!("update {} postponed by user", pv)),
            });
        }
    }
    let available = is_newer(&release.tag_name, &current).unwrap_or(false);
    Ok(UpdateCheckResult {
        available,
        current_version: current,
        version: Some(release.tag_name),
        notes: release.body,
        url: release.html_url,
        reason: if available {
            None
        } else {
            Some("up-to-date".to_string())
        },
    })
}

/// Record that the user postponed `version`. Returns the recorded stamp.
#[tauri::command]
pub fn updater_postpone(app: tauri::AppHandle, version: String) -> Result<PostponeResult, String> {
    if version.trim().is_empty() {
        return Err("cannot postpone: empty version".to_string());
    }
    let at = chrono::Utc::now().to_rfc3339();
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    store.set(POSTPONED_VERSION_KEY, serde_json::json!(version));
    store.set(POSTPONED_AT_KEY, serde_json::json!(at.clone()));
    store.save().map_err(|e| e.to_string())?;
    Ok(PostponeResult {
        postponed_version: version,
        postponed_at: at,
    })
}

/// Download the release asset and stage it for review. This function NEVER
/// executes the installer: it returns a user-facing confirmation payload so
/// the UI can ask the user for explicit approval before anything runs.
/// NOT VERIFIED: needs a real feed with signed artifacts on the target OS.
#[tauri::command]
pub async fn updater_download_install(
    app: tauri::AppHandle,
) -> Result<DownloadResult, String> {
    let repo = update_repo(&app)?
        .ok_or_else(|| "no-update-feed-configured: set update_repo in Settings first".to_string())?;
    let release = latest_release(&repo).await.map_err(|e| e.to_string())?;
    let assets = release.assets.unwrap_or_default();
    // Prefer .deb, then .AppImage — the formats this build ships.
    let pick = |suffix: &str| {
        assets
            .iter()
            .find(|a| a.name.to_lowercase().ends_with(suffix))
    };
    let asset = pick(".deb")
        .or_else(|| pick(".appimage"))
        .cloned()
        .ok_or_else(|| {
            format!(
                "release {} has no .deb or .AppImage asset to download",
                release.tag_name
            )
        })?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("guild-foundry-ai-updater")
        .build()
        .map_err(|e| format!("http client error: {}", e))?;
    let resp = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "download failed: request timed out".to_string()
            } else {
                format!("download failed: {}", e)
            }
        })?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("download failed reading body: {}", e))?;

    // Sanity check: refuse to stage tiny/empty payloads that cannot be a
    // real package (a minimal .deb is well over 100 KB).
    if (bytes.len() as u64) < 100_000 {
        return Err(format!(
            "downloaded asset {} is suspiciously small ({} bytes) — refusing to stage it",
            asset.name,
            bytes.len()
        ));
    }

    let dest = std::env::temp_dir().join(format!("guild-foundry-update-{}", asset.name));
    std::fs::write(&dest, &bytes).map_err(|e| format!("cannot stage update file: {}", e))?;
    let size = bytes.len() as u64;

    // Deliberately NOT executed here. The frontend shows this payload and
    // requires an explicit, separate user confirmation before any install
    // command is ever issued by the native side.
    Ok(DownloadResult {
        status: "awaiting-user-confirmation".to_string(),
        version: release.tag_name,
        asset_name: asset.name,
        downloaded_to: dest.to_string_lossy().to_string(),
        size_bytes: size,
        note: "Downloaded and staged for review. Nothing has been installed — install only after you explicitly confirm in the UI.".to_string(),
    })
}

/// Clear any recorded postponement (e.g. the user wants to be reminded again).
#[tauri::command]
pub fn updater_clear_postpone(app: tauri::AppHandle) -> Result<(), String> {
    let store = app.store(SETTINGS_STORE).map_err(|e| e.to_string())?;
    let mut changed = false;
    for key in [POSTPONED_VERSION_KEY, POSTPONED_AT_KEY] {
        if store.has(key) {
            store.delete(key);
            changed = true;
        }
    }
    if changed {
        store.save().map_err(|e| e.to_string())?;
    }
    Ok(())
}
