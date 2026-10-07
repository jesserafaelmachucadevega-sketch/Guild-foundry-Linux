// SPDX-License-Identifier: Apache-2.0
// Image generation for every model — local or frontier — through one API.
//
// The agent calls `media.generate_image` with a prompt; this module calls the
// Fal API (default model FLUX 2 Dev, ~$0.025/image), downloads the result, and
// saves it under the app-data `media/` directory. The tool result carries the
// saved file path; the frontend renders it with `convertFileSrc`.
//
// The Fal API key lives in the OS keyring under FAL_KEYRING_KEY — it never
// enters the database or the model context. Set it in Connections > Image
// generation. Default model is overridable via the `media.image_model`
// setting; the tool arg wins over the setting.

use std::time::Duration;

pub const FAL_KEYRING_KEY: &str = "gfa-media-fal-key";
pub const DEFAULT_MODEL: &str = "fal-ai/flux-2-dev";
pub const DEFAULT_VIDEO_MODEL: &str = "fal-ai/wan/v2.7/text-to-video";

/// fal.run model ids we officially support in the UI picker.
pub const KNOWN_MODELS: &[(&str, &str)] = &[
    ("fal-ai/flux-2-dev", "~$0.025/image — best quality per dollar"),
    ("fal-ai/flux-2-pro", "~$0.05/image — highest quality"),
    ("fal-ai/stable-diffusion-xl", "~$0.003/image — cheapest drafts"),
];

/// fal.ai video endpoint ids for the UI picker.
pub const KNOWN_VIDEO_MODELS: &[(&str, &str)] = &[
    ("fal-ai/wan/v2.7/text-to-video", "~$0.05/sec — budget, fast"),
    ("fal-ai/veo3.1", "premium — native audio, top quality"),
    ("fal-ai/kling-video/v3/pro/text-to-video", "premium — native audio, camera control"),
];

pub const DEFAULT_STT_MODEL: &str = "fal-ai/whisper";
/// Kokoro — natural human-sounding TTS (no robotic browser voice).
pub const DEFAULT_TTS_MODEL: &str = "fal-ai/kokoro/american-english";

/// Upload a local file to Fal storage; returns the public file URL.
/// Two-step flow: initiate → PUT bytes to the upload URL.
async fn fal_upload(
    client: &reqwest::Client,
    key: &str,
    path: &std::path::Path,
    content_type: &str,
) -> Result<String, String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("upload.bin");
    let init: serde_json::Value = post_json(
        client,
        "https://rest.alpha.fal.ai/storage/upload/initiate",
        key,
        &serde_json::json!({ "file_name": file_name, "content_type": content_type }),
    )
    .await?;
    let upload_url = init
        .get("upload_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Fal upload initiate returned no upload_url".to_string())?;
    let file_url = init
        .get("file_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Fal upload initiate returned no file_url".to_string())?
        .to_string();
    let bytes = std::fs::read(path).map_err(|e| format!("read failed: {}", e))?;
    let resp = client
        .put(upload_url)
        .header("Content-Type", content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|e| format!("Fal upload failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Fal upload error: {}", resp.status()));
    }
    Ok(file_url)
}

fn fal_key() -> Result<String, String> {
    crate::secrets::secret_get(FAL_KEYRING_KEY.to_string())?
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            "No Fal API key configured. Add one under Connections > Media generation.".to_string()
        })
}

