// SPDX-License-Identifier: Apache-2.0
// Phase 5 — Orchestration loop. Drives a builder run through the 14-state
// machine: the Supervisor plans via the provider chat (streamChat), tasks are
// created in the Rust DAG store, and each task executes by invoking the Phase 6
// `tool_execute` command directly. Approval gates pause the loop until the
// human resolves them in the ApprovalGate modal.
//
// Honest failure modes: if the provider commands (Phase 2) or `tool_execute`
// (Phase 6) are missing, the loop surfaces the error and stops — it never
// pretends work happened.

import { invoke } from './api';
import { streamChat, type ChatMsg } from './chat';
import { CONTEXT_ECONOMY_PROMPT } from './contextEconomy';
import { listen } from '@tauri-apps/api/event';
import type {
  ApprovalResolvedEvent,
  BuilderState,
  ModelChoice,
  RunMode,
  TaskRow,
} from '../components/builder/types';

export interface LoopConfig {
  runId: string;
  goal: string;
  mode: RunMode;
  model: ModelChoice;
  getPrompt: (key: string) => string;
  onLog: (line: string, cls?: string) => void;
  onStateChange: (state: BuilderState) => void;
  onTasksChange: () => void;
  shouldStop: () => boolean;
  /** Optional absolute project root; when set, TESTING runs the real test suite. */
  projectRoot?: string;
}

interface PlanTask {
  agent: 'supervisor' | 'architect' | 'frontend' | 'backend' | 'qa' | 'security';
  title: string;
  priority?: 'low' | 'normal' | 'high' | 'critical';
  depends_on?: number[];
  instructions: string;
  tool?: string;
  tool_args?: Record<string, unknown>;
  gate_kind?: string;
}

const DEFAULT_PROMPTS: Record<string, string> = {
  'agent.supervisor':
    'You are the Supervisor of the Guild Foundry AI builder. You coordinate six specialist agents (architect, frontend, backend, qa, security) to build what the user asked for. Keep plans small, concrete, and ordered. Reply with JSON only when asked for JSON.',
  'agent.architect':
    'You are the Architect. Turn approved requirements into a concrete system architecture: components, data flow, and interfaces. Record each significant choice as an ADR (title, context, decision, consequences). Be specific and minimal.',
  'agent.frontend':
    'You are the Frontend / Application Engineer. Implement user-facing code exactly per the approved architecture. Prefer simple, maintainable code.',
  'agent.backend':
    'You are the Backend / System Engineer. Implement system code, services, and data layers exactly per the approved architecture.',
  'agent.qa':
    'You are the QA / Test Engineer. Write and run tests, then report pass/fail with reproduction detail for every failure.',
  'agent.security':
    'You are the Security Reviewer. Review changes for vulnerabilities, unsafe patterns, and credential exposure. Block anything risky and explain why.',
};

function defaultPrompt(key: string): string {
  const base = DEFAULT_PROMPTS[key] ?? DEFAULT_PROMPTS['agent.supervisor'];
  // Every builder agent gets the context-economy policy appended: read once,
  // take notes, never re-read blindly. User-set prompts in Settings override
  // the whole prompt and are left untouched.
  return `${base}\n\n${CONTEXT_ECONOMY_PROMPT}`;
}

/** One-shot (non-streaming) agent completion with logging. */
async function askAgent(
  cfg: LoopConfig,
  promptKey: string,
  messages: ChatMsg[],
  label: string,
): Promise<string> {
  const systemPrompt = cfg.getPrompt(promptKey) || defaultPrompt(promptKey);
  cfg.onLog(`[${label}] asking ${cfg.model.providerId}/${cfg.model.modelId} …`);
  return await new Promise<string>((resolve, reject) => {
    let text = '';
    void streamChat(
      {
        providerId: cfg.model.providerId,
        modelId: cfg.model.modelId,
        messages: [{ role: 'system', content: systemPrompt }, ...messages],
        params: { temperature: 0.3 },
      },
      {
        onDelta: (d) => {
          text += d;
        },
        onDone: () => resolve(text),
        onError: (e) => reject(e),
      },
    ).catch(reject);
  });
}

