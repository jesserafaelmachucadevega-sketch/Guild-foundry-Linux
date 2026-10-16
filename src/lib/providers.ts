// Shared provider contract — implemented by Phase 2 Rust commands.
// Phases 3/5/6/7 import this file; do not modify its exported shapes.

import { invoke } from './api';

export type ProviderKind =
  | 'openai'
  | 'openai-compatible'
  | 'anthropic'
  | 'google'
  | 'openrouter'
  | 'ollama'
  | 'gguf'
  | 'command-code'
  | 'custom';

export type PricingVerified = 'free' | 'paid' | 'unknown';

/** True when this provider runs models on this machine (local files/servers).
 *  Drives the green-vs-blue bubble distinction in chat surfaces. */
export function isLocalProvider(p: ProviderSummary | undefined | null): boolean {
  if (!p) return false;
  if (p.kind === 'ollama' || p.kind === 'gguf') return true;
  try {
    const host = new URL(p.base_url).hostname;
    return host === 'localhost' || host === '127.0.0.1' || host === '::1' || host === '0.0.0.0';
  } catch {
    return false;
  }
}

export interface ProviderSummary {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  models_path: string;
  has_key: boolean;
  status: 'idle' | 'connected' | 'error';
  last_error_code?: string;
  last_check_at?: number;
}

export interface ModelInfo {
  /** Stable row key, `"<provider_id>:<model_id>"`. Favorites/sorting only. */
  id: string;
  /** Provider-side model id (e.g. `gpt-4o`). Send this as `model_id`. */
  model_id: string;
  name: string;
  provider_id: string;
  pricing_verified: PricingVerified;
  context_length?: number;
  supports_tools?: boolean;
  supports_vision?: boolean;
  supports_reasoning?: boolean;
}

export interface HandshakeReport {
  provider_id: string;
  ok: boolean;
  http_status?: number;
  /** Raw status/code, always present on failure (e.g. "401", "ENOTFOUND"). */
  raw_code: string;
  /** Plain-language explanation, e.g. "API Key Incorrect". */
  layman: string;
  detail: string;
  latency_ms: number;
  models_found: number;
  capabilities?: {
    tool_calling: boolean;
    vision: boolean;
    structured_output: boolean;
  };
}

export interface RouterSuggestion {
  provider_id: string;
  model_id: string;
  reason: string;
}

export const getProviders = (): Promise<ProviderSummary[]> =>
  invoke<ProviderSummary[]>('provider_list');

export const anyConnected = (): Promise<{ connected: boolean }> =>
  invoke<{ connected: boolean }>('providers_any_connected');

export const handshakeProvider = (providerId: string): Promise<HandshakeReport> =>
  invoke<HandshakeReport>('provider_handshake', { provider_id: providerId });

export const listModels = (providerId: string): Promise<ModelInfo[]> =>
  invoke<ModelInfo[]>('provider_list_models', { provider_id: providerId });

export const refreshModels = (providerId: string): Promise<ModelInfo[]> =>
  invoke<ModelInfo[]>('provider_refresh_models', { provider_id: providerId });

/**
 * camelCase wire payload for `provider_upsert` — mirrors the Rust
 * `ProviderUpsertPayload` struct (#[serde(rename_all = "camelCase")]) in
 * src-tauri/src/providers.rs. Nested under the `payload` invoke key because
 * the Rust command takes a single struct argument named `payload`.
 */
export interface ProviderUpsertPayload {
  id?: string;
  name: string;
  kind: ProviderKind;
  baseUrl?: string;
  modelsPath?: string;
  apiKey?: string;
}

export const upsertProvider = (p: {
  id?: string;
  name: string;
  kind: ProviderKind;
  /** snake_case inputs are also accepted and mapped to the camelCase payload. */
  base_url?: string;
  baseUrl?: string;
  models_path?: string;
  modelsPath?: string;
  api_key?: string;
  apiKey?: string;
}): Promise<ProviderSummary> => {
  const payload: ProviderUpsertPayload = {
    id: p.id,
    name: p.name,
    kind: p.kind,
    baseUrl: p.baseUrl ?? p.base_url,
    modelsPath: p.modelsPath ?? p.models_path,
    apiKey: p.apiKey ?? p.api_key,
  };
  return invoke<ProviderSummary>('provider_upsert', { payload });
};

export const deleteProvider = (providerId: string): Promise<void> =>
  invoke<void>('provider_delete', { provider_id: providerId });

export const suggestModel = (task: string): Promise<RouterSuggestion> =>
  invoke<RouterSuggestion>('provider_router_suggest', { task });
