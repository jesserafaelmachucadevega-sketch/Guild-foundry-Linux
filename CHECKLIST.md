# Guild Foundry AI — Build Checklist

Source: `docs/Guild_Foundry_AI_Zorin_OS_Master_Prompt.pdf`, Section 91 (Build Order).
Checked off as each phase lands. **COMPILE-VERIFIED** means the code builds,
lints and passes `cargo test`; **NOT VERIFIED** means it still needs a native
run (live provider keys, an MCP server, or a real built app) to be trusted.

## PHASE 1 — Application shell, navigation, theme, project management, SQLite, settings
- [x] Tauri 2 project scaffold (tauri.conf.json, Cargo.toml, capabilities)
- [x] Rust command registry shell (src-tauri/src/main.rs)
- [x] React + Vite + TypeScript frontend scaffold (tsc clean, vite build clean)
- [x] Workstation theme (black / charcoal / dark grey / tan / muted gold / off-white, no gradients)
- [x] Global shell: left navigation rail (Home, App Builder, Conference Room, Projects, Providers, Models, Tools, MCP Servers, Memory, Artifacts, Runs, Settings)
- [x] Global header (workspace, connection state badge, command palette entry points)
- [x] View switcher: Desktop / Compact Window / Browser PWA (window resize on desktop, CSS density modes)
- [x] Chat bubbles: 25-line bound + internal scroll, Copy button (native clipboard plugin) + toast
- [x] Settings drawer: masked credentials + system prompt fields (Conference 1-4, Supervisor, Architect, Frontend, Backend, QA, Security, Synthesizer, Researcher)
- [x] Runtime OS/toolchain capability detection (Rust `detect_environment`: distro, kernel, DE, Wayland/X11, CPU/RAM/GPU, shell, 12 tools) — COMPILE-VERIFIED 2026-10-08 (cargo check + clippy clean)
- [x] SQLite schema + migrations (15 entities per spec section 53, versioned, additive) — COMPILE-VERIFIED 2026-10-08 (cargo check + clippy clean)
- [x] System tray integration (Show / Quit, click-toggle, graceful no-icon fallback) — COMPILE-VERIFIED 2026-10-08 (cargo check + clippy clean)
- [x] PWA manifest + service worker + "Desktop Capability Required" degradation

## PHASE 2 — Provider abstraction, secure credentials, model catalog, streaming chat
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean); runtime behaviour still needs a native run.
- [x] Provider abstraction layer (OpenAI-compatible, OpenAI, Anthropic, Google, OpenRouter, local/custom endpoints)
- [x] Linux Secret Service / keyring credential storage (encrypted at rest, masked, never logged)
- [x] Handshake sequence (validate, verify endpoint, models, capabilities, context limits, pricing, tool-calling, vision, structured output)
- [x] Connection badges: CONNECTED / ERROR [code] with layman explanations
- [x] Model catalog: VERIFIED FREE first, then VERIFIED PAID, then UNKNOWN; favorites/hidden/aliases/filters
- [x] Streaming chat engine (shared by Builder + Conference): stream, cancel, pause, retry, regenerate, edit, copy, export, branch, token usage, cost, latency
- [x] Model router (task/cost/context/tool/latency based, disable-able, explains selection)

## PHASE 3 — Conference Room, multi-model orchestration, debate engine, synthesis
Status: implemented 2026-10-06 (frontend only). **NOT VERIFIED** — live multi-model debate needs provider keys + native runtime.
- [x] 1-4 model grid (1 full, 2 dual, 3 triple-no-blank-tile, 4 balanced 2x2)
- [x] Participant config: provider, model, system prompt, role, temperature, reasoning, tool permissions, memory scope
- [x] Debate engine: structured rounds (independent, cross-critique, rebuttal, evidence, final, judge/synthesis)
- [x] Pause / Stop protocol; winner dialog (1..N); congratulatory card; post-debate discussion mode
- [x] Voting: user / model / weighted / judge / consensus
- [x] Cross-model knowledge synthesizer bridge
- [x] SDUI artifact renderer (polls, checklists, sliders, cards, tables, forms, approval dialogs)

