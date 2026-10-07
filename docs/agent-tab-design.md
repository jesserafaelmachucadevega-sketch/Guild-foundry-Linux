# Agent Tab — Design Document

**Status:** Design (not yet implemented)
**Date:** 2026-10-07
**Author:** Akaia (for Jesse / Guild Foundry coder)

## 1. Overview

A new top-level **Agent** tab in Guild Foundry: a mission-control view where the user
converses with a tool-calling agent backed by any configured model (e.g. DeepSeek via
OpenRouter, Hermes via Ollama). The tab composes infrastructure that already exists
(agent loop, tool executor, MCP/OAuth, approval gates) into one coherent surface, and
adds the one genuinely new subsystem: a **tool-call adapter registry** so the Foundry
speaks every model family's tool-call dialect instead of mandating a single format.

Mental model: the model is the pilot, the Foundry is the exoskeleton. This tab is the
cockpit.

## 2. UI Composition (the tab)

New nav item in `src/components/AppShell.tsx` (`NAV_ITEMS`): `{ id: 'agent', label: 'Agent' }`.

Four panes, composed mostly from existing components:

| Pane | Purpose | Reuse |
|---|---|---|
| Conversation pane | Chat with the agent | `ChatBubble.tsx`, `src/lib/chat.ts` (`streamChat`) |
| Activity trace | Live feed of every tool call: name, arguments, result, duration, approval state | New component; emits from `toolLoop.ts` `onEvent` (`tool_call`, `tool_result`) |
| Connector sidebar | Connected services with status, scopes, connect/disconnect | Extend `mcp/McpPanel.tsx`; add first-class connector cards (see §4) |
| Approval queue | Pending approvals for consequential actions | Reuse `builder/ApprovalGate.tsx` modal pattern inline |

Reads execute silently and appear in the trace. Writes, sends, deletes, and purchases
pause the loop and wait in the approval queue — the same contract the Builder's
`ApprovalGate` already enforces.

## 3. Agent Loop Wiring

`src/lib/toolLoop.ts` already implements the core loop:

```
MODEL -> TOOL CALL -> TOOL EXECUTOR -> TOOL RESULT -> MODEL CONTEXT -> NEXT ACTION
```

with `maxTurns`, approval gates, and user-input gates. The Agent tab drives this loop
in **interactive mode** (one user message starts/resumes the loop) rather than the
Builder's run-graph mode. No new loop semantics are required — only a new driver.

The existing `ToolHandshake` (`manifest` + `system_prompt` + `session_id`) is the seam
where per-model-family format instructions are injected (see §5).

## 4. Connectors

Each connector has three layers. The MCP panel (`mcp/McpPanel.tsx`, `PermissionCenter.tsx`)
already covers the bones; what's missing is the user-facing onboarding and the vault
discipline.

### 4.1 Auth layer
- One-click OAuth 2.0 flow per connector (Gmail, Outlook/Hotmail, GitHub, …): browser
  window → provider consent → authorization code → token exchange in the **Rust backend**.
