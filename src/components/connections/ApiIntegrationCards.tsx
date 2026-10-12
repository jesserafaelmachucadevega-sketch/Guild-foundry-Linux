// SPDX-License-Identifier: Apache-2.0
// Direct API integrations — key fields, live handshake tests, and HTTP
// status badges for Tavily, Exa, Context7, GitHub, and Hugging Face.
//
// This is intentionally separate from the MCP-server templates: those connect
// a hosted MCP endpoint, while these cards store a raw API key in the OS
// keyring and probe the vendor REST API directly via `integration_handshake`.
// The same keys back the `web.tavily_search`, `web.exa_search`,
// `github.get_user`, and `hf.whoami` background tools, so the Personal Agent
// and Builder can dispatch them with no MCP server involved.

import React, { useEffect, useState } from 'react';
import { invoke } from '../../lib/api';

export interface IntegrationStatus {
  service: string;
  ok: boolean;
  http_status?: number | null;
  raw_code: string;
  layman: string;
  detail: string;
}

export type BadgeState =
  | { state: 'unconfigured' }
  | { state: 'key-saved' }
  | { state: 'testing' }
  | { state: 'ready'; code: string }
  | { state: 'error'; code: string; layman: string; detail: string };

interface ApiService {
  id: string;
  name: string;
  tagline: string;
  keyringKey: string;
  keyLabel: string;
  keyPlaceholder: string;
  note?: string;
}

const SERVICES: ApiService[] = [
  {
    id: 'tavily',
    name: 'Tavily Web Search',
    tagline: 'Fresh web answers with sources — the agent stays current.',
    keyringKey: 'gfa-integration:tavily',
    keyLabel: 'Tavily API key',
    keyPlaceholder: 'tvly-…',
    note: 'Key from tavily.com → API keys. Tests and searches spend Tavily credits (handshake ≈ 1 basic credit).',
  },
  {
    id: 'exa',
    name: 'Exa AI Search',
    tagline: 'Neural web search with page excerpts for research.',
    keyringKey: 'gfa-integration:exa',
    keyLabel: 'Exa API key',
    keyPlaceholder: 'Exa API key',
    note: 'Key from exa.ai → dashboard → API keys. Tests and searches spend Exa credits.',
  },
  {
    id: 'context7',
    name: 'Context7 Deep Research',
    tagline: 'Up-to-date library docs for code the agent writes.',
    keyringKey: 'gfa-integration:context7',
    keyLabel: 'Context7 API key',
    keyPlaceholder: 'Context7 API key',
    note: 'Key from context7.com → dashboard. The handshake speaks Context7’s native MCP endpoint; no credits are spent.',
  },
  {
    id: 'github',
    name: 'GitHub Developer API',
    tagline: 'Identity check plus repo access for the agent.',
    keyringKey: 'gfa-integration:github',
    keyLabel: 'Personal access token',
    keyPlaceholder: 'ghp_… / github_pat_…',
    note: 'Fine-grained PAT (read-only to start). For full repo tools, also connect the GitHub MCP server below.',
  },
  {
    id: 'huggingface',
    name: 'Hugging Face API',
    tagline: 'Model and dataset identity for the agent.',
    keyringKey: 'gfa-integration:huggingface',
    keyLabel: 'User access token',
    keyPlaceholder: 'hf_…',
    note: 'Token from huggingface.co → settings → tokens. Read role is enough for identity and public models.',
  },
];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function StatusBadge({ badge }: { badge: BadgeState }): React.ReactElement {
  switch (badge.state) {
    case 'unconfigured':
      return <span className="conn-badge">Unconfigured</span>;
    case 'key-saved':
      return <span className="conn-badge">Key saved</span>;
    case 'testing':
      return <span className="conn-badge">Testing…</span>;
    case 'ready':
      return <span className="conn-badge conn-badge-ok">{badge.code}</span>;
    case 'error': {
      const warn = badge.code === '429' || badge.code === '402';
      return (
        <span className={`conn-badge ${warn ? 'conn-badge-warn' : 'conn-badge-err'}`} title={badge.detail}>
          {badge.code} · {badge.layman}
        </span>
      );
    }
  }
}

function statusFromReport(r: IntegrationStatus): BadgeState {
  if (r.ok) return { state: 'ready', code: r.http_status ? `${r.http_status} OK` : 'Ready' };
  return { state: 'error', code: r.raw_code, layman: r.layman, detail: r.detail };
}

