// SPDX-License-Identifier: Apache-2.0
// Phase 2 — Provider Control Center: provider cards, add/edit form, handshake,
// connection badges (CONNECTED / ERROR [code] + layman explanation + raw code).
// Master spec sections 10-12. API keys are masked inputs, stored in the OS
// keyring via `provider_upsert`, and never displayed again.

import React, { useCallback, useEffect, useState } from 'react';
import './providers.css';
import { isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import {
  getProviders,
  handshakeProvider,
  refreshModels,
  upsertProvider,
  deleteProvider,
  type ProviderKind,
  type ProviderSummary,
  type HandshakeReport,
} from '../../lib/providers';

const KINDS: { id: ProviderKind; label: string; baseHint: string; needsKey: boolean }[] = [
  { id: 'openai', label: 'OpenAI', baseHint: 'https://api.openai.com/v1', needsKey: true },
  { id: 'openai-compatible', label: 'OpenAI-compatible', baseHint: 'https://your-host/v1', needsKey: true },
  { id: 'anthropic', label: 'Anthropic', baseHint: 'https://api.anthropic.com', needsKey: true },
  { id: 'google', label: 'Google AI', baseHint: 'https://generativelanguage.googleapis.com/v1beta', needsKey: true },
  { id: 'openrouter', label: 'OpenRouter', baseHint: 'https://openrouter.ai/api/v1', needsKey: true },
  { id: 'ollama', label: 'Local (Ollama-style)', baseHint: 'http://localhost:11434', needsKey: false },
  { id: 'custom', label: 'Custom endpoint', baseHint: 'https://your-host/v1', needsKey: true },
];

const MODELS_PATH_HINT: Record<ProviderKind, string> = {
  openai: '/models',
  'openai-compatible': '/models',
  anthropic: '/v1/models',
  google: '/models',
  openrouter: '/models',
  ollama: '/api/tags',
  custom: '/models',
};

interface Props {
  onToast: (text: string) => void;
}

interface FormState {
  id?: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  models_path: string;
  api_key: string;
}

const EMPTY_FORM: FormState = {
  name: '',
  kind: 'openai',
  base_url: '',
  models_path: '',
  api_key: '',
};

function kindLabel(kind: string): string {
  return KINDS.find((k) => k.id === kind)?.label ?? kind;
}

export function ProvidersPanel({ onToast }: Props): React.ReactElement {
  const desktop = isDesktop();
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [form, setForm] = useState<FormState | null>(null);
  const [lastReport, setLastReport] = useState<Record<string, HandshakeReport>>({});

  const reload = useCallback(async (): Promise<void> => {
    if (!isDesktop()) {
      setLoading(false);
      return;
    }
    try {
      setProviders(await getProviders());
    } catch (err) {
      if (!(err instanceof DesktopCapabilityRequired)) {
        onToast('Could not load providers');
      }
    } finally {
      setLoading(false);
    }
  }, [onToast]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const handleConnect = async (p: ProviderSummary): Promise<void> => {
    setBusy(p.id);
    try {
      const report = await handshakeProvider(p.id);
      setLastReport((r) => ({ ...r, [p.id]: report }));
      if (report.ok) {
        onToast(`${p.name}: CONNECTED — ${report.models_found} models`);
      } else {
        onToast(`${p.name}: ERROR [${report.raw_code}] — ${report.layman}`);
      }
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : 'Handshake failed',
      );
    } finally {
      setBusy(null);
      void reload();
    }
  };

  const handleRefresh = async (p: ProviderSummary): Promise<void> => {
    setBusy(p.id);
    try {
      const models = await refreshModels(p.id);
      onToast(`${p.name}: catalog refreshed — ${models.length} models`);
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : `Refresh failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(null);
      void reload();
    }
  };

  const handleDelete = async (p: ProviderSummary): Promise<void> => {
    if (!window.confirm(`Delete provider "${p.name}"? Its stored key is removed from the keyring.`)) {
      return;
    }
    try {
      await deleteProvider(p.id);
      onToast(`Deleted ${p.name}`);
      void reload();
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : 'Delete failed',
      );
    }
  };

  const openForm = (p?: ProviderSummary): void => {
    if (p) {
      setForm({
        id: p.id,
        name: p.name,
        kind: p.kind,
        base_url: p.base_url,
        models_path: p.models_path,
        api_key: '',
      });
    } else {
      setForm({ ...EMPTY_FORM });
    }
  };

  const saveForm = async (): Promise<void> => {
    if (!form) return;
    if (!form.name.trim()) {
      onToast('Provider name is required');
      return;
    }
    setBusy('form');
    try {
      const saved = await upsertProvider({
        id: form.id,
        name: form.name.trim(),
        kind: form.kind,
        base_url: form.base_url.trim(),
        models_path: form.models_path.trim(),
        api_key: form.api_key || undefined,
      });
      setForm(null);
      onToast(
        form.id
          ? `Saved ${saved.name}${form.api_key ? ' (key updated in keyring)' : ''}`
          : `Added ${saved.name}`,
      );
      void reload();
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : `Save failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusy(null);
    }
  };

  if (!desktop) {
    return (
      <div className="gf-providers">
        <div className="gf-pwa-notice">
          Provider management needs the desktop app: API keys are stored in the
          system keyring, which browsers cannot access.{' '}
          <strong>Desktop Capability Required.</strong>
        </div>
      </div>
    );
  }

  const kindMeta = KINDS.find((k) => k.id === form?.kind);

  return (
    <div className="gf-providers">
      <div className="gf-providers-head">
        <h2>Providers</h2>
        <button className="gf-btn primary" onClick={() => openForm()}>
          Add Provider
        </button>
      </div>

      {loading && <span className="gf-spin">Loading providers…</span>}

      {!loading && providers.length === 0 && !form && (
        <div className="gf-empty">
          No providers yet. Add your first provider to connect a model service —
          keys are stored in the system keyring, never in the app database.
        </div>
      )}

      {providers.map((p) => {
        const report = lastReport[p.id];
        const errCode = report && !report.ok ? report.raw_code : p.last_error_code;
        return (
          <div className="gf-provider-card" key={p.id}>
            <div className="gf-provider-card-top">
              <span className="gf-provider-name">{p.name}</span>
              <span className="gf-kind-chip">{kindLabel(p.kind)}</span>
              {p.status === 'connected' && <span className="gf-badge ok">CONNECTED</span>}
              {p.status === 'error' && (
                <span className="gf-badge err">ERROR{errCode ? ` [${errCode}]` : ''}</span>
              )}
              {p.status === 'idle' && <span className="gf-badge idle">IDLE</span>}
              {busy === p.id && <span className="gf-spin">working…</span>}
            </div>
            <span className="gf-provider-url">{p.base_url}</span>
            <div className="gf-provider-meta">
              <span>{p.has_key ? 'Key stored in keyring' : 'No key stored'}</span>
              {p.last_check_at != null && (
                <span>
                  Last check:{' '}
                  {new Date(p.last_check_at).toLocaleString()}
                </span>
              )}
            </div>
            {report && !report.ok && (
              <div className="gf-err-detail">
                <div>
                  <span className="raw">[{report.raw_code}]</span> {report.layman}
                </div>
                <div className="gf-muted">{report.detail}</div>
              </div>
            )}
            {p.status === 'error' && !(report && !report.ok) && p.last_error_code && (
              <div className="gf-err-detail">
                <span className="raw">[{p.last_error_code}]</span> — reconnect to
                see the full explanation.
              </div>
            )}
            <div className="gf-provider-actions">
              <button
                className="gf-btn primary"
                disabled={busy !== null}
                onClick={() => void handleConnect(p)}
              >
                Connect
              </button>
              <button
                className="gf-btn"
                disabled={busy !== null}
                onClick={() => void handleRefresh(p)}
              >
                Refresh Catalog
              </button>
              <button
                className="gf-btn"
                disabled={busy !== null}
                onClick={() => openForm(p)}
              >
                Edit
              </button>
              <button
                className="gf-btn danger"
                disabled={busy !== null}
                onClick={() => void handleDelete(p)}
              >
                Delete
              </button>
            </div>
          </div>
        );
      })}

      {form && (
        <div className="gf-provider-form">
          <h3>{form.id ? 'Edit Provider' : 'Add Provider'}</h3>
          <div>
            <label className="gf-label" htmlFor="pf-name">Name</label>
            <input
              id="pf-name"
              className="gf-input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="My model service"
            />
          </div>
          <div className="gf-form-row">
            <div>
              <label className="gf-label" htmlFor="pf-kind">Kind</label>
              <select
                id="pf-kind"
                className="gf-select"
                value={form.kind}
                onChange={(e) =>
                  setForm({ ...form, kind: e.target.value as ProviderKind })
                }
              >
                {KINDS.map((k) => (
                  <option key={k.id} value={k.id}>
                    {k.label}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="gf-label" htmlFor="pf-models-path">Models path</label>
              <input
                id="pf-models-path"
                className="gf-input"
                value={form.models_path}
                onChange={(e) => setForm({ ...form, models_path: e.target.value })}
                placeholder={form ? MODELS_PATH_HINT[form.kind] : '/models'}
              />
            </div>
          </div>
          <div>
            <label className="gf-label" htmlFor="pf-base">Base URL</label>
            <input
              id="pf-base"
              className="gf-input"
              value={form.base_url}
              onChange={(e) => setForm({ ...form, base_url: e.target.value })}
              placeholder={kindMeta?.baseHint ?? 'https://'}
            />
          </div>
          {(!kindMeta || kindMeta.needsKey) && (
            <div>
              <label className="gf-label" htmlFor="pf-key">API key</label>
              <input
                id="pf-key"
                className="gf-input"
                type="password"
                autoComplete="off"
                value={form.api_key}
                onChange={(e) => setForm({ ...form, api_key: e.target.value })}
                placeholder={
                  form.id
                    ? 'Leave blank to keep the stored key'
                    : 'Stored in the system keyring — never shown again'
                }
              />
            </div>
          )}
          <div className="gf-keyring-note">
            Credentials are encrypted at rest in the system keyring, masked in
            the UI, and never written to the app database or logs.
          </div>
          <div className="gf-row">
            <button
              className="gf-btn primary"
              disabled={busy === 'form'}
              onClick={() => void saveForm()}
            >
              {busy === 'form' ? 'Saving…' : 'Save'}
            </button>
            <button className="gf-btn" onClick={() => setForm(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
