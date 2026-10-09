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
npm run typecheck                     # TypeScript (must be clean)
npm run build                         # production frontend bundle
cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets
```

Rust needs **1.80 or newer** (`rust-version` in `src-tauri/Cargo.toml` is
authoritative — it is raised automatically by clippy's `incompatible_msrv`
lint if you use an API newer than the declared floor).

## 3a. If the project lives on a flash drive (exFAT / NTFS / FAT32)

**Do not run the toolchain from the flash drive itself.** exFAT and FAT32 have
no Unix permission bits, and NTFS via `ntfs-3g` usually arrives `noexec`:

```sh
# broken on exFAT — every npm binary is mode 644
sh: 1: tsc: Permission denied
```

`chmod +x` is a silent **no-op** on exFAT (the mode stays `-rw-r--r--`), so
there is no fix in place. Keep the git checkout on the flash drive if you like,
but build from a copy on a real filesystem:

```sh
# from the flash drive checkout
cp -r ~/Guild-foundry-Linux /home/$USER/Guild-foundry-Linux
cd /home/$USER/Guild-foundry-Linux && npm install
```

Two related symptoms on such a drive:

- `npm run typecheck` / `npm run build` / `npm run tauri:dev` fail with
  `Permission denied` even though the packages are installed.
- Cargo builds get very slow or fail outright. Point the target directory at a
  real filesystem: `export CARGO_TARGET_DIR=~/gfa-target`.

Verifying that a checkout is on a filesystem that can host a build:

```sh
stat -f -c %T .          # exfat / ntfs -> must copy out; ext4/btrfs -> fine
ls -l node_modules/.bin  # every entry needs the x bit
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

- The Rust backend has now been **compiled and tested locally** (Rust 1.99,
  aarch64-unknown-linux-gnu): `cargo check --all-targets`, `cargo clippy
  --all-targets` (no errors) and `cargo test` (5 passing) are all green, and CI
  re-runs the same checks on every push.
- Secrets (provider keys, Fal key, OAuth tokens) live in the OS keyring, never
  in the repo or the database.
- `ModelInfo` is defined **once**, in `src/lib/providers.ts`, and carries both
  `id` (`"<provider>:<model>"`, for favorites/sorting) and `model_id` (the
  provider-side name to actually send). Do not add a second copy of this type —
  a drifted duplicate is what previously made every chat request send
  `"model": "openai:gpt-4o"`.
- `docs/agent-tab-design.md` is the spec for the upcoming Agent tab;
  `docs/mcp-starter-pack.md` is the curated MCP server list.