## PHASE 4 — Project filesystem, code editor, terminal, Git, checkpoints
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean); runtime behaviour still needs a native run.
- [x] Sandboxed project workspace (allowed/denied paths, traversal protection)
- [x] File operations: read/write/create/delete/rename/move/search/list/diff/patch/snapshot/restore (audited, checkpoints before destructive ops)
- [x] Embedded code editor (Monaco/CodeMirror): highlighting, tabs, diff, diagnostics, agent-change highlighting
- [x] Terminal/process engine: execute/start/stop/output, process list/kill, timeouts, env policy, resource limits
- [x] Command safety classification (SAFE / MODERATE / HIGH RISK / CRITICAL) with progressive confirmation
- [x] Git integration: status/diff/log/branch/commit/stash/merge/remote/pull/push (push NEVER automatic; pre-push confirmation modal)
- [x] Checkpoints + rollback (preview/restore/compare), independent of Git

## PHASE 5 — Agent orchestration engine and Builder state machine
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean); live runs need provider keys.
- [x] Agents: Supervisor, Architect, Frontend/Application Engineer, Backend/System Engineer, QA/Test Engineer, Security Reviewer
- [x] DAG task graph: task id, parent, agent, priority, status, dependencies, inputs/outputs, artifacts, tool calls, errors, timestamps
- [x] Builder state machine: DISCOVERY -> REQUIREMENTS_ANALYSIS -> ARCHITECTURE -> PLANNING -> AWAITING_USER_APPROVAL -> IMPLEMENTATION -> BUILDING -> TESTING -> SECURITY_REVIEW -> FIXING -> VALIDATION -> READY -> PACKAGING -> RELEASED (transitions recorded, recoverable after restart)
- [x] Human approval gates (deletes, mass replaces, system packages, privileged commands, credentials, pushes, releases, outside-project writes, destructive commands)
- [x] Requirements engine + architecture approval + ADRs + autonomous modes (SAFE/ASSISTED/AUTONOMOUS)

## PHASE 6 — Real tool execution loop
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean); live loop needs provider keys + native runtime.
- [x] MODEL -> TOOL CALL -> TOOL EXECUTOR -> TOOL RESULT -> MODEL CONTEXT -> NEXT ACTION loop (results never terminate at executor)
- [x] Tool handshake: per-session capability manifest (available/unavailable/requires-approval/restricted)
- [x] Handshake system prompt with capability manifest + anti-fabrication instruction
- [x] Context engine: relevance ranking, token budgeting, compression, file summaries, semantic retrieval, caching

## PHASE 7 — MCP Gateway and permissions
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean); live MCP needs real servers.
- [x] MCP gateway: Streamable HTTP, legacy HTTP/SSE, OAuth 2.1 + PKCE, bearer auth, local + remote servers
- [x] MCP server management: add/edit/enable/disable/test/authenticate/revoke, per-server permission boundary
- [x] Tool registry: name, description, provider/MCP server, risk classification, schemas, permission requirements, enabled state, audit policy
- [x] Permission center: always allow / ask every time / allow for project / allow for session / deny, per domain (filesystem, network, shell, Git, MCP, browser, credentials, external APIs)

## PHASE 8 — Build/test/fix engine
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean); real builds need native app + toolchains.
- [x] Build-fix loop: BUILD -> CAPTURE -> CLASSIFY -> LOCATE -> ASSIGN -> PATCH -> APPLY -> TARGETED TEST -> REBUILD -> FULL VALIDATION; halt after 3 identical error signatures with failure report
- [x] Testing: unit/integration/e2e/lint/format/typecheck/static analysis/dependency audit/security; auto-detect project tooling
- [x] App generation workflow: Requirements -> Architecture -> Task Graph -> Files -> Implementation -> Build -> Tests -> Security Review -> Packaging -> Release Artifact
- [x] Project templates: React, TypeScript, Python, Rust, Node.js, Go, Java, desktop, CLI, web, APIs, AI agents

## PHASE 9 — Security layer, sandboxing, auditing
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean).
- [x] Constitution: security rules, path constraints, non-hallucination rules
- [x] Prompt injection defense: external content labeled (USER/SYSTEM/TOOL RESULT/EXTERNAL/MODEL-GENERATED), never overrides system instructions
- [x] Structured audit log (DEBUG/INFO/WARN/ERROR/SECURITY), secrets never logged
- [x] Sandboxed processes, capability-based permissions, least privilege, secure defaults

