// SPDX-License-Identifier: Apache-2.0
// Phase 5 — DAG task graph view: tasks as a dependency-ordered list with
// status colors, dependency badges, and expandable detail.

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';
import type { TaskRow } from './types';

function parseDeps(raw: string): string[] {
  try {
    const v: unknown = JSON.parse(raw || '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

export default function TaskGraph({ runId, refreshKey }: { runId: string; refreshKey: number }) {
  const [tasks, setTasks] = useState<TaskRow[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [error, setError] = useState('');

  const load = useCallback(() => {
    setError('');
    invoke<TaskRow[]>('agent_task_list', { run_id: runId })
      .then(setTasks)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [runId]);

  useEffect(() => {
    load();
  }, [load, refreshKey]);

  if (error) return <div className="bld-error">{error}</div>;
  if (tasks.length === 0) return <div className="gf-muted">No tasks yet — the Supervisor creates the plan at the PLANNING state.</div>;

  const titles = new Map(tasks.map((t) => [t.id, t.title]));

  return (
    <div>
      {tasks.map((t) => {
        const deps = parseDeps(t.dependencies);
        const open = expanded === t.id;
        return (
          <div key={t.id} className="bld-task">
            <div className="bld-task-head">
              <span className={`bld-status ${t.status}`}>{t.status}</span>
              <span className="bld-task-title">{t.title}</span>
              <span className="gf-muted" style={{ fontSize: 11 }}>{t.agent} · {t.priority}</span>
              <button className="gf-icon-btn" onClick={() => setExpanded(open ? null : t.id)} aria-label="details">
                {open ? '▾' : '▸'}
              </button>
            </div>
            {deps.length > 0 && (
              <div className="bld-task-deps">
                depends on: {deps.map((d) => titles.get(d) ?? d.slice(0, 8)).join(', ')}
              </div>
            )}
            {open && (
              <div className="bld-task-meta">
                <div>id: {t.id}</div>
                {t.parent_id && <div>parent: {t.parent_id}</div>}
                {t.errors && <div style={{ color: 'var(--gf-danger)', whiteSpace: 'pre-wrap' }}>errors: {t.errors}</div>}
                {t.outputs && t.outputs !== '{}' && <div>outputs: {t.outputs.slice(0, 500)}</div>}
                {t.tool_calls && t.tool_calls !== '[]' && <div>tool calls: {t.tool_calls.slice(0, 500)}</div>}
                {t.artifacts && t.artifacts !== '[]' && <div>artifacts: {t.artifacts.slice(0, 500)}</div>}
                <div>created: {t.created_at}{t.ended_at ? ` · ended: ${t.ended_at}` : ''}</div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
