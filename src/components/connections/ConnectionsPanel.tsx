// SPDX-License-Identifier: Apache-2.0
// Connections tab — the agent's friendly surface for account and app access.
// Builders keep the raw MCP panel; this tab answers "give the agent access to
// my Gmail / Outlook / GitHub / terminal" in one click per service.
//
// Everything connected here becomes an MCP server under the hood, which means
// its tools flow into the agent's tool registry automatically. Credentials go
// to the OS keychain only — the agent sees tool names, never tokens.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';
import type { McpServerView } from '../mcp/types';
import './connections.css';

interface Props {
  onToast: (msg: string) => void;
}

type AuthKind = 'oauth' | 'bearer' | 'builtin';

interface ConnectorTemplate {
  id: string;
  name: string;
  tagline: string;
  auth: AuthKind;
  /** Prefilled server URL; empty means the user pastes it from a registry. */
  urlHint: string;
  urlPlaceholder: string;
  oauth?: {
    authorizationUrl: string;
    tokenUrl: string;
    scopes: { value: string; label: string; recommended?: boolean }[];
    redirectUri: string;
  };
  bearerLabel?: string;
  bearerPlaceholder?: string;
  note?: string;
}

const TEMPLATES: ConnectorTemplate[] = [
  {
    id: 'gmail',
    name: 'Gmail',
    tagline: 'Let the agent read and send your email.',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Gmail MCP server URL (see starter pack doc)',
    oauth: {
      authorizationUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
      tokenUrl: 'https://oauth2.googleapis.com/token',
      scopes: [
        { value: 'https://www.googleapis.com/auth/gmail.readonly', label: 'Read mail', recommended: true },
        { value: 'https://www.googleapis.com/auth/gmail.send', label: 'Send mail' },
        { value: 'https://www.googleapis.com/auth/gmail.modify', label: 'Read + modify (labels, archive)' },
      ],
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
    },
    note: 'Start with Read-only. You need a Google Cloud OAuth client ID (Desktop app type).',
  },
  {
    id: 'outlook',
    name: 'Outlook',
    tagline: 'Let the agent read and send your Outlook mail.',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Outlook MCP server URL (see starter pack doc)',
    oauth: {
      authorizationUrl: 'https://login.microsoftonline.com/common/oauth2/v2.0/authorize',
      tokenUrl: 'https://login.microsoftonline.com/common/oauth2/v2.0/token',
      scopes: [
        { value: 'Mail.Read', label: 'Read mail', recommended: true },
        { value: 'Mail.Send', label: 'Send mail' },
      ],
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
    },
    note: 'Start with Mail.Read. Register an app in Azure Entra ID to get a client ID.',
  },
  {
    id: 'gcal',
    name: 'Google Calendar',
    tagline: 'Let the agent see and manage your schedule.',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Calendar MCP server URL',
    oauth: {
      authorizationUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
      tokenUrl: 'https://oauth2.googleapis.com/token',
      scopes: [
        { value: 'https://www.googleapis.com/auth/calendar.readonly', label: 'Read calendars', recommended: true },
        { value: 'https://www.googleapis.com/auth/calendar', label: 'Read + write calendars' },
      ],
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
    },
    note: 'Same Google Cloud OAuth client as Gmail works here.',
  },
  {
    id: 'github',
    name: 'GitHub',
    tagline: 'Repos, issues, PRs, and code search for the agent.',
    auth: 'bearer',
    urlHint: 'https://api.githubcopilot.com/mcp/',
    urlPlaceholder: 'MCP server URL (verify it is current)',
    bearerLabel: 'Personal access token',
    bearerPlaceholder: 'ghp_… / github_pat_…',
    note: 'Use a fine-grained PAT scoped to the repos the agent needs. Verify the server URL in the starter pack doc before connecting.',
  },
  {
    id: 'websearch',
    name: 'Web Search',
    tagline: 'Fresh information on demand — how the agent stays current.',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your search MCP server URL (Brave, Firecrawl…)',
    bearerLabel: 'API key',
    bearerPlaceholder: 'Brave / Firecrawl API key',
    note: 'This is what keeps a local model current: it looks things up instead of guessing from weights.',
  },
  {
    id: 'youtube',
    name: 'YouTube',
    tagline: 'Search videos and pull transcripts — look up anything, read it instead of watching.',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your YouTube MCP server URL (see starter pack doc)',
    bearerLabel: 'API key',
    bearerPlaceholder: 'Server API key (if required)',
    note: 'API-key servers like transcriptapi.com or web-data-toolkit work here. Some hosted servers use their own OAuth sign-in instead — check the starter pack doc.',
  },
  {
    id: 'music',
    name: 'Music Generation',
    tagline: 'Have the agent compose full songs — vocals, lyrics, any genre.',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your music MCP server URL (see starter pack doc)',
    bearerLabel: 'API key',
    bearerPlaceholder: 'AIMLAPI / PiAPI key',
    note: 'AIMLAPI wraps Suno, Udio and more behind one key. Suno is the strongest for full songs with vocals (v6, downloads on paid plans).',
  },
];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