/// Speech-to-text via Fal Whisper. Accepts a public audio URL or a local file
/// path (uploaded to Fal storage first). 99+ languages, auto-detected.
pub fn transcribe(
    audio_url: Option<&str>,
    audio_path: Option<&str>,
    language: Option<&str>,
    task: Option<&str>,
) -> Result<serde_json::Value, String> {
    let key = fal_key()?;
    let task = task.unwrap_or("transcribe");
    if task != "transcribe" && task != "translate" {
        return Err("task must be transcribe or translate".to_string());
    }

    tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| format!("HTTP client failed: {}", e))?;

        let url = match (audio_url, audio_path) {
            (Some(u), _) if !u.trim().is_empty() => u.trim().to_string(),
            (_, Some(p)) => {
                let path = std::path::Path::new(p);
                if !path.is_file() {
                    return Err(format!("audio file not found: {}", p));
                }
                let ct = match path.extension().and_then(|e| e.to_str()) {
                    Some("mp3") => "audio/mpeg",
                    Some("wav") => "audio/wav",
                    Some("m4a") => "audio/mp4",
                    Some("ogg") => "audio/ogg",
                    _ => "audio/webm",
                };
                fal_upload(&client, &key, path, ct).await?
            }
            _ => return Err("provide audio_url or audio_path".to_string()),
        };

        let mut body = serde_json::json!({
            "audio_url": url,
            "task": task,
            "chunk_level": "segment",
        });
        if let Some(l) = language {
            body["language"] = serde_json::json!(l);
        }
        let res = post_json(
            &client,
            &format!("https://fal.run/{}", DEFAULT_STT_MODEL),
            &key,
            &body,
        )
        .await?;
        Ok(serde_json::json!({
            "text": res.get("text").cloned().unwrap_or(serde_json::Value::Null),
            "chunks": res.get("chunks").cloned().unwrap_or(serde_json::Value::Null),
            "languages": res.get("inferred_languages").cloned().unwrap_or(serde_json::Value::Null),
        }))
    })
}

/// Text-to-speech via Fal. Returns the saved local MP3 path; the harness plays it.
pub fn speak(
    app: &tauri::AppHandle,
    text: &str,
    voice: Option<&str>,
) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Err("text must not be empty".to_string());
    }
    if text.len() > 5000 {
        return Err("text too long for one call (max 5000 chars)".to_string());
    }
    let key = fal_key()?;

    tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("HTTP client failed: {}", e))?;

        let mut body = serde_json::json!({ "text": text });
        if let Some(v) = voice {
            body["voice"] = serde_json::json!(v);
        }
        let res = post_json(
            &client,
            &format!("https://fal.run/{}", DEFAULT_TTS_MODEL),
            &key,
            &body,
        )
        .await?;
        let audio_url = res
            .get("audio")
            .and_then(|a| a.get("url"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| "Fal TTS returned no audio URL".to_string())?
            .to_string();

        let bytes = client
            .get(&audio_url)
            .send()
            .await
            .map_err(|e| format!("audio download failed: {}", e))?
            .bytes()
            .await
            .map_err(|e| format!("audio read failed: {}", e))?;

        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("media");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let fname = format!("tts-{}.mp3", chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"));
        let path = dir.join(&fname);
        std::fs::write(&path, &bytes).map_err(|e| format!("save failed: {}", e))?;

        Ok(serde_json::json!({
            "path": path.to_string_lossy(),
            "url": audio_url,
            "chars": text.len(),
        }))
    })
}

/// Frontend mic flow: base64 audio from MediaRecorder → temp file → transcribe.
/// Returns the transcript text directly (not a tool call).
#[tauri::command]
pub fn media_transcribe_mic(
    app: tauri::AppHandle,
    audio_base64: String,
    content_type: String,
    language: Option<String>,
) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_base64.trim())
        .map_err(|e| format!("bad base64 audio: {}", e))?;
    if bytes.is_empty() || bytes.len() > 25 * 1024 * 1024 {
        return Err("audio must be 1 byte–25 MB".to_string());
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("media")
        .join("mic");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = match content_type.as_str() {
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        _ => "webm",
    };
    let path = dir.join(format!(
        "mic-{}.{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"),
        ext
    ));
    std::fs::write(&path, &bytes).map_err(|e| format!("save failed: {}", e))?;
    let out = transcribe(None, Some(&path.to_string_lossy()), language.as_deref(), None)?;
    // Best-effort cleanup; the transcript is what matters.
    let _ = std::fs::remove_file(&path);
    out.get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "transcription returned no text".to_string())
}

