// SPDX-License-Identifier: Apache-2.0
// Phase 7 — MCP server management panel: server list, add/edit form,
// test, OAuth authenticate/revoke, per-server tool browser with manual
// tool calls, and per-server permission boundaries. Tabs into PermissionCenter.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import { PermissionCenter } from './PermissionCenter';
import type {
  McpAuthType,
  McpCallResult,
  McpServerInput,
  McpServerView,
  McpTestResult,
  McpToolInfo,
  McpTransport,
} from './types';
import './mcp.css';

interface Props {
  onToast: (msg: string) => void;
}

function errText(e: unknown): string {
  if (e instanceof DesktopCapabilityRequired) return 'Desktop Capability Required';
  return e instanceof Error ? e.message : String(e);
}

function csvToList(s: string): string[] {
  return s.split(',').map((x) => x.trim()).filter((x) => x.length > 0);
}

const EMPTY_FORM: McpServerInput & { bearer_token: string } = {
  name: '',
  transport: 'streamable_http',
  url: '',
  auth_type: 'none',
  oauth_client_id: '',
  oauth_authorization_url: '',
  oauth_token_url: '',
  oauth_scopes: '',
  oauth_redirect_uri: '',
  allowed_tools: [],
  denied_tools: [],
  bearer_present: false,
  bearer_token: '',
};

function viewToForm(v: McpServerView): McpServerInput & { bearer_token: string } {
  return {
    ...EMPTY_FORM,
    name: v.name,
    transport: (v.transport === 'sse' ? 'sse' : 'streamable_http') as McpTransport,
    url: v.url,
    auth_type: (['none', 'bearer', 'oauth'].includes(v.auth_type) ? v.auth_type : 'none') as McpAuthType,
    allowed_tools: v.allowed_tools,
    denied_tools: v.denied_tools,
    bearer_present: v.bearer_present,
  };
}

