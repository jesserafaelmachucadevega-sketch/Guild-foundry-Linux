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

/// fal.run model ids we officially support in the UI picker.
pub const KNOWN_MODELS: &[(&str, &str)] = &[
    ("fal-ai/flux-2-dev", "~$0.025/image — best quality per dollar"),
    ("fal-ai/flux-2-pro", "~$0.05/image — highest quality"),
    ("fal-ai/stable-diffusion-xl", "~$0.003/image — cheapest drafts"),
];

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
