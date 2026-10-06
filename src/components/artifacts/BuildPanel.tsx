// SPDX-License-Identifier: Apache-2.0
// Phase 8 — Build panel: start/cancel builds, live log tail, attempts,
// failure reports, and the generation-workflow stepper.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke, isDesktop } from '../../lib/api';

interface BuildStatus {
  state: string;
  log_tail: string[];
  attempts: number;
  error_signature: string | null;
  failure_report_path: string | null;
}

interface BuildLogLine {
  build_id: string;
  line: string;
  stream: 'stdout' | 'stderr' | 'stage' | string;
}

// Generation workflow (master spec section 64). Stages 0-4 belong to the
// Builder (Phase 5); this panel drives stages 5-9 from real backend state.
const WORKFLOW_STAGES = [
  'Requirements',
  'Architecture',
  'Task Graph',
  'Files',
  'Implementation',
  'Build',
  'Tests',
  'Security Review',
  'Packaging',
  'Release Artifact',
] as const;

const EXTERNAL_STAGES = new Set([0, 1, 2, 3, 4, 7]); // Phase 5 / Phase 9 owned

function stageIndexFor(state: string, released: boolean, packaged: boolean): number {
  if (released) return 9;
  if (packaged) return 8;
  switch (state) {
    case 'building':
    case 'fixing':
      return 5;
    case 'validating':
      return 6;
    case 'completed':
      return 7; // security review pending (Phase 9)
    default:
      return -1;
  }
}

function fmtErr(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

export function BuildPanel({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [projectRoot, setProjectRoot] = useState('');
  const [target, setTarget] = useState('debug');
  const [buildId, setBuildId] = useState<string | null>(null);
  const [status, setStatus] = useState<BuildStatus | null>(null);
  const [liveLines, setLiveLines] = useState<BuildLogLine[]>([]);
  const [packaged, setPackaged] = useState(false);
  const [released, setReleased] = useState(false);
  const logRef = useRef<HTMLDivElement>(null);
  const pollRef = useRef<number | null>(null);

  const running =
    status !== null &&
    ['queued', 'building', 'fixing', 'validating'].includes(status.state);

  // Live log tail via build://log events.
  useEffect(() => {
    if (!buildId || !isDesktop()) return;
    let unlisten: (() => void) | null = null;
    void listen<BuildLogLine>('build://log', (ev) => {
      if (ev.payload.build_id !== buildId) return;
      setLiveLines((prev) => [...prev.slice(-400), ev.payload]);
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      if (unlisten) unlisten();
    };
  }, [buildId]);

  // Poll build_status for attempts / state / failure report.
  const refresh = useCallback(async () => {
    if (!buildId) return;
    try {
      const s = await invoke<BuildStatus>('build_status', { buildId });
      setStatus(s);
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
      if (pollRef.current) window.clearInterval(pollRef.current);
    }
  }, [buildId, onToast]);

  useEffect(() => {
    if (!buildId) return;
    void refresh();
    pollRef.current = window.setInterval(() => void refresh(), 2000);
    return () => {
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, [buildId, refresh]);

  // Auto-scroll the log.
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [liveLines]);

  const start = useCallback(async () => {
    if (!projectRoot.trim()) {
      onToast('Enter a project root directory first.');
      return;
    }
    try {
      const id = await invoke<string>('build_start', {
        projectRoot: projectRoot.trim(),
        target,
      });
      setBuildId(id);
      setLiveLines([]);
      setStatus(null);
      setPackaged(false);
      setReleased(false);
      onToast(`Build started: ${id.slice(0, 8)}`);
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
    }
  }, [projectRoot, target, onToast]);

  const cancel = useCallback(async () => {
    if (!buildId) return;
    try {
      await invoke('build_cancel', { buildId });
      onToast('Build cancel requested.');
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
    }
  }, [buildId, onToast]);

  const runTests = useCallback(async () => {
    if (!projectRoot.trim()) {
      onToast('Enter a project root directory first.');
      return;
    }
    try {
      const summary = await invoke<string>('test_run', {
        projectRoot: projectRoot.trim(),
      });
      setLiveLines((prev) => [
        ...prev,
        ...summary.split('\n').map((line) => ({
          build_id: buildId ?? 'tests',
          line,
          stream: 'stdout',
        })),
      ]);
      onToast('Test run finished — see log.');
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
    }
  }, [projectRoot, buildId, onToast]);

  const activeStage = stageIndexFor(status?.state ?? '', released, packaged);

  return (
    <div>
      <div className="gf-pane-header">
        <span>Build &amp; Fix Engine</span>
      </div>

      <span className="gf-label">Generation workflow</span>
      <div className="gf-stepper" aria-label="Generation workflow">
        {WORKFLOW_STAGES.map((name, i) => {
          const cls =
            i === activeStage
              ? 'gf-step active'
              : activeStage > i && !EXTERNAL_STAGES.has(i)
                ? 'gf-step done'
                : 'gf-step';
          const external = EXTERNAL_STAGES.has(i);
          return (
            <span
              key={name}
              className={`${cls}${external ? ' gf-step external' : ''}`}
              title={
                external
                  ? `${name} — owned by ${i < 5 ? 'Builder (Phase 5)' : 'Security (Phase 9)'}`
                  : name
              }
            >
              {name}
            </span>
          );
        })}
      </div>

      <div className="gf-build-controls">
        <label className="gf-field">
          <span className="gf-label">Project root</span>
          <input
            className="gf-input"
            value={projectRoot}
            onChange={(e) => setProjectRoot(e.target.value)}
            placeholder="/path/to/project"
          />
        </label>
        <label className="gf-field" style={{ minWidth: 120 }}>
          <span className="gf-label">Target</span>
          <select
            className="gf-input"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
          >
            <option value="debug">debug</option>
            <option value="release">release</option>
          </select>
        </label>
        <button className="gf-btn primary" onClick={start} disabled={running}>
          Start build
        </button>
        <button className="gf-btn" onClick={cancel} disabled={!running}>
          Cancel
        </button>
        <button className="gf-btn" onClick={runTests}>
          Run tests
        </button>
      </div>

      {status && (
        <div className="gf-status-line">
          <span>
            State: <strong>{status.state}</strong>
          </span>
          <span>Attempts: {status.attempts}</span>
          {status.error_signature && (
            <span className="gf-hash">sig: {status.error_signature}</span>
          )}
        </div>
      )}

      <div className="gf-build-log" ref={logRef} aria-live="polite">
        {liveLines.length === 0 && (
          <span className="gf-muted">No log output yet — start a build.</span>
        )}
        {liveLines.map((l, i) => (
          <div key={i} className={l.stream === 'stage' ? 'stage' : l.stream === 'stderr' ? 'stderr' : ''}>
            {l.line}
          </div>
        ))}
      </div>

      {status?.state === 'halted' && (
        <div className="gf-failure-report">
          <div className="gf-pane-header">
            <span>Failure report</span>
          </div>
          <p>
            The build-fix loop halted after 3 identical error signatures (or the
            attempt budget ran out). Rule-based patching is heuristic — enable a
            model provider for LLM-generated patches.
          </p>
          {status.error_signature && (
            <p className="gf-hash">signature: {status.error_signature}</p>
          )}
          {status.failure_report_path && (
            <p className="gf-hash">report: {status.failure_report_path}</p>
          )}
        </div>
      )}
    </div>
  );
}
