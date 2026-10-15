// SPDX-License-Identifier: Apache-2.0
// localvoice.rs — fully offline speech-to-text via the whisper-helper binary.
//
// whisper.cpp and llama.cpp each bundle their own ggml fork; linking both
// into one process collides (duplicate ggml symbols). So transcription runs
// in a tiny second binary built from this same crate
// (src/bin/whisper-helper.rs) that links whisper.cpp ONLY — still fully
// in-process on this machine, no server, no cloud, no API key. The app
// pipes raw PCM to it and reads the transcript back.
//
// Model files: any whisper.cpp ggml model (e.g. ggml-base.en.bin,
// ggml-small.bin). Point at one with media_local_set_model; the default
// drop-in location is app-data/models/whisper/whisper-model.bin.
//
// Local speech OUTPUT (Kokoro ONNX) is not enabled in this build — it needs
// the ONNX runtime, espeak-ng for phonemization, and the Kokoro model files.
// The status command states that plainly; cloud TTS keeps working until
// those prerequisites are installed. Nothing here pretends otherwise.

use std::path::PathBuf;
use std::process::Command;

use serde_json::json;
use tauri::Manager;

pub const WHISPER_MODEL_KEY: &str = "media.whisper_model";

fn whisper_model_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(Some(v)) = crate::settings::settings_get(app.clone(), WHISPER_MODEL_KEY.to_string()) {
        if let Some(p) = v.as_str() {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
        }
    }
    if let Ok(dir) = app.path().app_data_dir() {
        let pb = dir.join("models").join("whisper").join("whisper-model.bin");
        if pb.is_file() {
            return Some(pb);
        }
    }
    None
}