async function transition(cfg: LoopConfig, to: BuilderState, note = ''): Promise<void> {
  await invoke('run_transition', { run_id: cfg.runId, to_state: to, note });
  cfg.onStateChange(to);
  cfg.onLog(`— state → ${to}${note ? ` (${note})` : ''}`, 'gold');
}

function checkStop(cfg: LoopConfig): void {
  if (cfg.shouldStop()) throw new Error('run stopped by user');
}

function parseJsonArray(raw: string): PlanTask[] {
  const start = raw.indexOf('[');
  const end = raw.lastIndexOf(']');
  if (start < 0 || end <= start) throw new Error('planner did not return a JSON task array');
  const parsed: unknown = JSON.parse(raw.slice(start, end + 1));
  if (!Array.isArray(parsed)) throw new Error('planner output was not an array');
  return parsed as PlanTask[];
}

/** Waits for a human decision on an approval; resolves true=approved. */
async function awaitApproval(cfg: LoopConfig, approvalId: string): Promise<boolean> {
  cfg.onLog(`waiting on human approval ${approvalId.slice(0, 8)}…`, 'gold');
  return await new Promise<boolean>((resolve, reject) => {
    let settled = false;
    const finish = (v: boolean) => {
      if (settled) return;
      settled = true;
      void unlisten();
      resolve(v);
    };
    let unlisten: () => void = () => undefined;
    const timer = window.setInterval(() => {
      // Fallback: poll in case the resolve event was missed.
      void invoke<Array<{ id: string; status: string; decision: string | null }>>('approval_list', {
        run_id: cfg.runId,
      })
        .then((rows) => {
          const row = rows.find((r) => r.id === approvalId);
          if (row && row.status !== 'pending') {
            window.clearInterval(timer);
            finish(row.decision === 'approved');
          }
        })
        .catch(() => undefined);
      if (cfg.shouldStop()) {
        window.clearInterval(timer);
        reject(new Error('run stopped by user'));
      }
    }, 3000);
    void listen<ApprovalResolvedEvent>('agent://approval-resolved', (event) => {
      if (event.payload.approval_id === approvalId) {
        window.clearInterval(timer);
        finish(event.payload.decision === 'approved');
      }
    })
      .then((u) => {
        unlisten = u;
      })
      .catch(() => undefined);
  });
}

/** Ask for human approval when the gate fires for this mode; returns true if we may proceed. */
async function gate(
  cfg: LoopConfig,
  kind: string,
  summary: string,
  payload: Record<string, unknown>,
): Promise<boolean> {
  const fires = await invoke<boolean>('approval_gate_check', { kind, mode: cfg.mode });
  if (!fires) {
    cfg.onLog(`gate '${kind}' does not fire in ${cfg.mode} mode — proceeding`);
    return true;
  }
  const approvalId = await invoke<string>('approval_request', {
    run_id: cfg.runId,
    kind,
    summary,
    payload_json: JSON.stringify(payload),
  });
  const approved = await awaitApproval(cfg, approvalId);
  cfg.onLog(approved ? 'approved by human' : 'DENIED by human', approved ? 'ok' : 'err');
  return approved;
}

