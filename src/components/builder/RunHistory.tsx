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

  const load = useCallback(() => {
    setError('');
    invoke<RunSummary[]>('run_history_list', { limit: 50 })
      .then(setRuns)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => {
    load();
  }, [load, refreshKey]);

  if (error) return <div className="bld-error">{error}</div>;
  if (runs.length === 0) return <div className="gf-muted">No builder runs yet.</div>;

  return (
    <div>
      {runs.map((r) => (
        <div
          key={r.id}
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
          <span className={`bld-status ${r.status === 'running' ? 'running' : r.status === 'completed' ? 'done' : 'pending'}`}>
            {r.status}
          </span>
        </div>
      ))}
    </div>
  );
}
