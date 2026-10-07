// Shared chat contract — implemented by Phase 2 Rust commands.
// Phases 3/5/6 import this file; do not modify its exported shapes.
// Rust side: `provider_chat_complete`, `provider_chat_stream` (emits
// Tauri event "provider://chat-chunk" {stream_id, delta, done, error?}),
// `provider_chat_cancel`.

import { invoke, isDesktop } from './api';
import { listen } from '@tauri-apps/api/event';

export interface ChatMsg {
  role: 'user' | 'assistant' | 'system';
  content: string;
}

export interface ChatParams {
  systemPrompt?: string;
  temperature?: number;
  maxTokens?: number;
  reasoningEffort?: 'low' | 'medium' | 'high';
}

export interface StreamCallbacks {
  onDelta: (delta: string) => void;
  onDone: (stats: { latencyMs: number; promptTokens?: number; completionTokens?: number }) => void;
  onError: (err: Error) => void;
}

export interface ChatOptions {
  providerId: string;
  modelId: string;
  messages: ChatMsg[];
  params?: ChatParams;
}

function newStreamId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID();
  return `s-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
}

interface ChunkPayload {
  stream_id: string;
  delta?: string;
  done?: boolean;
  error?: string;
  latency_ms?: number;
  prompt_tokens?: number;
  completion_tokens?: number;
}

/** Non-streaming completion. */
export async function completeChat(opts: ChatOptions): Promise<string> {
  const res = await invoke<{ content: string; latency_ms: number }>(
    'provider_chat_complete',
    {
      provider_id: opts.providerId,
      model_id: opts.modelId,
      messages: opts.messages,
      params: opts.params ?? {},
    },
  );
  return res.content;
}

/**
 * Streaming chat. Returns an unsubscribe/cancel function.
 * In PWA mode (no Tauri runtime) the invoke throws DesktopCapabilityRequired.
 */
export async function streamChat(
  opts: ChatOptions,
  cb: StreamCallbacks,
): Promise<() => void> {
  if (!isDesktop()) {
    cb.onErro
r(new Error('Desktop Capability Required'));
    return () => undefined;
  }
  const streamId = newStreamId();
  let settled = false;

  // Declare before registering so the callback can safely unlisten even if a
  // chunk event fires in the window before `await listen` resolves.
  let unlisten: () => void = () => undefined;
  unlisten = await listen<ChunkPayload>('provider://chat-chunk', (event) => {
    const p = event.payload;
    if (!p || p.stream_id !== streamId || settled) return;
    if (p.error) {
      settled = true;
      cb.onError(new Error(p.error));
      void unlisten();
      return;
    }
    if (p.delta) cb.onDelta(p.delta);
    if (p.done) {
      settled = true;
      cb.onDone({
        latencyMs: p.latency_ms ?? 0,
        promptTokens: p.prompt_tokens,
        completionTokens: p.completion_tokens,
      });
      void unlisten();
    }
  });

  try {
    await invoke('provider_chat_stream', {
      stream_id: streamId,
      provider_id: opts.providerId,
      model_id: opts.modelId,
      messages: opts.messages,
      params: opts.params ?? {},
    });
  } catch (err) {
    settled = true;
    void unlisten();
    cb.onError(err instanceof Error ? err : new Error(String(err)));
  }

  return () => {
    if (settled) return;
    settled = true;
    void unlisten();
    void invoke('provider_chat_cancel', { stream_id: streamId }).catch(() => undefined);
  };
}