/// Frontend "read aloud" flow: text → speech → local path for playback.
#[tauri::command]
pub fn media_speak_text(
    app: tauri::AppHandle,
    text: String,
    voice: Option<String>,
) -> Result<String, String> {
    let out = speak(&app, &text, voice.as_deref())?;
    out.get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "TTS returned no path".to_string())
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let resp = client
        .post(url)
        .header("Authorization", format!("Key {}", key))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Fal request failed: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("Fal read failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("Fal API error {}: {}", status, truncate(&text, 300)));
    }
    serde_json::from_str(&text).map_err(|e| format!("Fal bad JSON: {}", e))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

pub fn generate_image(
    app: &tauri::AppHandle,
    prompt: &str,
    model_arg: Option<&str>,
    image_size_arg: Option<&str>,
) -> Result<serde_json::Value, String> {
    if prompt.trim().is_empty() {
        return Err("prompt must not be empty".to_string());
    }
    let key = crate::secrets::secret_get(FAL_KEYRING_KEY.to_string())?
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            "No Fal API key configured. Add one under Connections > Image generation.".to_string()
        })?;

    // Tool arg wins; then the media.image_model setting; then the default.
    let setting_model: Option<String> = crate::settings::settings_get(app.clone(), "media.image_model".to_string())
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let model = model_arg
        .map(|s| s.to_string())
        .or(setting_model)
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    if model.trim().is_empty() || model.contains("..") || model.contains(' ') {
        return Err("invalid model id".to_string());
    }
    let image_size = image_size_arg.unwrap_or("square_hd");

    let out = tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("HTTP client failed: {}", e))?;
        let endpoint = format!("https://fal.run/{}", model.trim());
        let body = serde_json::json!({ "prompt": prompt, "image_size": image_size });
        let res = post_json(&client, &endpoint, &key, &body).await?;

        let img_url = res
            .get("images")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|o| o.get("url"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| "Fal returned no image URL".to_string())?
            .to_string();
        let seed = res.get("seed").cloned().unwrap_or(serde_json::Value::Null);

        // Download the bytes and save locally so the result survives URL expiry.
        let bytes = client
            .get(&img_url)
            .send()
            .await
            .map_err(|e| format!("image download failed: {}", e))?
            .bytes()
            .await
            .map_err(|e| format!("image read failed: {}", e))?;

        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("media");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let fname = format!(
            "img-{}.jpg",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%3f")
        );
        let path = dir.join(&fname);
        std::fs::write(&path, &bytes).map_err(|e| format!("save failed: {}", e))?;

        Ok::<serde_json::Value, String>(serde_json::json!({
            "path": path.to_string_lossy(),
            "url": img_url,
            "model": model,
            "seed": seed,
            "bytes": bytes.len(),
        }))
    })?;

    Ok(out)
}

