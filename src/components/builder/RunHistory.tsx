// SPDX-License-Identifier: Apache-2.0
// Phase 5 — Run history: recent builder runs, newest first. Selecting a run
// hands it to the parent so BuilderStudio can inspect or resume it.

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';
import type { RunSummary } from './types';

export default function RunHistory({
  onSelect,
  refreshKey,
  selectedId,
}: {
  onSelect: (run: RunSummary) => void;
  refreshKey: number;
  selectedId?: string;
}) {
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [error, setError] = useState('');
  const [expanded, setExpanded] = useState<string | null>(null);
  const [transitions, setTransitions] = useState<Record<string, Array<{ from_state: string; to_state: string; at: string; note: string }>>>({});

  const load = useCallback(() => {
    setError('');
    invoke<RunSummary[]>('run_history_list', { limit: 50 })
      .then(setRuns)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const toggleTransitions = (id: string) => {
    if (expanded === id) {
      setExpanded(null);
      return;
    }
    setExpanded(id);
    if (!transitions[id]) {
      void invoke<Array<{ from_state: string; to_state: string; at: string; note: string }>>('run_transitions', { run_id: id })
        .then((t) => setTransitions((prev) => ({ ...prev, [id]: t ?? [] })))
        .catch(() => setTransitions((prev) => ({ ...prev, [id]: [] })));
    }
  };

  useEffect(() => {
    load();
  }, [load, refreshKey]);

  if (error) return <div className="bld-error">{error}</div>;
  if (runs.length === 0) return <div className="gf-muted">No builder runs yet.</div>;

  return (
    <div>
      {runs.map((r) => (
        <div key={r.id}>
          <div
            className={`bld-history-item ${selectedId === r.id ? 'selected' : ''}`}
            onClick={() => onSelect(r)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === 'Enter') onSelect(r);
            }}
          >
            <div>
              <div style={{ fontSize: 13, fontWeight: 600 }}>
                {(r.goal || 'Untitled run').slice(0, 80)}
              </div>
              <div className="gf-muted" style={{ fontSize: 11 }}>
                {r.mode} · {r.state} · {r.status} · {r.started_at}
              </div>
            </div>
            <span className="gf-row" style={{ gap: 4 }}>
              <button
                className="gf-icon-btn"
                onClick={(e) => {
                  e.stopPropagation();
                  toggleTransitions(r.id);
                }}
                title="Show state transitions"
              >
                {expanded === r.id ? '▾' : '▸'}
              </button>
              <span className={`bld-status ${r.status === 'running' ? 'running' : r.status === 'completed' ? 'done' : 'pending'}`}>
                {r.status}
              </span>
            </span>
          </div>
          {expanded === r.id && (
            <div className="gf-muted" style={{ fontSize: 11, padding: '4px 8px 8px 12px' }}>
              {(transitions[r.id] ?? []).length === 0 ? (
                <span>No recorded transitions.</span>
              ) : (
                (transitions[r.id] ?? []).map((t, i) => (
                  <div key={i}>
                    {t.from_state} → {t.to_state} · {t.at}{t.note ? ` · ${t.note}` : ''}
                  </div>
                ))
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
