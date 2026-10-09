// SPDX-License-Identifier: Apache-2.0
// Phase 6 — frontend tool execution loop (master spec sections 38-39):
//
//   MODEL -> TOOL CALL -> TOOL EXECUTOR -> TOOL RESULT -> MODEL CONTEXT -> NEXT ACTION
//
// The critical invariant: tool results NEVER terminate at the executor. Every
// result returned by `tool_execute` is appended to the conversation as a
// `TOOL RESULT` message and fed back into the model until the model emits no
// more tool calls or `maxTurns` is reached.
//
// Gates (Phase 5 approval modal / inline user prompt) stop the loop and report
// their status via onEvent; the host resumes the loop after the user acts.

import { invoke } from './api';
import { streamChat, type ChatMsg } from './chat';

export interface ToolCall {
  tool: string;
  args: Record<string, unknown>;
}

/** Mirrors the Rust `ToolResult` shape. */
export interface ToolExecResult {
  ok: boolean;
  output: unknown;
  error?: string;
  duration_ms: number;
  needs_approval: boolean;
  needs_user: boolean;
  tool: string;
  code: string;
  executed_at: string;
}

/** Mirrors the Rust `Handshake` shape. */
export interface CapabilityManifest {
  available: string[];
  unavailable: string[];
  requires_approval: string[];
  restricted: string[];
}

export interface ToolHandshake {
  manifest: CapabilityManifest;
  system_prompt: string;
  session_id: string;
}

export type ToolLoopStatus =
  | 'done'
  | 'max_turns'
  | 'approval_required'
  | 'user_input_required'
  | 'error';

export type ToolLoopEvent =
  | { type: 'handshake'; manifest: CapabilityManifest }
  | { type: 'assistant_delta'; delta: string }
  | { type: 'assistant_done'; turn: number; text: string }
  | { type: 'tool_call'; callId: string; call: ToolCall }
  | { type: 'tool_result'; callId: string; result: ToolExecResult }
  | {
      type: 'loop_end';
      status: ToolLoopStatus;
      finalText: string;
      detail?: string;
    };

export interface ToolLoopOptions {
  providerId: string;
  modelId: string;
  systemPrompt?: string;
  sessionId: string;
  agentId: string;
  maxTurns: number;
  /** First user message. Defaults to a generic task-start prompt. */
  userMessage?: string;
  onEvent: (e: ToolLoopEvent) => void;
}

function newCallId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID();
  return `tc-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
}

/**
 * Extract ```tool_call fenced JSON blocks: `{tool, args}`.
 * Malformed blocks are NOT silently dropped — they produce a synthetic
 * `bad_args` result so the model can see its mistake and retry.
 */
export function parseToolCalls(text: string): Array<ToolCall | { malformed: string }> {
  const calls: Array<ToolCall | { malformed: string }> = [];
  const re = /```tool_call\s*\n([\s\S]*?)```/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    try {
      const parsed = JSON.parse(m[1].trim()) as { tool?: unknown; args?: unknown };
      if (parsed && typeof parsed.tool === 'string') {
        calls.push({
          tool: parsed.tool,
          args:
            parsed.args && typeof parsed.args === 'object'
              ? (parsed.args as Record<string, unknown>)
              : {},
        });
      } else {
        calls.push({ malformed: m[1].trim() });
      }
    } catch {
      calls.push({ malformed: m[1].trim() });
    }
  }
  return calls;
}

function malformedResult(block: string): ToolExecResult {
  return {
    ok: false,
    output: null,
    error: `could not parse tool_call block as JSON: ${block.slice(0, 200)}`,
    duration_ms: 0,
    needs_approval: false,
    needs_user: false,
    tool: '(unparsed)',
    code: 'bad_args',
    executed_at: new Date().toISOString(),
  };
}

/** Stream one assistant turn to completion, collecting the full text. */
function collectTurn(
  opts: ToolLoopOptions,
  messages: ChatMsg[],
  onDelta: (delta: string) => void,
  onCancel: (cancel: () => void) => void,
): Promise<string> {
  return new Promise((resolve, reject) => {
    let text = '';
    void streamChat(
      { providerId: opts.providerId, modelId: opts.modelId, messages },
      {
        onDelta: (d) => {
          text += d;
          onDelta(d);
        },
        onDone: () => resolve(text),
        onError: (e) => reject(e),
      },
    )
      .then((cancel) => onCancel(cancel))
      .catch(reject);
  });
}