type PermLevel = 'always_allow' | 'ask_every_time' | 'deny';

const PERM_OPTIONS: { id: PermLevel; label: string }[] = [
  { id: 'ask_every_time', label: 'Ask every time' },
  { id: 'always_allow', label: 'Always allow' },
  { id: 'deny', label: 'Deny' },
];

function PermSelect({
  domain,
  onToast,
}: {
  domain: string;
  onToast: (msg: string) => void;
}): React.ReactElement {
  const [level, setLevel] = useState<PermLevel>('ask_every_time');
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let live = true;
    void invoke<string>('perm_get', { domain })
      .then((l) => {
        if (!live) return;
        if (l === 'always_allow' || l === 'ask_every_time' || l === 'deny') setLevel(l);
        setLoaded(true);
      })
      .catch(() => setLoaded(true));
    return () => {
      live = false;
    };
  }, [domain]);

  const change = async (next: PermLevel) => {
    const prev = level;
    setLevel(next);
    try {
      await invoke('perm_set', { domain, level: next });
      onToast(
        next === 'always_allow'
          ? 'Always allowed — the agent will not ask first'
          : next === 'deny'
            ? 'Denied — the agent cannot use this at all'
            : 'The agent will ask for approval each time',
      );
    } catch (e) {
      setLevel(prev);
      onToast(errText(e));
    }
  };

  return (
    <label className="conn-perm">
      <span className="gf-muted conn-small">Permission</span>{' '}
      <select
        className="gf-input conn-perm-select"
        value={level}
        disabled={!loaded}
        onChange={(e) => void change(e.target.value as PermLevel)}
      >
        {PERM_OPTIONS.map((o) => (
          <option key={o.id} value={o.id}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

export function ConnectionsPanel({ onToast }: Props): React.ReactElement {
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [connecting, setConnecting] = useState<ConnectorTemplate | null>(null);
  const [url, setUrl] = useState('');
  const [clientId, setClientId] = useState('');
  const [token, setToken] = useState('');
  const [scopes, setScopes] = useState<string[]>([]);
  const [oauth, setOauth] = useState<{ serverId: string; authUrl: string; state: string; code: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<McpServerView[]>('mcp_server_list');
      setServers(list);
    } catch (e) {
      onToast(errText(e));
    }
  }, [onToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const openConnect = (t: ConnectorTemplate) => {
    setConnecting(t);
    setUrl(t.urlHint);
    setClientId('');
    setToken('');
    setScopes(t.oauth ? t.oauth.scopes.filter((s) => s.recommended).map((s) => s.value) : []);
    setOauth(null);
  };

  const toggleScope = (v: string) => {
    setScopes((prev) => (prev.includes(v) ? prev.filter((s) => s !== v) : [...prev, v]));
  };

  const doConnect = async () => {
    if (!connecting) return;
    if (!url.trim()) {
      onToast('Enter the MCP server URL first');
      return;
    }
    if (connecting.auth === 'oauth' && !clientId.trim()) {
      onToast('Enter your OAuth client ID first');
      return;
    }
    if (connecting.auth === 'bearer' && !token.trim()) {
      onToast('Enter your API token first');
      return;
    }
    setBusy(true);
    try {
      const input = {
        name: connecting.name,
        transport: 'streamable_http' as const,
        url: url.trim(),
        auth_type: connecting.auth,
        oauth_client_id: clientId.trim(),
        oauth_authorization_url: connecting.oauth?.authorizationUrl ?? '',
        oauth_token_url: connecting.oauth?.tokenUrl ?? '',
        oauth_scopes: scopes.join(' '),
        oauth_redirect_uri: connecting.oauth?.redirectUri ?? '',
        allowed_tools: [] as string[],
        denied_tools: [] as string[],
        bearer_present: connecting.auth === 'bearer' && token.length > 0,
      };
      const id = await invoke<string>('mcp_server_add', { input });
      if (connecting.auth === 'bearer' && token) {
        // Tokens go to the OS keyring only — never the database, never the model.
        await invoke('secret_set', { key: `gfa-mcp-bearer:${id}`, value: token });
      }
      if (connecting.auth === 'oauth') {
        const res = await invoke<{ auth_url: string; state: string }>('mcp_oauth_start', {
          server_id: id,
        });
        setOauth({ serverId: id, authUrl: res.auth_url, state: res.state, code: '' });
        onToast('Server added — complete sign-in below');
      } else {
        onToast(`${connecting.name} connected`);
        setConnecting(null);
      }
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const completeOAuth = async () => {
    if (!oauth || !oauth.code.trim()) {
      onToast('Paste the authorization code first');
      return;
    }
    try {
      await invoke('mcp_oauth_callback', { state: oauth.state, code: oauth.code.trim() });
      setOauth(null);
      setConnecting(null);
      onToast('Signed in');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const disconnect = async (s: McpServerView) => {
    const label = s.oauth_connected ? 'Disconnect (revoke access)' : 'Remove server';
    if (!window.confirm(`${label} "${s.name}"?`)) return;
    try {
      if (s.oauth_connected) {
        await invoke('mcp_oauth_revoke', { server_id: s.id });
      } else {
        await invoke('mcp_server_remove', { server_id: s.id });
      }
      onToast('Done');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const toggleEnabled = async (s: McpServerView) => {
    try {
      await invoke(s.enabled ? 'mcp_server_disable' : 'mcp_server_enable', { server_id: s.id });
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  return (
    <div className="gf-workspace">
      <div className="gf-pane conn-pane">
        <div className="gf-pane-header">
          <span>Connections</span>
          <span className="gf-muted"> — give the agent access to your accounts and apps</span>
        </div>

        <p className="conn-intro">
          Everything you connect here becomes a tool your agent can use — email, calendar,
          code, web search. Tokens are stored in your OS keychain; the agent sees tool
          names, never your credentials. Builders keep using the raw MCP panel; this tab
          is the agent's front door.
        </p>

        <h3 className="conn-section">Built-in</h3>
        <div className="conn-grid">
          <div className="conn-card">
            <div className="conn-card-head">
              <strong>Terminal</strong>
              <span className="conn-badge conn-badge-ok">Available</span>
            </div>
            <p className="conn-tagline">
              Full Linux shell for the agent: download files, install programs,
              run commands, clean up install files, uninstall programs — the whole
              lifecycle, one command at a time.
            </p>
            <p className="conn-note">
              Gated by the constitution: shell commands are classified by risk and
              consequential ones require your approval. Fine-grained per-tool rules
              live in the Permission Center.
            </p>
            <PermSelect domain="shell" onToast={onToast} />
          </div>
        </div>

        <h3 className="conn-section">Connect a service</h3>
        <div className="conn-grid">
          {TEMPLATES.map((t) => {
            const existing = servers.find(
              (s) => s.name.toLowerCase() === t.name.toLowerCase(),
            );
            const connected = existing && (existing.oauth_connected || existing.bearer_present || existing.enabled);
            return (
              <div className="conn-card" key={t.id}>
                <div className="conn-card-head">
                  <strong>{t.name}</strong>
                  {connected ? (
                    <span className="conn-badge conn-badge-ok">Connected</span>
                  ) : (
                    <span className="conn-badge">
                      {t.auth === 'oauth' ? 'OAuth' : t.auth === 'bearer' ? 'API key' : ''}
                    </span>
                  )}
                </div>
                <p className="conn-tagline">{t.tagline}</p>
                {t.note && <p className="conn-note">{t.note}</p>}
                {!connected && (
                  <button
                    className="gf-btn conn-connect"
                    onClick={() => openConnect(t)}
                    disabled={busy}
                  >
                    Connect
                  </button>
                )}
              </div>
            );
          })}
        </div>

        <h3 className="conn-section">Connected ({servers.length})</h3>
        <div className="conn-mcp-perm">
          <span className="gf-muted conn-small">
            Permission for all connected service tools:
          </span>{' '}
          <PermSelect domain="mcp" onToast={onToast} />
        </div>
        {servers.length === 0 ? (
          <p className="gf-muted">Nothing connected yet. Pick a service above.</p>
        ) : (
          <div className="conn-list">
            {servers.map((s) => (
              <div className="conn-row" key={s.id}>
                <div>
                  <strong>{s.name}</strong>{' '}
                  <span className="gf-muted conn-small">
                    {s.tool_count} tools · {s.enabled ? 'enabled' : 'disabled'}
                    {s.oauth_connected ? ' · signed in' : ''}
                  </span>
                </div>
                <div className="conn-row-actions">
                  <button className="gf-btn gf-btn-sm" onClick={() => void toggleEnabled(s)}>
                    {s.enabled ? 'Disable' : 'Enable'}
                  </button>
                  <button
                    className="gf-btn gf-btn-sm conn-danger"
                    onClick={() => void disconnect(s)}
                  >
                    {s.oauth_connected ? 'Disconnect' : 'Remove'}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {connecting && (
          <div className="conn-modal-backdrop" onClick={() => !busy && setConnecting(null)}>
            <div className="conn-modal" onClick={(e) => e.stopPropagation()}>
              <h3>Connect {connecting.name}</h3>
              <label className="gf-label">
                MCP server URL
                <input
                  className="gf-input conn-mono"
                  value={url}
                  placeholder={connecting.urlPlaceholder}
                  onChange={(e) => setUrl(e.target.value)}
                />
              </label>
              {connecting.auth === 'oauth' && (
                <>
                  <label className="gf-label">
                    OAuth client ID
                    <input
                      className="gf-input conn-mono"
                      value={clientId}
                      placeholder="Your app's client ID"
                      onChange={(e) => setClientId(e.target.value)}
                    />
                  </label>
                  {connecting.oauth && (
                    <fieldset className="conn-scopes">
                      <legend className="gf-label">Scopes (least privilege first)</legend>
                      {connecting.oauth.scopes.map((sc) => (
                        <label key={sc.value} className="conn-scope">
                          <input
                            type="checkbox"
                            checked={scopes.includes(sc.value)}
                            onChange={() => toggleScope(sc.value)}
                          />
                          <span>
                            {sc.label}
                            {sc.recommended && <em> — recommended</em>}
                          </span>
                        </label>
                      ))}
                    </fieldset>
                  )}
                  {connecting.note && <p className="conn-note">{connecting.note}</p>}
                </>
              )}
              {connecting.auth === 'bearer' && (
                <label className="gf-label">
                  {connecting.bearerLabel ?? 'API token'}
                  <input
                    type="password"
                    className="gf-input conn-mono"
                    value={token}
                    placeholder={connecting.bearerPlaceholder}
                    onChange={(e) => setToken(e.target.value)}
                  />
                </label>
              )}
              <div className="conn-modal-actions">
                <button className="gf-btn" onClick={() => setConnecting(null)} disabled={busy}>
                  Cancel
                </button>
                <button className="gf-btn gf-btn-primary" onClick={() => void doConnect()} disabled={busy}>
                  {busy ? 'Working…' : connecting.auth === 'oauth' ? 'Add & sign in' : 'Connect'}
                </button>
              </div>

              {oauth && (
                <div className="conn-oauth">
                  <p>
                    <strong>Step 1:</strong> open this sign-in URL, approve access, then
                    paste the authorization code below.
                  </p>
                  <textarea className="gf-textarea conn-mono" readOnly rows={4} value={oauth.authUrl} />
                  <button
                    className="gf-btn gf-btn-sm"
                    onClick={() => {
                      void navigator.clipboard?.writeText(oauth.authUrl).then(
                        () => onToast('Sign-in URL copied'),
                        () => onToast('Copy failed — select the URL manually'),
                      );
                    }}
                  >
                    Copy sign-in URL
                  </button>
                  <label className="gf-label">
                    <strong>Step 2:</strong> authorization code
                    <input
                      className="gf-input conn-mono"
                      value={oauth.code}
                      onChange={(e) => setOauth({ ...oauth, code: e.target.value })}
                      placeholder="Paste code here"
                    />
                  </label>
                  <button className="gf-btn gf-btn-primary" onClick={() => void completeOAuth()}>
                    Finish sign-in
                  </button>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
