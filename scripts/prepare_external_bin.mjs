#!/usr/bin/env node
// prepare_external_bin.mjs — stage the whisper-helper binary for bundling.
//
// Tauri's `externalBin` expects `src-tauri/binaries/whisper-helper-<target-triple>`
// to exist at bundle time. `tauri build` may only compile the default-run
// binary, so this script builds the helper explicitly (release profile) and
// copies it under the triple-suffixed name Tauri looks for.
//
// Runs via `beforeBundleCommand` in tauri.conf.json — after the app build,
// immediately before the .deb/AppImage is assembled.

import { execSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(here, "..");
const tauriDir = join(projectRoot, "src-tauri");

const triple = execSync("rustc -vV", { encoding: "utf8" }).match(/host:\s*(\S+)/)?.[1];
if (!triple) {
  console.error("prepare_external_bin: could not determine the rustc host triple");
  process.exit(1);
}

console.log(`prepare_external_bin: building whisper-helper for ${triple}`);
// The tauri CLI exports TAURI_CONFIG (the merged config, which includes
// externalBin) into hook environments; that makes the crate's build script
// require the sidecar before it can build the sidecar. Strip it so this
// bootstrap build reads the plain on-disk config.
const env = { ...process.env };
delete env.TAURI_CONFIG;
execSync("cargo build --release --bin whisper-helper", {
  cwd: tauriDir,
  stdio: "inherit",
  env,
});

const built = join(tauriDir, "target", "release", "whisper-helper");
if (!existsSync(built)) {
  console.error(`prepare_external_bin: expected binary missing: ${built}`);
  process.exit(1);
}

const dest = join(tauriDir, "binaries", `whisper-helper-${triple}`);
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(built, dest);
chmodSync(dest, 0o755);
console.log(`prepare_external_bin: staged ${dest}`);
