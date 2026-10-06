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
  | 'custom';

export type PricingVerified = 'free' | 'paid' | 'unknown';

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
  id: string;
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

export const upsertProvider = (p: {
  id?: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  models_path: string;
  api_key?: string;
}): Promise<ProviderSummary> => invoke<ProviderSummary>('provider_upsert', p);

export const deleteProvider = (providerId: string): Promise<void> =>
  invoke<void>('provider_delete', { provider_id: providerId });

export const suggestModel = (task: string): Promise<RouterSuggestion> =>
  invoke<RouterSuggestion>('provider_router_suggest', { task });