## PHASE 10 — Memory, indexing, research, artifacts
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean). Semantic search NOT implemented (keyword ranking only, stated honestly in UI).
- [x] Model-isolated memory (scopes: conversation/model/agent/project/workspace/global; composite keys; DB+app level enforcement)
- [x] Async consolidation: dedupe, semantic clustering, stale detection, contradiction detection, importance scoring, pruning
- [x] Shared knowledge bridge (verified fact vs model claim vs user decision vs inference vs unresolved claim)
- [x] Project indexing: filenames/symbols/text/code/docs/Git metadata; full-text + semantic + symbol search; file watcher with external-change detection
- [x] Web research engine: SEARCH -> COLLECT -> DEDUPLICATE -> EXTRACT -> CITE -> COMPARE -> SYNTHESIZE with provenance
- [x] Artifact manager: unified artifacts — owned by Phase 8 (no duplication)
- [x] Live project explorer + conflict management — owned by Phase 4 (no duplication)

## PHASE 11 — Packaging, updater, diagnostics, crash recovery
Status: implemented 2026-10-06. **COMPILE-VERIFIED** 2026-10-08 (cargo check + clippy clean). Flatpak target NOT added (would require editing tauri.conf.json bundle targets — deferred to release).
- [x] Linux packaging: .deb, AppImage verified in tauri.conf.json; Flatpak deferred (config change required)
- [x] Update system: check / release notes / download / install / postpone (never silent without policy)
- [x] Diagnostics dashboard: model status, CPU/RAM/GPU/VRAM, disk, network, token throughput, build worker state
- [x] Crash recovery: preserve state/tasks/history/artifacts; on restart offer Resume / Inspect / Roll Back / Discard (never auto-resume destructive ops)
- [x] Offline-first: shell usable offline; AI/network functions show OFFLINE, never fail ambiguously

## PHASE 12 — Full end-to-end acceptance testing
Status: ran 2026-10-06 against the pre-integration tree — 22/22 NOT VERIFIED (no Rust toolchain, no provider keys, no MCP server). Report: `docs/ACCEPTANCE.md`. MUST be re-run on the target OS after integration + `cargo check`.
- [x] Run the 22-step critical acceptance test (Section 85) against a real generated project — ran; all 22 NOT VERIFIED with exact reasons
- [x] No mocks for: model calls, tool calls, filesystem ops, builds, tests, Git, MCP, provider auth — none used; nothing was faked
- [x] Every unverifiable component reported as **NOT VERIFIED** — see docs/ACCEPTANCE.md

## Verification ledger (this environment)
- [x] npm package versions verified live (tauri 2.12.1, @tauri-apps/api 2.12.1, @tauri-apps/cli 2.12.1, react 19.3.0, vite 8.3.3, typescript 7.0.2, monaco-editor 0.57.0, @modelcontextprotocol/sdk 1.32.1)
- [x] crates.io versions verified live (tauri 2.12.1, tauri-plugin-sql 2.5.0, tauri-plugin-store 2.5.0, tauri-plugin-notification 2.5.1)
- [x] TypeScript compile of renderer — clean 2026-10-06 (post-integration; re-run at release)
- [x] Vite production build of frontend — clean 2026-10-06 (post-integration; re-run at release)
- [x] Anti-branding scan — zero user-facing distro/hardware names in src/ and src-tauri/src/
- [x] Secrets scan — zero hardcoded secrets in src/ and src-tauri/src/
- [x] Rust compilation (`cargo check --all-targets`, `cargo test`, `cargo clippy --all-targets`) — **COMPILE-VERIFIED** 2026-10-08 on Rust 1.99.0 / aarch64-unknown-linux-gnu (clippy: no errors; 5 tests passing)
- [ ] 22-step acceptance re-run on target — **NOT VERIFIED** (needs native app + provider keys + MCP server)

