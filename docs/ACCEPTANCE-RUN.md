# Acceptance run — desktop walkthrough

What this is: the one verification layer that cannot run from the build
machine — driving the real app. Automated layers already pass (34 cargo
tests, IPC contract 195 commands / 288 invoke sites, TypeScript typecheck,
Vite build). This is the script for the hands-on pass, ordered by risk:
**if something is going to break, it will break in step 1.**

Record for anything that fails: the screen, the exact error text, and what you
had just done. Screenshots welcome. Do not skip ahead — each step builds on
the last.

---

## 0. Start

```bash
cd /home/jesse/AI/Projects/GuildFoundryLinux
npm run tauri:dev
```

Wait for the window. The first Rust build after these changes can take a few
minutes; subsequent starts are fast.

## 1. The core loop (most important)

1. Open the **Agent** tab. Pick your provider and model in the selector.
2. Send: `List the files in this project and tell me what it is.` (a message
   that *forces* tool use).
3. Expect: a tool-call trace (filesystem tool), a real answer, no freeze
   while the tool runs (tool calls now run off the UI thread), and the reply
   bubble in blue (cloud model).
4. Send a second message immediately. Expect: the thread continues, and the
   model remembers the first exchange (SQLite-backed history).
5. If any of this errors: paste the error verbatim. This is the path that had
   never executed end-to-end until the IPC fix.

## 2. Approvals and revoke

1. In the Agent tab's tool settings, set a tool (e.g. terminal/shell) to
   **ask every time**, then ask the agent to run something harmless
   (e.g. `echo hello`).
2. Expect: an approval prompt appears; approving runs it once; with a one-shot
   grant, the *next* identical call asks again (one-shot consumes on first
   successful use).
3. Set it back to allow, or deny, and confirm the agent reports the refusal
   honestly instead of pretending.

## 3. Local model (your GGUF)

1. Local Models / provider settings → point at a **single-file or first-shard**
   GGUF (never a `-00003-of-00003` shard).
2. Send the same "list the files" message with the local model selected.
3. Expect: reply bubble **green** (local model), token counts appear in the
   reply footer/hud, and the Usage & Cost tab shows the local usage (tokens
   counted, cost 0).
4. Change temperature and send again — the sampling settings you set should
   be the ones used.

## 4. Voice (zero-cloud)

1. **Clips to listen to first** (generated during setup — this is the only
   part I could not hear):
   - `~/.local/share/ai.guildfoundry.desktop/media/kokoro-test-bella.wav`
   - `~/.local/share/ai.guildfoundry.desktop/media/kokoro-test-adam.wav`
   If the voice quality is not acceptable, tell me — the model is swappable.
2. In the agent chat, use **speak/read-aloud** on a message. Expect: it
   speaks with the local voice (no cloud call, no API key involved).
3. Connections → Local voice: the card should show **Kokoro ready**. Change
   the voice in the dropdown → Use voice → speak again; the voice changes.
4. Mic: hold to talk, say a sentence. Expect: text appears in the input via
   local Whisper (it needs a whisper.cpp ggml model — if the card says
   "No model yet", point it at one; or tell me and it falls back to cloud).

## 5. Memory — semantic search

1. Open the Memory panel. The badge should read
   **semantic on — n/n indexed**. If it says otherwise, click **Reindex**.
2. Store two entries with different wordings, e.g. key `car` /
   content `I drive a blue Subaru wagon`, then search for `vehicle`.
3. Expect: the entry is found and its hit shows the **semantic** badge —
   keyword search alone would miss it. (bge-small embeddings, fully offline.)

## 6. Usage & Cost

1. Send a few messages on a cloud model.
2. System & Settings → **Usage & Cost**: rows appear per day/provider/model.
3. Add a price for the model you use (per-million in/out). Spend becomes
   non-zero; before that the cost column shows "—" and the model is flagged
   **unpriced** (honest by design — no invented numbers).

## 7. Integrations (only what you have credentials for)

- **Email**: Connections → Email card → save an app password → Test. Expect
  "Ready". Then ask the agent: `Check my inbox and summarize anything urgent.`
  Expect: real unread mail from the last day, newest first
  (this is the new `inbox.fetch` tool).
- **Phone / SMS**: save Twilio creds → Test. Ask the agent to text you
  something. Then, if you want transcripts: tick **Record calls**, make a
  short call, hang up, wait ~30 s, press **Transcribe last call**. Expect:
  the transcript lands in Memory under `call-<sid>` and is spoken/timestamped
  locally.
- **Calendar**: save CalDAV creds → Test; ask the agent to create an event.

## 8. MCP servers

1. Tools & Registry → MCP Servers → add any stdio server (e.g. a `npx` one
   from the presets). Connect/Test once.
2. Then have the agent call one of its tools.
3. Expect: connect is slow once (process spawn), and the *tool call afterward
   is noticeably faster* — that is session warming; the child process is
   reused. Killing the server out from under it and calling again should
   recover by spawning a fresh one.

## 9. Projects / coding tab

1. Projects → open this repo folder.
2. In the coding column, ask: `What does src-tauri/src/mcp.rs do? Summarize
   in three bullets.` (read a real file, same tools as the main agent).
3. Try the file tree, search, diff, and a checkpoint (snapshot before write).
   Expect: search on a large tree no longer freezes the window.

## 10. Mobile companion (LAN)

1. Connections → **Mobile companion (LAN)** card → Start (default port 8787).
2. Copy the link (`http://<your-lan-ip>:8787/?k=…`) and open it on your phone
   on the same Wi-Fi.
3. Expect: the thread loads; send a message; the reply appears on the phone
   **and** in the desktop Agent tab (same thread, same model).
4. Open the link in a second browser with the token removed — expect a clear
   "missing or wrong token" message, not access.

## 11. Packaging (optional, slow)

```bash
npm run tauri:build
```

Expect: a `.deb` and AppImage under `src-tauri/target/release/bundle/`.
Install the deb and check that speech-to-text works from the installed app
(the whisper-helper sidecar now ships with the bundle).

---

## What "done" looks like

All steps either work or fail *legibly*. The bar is not perfection — it is
that nothing lies: no fake success, no dead button, no silent failure. Any
place the app cannot do something, it must say so and say why.

## Reporting failures

Paste: step number, what you did, exact error/screenshot. If the app freezes
rather than errors, note which step and what was on screen. I will turn each
report into a fix and a test so it cannot regress.
