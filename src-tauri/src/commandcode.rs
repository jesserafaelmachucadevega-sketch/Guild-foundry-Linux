// SPDX-License-Identifier: Apache-2.0
// commandcode.rs — the Command Code CLI as a provider (headless `cmd -p` runs).
//
// Command Code is a coding agent, not a chat model: every headless run loads a
// full coding-agent context before it answers. Measured on this machine: a
// one-word reply cost 14,834 input tokens. A run may use tools and loop for up
// to 100 turns, and the answer is printed once at the end — there is no token
// stream to relay. That is why the provider is labelled "coding only" in the
// UI, and why this module exists: run the CLI safely (hard deadline, pipes
// drained, cancellation, process killed) and report its real usage so the cost
// ledger is never quiet about what a run cost.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Provider kind stored in `providers.kind`.
pub const PROVIDER_KIND: &str = "command-code";
/// Marked in `providers.base_url`: this kind has no endpoint to poll.
pub const BASE_URL: &str = "cli://command-code";
/// Ceiling for one headless run. A coding run can legitimately take minutes;
/// nothing may pin a worker thread forever.
const RUN_DEADLINE: Duration = Duration::from_secs(600);
const LIST_DEADLINE: Duration = Duration::from_secs(60);
const VERSION_DEADLINE: Duration = Duration::from_secs(15);
const TICK: Duration = Duration::from_millis(200);

/// One finished headless run, as reported by the CLI's final `result` frame.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlessRun {
    pub text: String,
    /// True when Command Code stopped at its own turn limit: `text` is partial.
    pub partial: bool,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub duration_ms: u64,
    pub session_id: Option<String>,
}

/// Resolve the CLI: explicit override, then the usual install locations, then
/// a bare name so the OS PATH can answer.
pub fn binary() -> String {
    if let Ok(custom) = std::env::var("GFA_CMD_BINARY") {
        if !custom.trim().is_empty() {
            return custom;
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".npm-global/bin/cmd"));
        candidates.push(home.join(".local/bin/cmd"));
        candidates.push(home.join(".bun/bin/cmd"));
    }
    candidates.push(PathBuf::from("/usr/local/bin/cmd"));
    candidates.push(PathBuf::from("/usr/bin/cmd"));
    for c in candidates {
        if c.is_file() {
            return c.to_string_lossy().to_string();
        }
    }
    "cmd".to_string()
}

struct Captured {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
}

/// Spawn, drain both pipes on their own threads (a full pipe would deadlock the
/// child), poll for exit, and enforce the deadline. Cancellation kills the
/// child — the only way to interrupt a run that may take minutes.
fn run_capture(
    bin: &str,
    args: &[&str],
    deadline: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Captured, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .envs(crate::terminal::sanitized_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start the Command Code CLI ('{}'): {} [ECLI]", bin, e))?;
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| "Command Code CLI stdout unavailable [ECLI]".to_string())?;
    let mut err = child
        .stderr
        .take()
        .ok_or_else(|| "Command Code CLI stderr unavailable [ECLI]".to_string())?;
    let out_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    });
    let err_t = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    });

    let started = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    loop {
        if let Some(flag) = cancel {
            if flag.load(Ordering::Relaxed) {
                cancelled = true;
                let _ = child.kill();
                break;
            }
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => return Err(format!("could not wait for the Command Code CLI: {} [ECLI]", e)),
        }
        if started.elapsed() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(TICK);
    }
    let code = child.wait().ok().and_then(|s| s.code());
    Ok(Captured {
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
        code,
        timed_out,
        cancelled,
    })
}

/// Version string, when the CLI answers at all — used to report what is
/// installed instead of guessing.
pub fn version() -> Option<String> {
    let cap = run_capture(&binary(), &["--version"], VERSION_DEADLINE, None).ok()?;
    let line = cap.stdout.lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// Map the CLI's documented exit codes to sentences a person can act on.
fn cli_error(code: Option<i32>, stderr: &str) -> String {
    let why = match code {
        Some(3) => "Command Code is not signed in — run `cmd` once in a terminal and finish the login (exit 3).",
        Some(4) => "Command Code's own permission rules denied the run (exit 4).",
        Some(5) => "Command Code rate limit reached — wait a moment and try again (exit 5).",
        Some(6) => "Network failure while talking to Command Code (exit 6).",
        Some(7) => "Command Code's API server returned an error (exit 7).",
        Some(8) => "The run hit its turn limit before producing a final answer (exit 8).",
        Some(9) => "The model produced no response (exit 9).",
        Some(10) => "Not enough Command Code credits for this model (exit 10).",
        Some(130) => "The run was interrupted (exit 130).",
        Some(_) => "Command Code exited with an error.",
        None => "Command Code was stopped before it finished.",
    };
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines.iter().rev().take(3).cloned().collect::<Vec<_>>();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join(" ");
    if tail.trim().is_empty() {
        format!("{} [ECLI]", why)
    } else {
        format!("{} [ECLI] — {}", why, tail.trim())
    }
}

/// Lines that are prose, not models (the footer prints examples and a doc link).
const LIST_PROSE: &[&str] = &[
    "Available models",
    "cmd ",
    "Docs:",
    "Pass the full id",
    "Decision models",
];

/// Byte offset of the column gap in a model row: the id, a run of two or more
/// spaces, then a description. Single-space lines are headings or prose.
fn column_gap(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let gap_start = i;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i - gap_start >= 2 {
        Some(gap_start)
    } else {
        None
    }
}

/// Parse `cmd --list-models`: section headings, then one `id   description` row
/// per model. The ids come straight from the CLI — bare ids like
/// `claude-sonnet-4-6` are as real as `moonshotai/kimi-k2.5`, so nothing is
/// rewritten here.
pub fn parse_models(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || LIST_PROSE.iter().any(|p| trimmed.starts_with(p)) {
            continue;
        }
        let Some(gap) = column_gap(trimmed) else { continue };
        let id = trimmed[..gap].trim();
        if id.is_empty() {
            continue;
        }
        let desc = trimmed[gap..].trim();
        // The decision-model row embeds a whole example command; keep the part
        // that describes the model.
        let desc = desc
            .split(" — Run it headlessly")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !out.iter().any(|(existing, _)| existing == id) {
            out.push((id.to_string(), desc));
        }
    }
    out
}