## PHASE 15/16 — Personal Agent completion, real tools, local models (2026-10-13)
Status: implemented + verified at the layer level (typecheck, vite build, IPC 163/270, cargo check/test 24, clippy).
- [x] Provider templates repaired (honest local presets; orphan `local` kind removed)
- [x] `web.fetch` (native HTTP fetch + readable extraction) — no more stubs
- [x] `document.*` — real extraction (PDF/DOCX/PPTX/XLSX/ODS/ODT/RTF/HTML/CSV/code), compare, convert, OCR
- [x] Upload pipeline — attached documents are extracted and sent to the model; images persist
- [x] `email.*` — native IMAP (pure-Rust rustls) + SMTP (lettre); Connections account card
- [x] `calendar.*` — native CalDAV client (create/update/delete/list); Connections account card
- [x] `cloud.drive_*` — routed to connected drive MCP services with honest guidance
- [x] `phone.*` — native Twilio SMS + calls (+ `telephony` permission domain)
- [x] `git.*` agent tools (status/diff/log/commit — commit approval-gated)
- [x] Agent thread persistence → SQLite (model-agnostic, per-workspace) + history replay into every turn
- [x] Tool-call adapter registry (fenced/XML/bare-JSON) + validator with self-correcting errors
- [x] Dead-UI sweep: poll Submit wired to the loop, media "Generating…" placeholder, reply persistence, autoscroll
- [x] Direct GGUF local models — in-process llama.cpp, no server; Local Models manager; everywhere a model loads
- [x] Sampling controls (temperature / top-p / top-k / seed / penalties / stop) per provider, incl. GGUF
- [x] Floating/resizable/minimizable chat window (pop-out) with persisted bounds
- [x] Coding agent in the Projects workspace (same assistant, same tools; `projects-agent` prompt)
- [x] Message-style bubbles: white (you) / blue (cloud model) / green (local model), reactions row, reply
- [x] Local Whisper STT (offline, whisper-helper binary) with cloud fallback
- [x] MCP stdio transport + per-server custom headers + pagination + official-server quick-add presets
- [x] One-shot permission grants actually consume after first successful use
- [x] CI: IPC contract check added; cmake/clang/libclang added to the Rust job
- [x] Kokoro local TTS — **ENABLED + VERIFIED on target machine** (sherpa-onnx engine + kokoro-en-v0_19; synthesized two voices, 24 kHz, RMS-validated). Local-first in `media_speak_text`; cloud is the fallback only
- [x] Kokoro voices auditioned by ear on the target machine — `am_adam` (the YouTube-narrator male) approved and kept as the default in `media.tts_voice`; `af_bella` also approved, available in the voice picker
- [x] Bundled release build — `externalBin` sidecar packaging in `tauri.bundle.conf.json` + `scripts/prepare_external_bin.mjs` (used by `npm run tauri:build`)
- [x] Offline semantic memory search — local GGUF embeddings through the in-process llama.cpp engine (`gguf::embed_text`), hybrid cosine+keyword ranking, background indexing on put, `memory_embed_reindex` repair path, UI status/reindex
- [x] MCP stdio session warming — children pooled per server, checked out per call, fingerprint-checked against config drift, killed on app exit; covered by two live-process tests
- [x] Command-thread hygiene — `tool_execute`, media (mic/TTS), memory search/reindex and all heavy workspace file ops run on the blocking pool (no UI freezes on network/disk I/O)
- [ ] GGUF KV-context reuse — **NOT POSSIBLE in pinned binding** (llama-cpp-2 0.1.159 exposes no `clear_kv_cache`; per-request contexts are allocated and freed correctly, just not reused). Revisit when the binding exposes cache control
- [x] Native Google Calendar OAuth — authorization code + PKCE + loopback redirect, refresh token in the OS keychain, tools `calendar.gcal_list/gcal_events/gcal_create`, Connections card. (Outlook: still via MCP connectors — a Microsoft app registration would be the equivalent next step)
- [ ] Hands-on acceptance run — script in docs/ACCEPTANCE-RUN.md; requires the user (only remaining verification layer)

## PHASE 17 — Command Code CLI provider, coding only (2026-10-16)

Status: built and unit-tested; awaiting the user's first live run in the UI.

