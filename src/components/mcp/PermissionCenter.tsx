// SPDX-License-Identifier: Apache-2.0
// Phase 7 — Permission Center: 8 domains x 5 levels matrix + tool registry.
// Reads/writes the Rust permission commands (perm_list / perm_set) and the
// MCP tool registry (mcp_tool_registry_list / mcp_tool_set_enabled).

import React, { useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import { PERM_DOMAINS, PERM_LEVELS } from './types';
import type { PermEntry, PermLevel, RegistryTool } from './types';
import './mcp.css';

interface Props {
  onToast: (msg: string) => void;
}

function errText(e: unknown): string {
  if (e instanceof DesktopCapabilityRequired) return 'Desktop Capability Required';
  return e instanceof Error ? e.message : String(e);
}

function riskClass(r: string): string {
  return r === 'high' ? 'mcp-risk-high' : r === 'low' ? 'mcp-risk-low' : 'mcp-risk-mid';
}

export function PermissionCenter({ onToast }: Props): React.ReactElement {
  const [levels, setLevels] = useState<Record<string, PermLevel>>({});
  const [registry, setRegistry] = useState<RegistryTool[]>([]);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (!isDesktop()) return;
    (async () => {
      try {
        const entries = await invoke<PermEntry[]>('perm_list');
        const map: Record<string, PermLevel> = {};
        for (const e of entries) map[e.domain] = e.level;
        setLevels(map);
        const tools = await invoke<RegistryTool[]>('mcp_tool_registry_list');
        setRegistry(tools ?? []);
      } catch (e) {
        onToast(errText(e));
      } finally {
        setLoaded(true);
      }
    })();
  }, [onToast]);

  if (!isDesktop()) {
    return <div className="mcp-panel"><p className="mcp-muted">Desktop Capability Required</p></div>;
  }

  const setLevel = async (domain: string, level: PermLevel): Promise<void> => {
    const prev = levels[domain];
    setLevels((s) => ({ ...s, [domain]: level }));
    try {
      await invoke('perm_set', { domain, level });
    } catch (e) {
      setLevels((s) => ({ ...s, [domain]: prev }));
      onToast(errText(e));
    }
  };

  const toggleTool = async (name: string, enabled: boolean): Promise<void> => {
    setRegistry((r) => r.map((t) => (t.name === name ? { ...t, enabled } : t)));
    try {
      await invoke('mcp_tool_set_enabled', { tool_name: name, enabled });
    } catch (e) {
      setRegistry((r) => r.map((t) => (t.name === name ? { ...t, enabled: !enabled } : t)));
      onToast(errText(e));
    }
  };

  return (
    <div className="mcp-panel">
      <h2 className="mcp-title">Permission Center</h2>
      <p className="mcp-muted">
        Default policy for autonomous actions, per domain. Unset domains fall back to
        &ldquo;Ask every time&rdquo;.
      </p>

      {!loaded && <p className="mcp-muted">Loading&hellip;</p>}

      <table className="mcp-matrix">
        <thead>
          <tr>
            <th>Domain</th>
            {PERM_LEVELS.map((l) => (
              <th key={l.id} title={l.blurb}>{l.label}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {PERM_DOMAINS.map((d) => (
            <tr key={d.id}>
              <td>
                <div className="mcp-domain-label">{d.label}</div>
                <div className="mcp-muted mcp-small">{d.blurb}</div>
              </td>
              {PERM_LEVELS.map((l) => (
                <td key={l.id} className="mcp-radio-cell">
                  <input
                    type="radio"
                    name={`perm-${d.id}`}
                    checked={(levels[d.id] ?? 'ask_every_time') === l.id}
                    onChange={() => void setLevel(d.id, l.id)}
                    aria-label={`${d.label}: ${l.label}`}
                  />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>

      <h3 className="mcp-subtitle">Tool registry</h3>
      <p className="mcp-muted">
        Every tool discovered on MCP servers, with risk classification and audit policy.
        Disabling a tool removes it from agent use without deleting the server.
      </p>
      {registry.length === 0 ? (
        <p className="mcp-muted">No MCP tools registered yet. Add a server and browse its tools.</p>
      ) : (
        <table className="mcp-table">
          <thead>
            <tr>
              <th>Tool</th>
              <th>Server</th>
              <th>Risk</th>
              <th>Requires</th>
              <th>Audit</th>
              <th>Enabled</th>
            </tr>
          </thead>
          <tbody>
            {registry.map((t) => (
              <tr key={t.name}>
                <td>
                  <div className="mcp-mono">{t.name}</div>
                  {t.description && <div className="mcp-muted mcp-small">{t.description}</div>}
                </td>
                <td className="mcp-mono mcp-small">{t.mcp_server ?? '—'}</td>
                <td><span className={`mcp-badge ${riskClass(t.risk_class)}`}>{t.risk_class}</span></td>
                <td className="mcp-small">{t.permission_requirements.join(', ') || '—'}</td>
                <td className="mcp-small">{t.audit_policy}</td>
                <td>
                  <input
                    type="checkbox"
                    checked={t.enabled}
                    onChange={(e) => void toggleTool(t.name, e.target.checked)}
                    aria-label={`Enable ${t.name}`}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