/// Parse the NDJSON stream: the last `result` frame is the answer. Unknown
/// frame types are ignored on purpose — the event list grows over time, and
/// only the result line is contractual.
pub fn parse_run(ndjson: &str) -> Result<HeadlessRun, String> {
    let mut result: Option<serde_json::Value> = None;
    for line in ndjson.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t) else {
            continue;
        };
        if v.get("type").and_then(|x| x.as_str()) == Some("result") {
            result = Some(v);
        }
    }
    let v = result.ok_or_else(|| {
        "Command Code returned no result frame — the run was interrupted [ECLI].".to_string()
    })?;
    let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("error");
    if subtype == "error" {
        let detail = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("no detail given");
        return Err(format!("Command Code could not complete the run: {} [ECLI]", detail));
    }
    let text = v
        .get("finalText")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("Command Code finished without an answer [ECLI].".to_string());
    }
    let usage = v.get("usage").cloned().unwrap_or_else(|| serde_json::json!({}));
    let count = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    Ok(HeadlessRun {
        text,
        partial: subtype == "max_turns",
        prompt_tokens: count("inputTokens"),
        completion_tokens: count("outputTokens"),
        duration_ms: v.get("durationMs").and_then(|x| x.as_u64()).unwrap_or(0),
        session_id: v
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
    })
}