/// The helper ships next to the main binary (target/debug or the bundle).
fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for cand in [dir.join("whisper-helper"), dir.join("bin").join("whisper-helper")] {
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn transcribe(app: &tauri::AppHandle, samples: &[f32]) -> Result<String, String> {
    let model = whisper_model_path(app).ok_or_else(|| {
        "No local Whisper model configured. Pick a whisper.cpp ggml model under Connections → Local voice (or drop it in app-data/models/whisper/whisper-model.bin)."
            .to_string()
    })?;
    let helper = helper_path().ok_or_else(|| {
        "The whisper-helper binary is not next to the app yet — rebuild the app so the local transcription helper is built too."
            .to_string()
    })?;

    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    let tmp = std::env::temp_dir().join(format!("gfa-whisper-{}.pcm", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;

    // Hard ceiling: a wedged or corrupt model must not hang the tool call
    // forever — and if it did, the temp PCM file would leak with it.
    let spawn = Command::new(&helper)
        .arg(&model)
        .arg(&tmp)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match spawn {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("failed to run whisper-helper: {}", e));
        }
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let out = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break child.wait_with_output().map_err(|e| e.to_string())?,
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&tmp);
                    return Err(
                        "local transcription timed out after 180 s — the model may be corrupt or too large for this machine"
                            .to_string(),
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    };
    let _ = std::fs::remove_file(&tmp);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let snippet: String = err.trim().chars().take(300).collect();
        return Err(format!("local transcription failed: {}", snippet));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Transcribe raw 16 kHz mono f32 PCM (little-endian bytes, base64-encoded).
/// The frontend converts MediaRecorder output to this exact format.
#[tauri::command(rename_all = "snake_case")]
pub async fn media_transcribe_pcm(
    app: tauri::AppHandle,
    data_b64: String,
) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| format!("bad PCM payload: {}", e))?;
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if samples.len() < 1600 {
        return Err("recording too short (need at least 0.1s of audio)".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || transcribe(&app, &samples))
        .await
        .map_err(|e| e.to_string())?
}

/// Status of the local voice stack (drives the Connections card + mic path).
#[tauri::command]
pub fn media_local_voice_status(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let whisper = whisper_model_path(&app);
    let helper = helper_path();
    let kokoro = kokoro_status(&app);
    Ok(json!({
        "whisper_available": whisper.is_some() && helper.is_some(),
        "whisper_model": whisper.map(|p| p.to_string_lossy().to_string()),
        "whisper_helper": helper.map(|p| p.to_string_lossy().to_string()),
        "kokoro_available": kokoro.get("available").and_then(|v| v.as_bool()).unwrap_or(false),
        "kokoro_note": kokoro.get("note").and_then(|v| v.as_str()).unwrap_or(""),
        "kokoro": kokoro,
        "engine": "whisper.cpp (STT) + Kokoro (TTS) — both fully local",
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn media_local_set_model(
    app: tauri::AppHandle,
    kind: String,
    path: String,
) -> Result<(), String> {
    let trimmed = path.trim();
    match kind.as_str() {
        "whisper" => {
            let pb = PathBuf::from(trimmed);
            if !pb.is_file() {
                return Err(format!("file not found: {}", pb.display()));
            }
            crate::settings::settings_set(
                app,
                WHISPER_MODEL_KEY.to_string(),
                json!(pb.to_string_lossy().to_string()),
            )
            .map_err(|e| e.to_string())?;
        }
        "kokoro" => {
            let pb = PathBuf::from(trimmed);
            if !pb.is_dir() {
                return Err(format!("directory not found: {}", pb.display()));
            }
            if kokoro_files(&pb).is_none() {
                return Err(format!(
                    "{} is not a Kokoro model directory — expected model.onnx, voices.bin, tokens.txt and espeak-ng-data/ inside it",
                    pb.display()
                ));
            }
            crate::settings::settings_set(
                app,
                KOKORO_DIR_KEY.to_string(),
                json!(pb.to_string_lossy().to_string()),
            )
            .map_err(|e| e.to_string())?;
        }
        other => {
            return Err(format!(
                "unknown model kind '{}' — supported: 'whisper' (model file), 'kokoro' (model directory)",
                other
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Offline speech output — Kokoro via the sherpa-onnx engine (no cloud)
// ---------------------------------------------------------------------------

pub const KOKORO_DIR_KEY: &str = "media.kokoro_dir";
const SHERPA_BIN_KEY: &str = "media.sherpa_tts_bin";

/// Kokoro v0.19 English voices, indexed by speaker id (--sid). Names follow
/// the upstream voice packs (af_/am_ = US English, bf_/bm_ = UK English).
const KOKORO_VOICES: &[(&str, u32)] = &[
    ("af", 0),
    ("af_bella", 1),
    ("af_nicole", 2),
    ("af_sarah", 3),
    ("af_sky", 4),
    ("am_adam", 5),
    ("am_michael", 6),
    ("bf_emma", 7),
    ("bf_isabella", 8),
    ("bm_george", 9),
    ("bm_lewis", 10),
];
const KOKORO_DEFAULT_VOICE: &str = "af_bella";

fn setting_path(app: &tauri::AppHandle, key: &str) -> Option<PathBuf> {
    crate::settings::settings_get(app.clone(), key.to_string())
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| PathBuf::from(s.trim())))
        .filter(|p| !p.as_os_str().is_empty())
}

/// The sherpa-onnx TTS binary: explicit setting, then app-data/bin (the
/// drop-in spot), then ~/.local/bin, then $PATH.
fn sherpa_bin(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Some(p) = setting_path(app, SHERPA_BIN_KEY) {
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(base) = app.path().app_data_dir() {
        let p = base.join("bin").join("sherpa-onnx-offline-tts");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".local/bin/sherpa-onnx-offline-tts");
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("sherpa-onnx-offline-tts"))
            .find(|p| p.is_file())
    })
}

/// Kokoro model directory: explicit setting, then any subdirectory of
/// app-data/models/kokoro that looks like a model dir.
fn kokoro_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    if let Some(p) = setting_path(app, KOKORO_DIR_KEY) {
        if kokoro_files(&p).is_some() {
            return Some(p);
        }
    }
    let base = app
        .path()
        .app_data_dir()
        .ok()?
        .join("models")
        .join("kokoro");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && kokoro_files(p).is_some())
        .collect();
    dirs.sort();
    if let Some(d) = dirs.into_iter().next() {
        return Some(d);
    }
    if kokoro_files(&base).is_some() {
        return Some(base);
    }
    None
}

/// The four pieces a Kokoro model directory must contain.
fn kokoro_files(dir: &std::path::Path) -> Option<(PathBuf, PathBuf, PathBuf, PathBuf)> {
    let onnx = dir.join("model.onnx");
    let voices = dir.join("voices.bin");
    let tokens = dir.join("tokens.txt");
    let data = dir.join("espeak-ng-data");
    if onnx.is_file() && voices.is_file() && tokens.is_file() && data.is_dir() {
        Some((onnx, voices, tokens, data))
    } else {
        None
    }
}

/// Voice param → (sid, display name). Accepts a known name or a raw number;
/// unknown values fall back to the default voice.
fn resolve_voice(voice: Option<&str>) -> (u32, String) {
    if let Some(v) = voice.map(str::trim).filter(|v| !v.is_empty()) {
        if let Ok(id) = v.parse::<u32>() {
            return (id, format!("sid {}", id));
        }
        let lower = v.to_lowercase();
        if let Some((name, id)) = KOKORO_VOICES.iter().find(|(n, _)| *n == lower) {
            return (*id, name.to_string());
        }
    }
    let (name, id) = KOKORO_VOICES
        .iter()
        .find(|(n, _)| *n == KOKORO_DEFAULT_VOICE)
        .copied()
        .unwrap_or(("af_bella", 1));
    (id, name.to_string())
}

/// Status for the UI: engine binary, model dir, what is still missing.
pub fn kokoro_status(app: &tauri::AppHandle) -> serde_json::Value {
    let bin = sherpa_bin(app);
    let dir = kokoro_dir(app);
    let files_ok = dir
        .as_ref()
        .map(|d| kokoro_files(d).is_some())
        .unwrap_or(false);
    let mut missing: Vec<&str> = Vec::new();
    if bin.is_none() {
        missing.push("speech engine binary (sherpa-onnx-offline-tts)");
    }
    if dir.is_none() {
        missing.push("Kokoro model directory");
    } else if !files_ok {
        missing.push("model files (model.onnx, voices.bin, tokens.txt, espeak-ng-data/)");
    }
    let available = missing.is_empty();
    let note = if available {
        "Kokoro is ready — speech output is generated fully offline on this machine.".to_string()
    } else {
        format!(
            "Kokoro is not ready yet — missing: {}. Add it under Connections → Local voice.",
            missing.join("; ")
        )
    };
    json!({
        "available": available,
        "binary": bin.map(|p| p.to_string_lossy().to_string()),
        "dir": dir.map(|p| p.to_string_lossy().to_string()),
        "missing": missing,
        "voices": KOKORO_VOICES.iter().map(|(n, id)| json!({ "id": id, "name": n })).collect::<Vec<_>>(),
        "default_voice": KOKORO_DEFAULT_VOICE,
        "note": note,
    })
}

/// Synthesize speech locally with Kokoro and return the WAV path. Blocking;
/// callers run it on the blocking pool (media::speak does).
pub fn synthesize(
    app: &tauri::AppHandle,
    text: &str,
    voice: Option<&str>,
) -> Result<serde_json::Value, String> {
    let bin = sherpa_bin(app).ok_or_else(|| {
        "offline speech engine not installed yet (sherpa-onnx-offline-tts) — see Connections → Local voice".to_string()
    })?;
    let dir = kokoro_dir(app).ok_or_else(|| {
        "no Kokoro model directory configured — set one under Connections → Local voice".to_string()
    })?;
    let (onnx, voices_bin, tokens, data_dir) = kokoro_files(&dir).ok_or_else(|| {
        format!(
            "{} is missing Kokoro model files (model.onnx, voices.bin, tokens.txt, espeak-ng-data/)",
            dir.display()
        )
    })?;
    let (sid, voice_name) = resolve_voice(voice);

    let out_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("media");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let wav = out_dir.join(format!(
        "tts-local-{}.wav",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3f")
    ));
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(6))
        .unwrap_or(2);

    let spawn = Command::new(&bin)
        .arg(format!("--kokoro-model={}", onnx.display()))
        .arg(format!("--kokoro-voices={}", voices_bin.display()))
        .arg(format!("--kokoro-tokens={}", tokens.display()))
        .arg(format!("--kokoro-data-dir={}", data_dir.display()))
        .arg(format!("--sid={}", sid))
        .arg("--speed=1.0")
        .arg(format!("--num-threads={}", threads))
        .arg(format!("--output-filename={}", wav.display()))
        .arg(text)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match spawn {
        Ok(c) => c,
        Err(e) => return Err(format!("failed to run the offline speech engine: {}", e)),
    };
    // Same deadline discipline as whisper-helper: a wedged engine must never
    // hang the request, and a failed run must not leave a partial file.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    let out = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break child.wait_with_output().map_err(|e| e.to_string())?,
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_file(&wav);
                    return Err("offline speech generation timed out after 180 s".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    };
    if !out.status.success() {
        let _ = std::fs::remove_file(&wav);
        let err = String::from_utf8_lossy(&out.stderr);
        let snippet: String = err.trim().chars().take(300).collect();
        return Err(format!("offline speech generation failed: {}", snippet));
    }
    if !wav.is_file() {
        return Err("offline speech engine reported success but wrote no audio file".to_string());
    }
    Ok(json!({
        "path": wav.to_string_lossy(),
        "url": serde_json::Value::Null,
        "chars": text.chars().count(),
        "engine": "kokoro-offline",
        "voice": voice_name,
    }))
}