export function McpPanel({ onToast }: Props): React.ReactElement {
  const [tab, setTab] = useState<'servers' | 'permissions'>('servers');
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [loading, setLoading] = useState(true);
  const [formOpen, setFormOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [form, setForm] = useState(EMPTY_FORM);
  const [testResults, setTestResults] = useState<Record<string, McpTestResult>>({});
  const [testing, setTesting] = useState<Record<string, boolean>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [tools, setTools] = useState<Record<string, McpToolInfo[]>>({});
  const [callArgs, setCallArgs] = useState<Record<string, string>>({});
  const [calling, setCalling] = useState<Record<string, boolean>>({});
  const [callResults, setCallResults] = useState<Record<string, McpCallResult>>({});
  const [oauth, setOauth] = useState<{ serverId: string; authUrl: string; state: string; code: string } | null>(null);

  const refresh = useCallback(async (): Promise<void> => {
    if (!isDesktop()) {
      setLoading(false);
      return;
    }
    try {
      const list = await invoke<McpServerView[]>('mcp_server_list');
      setServers(list ?? []);
    } catch (e) {
      onToast(errText(e));
    } finally {
      setLoading(false);
    }
  }, [onToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!isDesktop()) {
    return (
      <div className="mcp-panel">
        <p className="mcp-muted">Desktop Capability Required</p>
      </div>
    );
  }

  const set = <K extends keyof typeof EMPTY_FORM>(k: K, v: (typeof EMPTY_FORM)[K]): void =>
    setForm((f) => ({ ...f, [k]: v }));

  const openAdd = (): void => {
    setEditingId(null);
    setForm(EMPTY_FORM);
    setFormOpen(true);
  };

  const openEdit = (v: McpServerView): void => {
    setEditingId(v.id);
    setForm(viewToForm(v));
    setFormOpen(true);
  };

  const saveServer = async (): Promise<void> => {
    const input: McpServerInput = {
      name: form.name.trim(),
      transport: form.transport,
      url: form.url.trim(),
      auth_type: form.auth_type,
      oauth_client_id: form.oauth_client_id || '',
      oauth_authorization_url: form.oauth_authorization_url || '',
      oauth_token_url: form.oauth_token_url || '',
      oauth_scopes: form.oauth_scopes || '',
      oauth_redirect_uri: form.oauth_redirect_uri || '',
      allowed_tools: form.allowed_tools ?? [],
      denied_tools: form.denied_tools ?? [],
      bearer_present: (form.bearer_token?.length ?? 0) > 0 || form.bearer_present === true,
    };
    try {
      let id = editingId;
      if (id) {
        await invoke('mcp_server_update', { server_id: id, input });
      } else {
        id = await invoke<string>('mcp_server_add', { input });
      }
      // Bearer <redacted> go to the keyring only — never the database. Phase 2's
      // secret_set is the primary writer; the Rust side reads the same keyring entry.
      if (form.bearer_token && form.bearer_token.length > 0 && id) {
        try {
          await invoke('secret_set', { key: `gfa-mcp-bearer:${id}`, value: form.bearer_token });
        } catch (e) {
          onToast(`Server saved, but the token could not be stored: ${errText(e)}`);
        }
      }
      setFormOpen(false);
      onToast(editingId ? 'Server updated' : 'Server added');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const removeServer = async (id: string, name: string): Promise<void> => {
    if (!window.confirm(`Remove MCP server "${name}"? Its stored credentials are deleted too.`)) return;
    try {
      await invoke('mcp_server_remove', { server_id: id });
      onToast('Server removed');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const toggleEnabled = async (v: McpServerView): Promise<void> => {
    try {
      await invoke(v.enabled ? 'mcp_server_disable' : 'mcp_server_enable', { server_id: v.id });
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const testServer = async (id: string): Promise<void> => {
    setTesting((t) => ({ ...t, [id]: true }));
    try {
      const res = await invoke<McpTestResult>('mcp_server_test', { server_id: id });
      setTestResults((r) => ({ ...r, [id]: res }));
    } catch (e) {
      onToast(errText(e));
    } finally {
      setTesting((t) => ({ ...t, [id]: false }));
    }
  };

  const toggleTools = async (v: McpServerView): Promise<void> => {
    const open = !expanded[v.id];
    setExpanded((e) => ({ ...e, [v.id]: open }));
    if (open && !tools[v.id]) {
      try {
        const list = await invoke<McpToolInfo[]>('mcp_list_tools', { server_id: v.id });
        setTools((t) => ({ ...t, [v.id]: list ?? [] }));
      } catch (e) {
        onToast(errText(e));
      }
    }
  };

  const callTool = async (serverId: string, toolName: string): Promise<void> => {
    const key = `${serverId}:${toolName}`;
    const argsJson = callArgs[key] ?? '{}';
    setCalling((c) => ({ ...c, [key]: true }));
    try {
      const res = await invoke<McpCallResult>('mcp_call_tool', {
        server_id: serverId,
        tool_name: toolName,
        args_json: argsJson,
      });
      setCallResults((r) => ({ ...r, [key]: res }));
      if (res.approval_required) onToast('Tool call needs approval (permission policy: ask every time)');
    } catch (e) {
      onToast(errText(e));
    } finally {
      setCalling((c) => ({ ...c, [key]: false }));
    }
  };

  const startOAuth = async (v: McpServerView): Promise<void> => {
    try {
      const res = await invoke<{ auth_url: string; state: string }>('mcp_oauth_start', {
        server_id: v.id,
      });
      setOauth({ serverId: v.id, authUrl: res.auth_url, state: res.state, code: '' });
    } catch (e) {
      onToast(errText(e));
    }
  };

  const completeOAuth = async (): Promise<void> => {
    if (!oauth || !oauth.code.trim()) {
      onToast('Paste the authorization code first');
      return;
    }
    try {
      await invoke('mcp_oauth_callback', { state: oauth.state, code: oauth.code.trim() });
      setOauth(null);
      onToast('Authenticated');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const revokeOAuth = async (v: McpServerView): Promise<void> => {
    if (!window.confirm(`Revoke OAuth access for "${v.name}"?`)) return;
    try {
      const res = await invoke<{ ok: boolean; detail: string }>('mcp_oauth_revoke', {
        server_id: v.id,
      });
      onToast(res.detail || 'Access revoked');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const authBadge = (v: McpServerView): React.ReactElement => {
    if (v.auth_type === 'oauth') {
      return v.oauth_connected ? (
        <span className="mcp-badge mcp-ok">OAuth connected</span>
      ) : (
        <span className="mcp-badge">OAuth not connected</span>
      );
    }
    if (v.auth_type === 'bearer') {
      return v.bearer_present ? (
        <span className="mcp-badge mcp-ok">Token stored</span>
      ) : (
        <span className="mcp-badge mcp-warn">No token</span>
      );
    }
    return <span className="mcp-badge">No auth</span>;
  };

  return (
    <div className="mcp-panel">
      <div className="mcp-tabs">
        <button
          type="button"
          className={tab === 'servers' ? 'mcp-tab active' : 'mcp-tab'}
          onClick={() => setTab('servers')}
        >
          Servers
        </button>
        <button
          type="button"
          className={tab === 'permissions' ? 'mcp-tab active' : 'mcp-tab'}
          onClick={() => setTab('permissions')}
        >
          Permission Center
        </button>
      </div>

      {tab === 'permissions' ? (
        <PermissionCenter onToast={onToast} />
      ) : (
        <>
          <div className="mcp-header-row">
            <h2 className="mcp-title">MCP Servers</h2>
            <button type="button" className="gf-btn primary" onClick={openAdd}>
              Add server
            </button>
          </div>
          <p className="mcp-muted">
            Local and remote servers over Streamable HTTP or legacy HTTP+SSE. Tools are
            normalized into the registry; each server enforces its own permission boundary.
          </p>

          {loading && <p className="mcp-muted">Loading&hellip;</p>}
          {!loading && servers.length === 0 && (
            <p className="mcp-muted">No MCP servers configured yet.</p>
          )}

          {servers.map((v) => {
            const test = testResults[v.id];
            return (
              <div key={v.id} className="mcp-server-card">
                <div className="mcp-server-head">
                  <div>
                    <span className="mcp-server-name">{v.name}</span>{' '}
                    <span className={`mcp-badge ${v.enabled ? 'mcp-ok' : ''}`}>
                      {v.enabled ? 'enabled' : 'disabled'}
                    </span>{' '}
                    <span className="mcp-badge">{v.transport === 'sse' ? 'HTTP+SSE' : 'Streamable HTTP'}</span>{' '}
                    {authBadge(v)}
                  </div>
                  <div className="mcp-muted mcp-small mcp-mono">{v.url}</div>
                </div>

                {test && (
                  <div className={`mcp-test ${test.ok ? 'mcp-ok-text' : 'mcp-err-text'}`}>
                    {test.ok
                      ? `Connected in ${test.latency_ms} ms — ${test.tool_count} tools${test.protocol_version ? ` (protocol ${test.protocol_version})` : ''}`
                      : `Failed after ${test.latency_ms} ms: ${test.error ?? 'unknown error'}`}
                  </div>
                )}

                <div className="mcp-btn-row">
                  <button type="button" className="gf-btn" onClick={() => void toggleEnabled(v)}>
                    {v.enabled ? 'Disable' : 'Enable'}
                  </button>
                  <button
                    type="button"
                    className="gf-btn"
                    disabled={testing[v.id] === true}
                    onClick={() => void testServer(v.id)}
                  >
                    {testing[v.id] ? 'Testing…' : 'Test'}
                  </button>
                  <button type="button" className="gf-btn" onClick={() => void toggleTools(v)}>
                    {expanded[v.id] ? 'Hide tools' : `Tools${v.tool_count > 0 ? ` (${v.tool_count})` : ''}`}
                  </button>
                  {v.auth_type === 'oauth' &&
                    (v.oauth_connected ? (
                      <button type="button" className="gf-btn" onClick={() => void revokeOAuth(v)}>
                        Revoke access
                      </button>
                    ) : (
                      <button type="button" className="gf-btn" onClick={() => void startOAuth(v)}>
                        Authenticate
                      </button>
                    ))}
                  <button type="button" className="gf-btn" onClick={() => openEdit(v)}>
                    Edit
                  </button>
                  <button type="button" className="gf-btn danger" onClick={() => void removeServer(v.id, v.name)}>
                    Remove
                  </button>
                </div>

                {expanded[v.id] && (
                  <div className="mcp-tools">
                    {(tools[v.id] ?? []).length === 0 && (
                      <p className="mcp-muted mcp-small">No tools discovered.</p>
                    )}
                    {(tools[v.id] ?? []).map((t) => {
                      const key = `${v.id}:${t.name}`;
                      const res = callResults[key];
                      return (
                        <div key={t.name} className="mcp-tool">
                          <div className="mcp-tool-head">
                            <span className="mcp-mono">{t.name}</span>{' '}
                            <span className={`mcp-badge mcp-risk-${t.risk_class === 'high' ? 'high' : t.risk_class === 'low' ? 'low' : 'mid'}`}>
                              {t.risk_class}
                            </span>{' '}
                            {!t.enabled && <span className="mcp-badge">disabled</span>}
                          </div>
                          {t.description && <div className="mcp-muted mcp-small">{t.description}</div>}
                          <details className="mcp-small">
                            <summary>Input schema</summary>
                            <pre className="mcp-pre">{JSON.stringify(t.input_schema, null, 2)}</pre>
                          </details>
                          <div className="mcp-call-row">
                            <input
                              className="gf-input mcp-mono"
                              placeholder='Arguments JSON, e.g. {"path": "/tmp"}'
                              value={callArgs[key] ?? '{}'}
                              onChange={(e) => setCallArgs((c) => ({ ...c, [key]: e.target.value }))}
                            />
                            <button
                              type="button"
                              className="gf-btn"
                              disabled={calling[key] === true || !t.enabled}
                              onClick={() => void callTool(v.id, t.name)}
                            >
                              {calling[key] ? 'Calling…' : 'Call'}
                            </button>
                          </div>
                          {res && (
                            <div className="mcp-result">
                              <div className="mcp-muted mcp-small">
                                {res.approval_required
                                  ? 'Approval required by permission policy — not executed.'
                                  : `${res.ok ? 'OK' : 'Error'} in ${res.latency_ms} ms · audit ${res.audit.decision} (${res.audit.policy})`}
                              </div>
                              <pre className="mcp-pre">{JSON.stringify(res.result, null, 2)}</pre>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}

          {formOpen && (
            <div className="mcp-modal-backdrop" onClick={() => setFormOpen(false)}>
              <div className="mcp-modal" onClick={(e) => e.stopPropagation()}>
                <h3 className="mcp-subtitle">{editingId ? 'Edit server' : 'Add server'}</h3>
                <label className="mcp-field">
                  <span>Name</span>
                  <input className="gf-input" value={form.name} onChange={(e) => set('name', e.target.value)} />
                </label>
                <label className="mcp-field">
                  <span>Transport</span>
                  <select
                    className="gf-select"
                    value={form.transport}
                    onChange={(e) => set('transport', e.target.value as McpTransport)}
                  >
                    <option value="streamable_http">Streamable HTTP</option>
                    <option value="sse">Legacy HTTP+SSE</option>
                  </select>
                </label>
                <label className="mcp-field">
                  <span>URL</span>
                  <input
                    className="gf-input mcp-mono"
                    placeholder="https://example.com/mcp"
                    value={form.url}
                    onChange={(e) => set('url', e.target.value)}
                  />
                </label>
                <label className="mcp-field">
                  <span>Authentication</span>
                  <select
                    className="gf-select"
                    value={form.auth_type}
                    onChange={(e) => set('auth_type', e.target.value as McpAuthType)}
                  >
                    <option value="none">None</option>
                    <option value="bearer">Bearer token</option>
                    <option value="oauth">OAuth 2.1 + PKCE</option>
                  </select>
                </label>
                {form.auth_type === 'bearer' && (
                  <label className="mcp-field">
                    <span>Bearer token{form.bearer_present ? ' (stored — enter a new one to replace)' : ''}</span>
                    <input
                      className="gf-input mcp-mono"
                      type="password"
                      autoComplete="off"
                      value={form.bearer_token}
                      onChange={(e) => set('bearer_token', e.target.value)}
                    />
                  </label>
                )}
                {form.auth_type === 'oauth' && (
                  <>
                    <label className="mcp-field">
                      <span>Client ID</span>
                      <input className="gf-input mcp-mono" value={form.oauth_client_id ?? ''} onChange={(e) => set('oauth_client_id', e.target.value)} />
                    </label>
                    <label className="mcp-field">
                      <span>Authorization URL</span>
                      <input className="gf-input mcp-mono" value={form.oauth_authorization_url ?? ''} onChange={(e) => set('oauth_authorization_url', e.target.value)} />
                    </label>
                    <label className="mcp-field">
                      <span>Token URL</span>
                      <input className="gf-input mcp-mono" value={form.oauth_token_url ?? ''} onChange={(e) => set('oauth_token_url', e.target.value)} />
                    </label>
                    <label className="mcp-field">
                      <span>Scopes (space separated, optional)</span>
                      <input className="gf-input mcp-mono" value={form.oauth_scopes ?? ''} onChange={(e) => set('oauth_scopes', e.target.value)} />
                    </label>
                    <label className="mcp-field">
                      <span>Redirect URI (optional)</span>
                      <input className="gf-input mcp-mono" placeholder="http://127.0.0.1:18793/oauth/callback" value={form.oauth_redirect_uri ?? ''} onChange={(e) => set('oauth_redirect_uri', e.target.value)} />
                    </label>
                  </>
                )}
                <label className="mcp-field">
                  <span>Allowed tools (comma separated; empty = all)</span>
                  <input
                    className="gf-input mcp-mono"
                    value={(form.allowed_tools ?? []).join(', ')}
                    onChange={(e) => set('allowed_tools', csvToList(e.target.value))}
                  />
                </label>
                <label className="mcp-field">
                  <span>Denied tools (comma separated)</span>
                  <input
                    className="gf-input mcp-mono"
                    value={(form.denied_tools ?? []).join(', ')}
                    onChange={(e) => set('denied_tools', csvToList(e.target.value))}
                  />
                </label>
                <div className="mcp-btn-row">
                  <button type="button" className="gf-btn primary" onClick={() => void saveServer()}>
                    Save
                  </button>
                  <button type="button" className="gf-btn" onClick={() => setFormOpen(false)}>
                    Cancel
                  </button>
                </div>
              </div>
            </div>
          )}

          {oauth && (
            <div className="mcp-modal-backdrop" onClick={() => setOauth(null)}>
              <div className="mcp-modal" onClick={(e) => e.stopPropagation()}>
                <h3 className="mcp-subtitle">Authenticate with OAuth</h3>
                <p className="mcp-muted mcp-small">
                  Open this URL in your browser, approve access, then paste the
                  authorization code below.
                </p>
                <textarea className="gf-textarea mcp-mono mcp-small" readOnly rows={4} value={oauth.authUrl} />
                <div className="mcp-btn-row">
                  <button
                    type="button"
                    className="gf-btn"
                    onClick={() => {
                      void navigator.clipboard?.writeText(oauth.authUrl).then(
                        () => onToast('Authorization URL copied'),
                        () => onToast('Copy failed'),
                      );
                    }}
                  >
                    Copy URL
                  </button>
                </div>
                <label className="mcp-field">
                  <span>Authorization code</span>
                  <input
                    className="gf-input mcp-mono"
                    value={oauth.code}
                    onChange={(e) => setOauth((o) => (o ? { ...o, code: e.target.value } : o))}
                  />
                </label>
                <div className="mcp-btn-row">
                  <button type="button" className="gf-btn primary" onClick={() => void completeOAuth()}>
                    Complete
                  </button>
                  <button type="button" className="gf-btn" onClick={() => setOauth(null)}>
                    Cancel
                  </button>
                </div>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
