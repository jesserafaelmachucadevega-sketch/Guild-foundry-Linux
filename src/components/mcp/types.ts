// SPDX-License-Identifier: Apache-2.0
// Phase 7 — Phase-local types for the MCP gateway + permission center UI.
// These mirror the serde shapes returned by src-tauri/src/mcp.rs (snake_case).

export type McpTransport = 'streamable_http' | 'sse';
export type McpAuthType = 'none' | 'bearer' | 'oauth';

export interface McpServerView {
  id: string;
  name: string;
  transport: string;
  url: string;
  enabled: boolean;
  auth_type: string;
  bearer_present: boolean;
  oauth_configured: boolean;
  oauth_connected: boolean;
  allowed_tools: string[];
  denied_tools: string[];
  tool_count: number;
}

export interface McpServerInput {
  name: string;
  transport: McpTransport;
  url: string;
  auth_type: McpAuthType;
  oauth_client_id?: string;
  oauth_authorization_url?: string;
  oauth_token_url?: string;
  oauth_scopes?: string;
  oauth_redirect_uri?: string;
  allowed_tools?: string[];
  denied_tools?: string[];
  bearer_present?: boolean;
}

export interface McpToolInfo {
  name: string;
  description: string;
  input_schema: unknown;
  output_schema: unknown;
  risk_class: string;
  enabled: boolean;
  server_id: string;
  server_name: string;
}

export interface McpTestResult {
  ok: boolean;
  latency_ms: number;
  tool_count: number;
  server_name: string;
  protocol_version: string;
  error: string | null;
}

export interface McpCallAudit {
  at: string;
  server_id: string;
  tool: string;
  args_sha256: string;
  decision: string;
  policy: string;
}

export interface McpCallResult {
  ok: boolean;
  result: unknown;
  is_error: boolean;
  latency_ms: number;
  approval_required: boolean;
  audit: McpCallAudit;
}

export interface RegistryTool {
  name: string;
  description: string;
  provider: string | null;
  mcp_server: string | null;
  risk_class: string;
  enabled: boolean;
  input_schema: unknown;
  output_schema: unknown;
  permission_requirements: string[];
  audit_policy: string;
}

export type PermLevel =
  | 'always_allow'
  | 'ask_every_time'
  | 'allow_project'
  | 'allow_session'
  | 'deny';

export interface PermEntry {
  domain: string;
  level: PermLevel;
}

export interface PermDomainMeta {
  id: string;
  label: string;
  blurb: string;
}

export const PERM_DOMAINS: PermDomainMeta[] = [
  { id: 'filesystem', label: 'Filesystem', blurb: 'Read and write files outside the project sandbox' },
  { id: 'network', label: 'Network', blurb: 'Outbound network requests to any host' },
  { id: 'shell', label: 'Shell', blurb: 'Execute shell commands and subprocesses' },
  { id: 'git', label: 'Git', blurb: 'Clone, commit, push, and other VCS operations' },
  { id: 'mcp', label: 'MCP', blurb: 'Call tools on connected MCP servers' },
  { id: 'browser', label: 'Browser', blurb: 'Drive web pages and read page content' },
  { id: 'credentials', label: 'Credentials', blurb: 'Read or use stored secrets and tokens' },
  { id: 'external_apis', label: 'External APIs', blurb: 'Call third-party APIs beyond model providers' },
  { id: 'media', label: 'Media generation', blurb: 'Generate images via the Fal API (costs per image)' },
];

export interface PermLevelMeta {
  id: PermLevel;
  label: string;
  blurb: string;
}

export const PERM_LEVELS: PermLevelMeta[] = [
  { id: 'always_allow', label: 'Always allow', blurb: 'Never ask' },
  { id: 'ask_every_time', label: 'Ask every time', blurb: 'Approve each action' },
  { id: 'allow_project', label: 'Allow for project', blurb: 'Approved within the active project' },
  { id: 'allow_session', label: 'Allow for session', blurb: 'Approved until the app restarts' },
  { id: 'deny', label: 'Deny', blurb: 'Block outright' },
];
