#!/usr/bin/env bash
# install-local.sh — rebuild the app and refresh the user-local copy.
#
# This is the update loop for the "we found more bugs" phase: run it after
# fixes land, and the `guild-foundry` command / app-menu entry immediately
# launch the new build. No sudo, nothing system-wide.
#
#   bash scripts/install-local.sh
#
# The optional system-wide path (menu entry everywhere, apt-managed,
# uninstallable) is printed at the end — run that only when the bugs are
# cleared, and re-run it after each fix round; apt upgrades it in place and
# your data (settings, keys, models, threads) is never touched.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> building release bundles (this takes a few minutes)…"
npm run tauri:build

APPIMAGE="$(ls -1 "$ROOT"/src-tauri/target/release/bundle/appimage/*.AppImage | head -1)"
DEB="$(ls -1 "$ROOT"/src-tauri/target/release/bundle/deb/*.deb | head -1)"

mkdir -p "$HOME/Applications"
cp -f "$APPIMAGE" "$HOME/Applications/GuildFoundryAI.AppImage"
chmod +x "$HOME/Applications/GuildFoundryAI.AppImage"

echo
echo "Updated:  $HOME/Applications/GuildFoundryAI.AppImage"
echo "Launch:   guild-foundry        (or from the app menu: Guild Foundry AI)"
echo
echo "System-wide install/upgrade (when you want it, any number of times):"
echo "  sudo apt install \"$DEB\""
