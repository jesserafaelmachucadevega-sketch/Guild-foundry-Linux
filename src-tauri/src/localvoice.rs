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

// ---------------------------------------------------------------------------
// WAV decoding for offline transcription (Twilio recordings are PCM WAV)
// ---------------------------------------------------------------------------

/// Decode a PCM WAV file to 16 kHz mono f32 for whisper-helper. Twilio
/// recordings are mono 8 kHz 16-bit PCM; linear interpolation is plenty for
/// speech at this quality level. Bounded: max ~1 hour of audio.
pub fn parse_wav_to_16k_mono(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_string());
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(size)
            .filter(|e| *e <= bytes.len())
            .ok_or("truncated WAV chunk")?;
        if id == b"fmt " && size >= 16 {
            fmt = Some((
                u16::from_le_bytes([bytes[body_start], bytes[body_start + 1]]),
                u16::from_le_bytes([bytes[body_start + 2], bytes[body_start + 3]]),
                u32::from_le_bytes([
                    bytes[body_start + 4],
                    bytes[body_start + 5],
                    bytes[body_start + 6],
                    bytes[body_start + 7],
                ]),
                u16::from_le_bytes([bytes[body_start + 14], bytes[body_start + 15]]),
            ));
        } else if id == b"data" {
            data = Some(&bytes[body_start..body_end]);
        }
        pos = body_end + (size & 1); // chunks are word-aligned
    }
    let (format, channels, rate, bits) = fmt.ok_or("WAV has no fmt chunk")?;
    if format != 1 {
        return Err(format!(
            "WAV format {} unsupported (need uncompressed PCM)",
            format
        ));
    }
    if bits != 8 && bits != 16 {
        return Err(format!("WAV {}-bit samples unsupported (need 8 or 16)", bits));
    }
    if channels == 0 || rate == 0 {
        return Err("invalid WAV header".to_string());
    }
    let data = data.ok_or("WAV has no data chunk")?;
    let sample_bytes = (bits as usize) / 8;
    let frame_bytes = sample_bytes * channels as usize;
    let frames = (data.len() / frame_bytes).min(rate as usize * 3600);

    let mut mono = Vec::with_capacity(frames);
    for i in 0..frames {
        let base = i * frame_bytes;
        let mut acc = 0f32;
        for c in 0..channels as usize {
            let off = base + c * sample_bytes;
            let s = if bits == 16 {
                i16::from_le_bytes([data[off], data[off + 1]]) as f32 / 32768.0
            } else {
                (data[off] as f32 - 128.0) / 128.0
            };
            acc += s;
        }
        mono.push(acc / channels as f32);
    }

    const TARGET: u32 = 16_000;
    if rate == TARGET {
        return Ok(mono);
    }
    let ratio = TARGET as f64 / rate as f64;
    let out_len = ((mono.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let frac = (src - i0 as f64) as f32;
        let s0 = mono.get(i0).copied().unwrap_or(0.0);
        let s1 = mono.get(i0 + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    Ok(out)
}

/// Transcribe a WAV file fully offline (local Whisper helper).
pub fn transcribe_wav_file(
    app: &tauri::AppHandle,
    path: &std::path::Path,
) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read recording: {}", e))?;
    if bytes.len() > 100 * 1024 * 1024 {
        return Err("recording too large to transcribe locally (max 100 MB)".to_string());
    }
    let samples = parse_wav_to_16k_mono(&bytes)?;
    if samples.len() < 1600 {
        return Err("recording is too short (less than 0.1 s of audio)".to_string());
    }
    transcribe(app, &samples)
}

#[cfg(test)]
mod wav_tests {
    use super::*;

    fn wav_bytes(rate: u32, channels: u16, bits: u16, frames: usize, sample: i16) -> Vec<u8> {
        let data_len = frames * channels as usize * (bits as usize / 8);
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        let byte_rate = rate * channels as u32 * (bits as usize / 8) as u32;
        b.extend_from_slice(&byte_rate.to_le_bytes());
        b.extend_from_slice(&(channels * (bits / 8)).to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        for _ in 0..frames * channels as usize {
            b.extend_from_slice(&sample.to_le_bytes());
        }
        b
    }

    #[test]
    fn parses_8khz_stereo_and_resamples_to_16k_mono() {
        let bytes = wav_bytes(8000, 2, 16, 800, 16384);
        let samples = parse_wav_to_16k_mono(&bytes).expect("parse");
        assert!(
            (samples.len() as f64 - 1600.0).abs() <= 2.0,
            "expected ~1600 samples, got {}",
            samples.len()
        );
        assert!(
            samples.iter().all(|s| (*s - 0.5).abs() < 0.01),
            "a constant tone must stay constant through the resampler"
        );
    }

    #[test]
    fn parses_16khz_mono_without_resampling() {
        let bytes = wav_bytes(16_000, 1, 16, 1600, -16384);
        let samples = parse_wav_to_16k_mono(&bytes).expect("parse");
        assert_eq!(samples.len(), 1600);
        assert!(samples.iter().all(|s| (*s + 0.5).abs() < 0.01));
    }

    #[test]
    fn rejects_non_wav_input() {
        assert!(parse_wav_to_16k_mono(b"definitely not audio at all").is_err());
    }
}