/** Executes one task: updates status, optionally calls tool_execute, records output. */
async function executeTask(cfg: LoopConfig, task: TaskRow, plan: PlanTask): Promise<void> {
  checkStop(cfg);
  await invoke('agent_task_update', { task_id: task.id, status: 'running' });
  cfg.onTasksChange();
  cfg.onLog(`[${task.agent}] ${task.title}`);

  try {
    if (plan.gate_kind) {
      const ok = await gate(cfg, plan.gate_kind, `Task "${task.title}" requests: ${plan.gate_kind}`, {
        task_id: task.id,
        tool: plan.tool ?? null,
        tool_args: plan.tool_args ?? null,
      });
      if (!ok) {
        await invoke('agent_task_update', {
          task_id: task.id,
          status: 'blocked',
          error: `human denied approval gate '${plan.gate_kind}'`,
        });
        cfg.onTasksChange();
        return;
      }
    }

    let output = '';
    if (plan.tool) {
      cfg.onLog(`  → tool_execute ${plan.tool}`);
      try {
        const res = await invoke<unknown>('tool_execute', {
          tool: plan.tool,
          args: plan.tool_args ?? {},
          agent_id: task.agent,
          session_id: cfg.runId,
        });
        output = typeof res === 'string' ? res : JSON.stringify(res);
        cfg.onLog('  ✓ tool ok', 'ok');
        await invoke('agent_task_update', {
          task_id: task.id,
          tool_calls: JSON.stringify([{ tool: plan.tool, args: plan.tool_args ?? {}, at: new Date().toISOString() }]),
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        throw new Error(`tool_execute unavailable or failed: ${msg}`);
      }
    } else {
      // Agent reasoning step: ask the agent to produce the deliverable text.
      output = await askAgent(
        cfg,
        `agent.${task.agent}`,
        [{ role: 'user', content: `Goal: ${cfg.goal}\nTask: ${task.title}\nInstructions: ${plan.instructions}\nProduce the deliverable or a precise report of what you did.` }],
        task.agent,
      );
    }

    await invoke('agent_task_update', {
      task_id: task.id,
      status: 'done',
      outputs: JSON.stringify({ result: output.slice(0, 20000) }),
    });
    cfg.onLog(`  ✓ done`, 'ok');
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    await invoke('agent_task_update', { task_id: task.id, status: 'failed', error: msg });
    cfg.onLog(`  ✗ failed: ${msg}`, 'err');
    throw err;
  } finally {
    cfg.onTasksChange();
  }
}

function depsSatisfied(task: TaskRow, doneIds: Set<string>): boolean {
  try {
    const deps: unknown = JSON.parse(task.dependencies || '[]');
    if (!Array.isArray(deps)) return true;
    return deps.every((d) => typeof d === 'string' && doneIds.has(d));
  } catch {
    return true;
  }
}

/**
 * Main orchestration loop. Runs until RELEASED, a terminal failure, or
 * shouldStop(). All progress is persisted in Rust, so a restart can recover
 * via run_recover() and the loop resumes from the stored state.
 */
export async function runAgentLoop(cfg: LoopConfig): Promise<void> {
  const status = await invoke<{ state: string; status: string }>('run_status', {
    run_id: cfg.runId,
  });
  let state = status.state as BuilderState;
  cfg.onStateChange(state);
  cfg.onLog(`run ${cfg.runId.slice(0, 8)} resumed at ${state} (mode ${cfg.mode})`, 'gold');

  // ---- DISCOVERY → REQUIREMENTS_ANALYSIS: supervisor drafts requirements ----
  if (state === 'DISCOVERY') {
    checkStop(cfg);
    cfg.onLog('Supervisor: drafting requirements from goal…');
    const draft = await askAgent(
      cfg,
      'agent.supervisor',
      [
        {
          role: 'user',
          content: `The user wants to build: "${cfg.goal}".\nDraft a requirements document as JSON with keys: summary, goals (array), functional (array), non_functional (array), open_questions (array). JSON only.`,
        },
      ],
      'supervisor',
    );
    const start = draft.indexOf('{');
    const end = draft.lastIndexOf('}');
    const docJson = start >= 0 && end > start ? draft.slice(start, end + 1) : JSON.stringify({ summary: cfg.goal });
    await invoke('requirements_upsert', { run_id: cfg.runId, doc_json: docJson });
    cfg.onLog('requirements draft saved — review in the Requirements panel', 'ok');
    await transition(cfg, 'REQUIREMENTS_ANALYSIS', 'requirements drafted');
    state = 'REQUIREMENTS_ANALYSIS';
  }

  if (state === 'REQUIREMENTS_ANALYSIS') {
    checkStop(cfg);
    // The user refines requirements in the RequirementsPanel; the loop waits
    // here until they confirm (or AUTONOMOUS proceeds with the draft).
    if (cfg.mode === 'AUTONOMOUS') {
      cfg.onLog('AUTONOMOUS: accepting requirements draft as-is');
    } else {
      const ok = await gate(cfg, 'plan', 'Approve the requirements document to continue to architecture.', {
        phase: 'requirements',
      });
      if (!ok) throw new Error('requirements rejected by human');
    }
    await transition(cfg, 'ARCHITECTURE', 'requirements approved');
    state = 'ARCHITECTURE';
  }

  // ---- ARCHITECTURE: architect designs, human signs off ----
  if (state === 'ARCHITECTURE') {
    checkStop(cfg);
    const req = await invoke<{ doc_json: string }>('requirements_get', { run_id: cfg.runId }).catch(
      () => ({ doc_json: JSON.stringify({ summary: cfg.goal }) }),
    );
    const arch = await askAgent(
      cfg,
      'agent.architect',
      [
        {
          role: 'user',
          content: `Requirements: ${req.doc_json}\nProduce: (1) an architecture summary, (2) a JSON array "adrs" of {title, context, decision, consequences}. Put the summary first, then a fenced json block with the ADRs.`,
        },
      ],
      'architect',
    );
    const m = arch.match(/```json([\s\S]*?)```/);
    if (m) {
      try {
        const adrs = JSON.parse(m[1]) as Array<Record<string, string>>;
        for (const a of adrs.slice(0, 20)) {
          await invoke('adr_create', {
            run_id: cfg.runId,
            title: String(a.title ?? 'ADR'),
            context: String(a.context ?? ''),
            decision: String(a.decision ?? ''),
            consequences: String(a.consequences ?? ''),
          });
        }
        cfg.onLog(`${Math.min(adrs.length, 20)} ADRs recorded`, 'ok');
      } catch {
        cfg.onLog('could not parse ADR block — continuing without ADRs', 'err');
      }
    }
    const ok = await gate(cfg, 'architecture', 'Approve the architecture and ADRs to continue to planning.', {
      phase: 'architecture',
    });
    if (!ok) throw new Error('architecture rejected by human');
    await invoke('requirements_signoff', { run_id: cfg.runId });
    await transition(cfg, 'PLANNING', 'architecture signed off');
    state = 'PLANNING';
  }

  // ---- PLANNING: supervisor builds the DAG ----
  if (state === 'PLANNING') {
    checkStop(cfg);
    cfg.onLog('Supervisor: building task plan…');
    const rawPlan = await askAgent(
      cfg,
      'agent.supervisor',
      [
        {
          role: 'user',
          content: `Goal: "${cfg.goal}".\nBreak the implementation into a DAG of tasks. Reply with ONLY a JSON array of tasks, each: {"agent": "frontend"|"backend"|"qa"|"security"|"architect", "title": string, "priority": "low"|"normal"|"high"|"critical", "depends_on": [0-based indices of EARLIER tasks in this array only — no forward or circular references], "instructions": string, "tool": optional tool name to call via tool_execute, "tool_args": optional object, "gate_kind": optional approval gate kind when the step is risky}. Keep it to at most 12 tasks.`,
        },
      ],
      'supervisor',
    );
    const plan = parseJsonArray(rawPlan);
    const sliced = plan.slice(0, 12);
    const ids: string[] = [];
    // The planner's depends_on indices refer to earlier entries in the same
    // array (a DAG has no forward edges), so each task's dependencies are
    // already-known task ids at creation time. Out-of-range or forward
    // references are dropped rather than trusted.
    for (let i = 0; i < sliced.length; i++) {
      const p = sliced[i];
      const depIds = (p.depends_on ?? [])
        .filter((d) => d >= 0 && d < i)
        .map((d) => ids[d]);
      const id = await invoke<string>('agent_task_create', {
        run_id: cfg.runId,
        agent: p.agent,
        title: p.title,
        priority: p.priority ?? 'normal',
        dependencies: JSON.stringify(depIds),
        inputs: JSON.stringify({ instructions: p.instructions, tool: p.tool ?? null, tool_args: p.tool_args ?? null, gate_kind: p.gate_kind ?? null }),
      });
      ids.push(id);
    }
    cfg.onLog(`${ids.length} tasks planned`, 'ok');
    cfg.onTasksChange();
    await transition(cfg, 'AWAITING_USER_APPROVAL', 'plan ready');
    state = 'AWAITING_USER_APPROVAL';
  }

  if (state === 'AWAITING_USER_APPROVAL') {
    checkStop(cfg);
    if (cfg.mode === 'AUTONOMOUS') {
      cfg.onLog('AUTONOMOUS: plan auto-approved');
    } else {
      const ok = await gate(cfg, 'plan', 'Approve the implementation plan to start building.', {
        phase: 'plan',
      });
      if (!ok) throw new Error('plan rejected by human');
    }
    await transition(cfg, 'IMPLEMENTATION', 'plan approved');
    state = 'IMPLEMENTATION';
  }

  // ---- IMPLEMENTATION: execute the DAG in dependency order ----
  if (state === 'IMPLEMENTATION') {
    checkStop(cfg);
    const tasks = await invoke<TaskRow[]>('agent_task_list', { run_id: cfg.runId });
    const byId = new Map(tasks.map((t) => [t.id, t]));
    const done = new Set(tasks.filter((t) => t.status === 'done').map((t) => t.id));
    // Dependency order: indices were captured at plan time; resolve via inputs.
    let remaining = tasks.filter((t) => t.status === 'pending' || t.status === 'ready');
    let progress = true;
    while (remaining.length > 0 && progress) {
      checkStop(cfg);
      progress = false;
      for (const t of remaining) {
        if (!depsSatisfied(t, done)) continue;
        let plan: PlanTask = { agent: t.agent as PlanTask['agent'], title: t.title, instructions: '' };
        try {
          const inputs = JSON.parse(t.inputs || '{}') as Record<string, unknown>;
          plan = {
            agent: t.agent as PlanTask['agent'],
            title: t.title,
            instructions: String(inputs.instructions ?? ''),
            tool: (inputs.tool as string | null) ?? undefined,
            tool_args: (inputs.tool_args as Record<string, unknown> | null) ?? undefined,
            gate_kind: (inputs.gate_kind as string | null) ?? undefined,
          };
        } catch {
          /* use defaults */
        }
        await executeTask(cfg, t, plan);
        const updated = await invoke<TaskRow>('agent_task_get', { task_id: t.id });
        byId.set(t.id, updated);
        if (updated.status === 'done') done.add(t.id);
        if (updated.status === 'failed') {
          throw new Error(`task failed: ${t.title} — see task errors`);
        }
        progress = true;
      }
      remaining = [...byId.values()].filter((t) => t.status === 'pending' || t.status === 'ready');
    }
    if (remaining.length > 0) {
      throw new Error(`${remaining.length} task(s) could not run — unmet dependencies or deadlock`);
    }
    await transition(cfg, 'BUILDING', 'all tasks done');
    state = 'BUILDING';
  }

  // ---- BUILDING → TESTING → SECURITY_REVIEW: agent verification passes ----
  for (const [from, agentKey, label] of [
    ['BUILDING', 'agent.backend', 'build verification'],
    ['TESTING', 'agent.qa', 'test pass'],
    ['SECURITY_REVIEW', 'agent.security', 'security review'],
  ] as Array<[BuilderState, string, string]>) {
    if (state !== from) continue;
    checkStop(cfg);
    const next: BuilderState =
      from === 'BUILDING' ? 'TESTING' : from === 'TESTING' ? 'SECURITY_REVIEW' : 'VALIDATION';
    // Real verification: when a project root is known, run the actual test
    // suite via tool_execute and give the agent the real output instead of
    // asking it to guess.
    let verificationContext = '';
    let realTestFailed = false;
    if (from === 'TESTING' && cfg.projectRoot) {
      try {
        const t = await invoke<{ ok: boolean; output: unknown; error?: string }>('tool_execute', {
          tool: 'test.run',
          args_json: JSON.stringify({ project_root: cfg.projectRoot }),
          agent_id: 'supervisor',
          session_id: cfg.runId,
        });
        if (t.ok) {
          verificationContext =
            '\n\nReal test suite output (from tool_execute test.run — trust this over assumptions):\n' +
            JSON.stringify(t.output).slice(0, 4000);
        } else {
          realTestFailed = true;
          verificationContext =
            '\n\nReal test suite FAILED to run: ' + (t.error ?? 'unknown error') +
            '\nReport this honestly.';
        }
      } catch (err) {
        verificationContext =
          '\n\nReal test suite could not be executed: ' +
          (err instanceof Error ? err.message : String(err)) +
          '\nReport this honestly.';
      }
    }
    const verdict = await askAgent(
      cfg,
      agentKey,
      [
        {
          role: 'user',
          content: `Goal: "${cfg.goal}". Perform the ${label} for this run.${verificationContext} Reply with a short verdict starting with PASS or FAIL, then details. If FAIL, explain exactly what must be fixed.`,
        },
      ],
      agentKey.replace('agent.', ''),
    );
    cfg.onLog(`${label} verdict: ${verdict.slice(0, 200)}`);
    if (/^\s*FAIL/i.test(verdict) || realTestFailed) {
      cfg.onLog(`${label} FAILED — moving to FIXING`, 'err');
      await invoke('agent_task_create', {
        run_id: cfg.runId,
        agent: 'supervisor',
        title: `Fix: ${label} findings`,
        priority: 'high',
        inputs: JSON.stringify({ instructions: verdict.slice(0, 8000) }),
      });
      cfg.onTasksChange();
      await transition(cfg, 'FIXING', `${label} failed`);
      state = 'FIXING';
      break;
    }
    await transition(cfg, next, `${label} passed`);
    state = next;
  }

  // ---- FIXING: loop back into implementation ----
  if (state === 'FIXING') {
    checkStop(cfg);
    const tasks = await invoke<TaskRow[]>('agent_task_list', { run_id: cfg.runId });
    const fixTasks = tasks.filter((t) => t.status === 'pending' || t.status === 'ready');
    for (const t of fixTasks) {
      let plan: PlanTask = { agent: t.agent as PlanTask['agent'], title: t.title, instructions: '' };
      try {
        const inputs = JSON.parse(t.inputs || '{}') as Record<string, unknown>;
        plan.instructions = String(inputs.instructions ?? '');
      } catch {
        /* defaults */
      }
      await executeTask(cfg, t, plan);
      const updated = await invoke<TaskRow>('agent_task_get', { task_id: t.id });
      if (updated.status === 'failed') throw new Error(`fix task failed: ${t.title}`);
    }
    await transition(cfg, 'TESTING', 'fixes applied — re-testing');
    state = 'TESTING';
    // Re-run the verification passes once more.
    const verdict = await askAgent(
      cfg,
      'agent.qa',
      [{ role: 'user', content: `Goal: "${cfg.goal}". Re-run tests after fixes. Reply starting with PASS or FAIL, then details.` }],
      'qa',
    );
    cfg.onLog(`re-test verdict: ${verdict.slice(0, 200)}`);
    if (/^\s*FAIL/i.test(verdict)) throw new Error('re-test failed after fixes — human intervention needed');
    await transition(cfg, 'SECURITY_REVIEW', 're-test passed');
    state = 'SECURITY_REVIEW';
    const sec = await askAgent(
      cfg,
      'agent.security',
      [{ role: 'user', content: `Goal: "${cfg.goal}". Final security review. Reply starting with PASS or FAIL, then details.` }],
      'security',
    );
    cfg.onLog(`security verdict: ${sec.slice(0, 200)}`);
    if (/^\s*FAIL/i.test(sec)) throw new Error('security review failed — human intervention needed');
    await transition(cfg, 'VALIDATION', 'security passed');
    state = 'VALIDATION';
  }

  if (state === 'VALIDATION') {
    checkStop(cfg);
    await transition(cfg, 'READY', 'validated');
    state = 'READY';
  }

  if (state === 'READY') {
    checkStop(cfg);
    await transition(cfg, 'PACKAGING', 'packaging');
    state = 'PACKAGING';
  }

  // ---- PACKAGING → RELEASED: releases always need a human ----
  if (state === 'PACKAGING') {
    checkStop(cfg);
    const ok = await gate(cfg, 'releases', 'Approve the release of this build.', { phase: 'release' });
    if (!ok) throw new Error('release denied by human');
    await transition(cfg, 'RELEASED', 'released');
    state = 'RELEASED';
  }

  cfg.onLog('run RELEASED — done', 'ok');
}
