# Linux Quickstart — Guild Foundry AI (2026-10-07 build)

One-time setup on a fresh Ubuntu/Debian-family machine, then what's new to try.

## 1. Prerequisites

```sh
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Node 22+ (via your preferred method)

# Tauri 2 system deps
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf
```

## 2. Clone and run

```sh
git clone https://github.com/jesserafaelmachucadevega-sketch/guild-foundry-Linux.git
cd guild-foundry-Linux
npm install
python3 scripts/gen_icons.py   # icons are gitignored; regenerate locally
npm run tauri:dev              # dev mode with hot reload
```

Production build:

```sh
npm run tauri:build            # produces .deb and .AppImage
```

## 3. Verify

```sh
npm run typecheck              # TypeScript (must be clean)
cargo check                    # Rust — CI also runs this on every push
```

## 4. What's new (2026-10-07) — try this

- **Connections tab** (nav: `Conn.`): one-click templates for Gmail, Outlook,
  Google Calendar (OAuth), GitHub, Web Search, YouTube, Music, Browser
  Automation. Each has Ask every time / Always allow / Deny permission control.
- **Terminal card**: built-in shell for the agent (download, install, run, clean
  up, uninstall) — permission-gated like everything else.
- **Media generation card**: paste a [Fal API key](https://fal.ai) to unlock
  `media.generate_image` (FLUX 2 Dev default, ~$0.025/image) and
  `media.generate_video` (Wan 2.7 default, ~$0.05/sec) for every loaded model,
  local or frontier.
- **Interactive artifacts**: agents can call `artifact.create` to render polls,
  checklists, sliders, cards, sticky notes, and whiteboards inline in chat.
- **Prompt caching**: Anthropic-family requests carry cache breakpoints —
  repeated turns reuse the cached system prompt at ~10% input cost.
- **Rate limiting**: `:free` models are throttled (20 req/min, 1,000/day);
  paid and local models bypass entirely.
- **Constitution**: `security.rs` §§1–14 — prohibitions (§§1–7) plus the
  behavioral layer (§§8–14: tool doctrine, honesty, proactivity, character,
  context, communication, growth).

## 5. Notes for the coder

- Rust was **not** compiled during this build session (no toolchain) — `cargo
  check` locally and the CI workflow on push are the verifiers. The TS side
  typechecks clean.
- Secrets (provider keys, Fal key, OAuth tokens) live in the OS keyring, never
  in the repo or the database.
- `docs/agent-tab-design.md` is the spec for the upcoming Agent tab;
  `docs/mcp-starter-pack.md` is the curated MCP server list.
