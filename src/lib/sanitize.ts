// SPDX-License-Identifier: Apache-2.0
// sanitize.ts — display-only filtering of machine syntax that some models
// leak into their visible replies (template markers, tool-call tags, fenced
// tool calls). The raw text is never modified — the tool loop still parses
// it — this only cleans what the user sees.

export function stripMachineSyntax(text: string): string {
  if (!text) return text;
  let out = text;
  // Fenced tool calls (this app's own dialect), closed or still streaming.
  out = out.replace(/```tool_call\s*\n[\s\S]*?```/g, '');
  out = out.replace(/```tool_call\s*\n[\s\S]*$/g, '');
  // <tool_call> … </tool_call> (other harnesses' templates), including a
  // block that has not closed yet mid-stream.
  out = out.replace(/<tool_call>[\s\S]*?<\/tool_call>/gi, '');
  out = out.replace(/<tool_call\b[^>]*>[\s\S]*$/gi, '');
  // Full-width-pipe template tokens: ｜tool｜, ｜CO…｜, bare or angle-wrapped.
  out = out.replace(/<｜[^｜>\n]*｜>/g, '');
  out = out.replace(/｜[^｜\n]{0,40}｜/g, '');
  // ASCII template tokens such as <|tool_call|>.
  out = out.replace(/<\|[^|>\n]{0,40}\|>/g, '');
  // Stray closing tags left behind by partial matches.
  out = out.replace(/<\/?tool_call>/gi, '');
  // Collapse the whitespace the removals leave behind.
  out = out.replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n');
  return out.trim();
}
