// SPDX-License-Identifier: Apache-2.0
// Phase 5 — Builder Studio: goal input, autonomous-mode select, run controls,
// the 14-state progress rail, task graph, requirements panel, run history,
// and the orchestration log. Drives src/lib/agentLoop.ts.

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '../../lib/api';
import { getProviders, listModels } from '../../lib/providers';
import { runAgentLoop } from '../../lib/agentLoop';
import TaskGraph from './TaskGraph';
import ApprovalGate from './ApprovalGate';
import RunHistory from './RunHistory';
import RequirementsPanel from './RequirementsPanel';
import './builder.css';
import {
  BUILDER_STATES,
  type AgentInfo,
  type BuilderState,
  type ModelChoice,
  type RunMode,
  type RunSummary,
} from './types';

interface LogLine {
  text: string;
  cls?: string;
  at: string;
}

const MODES: RunMode[] = ['SAFE', 'ASSISTED', 'AUTONOMOUS'];

const MODE_HELP: Record<RunMode, string> = {
  SAFE: 'Every approval gate asks a human first.',
  ASSISTED: 'Only high-risk gates ask; the rest proceed.',
  AUTONOMOUS: 'Only credentials, pushes, and releases ask.',
};

export default function BuilderStudio() {
  const [goal, setGoal] = useState('');
  const [mode, setMode] = useState<RunMode>('SAFE');
  const [providers, setProviders] = useState<Array<{ id: string; name: string; status: string }>>([]);
  const [providerId, setProviderId] = useState('');
  const [models, setModels] = useState<Array<{ id: string; name: string }>>([]);
  const [modelId, setModelId] = useState('');
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [run, setRun] = useState<RunSummary | null>(null);
  const [log, setLog] = useState<LogLine[]>([]);
  const [taskTick, setTaskTick] = useState(0);
  const [historyTick, setHistoryTick] = useState(0);
  const [tab, setTab] = useState<'tasks' | 'requirements' | 'history'>('tasks');
  const [error, setError] = useState('');
  const [projectRoot, setProjectRoot] = useState('');
  const [prompts, setPrompts] = 
useState<Record<string, string>>({});
  const stopRef = useRef(false);
  const runningRef = useRef(false);

  const logLine = useCallback((text: string, cls?: string) => {
    setLog((prev) => [...prev.slice(-400), { text, cls, at: new Date().toLocaleTimeString() }]);
  }, []);

  const refreshRun = useCallback(async (id: string) => {
    try {
      const s = await invoke<RunSummary>('run_status', { run_id: id });
      setRun(s);
      return s;
    } catch (e: unknown) {
      logLine(`run_status failed: ${e instanceof Error ? e.message : String(e)}`, 'err');
      return null;
    }
  }, [logLine]);

  useEffect(() => {
    invoke<AgentInfo[]>('agent_list')
      .then(setAgents)
      .catch((e: unknown) => logLine(`agent_list: ${e instanceof Error ? e.message : String(e)}`, 'err'));
    invoke<Record<string, string>>('settings_load_prompts')
      .then(setPrompts)
      .catch(() => undefined);
    // Default project root (same convention as the Projects panel). Optional:
    // when empty, the TESTING state skips real test execution.
    invoke<string>('ws_project_root', { root: null })
      .then(setProjectRoot)
      .catch(() => undefined);
    getProviders()
      .then((ps) => {
        setProviders(ps.map((p) => ({ id: p.id, name: p.name, status: p.status })));
        const first = ps.find((p) => p.status === 'connected') ?? ps[0];
        if (first) setProviderId(first.id);
      })
      .catch((e: unknown) => logLine(`providers: ${e instanceof Error ? e.message : String(e)}`, 'err'));
  }, [logLine]);

  useEffect(() => {
    if (!providerId) return;
    listModels(providerId)
      .then((ms) => {
        setModels(ms.map((m) => ({ id: m.id, name: m.name })));
        if (ms[0]) setModelId(ms[0].id);
      })
      .catch(() => setModels([]));
  }, [providerId]);

  const model: ModelChoice = { providerId, modelId };

  const launchLoop = useCallback(
    async (runId: string, runGoal: string, runMode: RunMode) => {
      if (runningRef.current) return;
      runningRef.current = true;
      stopRef.current = false;
      setError('');
      logLine(`starting orchestration loop (mode ${runMode})`, 'gold');
      try {
        await runAgentLoop({
          runId,
          goal: runGoal,
         
 mode: runMode,
          model: { providerId, modelId },
          getPrompt: (key) => prompts[key] ?? '',
          onLog: logLine,
          onStateChange: (s: BuilderState) =>
            setRun((r) => (r ? { ...r, state: s } : r)),
          onTasksChange: () => {
            setTaskTick((t) => t + 1);
            void refreshRun(runId);
          },
          shouldStop: () => stopRef.current,
          projectRoot: projectRoot || undefined,
        });
      } catch (e: unknown) {
        const msg = e instanceof Error ? e.message : String(e);
        logLine(`loop ended: ${msg}`, 'err');
        setError(msg);
      } finally {
        runningRef.current = false;
        void refreshRun(runId);
        setHistoryTick((t) => t + 1);
      }
    },
    [providerId, modelId, prompts, logLine, refreshRun],
  );

  const startRun = async () => {
    const g = goal.trim();
    if (!g) {
      setError('Describe what to build first.');
      return;
    }
    if (!providerId || !modelId) {
      setError('Pick a provider and model first.');
      return;
    }
    setError('');
    try {
      const runId = await invoke<string>('run_start', { goal: g, mode });
      logLine(`run started: ${runId.slice(0, 8)}`, 'ok');
      await refreshRun(runId);
      setHistoryTick((t) => t + 1);
      void launchLoop(runId, g, mode);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const cancelRun = async () => {
    if (!run) return;
    stopRef.current = true;
    try {
      await invoke('run_cancel', { run_id: run.id });
      logLine('run cancelled', 'err');
    } catch (e: unknown) {
      logLine(`cancel: ${e instanceof Error ? e.message : String(e)}`, 'err');
    }
    void refreshRun(run.id);
    setHistoryTick((t) => t + 1);
  };

  const recoverRuns = async () => {
    setError('');
    try {
      const found = await invoke<RunSummary[]>('run_recover');
      if (found.length === 0) {
        logLine('no interrupted runs to recover', 'gold');
        return;
    
  }
      const r = found[0];
      logLine(`recovering run ${r.id.slice(0, 8)} at ${r.state}`, 'gold');
      setRun(r);
      void launchLoop(r.id, r.goal, r.mode as RunMode);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const selectHistoryRun = (r: RunSummary) => {
    if (runningRef.current) {
      setError('A run is active — cancel it before inspecting another.');
      return;
    }
    setRun(r);
    setGoal(r.goal);
    setMode(r.mode as RunMode);
    void refreshRun(r.id);
    if (r.status === 'running') {
      logLine(`resuming run ${r.id.slice(0, 8)} at ${r.state}`, 'gold');
      void launchLoop(r.id, r.goal, r.mode as RunMode);
    }
  };

  const stateIdx = run ? BUILDER_STATES.indexOf(run.state as BuilderState) : -1;

  return (
    <div className="bld-wrap">
      <ApprovalGate />
      <div className="bld-grid">
        <div>
          <div className="bld-panel">
            <h3>New build</h3>
            <div className="gf-label">Goal</div>
            <textarea
              className="gf-textarea"
              style={{ width: '100%', minHeight: 70 }}
              placeholder="Describe what to build…"
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
            />
            <div className="gf-label" style={{ marginTop: 8 }}>Project root</div>
            <input
              className="gf-input"
              style={{ width: '100%' }}
              placeholder="/absolute/path/to/project (optional — enables real builds and tests)"
              value={projectRoot}
              onChange={(e) => setProjectRoot(e.target.value)}
            />
            <div className="gf-label" style={{ marginTop: 8 }}>Autonomy mode</div>
            <select className="gf-select" value={mode} onChange={(e) => setMode(e.target.value as RunMode)} style={{ width: '100%' }}>
              {MODES.map((m) => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
            <div className="gf-muted" style={{ fontSize: 11, margin: '4px 0 8px' }}>{MODE_HELP[mode]}</div>
            <div className="gf-label">Provider</div>
            <select className="gf-select" value={providerId} onChange={(e) => setProviderId(e.target.value)} style={{ width: '100%' }}>
              {providers.map((p) => (
                <optio
n key={p.id} value={p.id}>{p.name} ({p.status})</option>
              ))}
            </select>
            <div className="gf-label" style={{ marginTop: 8 }}>Model</div>
            <select className="gf-select" value={modelId} onChange={(e) => setModelId(e.target.value)} style={{ width: '100%' }}>
              {models.map((m) => (
                <option key={m.id} value={m.id}>{m.name}</option>
              ))}
            </select>
            <div className="bld-form-row" style={{ marginTop: 12 }}>
              <button className="gf-btn primary" onClick={() => void startRun()} disabled={runningRef.current}>
                Start build
              </button>
              <button className="gf-btn danger" onClick={() => void cancelRun()} disabled={!run || run.status !== 'running'}>
                Cancel
              </button>
              <button className="gf-btn" onClick={() => void recoverRuns()}>
                Recover
              </button>
            </div>
            {error && <div className="bld-error">{error}</div>}
          </div>

          <div className="bld-panel" style={{ marginTop: 12 }}>
            <h3>State machine</h3>
            {run ? (
              <>
                <div className="gf-muted" style={{ fontSize: 11 }}>
                  {run.mode} · {run.status} · {run.pending_approvals} approval(s) pending
                </div>
                <div className="bld-rail">
                  {BUILDER_STATES.map((s, i) => (
                    <div
                      key={s}
                      className={`bld-rail-state ${i < stateIdx ? 'done' : ''} ${i === stateIdx ? 'current' : ''}`}
                    >
                      <span className="dot" />
                      {s}
                    </div>
                  ))}
                </div>
              </>
            ) : (
              <div className="gf-muted">No active run.</div>
            )}
          </div>

          <div className="bld-panel" style={{ 
marginTop: 12 }}>
            <h3>Agents ({agents.length})</h3>
            {agents.map((a) => (
              <div key={a.id} style={{ marginBottom: 8 }}>
                <div style={{ fontSize: 12, fontWeight: 600 }}>{a.name}</div>
                <div className="gf-muted" style={{ fontSize: 11 }}>{a.role}</div>
              </div>
            ))}
          </div>
        </div>

        <div>
          <div className="bld-panel">
            <div className="gf-row" style={{ gap: 8, marginBottom: 8 }}>
              {(['tasks', 'requirements', 'history'] as const).map((t) => (
                <button key={t} className={`gf-btn ${tab === t ? 'primary' : ''}`} onClick={() => setTab(t)}>
                  {t === 'tasks' ? 'Task graph' : t === 'requirements' ? 'Requirements' : 'Run history'}
                </button>
              ))}
              <span className="gf-muted" style={{ fontSize: 11, marginLeft: 'auto' }}>
                {run ? `${run.id.slice(0, 8)} · ${run.state}` : 'no run selected'}
              </span>
            </div>
            {tab === 'tasks' &&
              (run ? (
                <TaskGraph runId={run.id} refreshKey={taskTick} />
              ) : (
                <div className="gf-muted">Start a build to see its task graph.</div>
              ))}
            {tab === 'requirements' &&
              (run ? (
                <RequirementsPanel runId={run.id} model={model} onChanged={() => { setTaskTick((t) => t + 1); void refreshRun(run.id); }} />
              ) : (
                <div className="gf-muted">Start a build to refine requirements.</div>
              ))}
            {tab === 'history' && (
              <RunHistory onSelect={selectHistoryRun} refreshKey={historyTick} selectedId={run?.id} />
            )}
          </div>

          <div className="bld-panel" style={{ marginTop: 12 }}>
            <h3>Orchestration log</h3>
            <div className="bld-log">
              {log.map((l, i) => (
                <div ke
y={i} className={l.cls}>
                  <span className="t">[{l.at}]</span> {l.text}
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
