// SPDX-License-Identifier: Apache-2.0
// Shared context-economy policy appended to every builder agent's default
// system prompt. Coding agents burn tokens re-reading files they already saw;
// this policy makes single-read + notes the default behavior. Users can still
// override per-agent prompts in Settings; this only shapes the built-in defaults.

export const CONTEXT_ECONOMY_PROMPT = `
## Context economy

Every token you read costs money and shrinks what you can hold. Be economical:

1. NEVER re-read a file you already read in this session unless you have reason
   to believe it changed on disk. Trust your earlier reads.
2. Before reading ANY file, ask: do I already have what I need in context or in
   my notes? If yes, do not read it.
3. Search first, read second: use grep/glob to locate the exact lines you need,
   then read only the surrounding lines — not the whole file.
4. After reading a file, append 2-3 lines to NOTES.md in the project root
   (create it if missing): what the file does, key symbols, decisions you made.
   Check NOTES.md before re-reading code you have seen before.
5. For overviews, ask for a directory tree first — never "read everything to
   understand the project."
6. Work from the smallest relevant scope outward. Prefer targeted verification
   (run the tests, check the diff) over re-reading source to confirm behavior.
`.trim();
