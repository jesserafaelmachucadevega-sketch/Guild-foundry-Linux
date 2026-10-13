// SPDX-License-Identifier: Apache-2.0
// Tool-call adapter registry (Agent tab design §5).
//
// Models emit tool calls in different dialects. Stage 1 (the handshake
// prompt) tells the model what to emit; Stage 2 (here) parses whatever
// family's dialect comes back; Stage 3 (validateCall) repairs/rejects and
// bounces structured errors so the model self-corrects on the next turn.
// The executor only ever receives validated, normalized calls.

export interface ToolCall {
  tool: string;
  args: Record<string, unknown>;
}

export type ParsedCall = ToolCall | { malformed: string };

export interface ToolCallAdapter {
  id: string;
  label: string;
  /** True if `text` looks like this dialect (fallback selection). */
  detect(text: string): boolean;
  /** Parse all calls. Never throws; unparseable segments become malformed. */
  parse(text: string): ParsedCall[];
}

function parseJsonCall(raw: string): ParsedCall {
  try {
    const parsed = JSON.parse(raw.trim()) as { tool?: unknown; args?: unknown };
    if (parsed && typeof parsed.tool === 'string') {
      return {
        tool: parsed.tool,
        args:
          parsed.args && typeof parsed.args === 'object'
            ? (parsed.args as Record<string, unknown>)
            : {},
      };
    }
    return { malformed: raw.trim() };
  } catch {
    return { malformed: raw.trim() };
  }
}

/** ```tool_call {tool,args}``` — the canonical handshake dialect. */
export const fencedAdapter: ToolCallAdapter = {
  id: 'fenced',
  label: 'Fenced tool_call blocks',
  detect: (t) => /```tool_call\s*\n/.test(t),
  parse: (text) => {
    const out: ParsedCall[] = [];
    const re = /```tool_call\s*\n([\s\S]*?)```/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) out.push(parseJsonCall(m[1]));
    return out;
  },
};

/** `<tool_call>{...}</tool_call>` — XML-wrapped JSON (Hermes-style families). */
export const xmlAdapter: ToolCallAdapter = {
  id: 'xml',
  label: 'XML tool_call tags',
  detect: (t) => /<tool_call>/.test(t),
  parse: (text) => {
    const out: ParsedCall[] = [];
    const re = /<tool_call>\s*([\s\S]*?)\s*<\/tool_call>/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) out.push(parseJsonCall(m[1]));
    return out;
  },
};

/** Balanced-brace scan so nested args objects survive. */
function scanJsonObjects(text: string): string[] {
  const objs: string[] = [];
  let depth = 0;
  let start = -1;
  let inStr = false;
  let esc = false;
  for (let i = 0; i < text.length; i += 1) {
    const c = text[i];
    if (inStr) {
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') {
      inStr = true;
      continue;
    }
    if (c === '{') {
      if (depth === 0) start = i;
      depth += 1;
    } else if (c === '}') {
      depth -= 1;
      if (depth === 0 && start >= 0) {
        objs.push(text.slice(start, i + 1));
        start = -1;
      }
    }
  }
  return objs;
}

/** Bare JSON objects mentioning "tool" — models that forget the fence. */
export const bareJsonAdapter: ToolCallAdapter = {
  id: 'bare-json',
  label: 'Bare JSON tool objects',
  detect: (t) => /"tool"\s*:/.test(t) && t.includes('{'),
  parse: (text) => {
    const out: ParsedCall[] = [];
    for (const obj of scanJsonObjects(text)) {
      if (!/"tool"\s*:/.test(obj)) continue;
      out.push(parseJsonCall(obj));
    }
    return out;
  },
};

const registry: ToolCallAdapter[] = [fencedAdapter, xmlAdapter, bareJsonAdapter];

export function getAdapter(id: string): ToolCallAdapter | undefined {
  return registry.find((a) => a.id === id);
}

/** First adapter that recognizes the text; `fenced` is the default. */
export function detectAdapter(text: string, preferredId?: string): ToolCallAdapter {
  if (preferredId) {
    const preferred = getAdapter(preferredId);
    if (preferred && preferred.detect(text)) return preferred;
  }
  for (const a of registry) {
    if (a.detect(text)) return a;
  }
  return fencedAdapter;
}

export interface ValidationResult {
  ok: boolean;
  call?: ToolCall;
  /** Structured error fed back to the model when !ok. */
  error?: string;
}

/** Schema-light check: the tool must exist in this session's manifest. */
export function validateCall(
  call: ParsedCall,
  manifest: { available: string[]; requires_approval?: string[]; needs_setup?: string[] },
): ValidationResult {
  if ('malformed' in call) {
    return { ok: false, error: `Unparseable tool call: ${call.malformed.slice(0, 300)}` };
  }
  const known = [
    ...manifest.available,
    ...(manifest.requires_approval ?? []),
    ...(manifest.needs_setup ?? []),
  ];
  if (!known.includes(call.tool)) {
    return {
      ok: false,
      error: `Unknown tool "${call.tool}". Available tools: ${known.slice(0, 60).join(', ')}`,
    };
  }
  return { ok: true, call };
}