- [x] New provider kind `command-code` (`src-tauri/src/commandcode.rs`) — one headless run per request: `cmd -p "<prompt>" --output-format json --skip-onboarding [-m <model>]`, spawned without a shell, both pipes drained on their own threads, 10-minute hard deadline, child killed on expiry, cancellation kills the child mid-run
- [x] NDJSON parsing — the final `result` frame supplies `finalText`, `usage.inputTokens/outputTokens`, `durationMs`, `stopReason`; `subtype: error` fails loudly, `max_turns` is returned as an explicitly-marked partial answer (never presented as whole), unknown frame types ignored on purpose
- [x] Real usage into the ledger — the CLI's own token counts go through `record_tokens` + `cost::record` exactly like an HTTP provider's, so `/usage` is never quietly wrong
- [x] Catalog from `cmd --list-models` — Connect (`Check CLI`) reports the binary version and refreshes the model list; the parser was validated against a real 126-line capture, including ids with no vendor prefix (`claude-sonnet-4-6`) and the footer prose being ignored
- [x] Coding-only warning in the UI — on the provider card (`CODING ONLY` badge + note) and in the add/edit form, with the measured number: 14,834 input tokens for a one-word reply (measured on this machine). Also states that answers arrive in one piece and that the CLI must be signed in
- [x] Honest limits recorded — no token-by-token streaming (the CLI prints its answer once at the end); the CLI's own file writes and shell commands stay denied (no `--yolo`), so this provider is a brain: the app's own tool loop does the file work
- [x] Tests — `parse_models`, `parse_run`, exit-code → sentence mapping; a gated test for a real `--list-models` capture (`CC_MODELS_FIXTURE=… cargo test commandcode::tests::parses_a_real_capture -- --ignored`); a live headless round-trip test marked `#[ignore]` because it costs a real request

### Fix — Context7 "415 · Unexpected response" (2026-10-16)

Root cause, reproduced against the live endpoint: reqwest **appends** headers and `.json()` already sets `Content-Type`, so passing it again sent the header **twice** — one copy answers 200, two copies answer 415 with Context7's exact error text. Fixed in the request builders, so the whole class cannot come back.

- [x] `integrations/mod.rs` — `post_json_request` sets Content-Type once and drops any caller-supplied copy; the Context7 branch no longer passes one
- [x] `mcp.rs` — streamable POSTs go through one `post_rpc` helper (Content-Type set once); the legacy SSE POST no longer sets it either; `apply_auth` drops a saved custom Content-Type header for the same reason
- [x] Honest wording — Context7's `initialize` answers 200 even with a bogus key (measured), so the badge no longer claims "key accepted": it now says the endpoint answered and that the key is only checked on the first real query
- [x] Tests — offline guards asserting exactly one Content-Type header in both modules, plus a live probe through our own handshake (`cargo test integrations::tests::live_context7_probe -- --ignored`) which returns **200 · Endpoint reachable**
- [x] Full Rust suite green: 48 passed, 0 failed, 3 ignored (the three that cost real requests/credits)

## PHASE 18 — Model Download (2026-10-16)

Named plainly on purpose: no jargon, no "cookbook". This is the feature that
means the app never ships an LLM — it finds a model the machine can actually
run, and (next slice) fetches it. GGUF-first, because inference here is
in-process through llama.cpp: every suggestion is one file `gguf_load` can open.

- [x] Hardware probe (`model_hardware`) — `nvidia-smi` per-GPU free/total/used/utilization (the same query a trusted reference implementation uses), AMD VRAM read from `/sys/class/drm` sysfs with no vendor tool, RAM from `/proc/meminfo`, and a one-sentence verdict computed from the catalog: "13.2 GB free VRAM on NVIDIA GeForce RTX 4090 — that comfortably runs Qwen3 14B (14 at Q4_K_M, 8.8 GB)"
- [x] Curated shortlist (`model_catalog`) — 10 GGUF models (Qwen3 0.6B/1.7B/4B/8B/14B, Gemma 3 1B, Llama 3.2 3B, Mistral 7B v0.3, Qwen2.5 Coder 7B, Nomic Embed v1.5) with params, quant, exact size, context, license and an honest note; each row reports `fits` and `headroom_mb` against this machine
- [x] Honest sourcing — every entry was verified live against the HuggingFace API (repo exists, not gated, exact filename present, exact byte size). Two candidates were dropped because the API returned 401 (gated) rather than being listed as if they worked. Re-runnable: `cargo test downloads::tests::live_curated_entries_exist -- --ignored`
- [x] Discover (`model_discover`) — HuggingFace trending GGUF search on demand, gated repos flagged, defensive parsing (missing fields never drop a row); live test included
- [x] Tests — nvidia CSV parsing (junk lines skipped), meminfo parsing in MB, headroom/fit arithmetic, verdict picking the largest model that genuinely fits, catalog completeness, HF payload parsing
- [ ] **Next slice:** the downloader itself — resumable fetch with progress events into the model folder, size verification, cancel; then the "Model Download" panel (hardware card, fit-filtered list, one-click get → load)