/**
 * Run the agentic tool loop. Returns a `cancel()` handle.
 *
 * Flow: handshake (capability manifest + anti-fabrication system prompt) ->
 * per turn: stream assistant text -> parse tool calls -> execute each via
 * `tool_execute` -> append `TOOL RESULT` messages -> repeat.
 */
export function runToolLoop(opts: ToolLoopOptions): { cancel: () => void } {
  let cancelled = false;
  let cancelStream: (() => void) | null = null;
  const cancel = () => {
    cancelled = true;
    if (cancelStream) cancelStream();
  };

  void (async () => {
    try {
      // 1. Handshake: per-session capability manifest + anti-fabrication prompt.
      const handshake = await invoke<ToolHandshake>('tool_handshake', {
        session_id: opts.sessionId,
      });
      if (cancelled) return;
      opts.onEvent({ type: 'handshake', manifest: handshake.manifest });
      // Quota awareness: free-tier models are told their budget so they spend
      // turns wisely. Paid/local models get no notice (None) — no constraint.
      let quotaNotice = '';
      try {
        const notice = await invoke<string | null>('ratelimit_quota_notice', {
          provider_id: opts.providerId,
          model_id: opts.modelId,
        });
        if (notice) quotaNotice = notice;
      } catch {
        // Non-fatal: the loop works without the notice; enforcement lives
        // server-side in the rate limiter regardless.
      }
      const system = [opts.systemPrompt, handshake.system_prompt, quotaNotice]
        .filter((s): s is string => !!s && s.length > 0)
        .join('\n\n');

      const messages: ChatMsg[] = [
        { role: 'system', content: system },
        { role: 'user', content: opts.userMessage ?? 'Begin executing your assigned task.' },
      ];

      // 2. Turn loop.
      let finalText = '';
      for (let turn = 0; turn < opts.maxTurns && !cancelled; turn += 1) {
        const text = await collectTurn(
          opts,
          messages,
          (delta) => opts.onEvent({ type: 'assistant_delta', delta }),
          (c) => {
            cancelStream = c;
          },
        );
        if (cancelled) break;
        finalText = text;
        opts.onEvent({ type: 'assistant_done', turn, text });
        messages.push({ role: 'assistant', content: text });

        const calls = parseToolCalls(text);
        if (calls.length === 0) {
          opts.onEvent({ type: 'loop_end', status: 'done', finalText });
          return;
        }

        for (const call of calls) {
          if (cancelled) break;
          const callId = newCallId();
          if ('malformed' in call) {
            const result = malformedResult(call.malformed);
            opts.onEvent({ type: 'tool_result', callId, result });
            messages.push({
              role: 'user',
              content: `TOOL RESULT [unparsed tool_call]:\n${JSON.stringify(result)}`,
            });
            continue;
          }
          opts.onEvent({ type: 'tool_call', callId, call });
          let result: ToolExecResult;
          try {
            result = await invoke<ToolExecResult>('tool_execute', {
              tool: call.tool,
              args_json: JSON.stringify(call.args),
              agent_id: opts.agentId,
              session_id: opts.sessionId,
            });
          } catch (err) {
            // Desktop Capability Required (PWA) or IPC failure — reported as a
            // result so the model sees the failure instead of hanging.
            result = {
              ok: false,
              output: null,
              error: err instanceof Error ? err.message : String(err),
              duration_ms: 0,
              needs_approval: false,
              needs_user: false,
              tool: call.tool,
              code: 'exec_error',
              executed_at: new Date().toISOString(),
            };
          }
          opts.onEvent({ type: 'tool_result', callId, result });

          // Gates stop the loop; the host resumes after the user acts
          // (Phase 5 approval modal / inline prompt), feeding the decision
          // back in as the next TOOL RESULT.
          if (result.needs_approval) {
            opts.onEvent({
              type: 'loop_end',
              status: 'approval_required',
              finalText: text,
              detail: result.error,
            });
            return;
          }
          if (result.needs_user) {
            opts.onEvent({
              type: 'loop_end',
              status: 'user_input_required',
              finalText: text,
              detail: JSON.stringify(result.output),
            });
            return;
          }

          messages.push({
            role: 'user',
            content: `TOOL RESULT [${call.tool}]:\n${JSON.stringify(result)}`,
          });
        }
      }

      opts.onEvent({
        type: 'loop_end',
        status: cancelled ? 'done' : 'max_turns',
        finalText,
        detail: cancelled ? 'cancelled by user' : `reached maxTurns=${opts.maxTurns}`,
      });
    } catch (err) {
      opts.onEvent({
        type: 'loop_end',
        status: 'error',
        finalText: '',
        detail: err instanceof Error ? err.message : String(err),
      });
    }
  })();

  return { cancel };
}
