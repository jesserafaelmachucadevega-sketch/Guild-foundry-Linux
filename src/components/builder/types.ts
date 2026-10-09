// Phase 5 — Builder/orchestration types. Phase-local; do not move into src/lib/types.ts.

export type RunMode = 'SAFE' | 'ASSISTED' | 'AUTONOMOUS';

export const BUILDER_STATES = [
  'DISCOVERY',
  'REQUIREMENTS_ANALYSIS',
  'ARCHITECTURE',
  'PLANNING',
  'AWAITING_USER_APPROVAL',
  'IMPLEMENTATION',
  'BUILDING',
  'TESTING',
  'SECURITY_REVIEW',
  'FIXING',
  'VALIDATION',
  'READY',
  'PACKAGING',
  'RELEASED',
] as const;

export type BuilderState = (typeof BUILDER_STATES)[number];

export interface AgentInfo {
  id: string;
  name: string;
  kind: string;
  role: string;
  prompt_key: string;
}

export interface RunSummary {
  id: string;
  kind: string;
  goal: string;
  mode: string;
  state: string;
  status: string;
  started_at: string;
  ended_at: string | null;
  summary: string;
  task_counts: Record<string, number>;
  pending_approvals: number;
}

export interface TaskRow {
  id: string;
  run_id: string;
  parent_id: string | null;
  agent: string;
  title: string;
  priority: string;
  status: string;
  dependencies: string;
  inputs: string;
  outputs: string;
  artifacts: string;
  tool_calls: string;
  errors: string;
  created_at: string;
  updated_at: string;
  started_at: string | null;
  ended_at: string | null;
}

export interface ApprovalRow {
  id: string;
  run_id: string;
  kind: string;
  summary: string;
  payload_json: string;
  status: string;
  decision: string | null;
  reason: string;
  requested_at: string;
  resolved_at: string | null;
}

export interface ApprovalEvent {
  approval_id: string;
  run_id: string;
  kind: string;
  summary: string;
  payload_json: string;
  requested_at: string;
}

export interface ApprovalResolvedEvent {
  approval_id: string;
  decision: string;
}

export interface TransitionRow {
  id: string;
  from_state: string;
  to_state: string;
  note: string;
  at: string;
}

export interface RequirementsDoc {
  run_id: string;
  doc_json: string;
  architecture_approved: boolean;
  updated_at: string;
}

export interface AdrRow {
  id: string;
  run_id: string;
  title: string;
  context: string;
  decision: string;
  consequences: string;
  created_at: string;
}

export interface ModelChoice {
  providerId: string;
  modelId: string;
}
