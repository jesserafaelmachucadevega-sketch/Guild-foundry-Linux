// SPDX-License-Identifier: Apache-2.0
// Phase 10 — Web research engine (master spec: memory/indexing/research).
//
// Pipeline: SEARCH -> COLLECT -> DEDUPLICATE -> EXTRACT -> CITE ->
//           COMPARE -> SYNTHESIZE. Every run keeps provenance (source URLs)
//           from the first stage to the final synthesis, and the frontend
//           shows `research://update` progress events {job_id, stage,
//           progress, message}.
//
// Honest limitations (documented, not hidden):
//   - SEARCH scrapes DuckDuckGo's HTML endpoint with a browser UA and
//     regex-parses result links/titles/snippets. This is FRAGILE: DDG can
//     change markup, throttle, or serve a CAPTCHA at any time. If parsing
//     yields zero results the job reports `no_results` instead of guessing.
//   - SYNTHESIZE is EXTRACTIVE (ranks real sentences by query overlap),
//     not LLM-generated. It is labeled as such in every payload.
//   - Rust cannot compile in this build environment; this is NOT VERIFIED
//     until `cargo build` runs. Network is required at runtime; the engine
//     reports OFFLINE-style errors when fetches fail.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager};

use crate::memory::open_db;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn migrate_research(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS research_jobs (
            id          TEXT PRIMARY KEY,
            query       TEXT NOT NULL,
            state       TEXT NOT NULL,      -- running | done | failed
            stage       TEXT NOT NULL DEFAULT '',
            progress    REAL NOT NULL DEFAULT 0.0,
            results_json TEXT NOT NULL DEFAULT '[]',
            error       TEXT NOT NULL DEFAULT '',
            started_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );",
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitedResult {
    pub index: usize,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub key_sentences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStatus {
    pub id: String,
    pub query: String,
    pub state: String,
    pub stage: String,
    pub progress: f64,
    pub results: Vec<CitedResult>,
    pub synthesis: String,
    pub comparisons: Vec<String>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResearchUpdate {
    job_id: String,
    stage: String,
    progress: f64,
    message: String,
}

static JOBS: OnceLock<Mutex<HashMap<String, ResearchStatus>>> = OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<String, ResearchStatus>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn set_status(s: ResearchStatus) {
    if let Ok(mut j) = jobs().lock() {
        j.insert(s.id.clone(), s);
    }
}

fn get_status(id: &str) -> Option<ResearchStatus> {
    jobs().lock().ok()?.get(id).cloned()
}

fn emit_update(app: &tauri::AppHandle, job_id: &str, stage: &str, progress: f64, message: &str) {
    let _ = app.emit(
        "research://update",
        ResearchUpdate {
            job_id: job_id.to_string(),
            stage: stage.to_string(),
            progress,
            message: message.to_string(),
        },
    );
}

/// Start a research job. Returns the job_id immediately; the pipeline runs
/// in a background tokio task and emits `research://update` events.
#[tauri::command]
pub fn research_start(app: tauri::AppHandle, query: String) -> Result<String, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Err("refused: research query must not be empty".to_string());
    }
    let job_id = uuid::Uuid::new_v4().to_string();
    let status = ResearchStatus {
        id: job_id.clone(),
        query: q.clone(),
        state: "running".to_string(),
        stage: "search".to_string(),
        progress: 0.0,
        results: Vec::new(),
        synthesis: String::new(),
        comparisons: Vec::new(),
        error: String::new(),
    };
    set_status(status.clone());
    if let Ok(conn) = open_db(&app) {
        let _ = migrate_research(&conn);
        let _ = conn.execute(
            "INSERT INTO research_jobs (id, query, state, stage, progress,
                                        results_json, error, started_at, updated_at)
             VALUES (?1, ?2, 'running', 'search', 0.0, '[]', '', ?3, ?3)",
            params![job_id, q, now_iso()],
        );
    }
    let app_handle = app.clone();
    let jid = job_id.clone();
    tokio::spawn(async move {
        match run_pipeline(&app_handle, &jid, &q).await {
            Ok(mut s) => {
                s.state = "done".to_string();
                s.stage = "synthesize".to_string();
                s.progress = 1.0;
                set_status(s.clone());
                persist_done(&app_handle, &s);
                emit_update(&app_handle, &jid, "synthesize", 1.0, "research complete");
            }
            Err(e) => {
                let mut s = get_status(&jid).unwrap_or(status);
                s.state = "failed".to_string();
                s.error = e.clone();
                set_status(s.clone());
                persist_done(&app_handle, &s);
                emit_update(&app_handle, &jid, "failed", s.progress, &format!("failed: {}", e));
            }
        }
    });
    Ok(job_id)
}

