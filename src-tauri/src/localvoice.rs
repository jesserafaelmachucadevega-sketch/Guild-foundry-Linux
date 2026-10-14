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
    Ok(json!({
        "whisper_available": whisper.is_some() && helper.is_some(),
        "whisper_model": whisper.map(|p| p.to_string_lossy().to_string()),
        "whisper_helper": helper.map(|p| p.to_string_lossy().to_string()),
        "kokoro_available": false,
        "kokoro_note": "Local speech output (Kokoro ONNX) is not enabled in this build — it needs the ONNX runtime, espeak-ng for phonemization, and the Kokoro model files. Cloud TTS keeps working until then.",
        "engine": "whisper.cpp (local helper)",
    }))
}

#[tauri::command(rename_all = "snake_case")]
pub fn media_local_set_model(
    app: tauri::AppHandle,
    kind: String,
    path: String,
) -> Result<(), String> {
    if kind != "whisper" {
        return Err(
            "only 'whisper' models are supported for local transcription right now".to_string(),
        );
    }
    let pb = PathBuf::from(path.trim());
    if !pb.is_file() {
        return Err(format!("file not found: {}", pb.display()));
    }
    crate::settings::settings_set(
        app.clone(),
        WHISPER_MODEL_KEY.to_string(),
        json!(pb.to_string_lossy().to_string()),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
