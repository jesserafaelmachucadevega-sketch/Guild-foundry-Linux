// Shared types for Guild Foundry AI (renderer + main agree on these shapes).

export type ViewMode = 'desktop' | 'compact' | 'pwa';
export type Workspace = 'builder' | 'conference';

export type ProviderKind =
  | 'openai'
  | 'openai-compatible'
  | 'anthropic'
  | 'google'
  | 'ollama';

export interface ProviderConfig {
  id: string;
  name: string;
  kind: ProviderKind;
  baseUrl: string;
  modelsPath: string; // e.g. "/v1/models"
  apiKeyEnvLabel: string; // label shown in Settings, e.g. "API key"
}

export type ConnectionStatus =
  | { state: 'idle' }
  | { state: 'checking' }
  | { state: 'connected'; latencyMs: number }
  | { state: 'error'; code: number | string; detail: string };

export interface ModelInfo {
  id: string;
  name: string;
  providerId: string;
  free: boolean;
  tags: string[];
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'winner';
  content: string;
  modelId?: string;
  modelName?: string;
  createdAt: number;
  parentId?: string | null;
  artifacts?: Artifact[];
}

export type ArtifactKind = 'poll' | 'checklist' | 'slider' | 'card';

export interface Artifact {
  kind: ArtifactKind;
  title: string;
  // poll
  options?: string[];
  // checklist
  items?: { label: string; checked: boolean }[];
  // slider
  min?: number;
  max?: number;
  value?: number;
  // card
  body?: string;
}

export interface AgentConfig {
  id: string;
  name: string;
  systemPrompt: string;
  providerId: string;
  modelId: string;
}

export interface ToastMsg {
  id: number;
  text: string;
}

export type BuildPhase =
  | 'DISCOVERY'
  | 'PLANNING'
  | 'AWAITING_USER_APPROVAL'
  | 'IMPLEMENTATION'
  | 'BUILDING'
  | 'TESTING'
  | 'FIXING'
  | 'READY';

export interface ProjectFile {
  path: string;
  content: string;
  updatedAt: number;
}

export interface PermissionRule {
  agentId: string;
  tool: string;
  mode: 'ALLOW_ALWAYS' | 'ASK_USER' | 'DENY';
}

export interface Diagnostics {
  models: { modelId: string; state: string }[];
  memoryMb: number;
  tokensPerSec: number;
  buildWorker: string;
}