/// Generate a short video clip via the Fal queue API.
///
/// Video takes minutes, so unlike images this uses the async queue:
/// submit → poll status → fetch result → download → save locally.
/// Costs are per-second (roughly $0.05/sec budget to $0.30+/sec premium),
/// so the tool description warns the model to confirm duration with the user.
pub fn generate_video(
    app: &tauri::AppHandle,
    prompt: &str,
    model_arg: Option<&str>,
    duration_secs: Option<u64>,
    aspect_ratio_arg: Option<&str>,
) -> Result<serde_json::Value, String> {
    if prompt.trim().is_empty() {
        return Err("prompt must not be empty".to_string());
    }
    let key = crate::secrets::secret_get(FAL_KEYRING_KEY.to_string())?
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            "No Fal API key configured. Add one under Connections > Media generation.".to_string()
        })?;

    let setting_model: Option<String> = crate::settings::settings_get(app.clone(), "media.video_model".to_string())
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let model = model_arg
        .map(|s| s.to_string())
        .or(setting_model)
        .unwrap_or_else(|| DEFAULT_VIDEO_MODEL.to_string());
    if model.trim().is_empty() || model.contains("..") || model.contains(' ') {
        return Err("invalid model id".to_string());
    }
    let duration_secs = duration_secs.unwrap_or(5).clamp(3, 15);
    let aspect_ratio = aspect_ratio_arg.unwrap_or("16:9");

    let out = tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("HTTP client failed: {}", e))?;

        // 1. Submit to the queue.
        let submit_url = format!("https://queue.fal.run/{}", model.trim());
        let body = serde_json::json!({
            "prompt": prompt,
            "duration": format!("{}s", duration_secs),
            "aspect_ratio": aspect_ratio,
        });
        let submit: serde_json::Value = post_json(&client, &submit_url, &key, &body).await?;
        let request_id = submit
            .get("request_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Fal queue returned no request_id".to_string())?
            .to_string();

        // 2. Poll until done (video takes minutes; poll every 5s, up to 12 min).
        let status_url = format!("{}/requests/{}/status", submit_url, request_id);
        let deadline = std::time::Instant::now() + Duration::from_secs(720);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(format!(
                    "video generation timed out after 12 minutes (request {})",
                    request_id
                ));
            }
            // Status endpoint: GET per docs, but some deployments 405 — fall back to POST.
            let stext = {
                let get = client
                    .get(&status_url)
                    .header("Authorization", format!("Key {}", key))
                    .send()
                    .await;
                let use_post = matches!(&get, Ok(r) if r.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED);
                if use_post {
                    client
                        .post(&status_url)
                        .header("Authorization", format!("Key {}", key))
                        .json(&serde_json::json!({}))
                        .send()
                        .await
                        .map_err(|e| format!("Fal status failed: {}", e))?
                        .text()
                        .await
                        .map_err(|e| format!("Fal read failed: {}", e))?
                } else {
                    get.map_err(|e| format!("Fal status failed: {}", e))?
                        .text()
                        .await
                        .map_err(|e| format!("Fal read failed: {}", e))?
                }
            };
            let status: serde_json::Value =
                serde_json::from_str(&stext).map_err(|e| format!("Fal bad JSON: {}", e))?;
            let st = status.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if st == "COMPLETED" {
                break;
            }
            if st == "FAILED" {
                return Err(format!(
                    "video generation failed: {}",
                    truncate(&status.to_string(), 300)
                ));
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        // 3. Fetch the result.
        let result_url = format!("{}/requests/{}", submit_url, request_id);
        let resp = client
            .get(&result_url)
            .header("Authorization", format!("Key {}", key))
            .send()
            .await
            .map_err(|e| format!("Fal result failed: {}", e))?;
        let rtext = resp.text().await.map_err(|e| format!("Fal read failed: {}", e))?;
        let result: serde_json::Value =
            serde_json::from_str(&rtext).map_err(|e| format!("Fal bad JSON: {}", e))?;
        let vid_url = result
            .get("video")
            .and_then(|v| v.get("url"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| "Fal returned no video URL".to_string())?
            .to_string();

        // 4. Download and save locally.
        let bytes = client
            .get(&vid_url)
            .send()
            .await
            .map_err(|e| format!("video download failed: {}", e))?
            .bytes()
            .await
            .map_err(|e| format!("video read failed: {}", e))?;

        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("media");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let fname = format!("vid-{}.mp4", chrono::Utc::now().format("%Y%m%dT%H%M%S%3f"));
        let path = dir.join(&fname);
        std::fs::write(&path, &bytes).map_err(|e| format!("save failed: {}", e))?;

        Ok::<serde_json::Value, String>(serde_json::json!({
            "path": path.to_string_lossy(),
            "url": vid_url,
            "model": model,
            "duration_secs": duration_secs,
            "bytes": bytes.len(),
        }))
    })?;

    Ok(out)
}
