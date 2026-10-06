// SPDX-License-Identifier: Apache-2.0
// Phase 9 — Security components: audit log viewer + constitution display.
// Phase-local types; never touches shared lib files or App.tsx.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';
import './security.css';

export interface AuditEntry {
  id: number;
  ts: string;
  level: 'DEBUG' | 'INFO' | 'WARN' | 'ERROR' | 'SECURITY' | string;
  category: string;
  message: string;
  meta: Record<string, unknown> | null;
}

const LEVELS = ['ALL', 'DEBUG', 'INFO', 'WARN', 'ERROR', 'SECURITY'] as const;

const REFRESH_MS = 5000;

function describeError(err: unknown): string {
  if (err instanceof DesktopCapabilityRequired) return 'Desktop Capability Required';
  return err instanceof Error ? err.message : 'Audit query failed';
}

export function AuditPanel(): React.ReactElement {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [level, setLevel] = useState<string>('ALL');
  const [category, setCategory] = useState('');
  const [securityOnly, setSecurityOnly] = useState(false);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const effectiveLevel = securityOnly ? 'SECURITY' : level;
      const rows = await invoke<AuditEntry[]>('sec_audit_query', {
        level_filter: effectiveLevel === 'ALL' ? null : effectiveLevel,
        category_filter: category.trim() === '' ? null : category.trim(),
        limit: 200,
      });
      setEntries(rows ?? []);
      setError(null);
    } catch (err) {
      setError(describeError(err));
    }
  }, [level, category, securityOnly]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!autoRefresh) return;
    const id = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(id);
  }, [autoRefresh, load]);

  return (
    <div className="gf-workspace">
      <div className="gf-pane sec-pane">
        <div className="gf-pane-header">
          <span>Audit Log</span>
          <span className="gf-muted sec-count">
            {entries.length} entr{entries.length === 1 ? 'y' : 'ies'}
          </span>
        </div>

        <div className="sec-filters">
          <label className="gf-label" htmlFor="sec-level">
            Level
          </label>
          <select
            id="sec-level"
            className="gf-select"
            value={securityOnly ? 'SECURITY' : level}
            disabled={securityOnly}
            onChange={(e) => setLevel(e.target.value)}
          >
            {LEVELS.map((l) => (
              <option key={l} value={l}>
                {l}
              </option>
            ))}
          </select>

          <label className="gf-label" htmlFor="sec-category">
            Category
          </label>
          <input
            id="sec-category"
            className="gf-input"
            type="text"
            placeholder="filter category…"
            value={category}
            onChange={(e) => setCategory(e.target.value)}
          />

          <label className="sec-check">
            <input
              type="checkbox"
              checked={securityOnly}
              onChange={(e) => setSecurityOnly(e.target.checked)}
            />
            SECURITY only
          </label>

          <label className="sec-check">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
            />
            Auto-refresh
          </label>

          <button type="button" className="gf-btn" onClick={() => void load()}>
            Refresh
          </button>
        </div>

        {error !== null && <p className="sec-error">{error}</p>}

        <div className="sec-table-wrap">
          <table className="sec-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Level</th>
                <th>Category</th>
                <th>Message</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <tr key={e.id}>
                  <td className="sec-ts">{e.ts}</td>
                  <td>
                    <span className={`sec-badge level-${e.level.toLowerCase()}`}>
                      {e.level}
                    </span>
                  </td>
                  <td className="sec-cat">{e.category}</td>
                  <td className="sec-msg" title={e.message}>
                    {e.message}
                  </td>
                </tr>
              ))}
              {entries.length === 0 && error === null && (
                <tr>
                  <td colSpan={4} className="gf-muted sec-empty">
                    No audit entries yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        <p className="gf-muted sec-note">
          Secrets are scrubbed before any entry is written. Newest entries
          first; the log keeps the most recent 100,000 rows.
        </p>
      </div>
    </div>
  );
}