fn persist_done(app: &tauri::AppHandle, s: &ResearchStatus) {
    if let Ok(conn) = open_db(app) {
        let _ = conn.execute(
            "UPDATE research_jobs SET state = ?1, stage = ?2, progress = ?3,
                 results_json = ?4, error = ?5, updated_at = ?6 WHERE id = ?7",
            params![
                s.state,
                s.stage,
                s.progress,
                serde_json::to_string(&s.results).unwrap_or_else(|_| "[]".into()),
                s.error,
                now_iso(),
                s.id
            ],
        );
    }
}

/// Live status of a research job (in-memory; persisted copy in research_jobs).
#[tauri::command]
pub fn research_status(app: tauri::AppHandle, job_id: String) -> Result<ResearchStatus, String> {
    if let Some(s) = get_status(&job_id) {
        return Ok(s);
    }
    // Fall back to the persisted row (e.g. after app restart).
    let conn = open_db(&app)?;
    let _ = migrate_research(&conn);
    conn.query_row(
        "SELECT id, query, state, stage, progress, results_json, error
         FROM research_jobs WHERE id = ?1",
        params![job_id],
        |row| {
            Ok(ResearchStatus {
                id: row.get(0)?,
                query: row.get(1)?,
                state: row.get(2)?,
                stage: row.get(3)?,
                progress: row.get(4)?,
                results: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
                synthesis: String::new(),
                comparisons: Vec::new(),
                error: row.get(6)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Pipeline stages (all async, called from the background task).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RawHit {
    title: String,
    url: String,
    snippet: String,
}

/// SEARCH: scrape DDG HTML results. FRAGILE by design — documented above.
async fn stage_search(client: &reqwest::Client, query: &str) -> Result<Vec<RawHit>, String> {
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding_simple(query)
    );
    let html = client
        .get(&url)
        .header("User-Agent", UA)
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| format!("search request failed: {}", e))?
        .text()
        .await
        .map_err(|e| format!("search read failed: {}", e))?;

    // result__a anchors carry href + title; result__snippet carries the blurb.
    let link_re = regex::Regex::new(
        r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .map_err(|e| e.to_string())?;
    let snip_re = regex::Regex::new(
        r#"(?s)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#,
    )
    .map_err(|e| e.to_string())?;

    let snippets: Vec<String> = snip_re
        .captures_iter(&html)
        .map(|c| strip_tags(&c[1]))
        .collect();
    let mut hits = Vec::new();
    for (i, cap) in link_re.captures_iter(&html).enumerate() {
        if i >= 10 {
            break;
        }
        let href = cap[1].to_string();
        let url = decode_ddg_href(&href);
        if url.is_empty() || url.starts_with("https://duckduckgo.com") {
            continue;
        }
        hits.push(RawHit {
            title: strip_tags(&cap[2]),
            url,
            snippet: snippets.get(i).cloned().unwrap_or_default(),
        });
    }
    if hits.is_empty() {
        return Err("no_results: search returned no parseable results (DDG markup may have changed or the request was throttled)".to_string());
    }
    Ok(hits)
}

/// Decode DDG's `/l/?uddg=<urlencoded>` redirect wrappers.
fn decode_ddg_href(href: &str) -> String {
    if let Some(rest) = href.strip_prefix("//duckduckgo.com/l/?uddg=") {
        return percent_decode(rest);
    }
    if href.starts_with("//") {
        return format!("https:{}", href);
    }
    href.to_string()
}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h: String = chars.by_ref().take(2).collect();
            if let Ok(b) = u8::from_str_radix(&h, 16) {
                out.push(b as char);
                continue;
            }
            out.push('%');
            out.push_str(&h);
        } else if c == '+' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    // uddg payloads often carry a trailing &rut=... token; strip at first &.
    out.split('&').next().unwrap_or(&out).to_string()
}

fn urlencoding_simple(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn strip_tags(s: &str) -> String {
    let re = regex::Regex::new(r"(?s)<script.*?</script>|<style.*?</style>|<[^>]+>").unwrap();
    let t = re.replace_all(s, " ");
    let ws = regex::Regex::new(r"\s+").unwrap();
    ws.replace_all(&t, " ").trim().to_string()
}

/// COLLECT: fetch page text (best-effort, per-page timeout, 500KB cap).
async fn stage_collect(client: &reqwest::Client, hits: &[RawHit]) -> Vec<(RawHit, String)> {
    let mut out = Vec::new();
    for hit in hits.iter().take(6) {
        let text = async {
            let body = client
                .get(&hit.url)
                .header("User-Agent", UA)
                .timeout(std::time::Duration::from_secs(20))
                .send()
                .await?
                .bytes()
                .await?;
            let capped: Vec<u8> = body.iter().take(500_000).copied().collect();
            Ok::<String, reqwest::Error>(strip_tags(&String::from_utf8_lossy(&capped)))
        }
        .await
        .unwrap_or_default();
        if !text.trim().is_empty() {
            out.push((hit.clone(), text));
        }
    }
    out
}

/// Canonicalize a URL for dedupe: lowercase host, strip query/fragment.
fn canonical_url(u: &str) -> String {
    let u = u.trim().to_lowercase();
    let u = u.split('#').next().unwrap_or(&u).to_string();
    u.split('?').next().unwrap_or(&u).to_string()
}

fn split_sentences(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"(?<=[.!?])\s+").unwrap();
    re.split(text)
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() > 40 && s.len() < 600)
        .collect()
}

fn query_tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

fn sentence_score(sentence: &str, tokens: &[String]) -> f64 {
    let s = sentence.to_lowercase();
    let hits = tokens.iter().filter(|t| s.contains(t.as_str())).count();
    if tokens.is_empty() {
        0.0
    } else {
        hits as f64 / tokens.len() as f64
    }
}

async fn run_pipeline(
    app: &tauri::AppHandle,
    job_id: &str,
    query: &str,
) -> Result<ResearchStatus, String> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("http client failed: {}", e))?;
    let tokens = query_tokens(query);

    // SEARCH
    emit_update(app, job_id, "search", 0.1, "searching the web");
    let hits = stage_search(&client, query).await?;
    let mut st = get_status(job_id).ok_or("job vanished")?;
    st.stage = "collect".to_string();
    st.progress = 0.3;
    set_status(st);

    // COLLECT
    emit_update(app, job_id, "collect", 0.3, "fetching result pages");
    let pages = stage_collect(&client, &hits).await;
    if pages.is_empty() {
        return Err("collect failed: no pages could be fetched".to_string());
    }
    emit_update(app, job_id, "deduplicate", 0.5, "removing duplicates");

    // DEDUPLICATE (by canonical URL)
    let mut seen: HashSet<String> = HashSet::new();
    let mut unique: Vec<(RawHit, String)> = Vec::new();
    for (hit, text) in pages {
        let c = canonical_url(&hit.url);
        if seen.insert(c) {
            unique.push((hit, text));
        }
    }

    // EXTRACT + CITE
    emit_update(app, job_id, "extract", 0.65, "extracting key sentences");    let mut results: Vec<CitedResult> = Vec::new();
    for (i, (hit, text)) in unique.iter().enumerate() {
        let mut scored: Vec<(f64, String)> = split_sentences(text)
            .into_iter()
            .map(|s| (sentence_score(&s, &tokens), s))
            .filter(|(sc, _)| *sc > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let key_sentences: Vec<String> =
            scored.into_iter().take(3).map(|(_, s)| s).collect();
        results.push(CitedResult {
            index: i + 1,
            title: hit.title.clone(),
            url: hit.url.clone(),
            snippet: hit.snippet.clone(),
            key_sentences,
        });
    }

    // COMPARE: pairwise Jaccard overlap of top terms between sources.
    emit_update(app, job_id, "cite", 0.75, "attaching citations");
    emit_update(app, job_id, "compare", 0.8, "comparing sources");
    let mut comparisons: Vec<String> = Vec::new();
    let top_terms: Vec<HashSet<String>> = results
        .iter()
        .map(|r| {
            r.key_sentences
                .join(" ")
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| t.len() > 3)
                .take(40)
                .map(|t| t.to_string())
                .collect()
        })
        .collect();
    for i in 0..results.len() {
        for j in (i + 1)..results.len() {
            let (a, b) = (&top_terms[i], &top_terms[j]);
            let inter = a.intersection(b).count() as f64;
            let union = a.union(b).count().max(1) as f64;
            let jaccard = inter / union;
            comparisons.push(format!(
                "Sources [{}] and [{}] agree {:.0}% on key terms.",
                i + 1,
                j + 1,
                jaccard * 100.0
            ));
        }
    }

    // SYNTHESIZE (extractive): rank every key sentence globally by query
    // overlap and join the top ones, each carrying its source citation.
    emit_update(app, job_id, "synthesize", 0.9, "synthesizing findings");
    let mut all: Vec<(f64, usize, String)> = Vec::new();
    for r in &results {
        for s in &r.key_sentences {
            all.push((sentence_score(s, &tokens), r.index, s.clone()));
        }
    }
    all.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut synthesis = String::from("Extractive summary (assembled from source sentences, not LLM-generated):\n");
    for (score, idx, s) in all.iter().take(8) {
        if *score <= 0.0 {
            continue;
        }
        synthesis.push_str(&format!("\n- {} [{}]", s, idx));
    }
    if all.iter().take(8).all(|(sc, _, _)| *sc <= 0.0) {
        synthesis.push_str("\n- No sentences matched the query strongly; see sources below.");
    }

    let mut st = get_status(job_id).ok_or("job vanished")?;
    st.results = results;
    st.synthesis = synthesis;
    st.comparisons = comparisons;
    Ok(st)
}