- Tokens are stored in the OS keychain via Tauri (never `localStorage`, never the
  renderer process beyond what's needed to display connection status).

### 4.2 Credential vault — hard rules
1. Tokens **never** enter the model's context, system prompt, tool schemas, logs, or
   the activity trace.
2. The model only ever sees tool names and JSON schemas (e.g. `gmail.search(query)`).
3. The Rust backend attaches credentials server-side at execution time inside
   `tool_execute`.
4. The `security/` audit panel logs *that* a credential was used, never the value.

### 4.3 Tool layer
- Each connector exposes functions with JSON schemas; MCP servers are the preferred
  mechanism (adopt MCP client support and inherit the ecosystem's prebuilt servers).
- All connector tools normalize to the internal `ToolCall { tool, args }` shape.
- Built-in tools live in the Rust registry (`terminal.exec`, `media.generate_image`,
  …) and are available to every model — local or frontier — with no per-model work.

### 4.4 Permission layer
- Per-connector scopes (read-only vs read-write), configured in `PermissionCenter`.
- Per-action rules: reads auto-approved; sends/deletes/purchases/external posts
  require explicit user approval via the approval queue.
- Every tool belongs to a permission domain (`shell`, `mcp`, `media`, …) with three
  levels: Ask every time (default), Always allow, Deny. Cost-incurring tools
  (e.g. `media.generate_image`) default to Ask every time.

### 4.5 Media rendering contract
- `media.generate_image` returns `{ path, url, model, seed, bytes }`; `media.generate_video`
  returns `{ path, url, model, duration_secs, bytes }`. The conversation pane renders
  `output.path` as an image (`convertFileSrc`) or video (`<video>` tag) — local files,
  surviving remote URL expiry. Any future media tool follows the same contract:
  absolute local path in `output.path`, remote URL in `output.url`.

### 4.6 Interactive artifacts contract
- The agent creates polls, checklists, sliders, cards, stickies, and whiteboards with
  the `artifact.create` tool (domain `interaction`, risk Low). The tool validates the
  payload and returns `{ artifact: {...} }`.
- The Agent tab appends `result.output.artifact` to the current assistant message's
  `artifacts` array; `ArtifactRenderer` renders each kind inline (whiteboard notes are
  positioned by x/y percentage; the user can rearrange in a future pass).
- Artifacts persist with the conversation — no separate save step.


### 4.7 Browser-use tool (MCP-hosted, not bundled)
Decision (2026-10-07): no bundled Chromium — it would bloat the install. Browser
automation comes from a hosted Playwright MCP server connected via the Connections
tab, keeping the app lean. A native CDP driver remains a future option for tighter
permission integration, but it is not on the roadmap while the MCP path works.

### 4.8 Watching the agent browse
Yes — the user can watch the agent work a page, the same way they watch an
assistant's browser session today. Mechanism: every browser tool call returns a
screenshot; the Agent tab's activity trace pane renders them as a filmstrip in
action order, which reads as a live feed. True CDP screencast streaming is the
fancier future version; per-action screenshots deliver ~90% of the experience
with no extra architecture.

## 5. Tool-Call Adapter Registry (new subsystem)

### 5.1 Problem
`parseToolCalls()` in `src/lib/toolLoop.ts` currently understands exactly one dialect:
fenced ` ```tool_call ` JSON blocks (`{tool, args}`). Every model family emits tool
calls differently:

| Family | Dialect |
|---|---|
| OpenAI / DeepSeek / OpenAI-compatible | `tool_calls` array in the JSON response |
| Hermes 2 Pro | `<tool_call>{...}</tool_call>` XML-wrapped JSON |
| Hermes 3/4 (ChatML) | Structured JSON per ChatML tool template |
| Qwen / Llama tool variants | Family-specific JSON schemas |
| Anthropic | `tool_use` content blocks |

Mandating one format via the system prompt works until a model drifts off-format
mid-run — which they do. The Foundry must *recognize* each family's dialect, not just
dictate one.

### 5.2 Architecture: adapter registry + handshake instruction + validator

**Design guarantee: no matter which model family is loaded, the loop converges on
valid tool calls.** The three stages below are not optional layers — together they are
the mechanism that enforces this. A model that emits the wrong format is not an error
state; it is an expected event the loop is designed to absorb and correct.

```
model output
  → Stage 1: Handshake instructs the family's dialect (prompt-side)
  → Stage 2: Adapter parses that family's dialect → normalized ToolCall {tool, args}
  → Stage 3: Validator checks + repairs; failures bounce back as structured
              errors → model corrects itself on the next turn (correction loop)
  → Executor (tool_execute) only ever receives validated calls
```

**Stage 1 — Handshake (prompt-side).** The `ToolHandshake.system_prompt` includes the
tool-call format instructions *specific to the selected model family*, generated by
that family's adapter (`formatInstructions()`). The model must be told what to emit;
an uninstructed model won't know the convention. This is necessary but not sufficient —
models drift off-format mid-run, which is why Stages 2 and 3 exist.

**Stage 2 — Adapter (Foundry-side).** A registry of parsers, one per family, all
normalizing to the internal `ToolCall` shape. The executor never knows which family
produced the call. If the model's family is unknown or misconfigured, `detectAdapter()`
falls back to whichever adapter recognizes the output — the Foundry classifies the
dialect rather than failing.

**Stage 3 — Validator (correction loop).** This is the enforcement point:
1. *Repair* what is repairable: malformed JSON (single quotes, trailing text,
   unclosed tags), wrapped or double-encoded payloads.
2. *Reject* what is not: unknown tool names, missing required arguments. Rejection
   produces a structured error naming the problem and listing the valid tools.
3. *Bounce back*: the error is appended to the conversation as a `TOOL RESULT [error]`
   message and the loop continues. The model sees exactly what was wrong and re-emits
   the call correctly on the next turn. **The run never crashes on a bad tool call —
   it corrects.**

The invariant: `tool_execute` only ever receives validated, normalized calls, regardless
of which family produced them or how sloppy the raw output was.

### 5.3 Proposed TypeScript interface

```ts
// src/lib/toolAdapters.ts (new file)

import type { ToolCall } from './toolLoop';

/** One model family's tool-call dialect. */
export interface ToolCallAdapter {
  /** Stable id, e.g. 'openai', 'hermes2', 'hermes3', 'deepseek', 'qwen3', 'anthropic'. */
  id: string;
  /** Human label for the Models catalog. */
  label: string;
  /**
   * Format instructions injected into the ToolHandshake system_prompt so the
   * model knows which dialect to emit. `tools` is the JSON-schema list of
   * available tools for this session.
   */
  formatInstructions(tools: Array<{ name: string; schema: unknown }>): string;
  /**
   * Return true if this adapter believes it can parse the given assistant text.
   * Used to select the adapter when the model's family is unknown.
   */
  detect(text: string): boolean;
  /**
   * Extract tool calls. Never throws: unparseable segments are returned as
   * `{ malformed }` entries for the validator stage.
   */
  parse(text: string): Array<ToolCall | { malformed: string }>;
}

export interface ValidationResult {
  ok: boolean;
  call?: ToolCall;
  /** Structured error fed back to the model when !ok. */
  error?: string;
}

/** Schema-check a parsed call against the session's tool manifest. */
export function validateCall(
  call: ToolCall | { malformed: string },
  manifest: { available: string[] },
): ValidationResult {
  if ('malformed' in call) {
    return { ok: false, error: `Unparseable tool call: ${call.malformed}` };
  }
  if (!manifest.available.includes(call.tool)) {
    return {
      ok: false,
      error: `Unknown tool "${call.tool}". Available: ${manifest.available.join(', ')}`,
    };
  }
  return { ok: true, call };
}

/** Registry: family id -> adapter. */
const registry = new Map<string, ToolCallAdapter>();

export function registerAdapter(a: ToolCallAdapter): void {
  registry.set(a.id, a);
}

export function getAdapter(familyId: string): ToolCallAdapter | undefined {
  return registry.get(familyId);
}

/** Fallback: first adapter whose detect() matches. */
export function detectAdapter(text: string): ToolCallAdapter | undefined {
  for (const a of registry.values()) if (a.detect(text)) return a;
  return undefined;
}
```

### 5.4 Built-in adapters to implement

1. **`fenced`** (current behavior, keep as fallback): ` ```tool_call \n {tool, args} \n``` `.
2. **`openai`**: parses the provider's native `tool_calls` response array. Covers
   DeepSeek via OpenAI-compatible endpoints.
3. **`hermes2`**: `<tool_call>{...}</tool_call>` XML-wrapped JSON (Hermes 2 Pro).
4. **`hermes3`**: ChatML tool template JSON (Hermes 3/4).
5. **`anthropic`**: `tool_use` content blocks.

Each adapter's `formatInstructions()` produces the handshake text for its family —
this is the "tell the AI during the handshake" half, now co-located with the parser
that reads the result.

### 5.5 Wiring into the existing loop

- `src/lib/toolLoop.ts`: replace the direct `parseToolCalls(text)` call with
  `getAdapter(modelFamily).parse(text)` → `validateCall(...)` per call.
- `ModelInfo` (`src/lib/providers.ts`) gains a `toolCallFamily: string` field so the
  Models catalog records which adapter each model needs. Default: `'fenced'`.
- `ToolHandshake.system_prompt` is composed as: base agent prompt + the selected
  adapter's `formatInstructions(tools)`.
- Malformed/unknown-tool results feed back as `TOOL RESULT [error]` messages so the
  model self-corrects — the loop already supports this path.
- **Per §5.2's guarantee:** after this wiring, swapping the model family changes only
  which adapter is selected and which format instructions the handshake carries. The
  executor, the approval gates, and the activity trace are family-agnostic.

## 6. Implementation phases

1. **Phase A — Adapter registry + 2 adapters** (`fenced`, `openai`): refactor
   `parseToolCalls` behind the registry; no UI changes. Proves the seam.
2. **Phase B — Agent tab shell**: nav item + four panes wired to `toolLoop.ts` in
   interactive mode; reuse `ApprovalGate`, `ChatBubble`, MCP panel.
3. **Phase C — Remaining adapters** (`hermes2`, `hermes3`, `anthropic`) + validator
   repair pass + `toolCallFamily` in the model catalog.
4. **Phase D — Connector onboarding UX**: one-click OAuth cards, vault hardening
   audit, per-connector scopes in `PermissionCenter`.
5. **Phase E — Browser-use tool** for API-less services (Replit, Lovable).

## 7. Token economy

- **Prompt caching** (built): Anthropic-family requests carry `cache_control`
  breakpoints after the system prompt and after the last message. The stable prefix
  (system prompt with tool defs, constitution, quota notice + conversation history)
  is reused across turns at ~10% of input-token price instead of being reprocessed
  every turn. OpenAI and Google cache automatically; Ollama has no per-token cost.
- **Two-tier verbosity** (policy): terse internally (reasoning, tool args), eloquent
  only in user-facing text. Enforced via the agent system prompt, not code.
- **Tool-output truncation**: cap file reads and command output; summarize the rest.
  One uncapped output can cost more than the rest of the run.
- **Stop conditions**: max turns, no repeated identical tool calls, halt when tool
  results stop changing the plan.

## 8. Open questions for Jesse

- Which model should be the default pilot for the Agent tab? (DeepSeek via OpenRouter
  is the current recommendation: strong function calling, low cost.)
- Should the activity trace show full tool arguments by default, or redact
  argument values for privacy?
- For the browser-use tool: full visible Chromium window, or headless with
  screenshots in the trace?

## 9. Voice I/O (built 2026-10-07)

Both directions, same Fal key as image/video (no browser voices anywhere):

- **Agent listens** — `media.transcribe` tool (Fal Whisper, 99+ languages,
  auto-detect; `audio_url` or `audio_path`, optional `translate` task).
  Frontend mic flow: `VoiceControls` records via MediaRecorder →
  `media_transcribe_mic` command → transcript lands in the chat input.
- **Agent speaks** — `media.speak` tool and `media_speak_text` command via
  **Kokoro** (`fal-ai/kokoro/american-english`): natural human speech, never the
  robotic browser voice. Audio saved to app-data `media/`, played with a plain
  HTMLAudioElement.
- **Mute/unmute**: `media.speech_enabled` setting. When unmuted, new agent
  messages are dictated immediately (ConferenceRoom wired; Agent tab reuses the
  same `VoiceControls` + `speakText` helpers).
- **Copy**: every message bubble already carries a Copy button.
