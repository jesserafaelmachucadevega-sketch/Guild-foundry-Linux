// SPDX-License-Identifier: Apache-2.0
// Phase 6 — Tool Panel: registry table with risk badges, enable/disable
// toggles (persisted via settings), capability manifest view, and a safe
// "test run" for low-risk tools.

import React, { useCallback, useEffect, useState } from 'react';
import './tools.css';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import type { CapabilityManifest, ToolExecResult } from '../../lib/toolLoop';

interface ToolDefRow {
  name: string;
  description: string;
  schema: unknown;
  risk: 'LOW' | 'MODERATE' | 'HIGH' | 'CRITICAL';
  domain: string;
  ready: boolean;
}

interface Props {
  onToast?: (t: string) => void;
}

function settingKey(name: string): string {
  return `tools.enabled.${name}`;
}

/** Sample args used by the "test run" button (low-risk tools only). */
function sampleArgs(tool: string): Record<string, unknown> | null {
  switch (tool) {
    case 'user.ask':
      return {
        question: 'Panel self-test: the tool loop is reachable. No answer is needed.',
        options: ['Acknowledged'],
      };
    default:
      return null;
  }
}

const RISK_CLASS: Record<ToolDefRow['risk'], string> = {
  LOW: 'gf-tools-risk-low',
  MODERATE: 'gf-tools-risk-moderate',
  HIGH: 'gf-tools-risk-high',
  CRITICAL: 'gf-tools-risk-critical',
};

