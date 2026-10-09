// SPDX-License-Identifier: Apache-2.0
// Pre-populated LLM provider presets — researched V1 handshake (models) URLs.
// The handshake in src-tauri/src/providers.rs does GET {base_url}{models_path}
// with `Authorization: Bearer <key>` (except google/anthropic/ollama families).
// All entries below are OpenAI-compatible unless noted.

import type { ProviderKind } from './providers';

export interface ProviderTemplate {
  /** Stable template id (used as default name slug). */
  id: string;
  /** Display name shown in the Provider dropdown. */
  name: string;
  kind: ProviderKind;
  base_url: string;
  models_path: string;
  /** Full handshake URL for display (base + models path). */
  handshakeUrl: string;
  keyPlaceholder: string;
  help: string;
}

const t = (
  id: string,
  name: string,
  kind: ProviderKind,
  base_url: string,
  models_path: string,
  keyPlaceholder: string,
  help: string,
): ProviderTemplate => ({
  id,
  name,
  kind,
  base_url,
  models_path,
  handshakeUrl: `${base_url.replace(/\/$/, '')}${models_path}`,
  keyPlaceholder,
  help,
});

export const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  t(
    'openai',
    'OpenAI',
    'openai',
    'https://api.openai.com/v1',
    '/models',
    'sk-…',
    'Get a key at platform.openai.com → API keys. Handshake: GET /v1/models.',
  ),
  t(
    'openrouter',
    'OpenRouter',
    'openrouter',
    'https://openrouter.ai/api/v1',
    '/models',
    'sk-or-v1-…',
    'Get a key at openrouter.ai → Keys. Fully OpenAI-compatible.',
  ),
  t(
    'mistral',
    'Mistral (Mistral AI)',
    'openai-compatible',
    'https://api.mistral.ai/v1',
    '/models',
    'Mistral API key',
    'Get a key at console.mistral.ai. OpenAI-compatible chat + models.',
  ),
  t(
    'nvidia',
    'NVIDIA (Build / NIM)',
    'openai-compatible',
    'https://integrate.api.nvidia.com/v1',
    '/models',
    'nvapi-…',
    'Get a key at build.nvidia.com → Generate Key. All NIM endpoints share this base.',
  ),
  t(
    'cerebras',
    'Cerebras',
    'openai-compatible',
    'https://api.cerebras.ai/v1',
    '/models',
    'csk-…',
    'Get a key at cloud.cerebras.ai. OpenAI-compatible chat API.',
  ),
  t(
    'siliconflow',
    'SiliconFlow',
    'openai-compatible',
    'https://api.siliconflow.cn/v1',
    '/models',
    'sk-… (SiliconFlow)',
    'CN endpoint shown; global alternative is https://api.siliconflow.com/v1. Key at cloud.siliconflow.cn → API keys.',
  ),
  t(
    'deepinfra',
    'DeepInfra',
    'openai-compatible',
    'https://api.deepinfra.com/v1/openai',
    '/models',
    'DeepInfra token',
    'Base is /v1/openai (OpenAI-compat layer). Handshake: GET /v1/openai/models.',
  ),
  t(
    'together',
    'Together AI',
    'openai-compatible',
    'https://api.together.xyz/v1',
    '/models',
    'Together API key',
    'Legacy base api.together.xyz/v1 (new docs use api.together.ai/v1 — both OpenAI-compatible). Key at api.together.xyz → Settings.',
  ),
  t(
    'groq',
    'Groq',
    'openai-compatible',
    'https://api.groq.com/openai/v1',
    '/models',
    'gsk_…',
    'Get a key at console.groq.com → API Keys. OpenAI base URL documented as https://api.groq.com/openai/v1.',
  ),
  t(
    'google-gemini',
    'Google Gemini (AI Studio)',
    'google',
    'https://generativelanguage.googleapis.com/v1beta',
    '/models',
    'AIza… (Gemini API key)',
    'Get a key in Google AI Studio (aistudio.google.com → Get API key). Uses ?key= query auth. OpenAI-compat alt: …/v1beta/openai/.',
  ),
  t(
    'azure-openai',
    'Microsoft Azure (OpenAI)',
    'openai-compatible',
    'https://YOUR-RESOURCE.openai.azure.com/openai/v1',
    '/models',
    'Azure OpenAI API key',
    'Replace YOUR-RESOURCE with your Azure resource name. New v1 route: {endpoint}/openai/v1. Key in Azure portal → Keys and Endpoint.',
  ),
];

export function templateByName(name: string): ProviderTemplate | undefined {
  return PROVIDER_TEMPLATES.find((p) => p.name.toLowerCase() === name.trim().toLowerCase());
}