function ServiceCard({ svc, onToast }: { svc: ApiService; onToast: (msg: string) => void }): React.ReactElement {
  const [key, setKey] = useState('');
  const [show, setShow] = useState(false);
  const [hasKey, setHasKey] = useState(false);
  const [busy, setBusy] = useState(false);
  const [badge, setBadge] = useState<BadgeState>({ state: 'unconfigured' });

  useEffect(() => {
    let live = true;
    void invoke<string[]>('secret_list_keys')
      .then((keys) => {
        if (!live) return;
        const saved = keys.includes(svc.keyringKey);
        setHasKey(saved);
        if (saved) setBadge({ state: 'key-saved' });
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [svc.keyringKey]);

  const saveKey = async () => {
    if (!key.trim()) {
      onToast(`Paste your ${svc.name} key first`);
      return;
    }
    setBusy(true);
    try {
      await invoke('secret_set', { key: svc.keyringKey, value: key.trim() });
      setHasKey(true);
      setKey('');
      setBadge({ state: 'key-saved' });
      onToast(`${svc.name} key saved to the OS keychain`);
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const removeKey = async () => {
    if (!window.confirm(`Remove the ${svc.name} key? Its tools will stop working.`)) return;
    try {
      await invoke('secret_delete', { key: svc.keyringKey });
      setHasKey(false);
      setBadge({ state: 'unconfigured' });
      onToast(`${svc.name} key removed`);
    } catch (e) {
      onToast(errText(e));
    }
  };

  const test = async () => {
    const typed = key.trim();
    if (!typed && !hasKey) {
      onToast(`Save a ${svc.name} key first, or paste one to probe it`);
      return;
    }
    setBusy(true);
    setBadge({ state: 'testing' });
    try {
      const report = await invoke<IntegrationStatus>('integration_handshake', {
        service: svc.id,
        api_key: typed || undefined,
      });
      setBadge(statusFromReport(report));
      onToast(
        report.ok
          ? `${svc.name}: ${report.http_status ?? 200} OK — ${report.layman}`
          : `${svc.name}: ERROR [${report.raw_code}] — ${report.layman}`,
      );
    } catch (e) {
      setBadge({ state: 'error', code: 'ERR', layman: 'Handshake failed', detail: errText(e) });
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>{svc.name}</strong>
        <StatusBadge badge={badge} />
      </div>
      <p className="conn-tagline">{svc.tagline}</p>
      <div className="conn-keyrow">
        <input
          type={show ? 'text' : 'password'}
          className="gf-input conn-mono"
          value={key}
          placeholder={hasKey ? `New ${svc.keyLabel} (optional — tests unsaved)` : svc.keyPlaceholder}
          autoComplete="off"
          onChange={(e) => setKey(e.target.value)}
        />
        <button className="gf-icon-btn" title={show ? 'Hide key' : 'Show key'} onClick={() => setShow((s) => !s)}>
          {show ? 'Hide' : 'Show'}
        </button>
      </div>
      <div className="gf-row" style={{ gap: 6, flexWrap: 'wrap' }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void saveKey()} disabled={busy || !key.trim()}>
          Save
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void test()} disabled={busy}>
          {busy ? 'Working…' : 'Test handshake'}
        </button>
        {hasKey && (
          <button className="gf-btn gf-btn-sm conn-danger" onClick={() => void removeKey()} disabled={busy}>
            Remove
          </button>
        )}
      </div>
      {svc.note && <p className="conn-note" style={{ marginTop: 8 }}>{svc.note}</p>}
      {badge.state === 'error' && <p className="conn-note">{badge.detail}</p>}
    </div>
  );
}

export function ApiIntegrationCards({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  return (
    <>
      <h3 className="conn-section">API integrations</h3>
      <p className="conn-note">
        Raw API keys for search, research, and repo services. Keys stay in the OS keychain; the agent calls{' '}
        <span className="conn-mono">web.tavily_search</span>, <span className="conn-mono">web.exa_search</span>,{' '}
        <span className="conn-mono">github.get_user</span>, and <span className="conn-mono">hf.whoami</span> in the
        background. Test spends nothing except Tavily/Exa (≈1 search credit).
      </p>
      <div className="conn-grid">
        {SERVICES.map((s) => (
          <ServiceCard key={s.id} svc={s} onToast={onToast} />
        ))}
      </div>
    </>
  );
}
