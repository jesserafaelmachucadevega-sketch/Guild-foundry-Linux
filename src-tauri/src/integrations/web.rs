// SPDX-License-Identifier: Apache-2.0
// web.fetch — fetch an HTTP(S) page and return it as cleaned text, light
// markdown, or capped raw HTML.
//
// Static pages only. JavaScript-rendered pages are NOT executed here — the
// tool never pretends a page rendered; for those, connect a browser
// automation MCP server and call it through mcp.call_tool.

use std::sync::OnceLock;
use std::time::Duration;

/// Hard cap on the fetched body (2 MB) so huge pages cannot exhaust memory.
const MAX_BYTES: usize = 2 * 1024 * 1024;

fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (compatible; guild-foundry-ai/0.1.0)")
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to build web HTTP client")
    })
}

/// Fetch `url` and return `{url, title, format, truncated, chars, content}`.
/// `format`: "text" (default), "markdown", or "html".
pub fn fetch(
    url: &str,
    format: Option<&str>,
    timeout_secs: Option<u64>,
) -> Result<serde_json::Value, String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("url must start with http:// or https://".to_string());
    }
    let timeout = timeout_secs.unwrap_or(30).clamp(5, 120);
    let fmt = format.unwrap_or("text").to_lowercase();
    if !["text", "markdown", "html"].contains(&fmt.as_str()) {
        return Err(format!(
            "unsupported format '{}' (use text, markdown, or html)",
            fmt
        ));
    }
    tauri::async_runtime::block_on(async move {
        let resp = tokio::time::timeout(Duration::from_secs(timeout), http().get(u).send())
            .await
            .map_err(|_| format!("request timed out after {}s", timeout))?
            .map_err(|e| format!("request failed: {}", e))?;

        let status = resp.status().as_u16();
        if !(200..400).contains(&status) {
            return Err(format!("fetch failed: HTTP {}", status));
        }
        let final_url = resp.url().to_string();

        // Stream with a hard byte cap so huge pages cannot exhaust memory.
        let mut resp = resp;
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("read failed: {}", e))?
        {
            buf.extend_from_slice(&chunk);
            if buf.len() >= MAX_BYTES {
                break;
            }
        }
        let truncated = buf.len() >= MAX_BYTES;
        let raw = String::from_utf8_lossy(&buf).to_string();
        let title = extract_title(&raw);

        let body = match fmt.as_str() {
            "html" => raw.clone(),
            "markdown" => html_to_markdown(&raw),
            _ => html_to_text(&raw),
        };
        Ok(serde_json::json!({
            "url": final_url,
            "title": title,
            "format": fmt,
            "truncated": truncated,
            "chars": body.len(),
            "content": body,
        }))
    })
}

// ---------------------------------------------------------------------------
// HTML helpers (also used by document.rs for .html extraction)
// ---------------------------------------------------------------------------

fn regexes() -> &'static HtmlRegexes {
    static R: OnceLock<HtmlRegexes> = OnceLock::new();
    R.get_or_init(|| {
        // Rust's regex has no backreferences, so each boilerplate tag gets its
        // own open/close pair.
        let boilerplate = ["script", "style", "noscript", "svg", "template"]
            .iter()
            .map(|tag| {
                regex::Regex::new(&format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>")).unwrap()
            })
            .collect();
        HtmlRegexes {
            boilerplate,
            comment: regex::Regex::new(r"(?is)<!--.*?-->").unwrap(),
            head: regex::Regex::new(r"(?is)<head\b[^>]*>.*?</head\s*>").unwrap(),
            title: regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title\s*>").unwrap(),
            anchor: regex::Regex::new(r#"(?is)<a\s[^>]*?href\s*=\s*["']([^"']+)["'][^>]*?>(.*?)</a\s*>"#).unwrap(),
            listitem: regex::Regex::new(r"(?is)<li\b[^>]*>(.*?)</li\s*>").unwrap(),
            strong: regex::Regex::new(r"(?is)<(?:strong|b)\b[^>]*>(.*?)</(?:strong|b)\s*>").unwrap(),
            em: regex::Regex::new(r"(?is)<(?:em|i)\b[^>]*>(.*?)</(?:em|i)\s*>").unwrap(),
            blocks: regex::Regex::new(
                r"(?is)</?(p|div|section|article|header|footer|main|aside|nav|ul|ol|tr|table|form|blockquote|pre|figure|figcaption|dl|dt|dd|h[1-6])\b[^>]*>",
            ).unwrap(),
            breaks: regex::Regex::new(r"(?is)<br\s*/?>").unwrap(),
            tags: regex::Regex::new(r"(?is)<[^>]+>").unwrap(),
            numeric_entity: regex::Regex::new(r"&#(x?[0-9A-Fa-f]+);").unwrap(),
            named_entity: regex::Regex::new(r"&([a-zA-Z][a-zA-Z0-9]{1,31});").unwrap(),
            blank_lines: regex::Regex::new(r"\n{3,}").unwrap(),
            spaces: regex::Regex::new(r"[ \t]{2,}").unwrap(),
        }
    })
}

struct HtmlRegexes {
    boilerplate: Vec<regex::Regex>,
    comment: regex::Regex,
    head: regex::Regex,
    title: regex::Regex,
    anchor: regex::Regex,
    listitem: regex::Regex,
    strong: regex::Regex,
    em: regex::Regex,
    blocks: regex::Regex,
    breaks: regex::Regex,
    tags: regex::Regex,
    numeric_entity: regex::Regex,
    named_entity: regex::Regex,
    blank_lines: regex::Regex,
    spaces: regex::Regex,
}

fn heading_regex(level: u8) -> regex::Regex {
    regex::Regex::new(&format!(r"(?is)<h{}\b[^>]*>(.*?)</h{}\s*>", level, level)).unwrap()
}

/// Remove script/style/head/comments so extraction sees readable content.
fn strip_boilerplate(html: &str) -> String {
    let r = regexes();
    let mut s = html.to_string();
    for re in &r.boilerplate {
        s = re.replace_all(&s, " ").to_string();
    }
    s = r.comment.replace_all(&s, " ").to_string();
    s = r.head.replace_all(&s, " ").to_string();
    s
}

fn extract_title(html: &str) -> String {
    regexes()
        .title
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| collapse_whitespace(&decode_entities(&regexes().tags.replace_all(m.as_str(), ""))))
        .unwrap_or_default()
}