/// Run one headless turn. `model` empty/None lets the CLI use the model the
/// user configured for Command Code itself.
pub fn run_headless(
    prompt: &str,
    model: Option<&str>,
    cancel: Option<&AtomicBool>,
) -> Result<HeadlessRun, String> {
    let bin = binary();
    let mut args: Vec<String> = vec![
        "-p".to_string(),
        prompt.to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        // Onboarding is an interactive interview; a headless run must never
        // stop to ask a question nobody can answer.
        "--skip-onboarding".to_string(),
    ];
    if let Some(m) = model.map(|m| m.trim()).filter(|m| !m.is_empty()) {
        args.push("-m".to_string());
        args.push(m.to_string());
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let cap = run_capture(&bin, &refs, RUN_DEADLINE, cancel)?;
    if cap.cancelled {
        return Err("Stream cancelled [CANCELLED]".to_string());
    }
    if cap.timed_out {
        return Err(format!(
            "Command Code did not finish within {} minutes and was stopped [ECLI_TIMEOUT].",
            RUN_DEADLINE.as_secs() / 60
        ));
    }
    if cap.stdout.trim().is_empty() {
        // No frames at all: the exit code is the only signal available.
        return Err(cli_error(cap.code, &cap.stderr));
    }
    parse_run(&cap.stdout)
}

/// The model list the signed-in account can use.
pub fn list_models() -> Result<Vec<(String, String)>, String> {
    let cap = run_capture(&binary(), &["--list-models"], LIST_DEADLINE, None)?;
    if cap.timed_out {
        return Err(
            "The Command Code CLI did not answer `--list-models` within 60 s [ECLI_TIMEOUT].".to_string(),
        );
    }
    if cap.code != Some(0) {
        return Err(cli_error(cap.code, &cap.stderr));
    }
    let models = parse_models(&cap.stdout);
    if models.is_empty() {
        return Err(
            "The Command Code CLI listed no models — run `cmd` once in a terminal and sign in [ECLI_AUTH]."
                .to_string(),
        );
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Column-for-column copy of the CLI's real output, padding included.
    const LIST_FIXTURE: &str = "Available models  ·  92 models\n\nOpen Source\n\ndeepseek/deepseek-v4-flash             fast hybrid-attention reasoning (default)\nmoonshotai/kimi-k2.5                   long-horizon coding with vision\n\nAnthropic\n\nclaude-sonnet-4-6                      older Sonnet, still fast & capable\n\nPass the full id, or just the short name after the last \"/\":\ncmd --model moonshotai/kimi-k2.5\n\nDocs:  https://commandcode.ai/docs/reference/cli/models\n\nDecision models (headless only)\ntypesafe/jev  typed questions in, probabilities out, everything else free — Run it headlessly: cmd -p '{\"state\": \"...\"}' -m typesafe/jev\n";

    #[test]
    fn parses_model_ids_and_descriptions() {
        let models = parse_models(LIST_FIXTURE);
        let ids: Vec<&str> = models.iter().map(|(id, _)| id.as_str()).collect();
        // Bare ids (no slash) are real: nothing is rewritten or dropped.
        assert_eq!(
            ids,
            vec![
                "deepseek/deepseek-v4-flash",
                "moonshotai/kimi-k2.5",
                "claude-sonnet-4-6",
                "typesafe/jev"
            ]
        );
        assert_eq!(models[0].1, "fast hybrid-attention reasoning (default)");
        assert_eq!(models[2].1, "older Sonnet, still fast & capable");
        // Headings and footer prose are never mistaken for models.
        assert!(!models.iter().any(|(id, _)| id == "Open Source" || id == "Anthropic"));
        assert!(!models.iter().any(|(id, _)| id.starts_with("cmd")));
        assert!(!models.iter().any(|(id, _)| id.contains("docs")));
        // The decision-model description keeps its sentence, not its example command.
        assert!(models[3].1.starts_with("typed questions in, probabilities out"));
        assert!(!models[3].1.contains("cmd -p"));
    }

    #[test]
    fn result_frame_carries_text_partial_flag_and_usage() {
        let ok = r#"{"type":"event","event":{"type":"run_start","sessionId":"abc"}}
{"type":"event","event":{"type":"turn_start","turnNumber":1}}
{"type":"result","subtype":"success","sessionId":"abc","stopReason":"end_turn","usage":{"inputTokens":14834,"outputTokens":25,"cacheReadTokens":5760,"cacheWriteTokens":0},"durationMs":3745,"finalText":"ok"}"#;
        let run = parse_run(ok).expect("a success frame parses");
        assert_eq!(run.text, "ok");
        assert_eq!(run.prompt_tokens, 14834);
        assert_eq!(run.completion_tokens, 25);
        assert_eq!(run.duration_ms, 3745);
        assert!(!run.partial);
        assert_eq!(run.session_id.as_deref(), Some("abc"));

        let maxed = r#"{"type":"result","subtype":"max_turns","usage":{"inputTokens":9,"outputTokens":3},"durationMs":1,"finalText":"partial"}"#;
        let run = parse_run(maxed).expect("max_turns is a partial answer, not an error");
        assert!(run.partial);
        assert_eq!(run.text, "partial");
    }

    #[test]
    fn error_and_empty_runs_fail_loudly() {
        let err = r#"{"type":"result","subtype":"error","usage":{"inputTokens":0,"outputTokens":0},"durationMs":12,"finalText":"","error":"not authenticated"}"#;
        let msg = parse_run(err).expect_err("error frames must never look like answers");
        assert!(msg.contains("not authenticated"), "{}", msg);

        let empty = r#"{"type":"result","subtype":"success","usage":{},"durationMs":1,"finalText":"   "}"#;
        assert!(parse_run(empty).is_err());

        // Malformed lines are ignored, but a stream with no result frame fails.
        assert!(parse_run("not json\n{\"type\":\"event\"}\n").is_err());
    }

    #[test]
    fn exit_codes_become_sentences() {
        assert!(cli_error(Some(3), "").contains("not signed in"));
        assert!(cli_error(Some(10), "").contains("credits"));
        assert!(cli_error(Some(6), "getaddrinfo ENOTFOUND api.commandcode.ai").contains("ENOTFOUND"));
        assert!(cli_error(None, "").contains("stopped"));
    }

    /// Live round trip against the real CLI. Costs a real request (~15k input
    /// tokens): `cargo test --lib commandcode -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_headless_run_returns_text_and_usage() {
        let run = run_headless("Reply with exactly one word: ok", None, None).expect("live run");
        assert!(!run.text.is_empty());
        assert!(run.prompt_tokens > 0, "usage must be reported");
    }

    /// Parse a genuine `cmd --list-models` capture. No fixture is baked in on
    /// purpose — the catalog changes; point the test at a fresh capture:
    /// `CC_MODELS_FIXTURE=/tmp/cc-models.txt cargo test commandcode::tests::parses_a_real_capture -- --ignored`
    #[test]
    #[ignore]
    fn parses_a_real_capture_when_provided() {
        let Ok(path) = std::env::var("CC_MODELS_FIXTURE") else {
            return;
        };
        let text = std::fs::read_to_string(&path).expect("capture readable");
        let models = parse_models(&text);
        assert!(models.len() > 50, "expected a large catalog, got {}", models.len());
        assert!(
            models.iter().any(|(id, _)| id == "claude-sonnet-4-6"),
            "bare ids (no slash) must survive"
        );
        for (id, desc) in &models {
            assert!(!id.contains(' ') && !id.is_empty(), "bad id: {:?}", id);
            assert!(!desc.contains("Available models"), "prose leaked into {:?}", id);
        }
    }
}
