# Guild Foundry AI

AI-native developer workstation and collaboration environment for the Linux
desktop. Built with **Tauri 2** (Rust backend) + **React + TypeScript + Vite**
(frontend) + **SQLite**.

Two workspaces in one app:

- **App Builder Studio** — multi-agent software development (Supervisor,
  Architect, Frontend, Backend, QA, Security agents) with a deterministic
  build state machine and human approval gates.
- **Conference Room** — multi-model collaboration and debate hub (1–4
  concurrent models, debate engine, voting, synthesis, model-isolated memory).

Full specification: `docs/Guild_Foundry_AI_Zorin_OS_Master_Prompt.pdf`
Build progress: `CHECKLIST.md` (12 phases, checked off as they land).

## Architecture rule

The Rust layer owns **all** privileged operations: filesystem, processes,
credentials (system keyring), notifications, tray, window management,
subprocesses, terminal, local database, OS detection, permissions. The web
frontend owns presentation and application state only. The frontend never
receives unrestricted filesystem / shell / credential / process capabilities.

## Prerequisites (build machine)

- Rust toolchain (rustup): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- Node.js 22+
- Tauri 2 Linux dependencies (Debian/Ubuntu family):

```sh
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf libsecret-1-dev libglib2.0-dev pkg-config
```

**Important**: The `libsecret-1-dev` and related system packages are required for
OS keyring access (credential storage). Without them, provider API keys and
other secrets cannot be stored securely.

## Develop

```sh
npm install
python3 scripts/gen_icons.py   # regenerate PNG/ICO icons (not committed)
npm run tauri:dev
```

Frontend-only (browser / PWA preview):

```sh
npm run dev
```

## Build

```sh
npm run tauri:build        # produces .deb and .AppImage (bundle.targets)
```

## Verify

```sh
npm run typecheck          # TypeScript, renderer
npm run build:frontend     # Vite production bundle
cargo check                # Rust backend
cargo test                 # Rust tests
```

Rust verification (`cargo check` / `cargo build` / `cargo test`) requires the toolchain and
system dependencies above. Status is tracked in `CHECKLIST.md`; anything not
verified in a given environment is marked **NOT VERIFIED**.

## PWA fallback

The same frontend bundle runs as a browser PWA (`public/manifest.webmanifest`,
`public/sw.js`). Native-only capabilities are never faked: the UI displays
**"Desktop Capability Required"** instead of failing silently.

## License

Apache License 2.0 — see [LICENSE](LICENSE).