/// Readable plain text: block tags become newlines, remaining tags dropped,
/// entities decoded, whitespace collapsed.
pub(crate) fn html_to_text(html: &str) -> String {
    let r = regexes();
    let s = strip_boilerplate(html);
    let s = r.breaks.replace_all(&s, "\n");
    let s = r.blocks.replace_all(&s, "\n");
    let s = r.tags.replace_all(&s, " ");
    let s = decode_entities(&s);
    collapse_whitespace(&s)
}

/// Light markdown: headings, links, bold/emphasis, list items, paragraphs.
pub(crate) fn html_to_markdown(html: &str) -> String {
    let r = regexes();
    let mut s = strip_boilerplate(html).to_string();
    for level in 1u8..=6 {
        let hashes = "#".repeat(level as usize);
        let re = heading_regex(level);
        s = re
            .replace_all(&s, format!("\n{} $1\n", hashes).as_str())
            .to_string();
    }
    s = r.anchor.replace_all(&s, "[$2]($1)").to_string();
    s = r.strong.replace_all(&s, "**$1**").to_string();
    s = r.em.replace_all(&s, "*$1*").to_string();
    s = r.listitem.replace_all(&s, "\n- $1").to_string();
    s = r.breaks.replace_all(&s, "\n").to_string();
    s = r.blocks.replace_all(&s, "\n").to_string();
    s = r.tags.replace_all(&s, " ").to_string();
    s = decode_entities(&s);
    collapse_whitespace(&s)
}

fn collapse_whitespace(s: &str) -> String {
    let blank = regexes().blank_lines.replace_all(s, "\n\n");
    let spaces = regexes().spaces.replace_all(&blank, " ");
    let lines: Vec<&str> = spaces.lines().map(|l| l.trim_end()).collect();
    lines.join("\n").trim().to_string()
}

/// Decode numeric (&#65; / &#x41;) and common named entities.
pub(crate) fn decode_entities(s: &str) -> String {
    let r = regexes();
    let s = r
        .numeric_entity
        .replace_all(s, |caps: &regex::Captures| {
            let raw = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let code = if let Some(hex) = raw.strip_prefix('x').or_else(|| raw.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                raw.parse::<u32>().ok()
            };
            code.and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| caps.get(0).map(|m| m.as_str().to_string()).unwrap_or_default())
        })
        .to_string();
    r.named_entity
        .replace_all(&s, |caps: &regex::Captures| {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            match name {
                "amp" => "&".to_string(),
                "lt" => "<".to_string(),
                "gt" => ">".to_string(),
                "quot" => "\"".to_string(),
                "apos" | "39" => "'".to_string(),
                "nbsp" | "ensp" | "emsp" | "thinsp" => " ".to_string(),
                "mdash" => "—".to_string(),
                "ndash" => "–".to_string(),
                "hellip" => "…".to_string(),
                "lsquo" => "‘".to_string(),
                "rsquo" => "’".to_string(),
                "ldquo" => "“".to_string(),
                "rdquo" => "”".to_string(),
                "laquo" => "«".to_string(),
                "raquo" => "»".to_string(),
                "bull" => "•".to_string(),
                "middot" => "·".to_string(),
                "copy" => "©".to_string(),
                "reg" => "®".to_string(),
                "trade" => "™".to_string(),
                "deg" => "°".to_string(),
                "times" => "×".to_string(),
                "divide" => "÷".to_string(),
                "euro" => "€".to_string(),
                "pound" => "£".to_string(),
                "yen" => "¥".to_string(),
                "cent" => "¢".to_string(),
                "sect" => "§".to_string(),
                "para" => "¶".to_string(),
                "plusmn" => "±".to_string(),
                "frac12" => "½".to_string(),
                "check" => "✓".to_string(),
                other => format!("&{};", other),
            }
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_and_tags() {
        let html = "<html><head><title>Hi &amp; Bye</title><style>p{}</style></head>\
                    <body><script>alert('x')</script><h1>Hello</h1><p>World &mdash; now</p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World — now"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("p{}"));
        assert_eq!(extract_title(html), "Hi & Bye");
    }

    #[test]
    fn markdown_keeps_links_and_headings() {
        let html = "<h2>Title</h2><p>See <a href=\"https://example.com\">the site</a>.</p><ul><li>One</li><li>Two</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("## Title"));
        assert!(md.contains("[the site](https://example.com)"));
        assert!(md.contains("- One"));
    }

    #[test]
    fn decodes_numeric_entities() {
        assert_eq!(decode_entities("A &#65; &#x42; stop"), "A A B stop");
    }
}