export function ToolPanel({ onToast }: Props): React.ReactElement {
  const [tools, setTools] = useState<ToolDefRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [enabled, setEnabled] = useState<Record<string, boolean>>({});
  const [manifest, setManifest] = useState<CapabilityManifest | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{ tool: string; result: ToolExecResult } | null>(null);

  const toast = useCallback(
    (t: string) => {
      if (onToast) onToast(t);
    },
    [onToast],
  );

  const load = useCallback(async () => {
    if (!isDesktop()) {
      setError('Desktop Capability Required');
      return;
    }
    try {
      const rows = await invoke<ToolDefRow[]>('tool_list');
      setTools(rows);
      const states: Record<string, boolean> = {};
      for (const r of rows) {
        try {
          const v = await invoke<unknown>('settings_get', { key: settingKey(r.name) });
          states[r.name] = v === null || v === undefined ? true : v === true;
        } catch {
          states[r.name] = true;
        }
      }
      setEnabled(states);
      setError(null);
    } catch (err) {
      setError(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const toggleEnabled = useCallback(
    async (name: string) => {
      const next = !(enabled[name] ?? true);
      try {
        await invoke('settings_set', { key: settingKey(name), value: next });
        setEnabled((s) => ({ ...s, [name]: next }));
        toast(`Tool ${name} ${next ? 'enabled' : 'disabled'}`);
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err));
      }
    },
    [enabled, toast],
  );

  const refreshManifest = useCallback(async () => {
    try {
      const hs = await invoke<{ manifest: CapabilityManifest }>('tool_handshake', {
        session_id: 'ui',
      });
      setManifest(hs.manifest);
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err));
    }
  }, [toast]);

  const testRun = useCallback(
    async (name: string) => {
      const args = sampleArgs(name);
      if (!args) return;
      setTesting(name);
      setTestResult(null);
      try {
        const result = await invoke<ToolExecResult>('tool_execute', {
          tool: name,
          args_json: JSON.stringify(args),
          agent_id: 'ui',
          session_id: `ui-test-${Date.now()}`,
        });
        setTestResult({ tool: name, result });
      } catch (err) {
        setTestResult({
          tool: name,
          result: {
            ok: false,
            output: null,
            error: err instanceof Error ? err.message : String(err),
            duration_ms: 0,
            needs_approval: false,
            needs_user: false,
            tool: name,
            code: 'exec_error',
            executed_at: new Date().toISOString(),
          },
        });
      } finally {
        setTesting(null);
      }
    },
    [],
  );

  return (
    <div className="gf-workspace">
      <div className="gf-pane" style={{ flex: 1, padding: 16, overflowY: 'auto' }}>
        <div className="gf-pane-header">
          <span>Tools</span>
          <span className="gf-row" style={{ gap: 8 }}>
            <button type="button" className="gf-btn" onClick={() => void refreshManifest()}>
              Refresh capability manifest
            </button>
            <button type="button" className="gf-btn" onClick={() => void load()}>
              Reload registry
            </button>
          </span>
        </div>

        {error && (
          <div className="gf-tools-error" role="alert">
            {error}
          </div>
        )}

        {manifest && (
          <div className="gf-tools-manifest">
            <span className="gf-label">Capability manifest (session: ui)</span>
            {(
              [
                ['Available', manifest.available],
                ['Requires approval', manifest.requires_approval],
                ['Unavailable', manifest.unavailable],
                ['Restricted', manifest.restricted],
              ] as Array<[string, string[]]>
            ).map(([label, names]) => (
              <div key={label} className="gf-tools-manifest-row">
                <span className="gf-muted">{label}:</span>{' '}
                <span>{names.length > 0 ? names.join(', ') : '(none)'}</span>
              </div>
            ))}
          </div>
        )}

        {!tools && !error && <p className="gf-muted">Loading tool registry…</p>}

        {tools && (
          <table className="gf-tools-table">
            <thead>
              <tr>
                <th>Tool</th>
                <th>Description</th>
                <th>Risk</th>
                <th>Domain</th>
                <th>Status</th>
                <th>Enabled</th>
                <th>Test</th>
              </tr>
            </thead>
            <tbody>
              {tools.map((t) => {
                const isOn = enabled[t.name] ?? true;
                const canTest = t.risk === 'LOW' && t.ready && sampleArgs(t.name) !== null;
                return (
                  <tr key={t.name} className={isOn ? '' : 'gf-tools-row-off'}>
                    <td className="gf-tools-name">{t.name}</td>
                    <td className="gf-muted">{t.description}</td>
                    <td>
                      <span className={`gf-badge ${RISK_CLASS[t.risk]}`}>{t.risk}</span>
                    </td>
                    <td className="gf-muted">{t.domain}</td>
                    <td>
                      {t.ready ? (
                        <span className="gf-badge ok">ready</span>
                      ) : (
                        <span className="gf-badge idle" title="Backing module pending (Phase 4/8)">
                          pending
                        </span>
                      )}
                    </td>
                    <td>
                      <button
                        type="button"
                        role="switch"
                        aria-checked={isOn}
                        aria-label={`Enable ${t.name}`}
                        className={`gf-tools-switch${isOn ? ' on' : ''}`}
                        onClick={() => void toggleEnabled(t.name)}
                      >
                        <span className="gf-tools-knob" />
                      </button>
                    </td>
                    <td>
                      <button
                        type="button"
                        className="gf-btn"
                        disabled={!canTest || testing === t.name}
                        title={
                          canTest
                            ? 'Run a safe self-test of this tool'
                            : 'Test runs are only available for ready, low-risk tools'
                        }
                        onClick={() => void testRun(t.name)}
                      >
                        {testing === t.name ? 'Running…' : 'Test run'}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}

        {testResult && (
          <div className="gf-tools-result">
            <div className="gf-pane-header">
              <span>
                Test run: {testResult.tool} —{' '}
                {testResult.result.ok ? (
                  <span className="gf-gold-text">ok</span>
                ) : (
                  <span className="gf-muted">
                    gate/error ({testResult.result.code}
                    {testResult.result.needs_approval ? ', approval required' : ''}
                    {testResult.result.needs_user ? ', user input required' : ''})
                  </span>
                )}
              </span>
              <button type="button" className="gf-icon-btn" onClick={() => setTestResult(null)}>
                ✕
              </button>
            </div>
            <pre>{JSON.stringify(testResult.result, null, 2)}</pre>
          </div>
        )}

        <p className="gf-muted" style={{ marginTop: 12, fontSize: 12 }}>
          Test runs execute for real through the tool executor — there is no dry-run
          mode. Only low-risk tools offer a test run, and every execution is
          permission-gated and logged.
        </p>
      </div>
    </div>
  );
}
