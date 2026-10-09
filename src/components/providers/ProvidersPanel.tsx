// SPDX-License-Identifier: Apache-2.0
// Phase 2 — Provider Control Center: provider cards, add/edit form, handshake,
// connection badges (CONNECTED / ERROR [code] + layman explanation + raw code).
// Master spec sections 10-12. API keys are masked inputs, stored in the OS
// keyring via `provider_upsert`, and never displayed again.

import React, { useCallback, useEffect, useMemo, useState } from 'react';
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
import { PROVIDER_TEMPLATES, type ProviderTemplate } from '../../lib/providerTemplates';

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
  kind: 'openai-compatible',
  base_url: '',
  models_path: '/models',
  api_key: '',
};

function kindLabel(kind: string): string {
  return KINDS.find((k) => k.id === kind)?.label ?? kind;
}

const ADD_NEW_VALUE = '__add_new__';

function formFromTemplate(tpl: ProviderTemplate): FormState {
  return {
    name: tpl.name,
    kind: tpl.kind,
    base_url: tpl.base_url,
    models_path: tpl.models_path,
    api_key: '',
  };
}

function formFromSaved(p: ProviderSummary): FormState {
  return {
    id: p.id,
    name: p.name,
    kind: (p.kind as ProviderKind) ?? 'openai-compatible',
    base_url: p.base_url,
    models_path: p.models_path,
    api_key: '',
  };
}

export function ProvidersPanel({ onToast }: Props): React.ReactElement {
  const desktop = isDesktop();
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [form, setForm] = useState<FormState | null>(null);
  const [lastReport, setLastReport] = useState<Record<string, HandshakeReport>>({});
  // Dropdown selection: saved provider id | preset name | __add_new__ | ''
  const [selected, setSelected] = useState<string>('');
  const [editingSavedId, setEditingSavedId] = useState<string | null>(null);

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

  const savedById = useMemo(() => {
    const m = new Map<string, ProviderSummary>();
    for (const p of providers) m.set(p.id, p);
    return m;
  }, [providers]);

  const selectedSaved = selected && selected !== ADD_NEW_VALUE
    ? (savedById.get(selected) ?? providers.find((p) => p.name === selected))
    : undefined;

  const selectedTemplate: ProviderTemplate | undefined = useMemo(() => {
    if (!selected || selected === ADD_NEW_VALUE || selectedSaved) return undefined;
    return PROVIDER_TEMPLATES.find((tpl) => tpl.name === selected);
  }, [selected, selectedSaved]);

  const handleSelect = (value: string): void => {
    setSelected(value);
    setEditingSavedId(null);
    if (!value) {
      setForm(null);
      return;
    }
    if (value === ADD_NEW_VALUE) {
      // "Add New" is always last in the dropdown: blank connection fields.
      setForm({ ...EMPTY_FORM });
      return;
    }
    const saved = savedById.get(value) ?? providers.find((p) => p.name === value);
    if (saved) {
      // Saved provider: populate stored URL + kind; key stays in the keyring,
      // so reconnect is just Connect — no re-entry needed.
      setForm(formFromSaved(saved));
      return;
    }
    const tpl = PROVIDER_TEMPLATES.find((tpl) => tpl.name === value);
    if (tpl) {
      setForm(formFromTemplate(tpl));
      return;
    }
    setForm({ ...EMPTY_FORM });
  };

  const handleKindChange = (kind: ProviderKind): void => {
    setForm((f) => {
      if (!f) return f;
      const next: FormState = { ...f, kind };
      // Auto-fill the V1 base URL + models path when the user switches kind
      // on a fresh (unsaved) form so they never submit a blank endpoint.
      if (!f.id && (!f.base_url.trim() || KINDS.some((k) => k.baseHint === f.base_url))) {
        const hint = KINDS.find((k) => k.id === kind)?.baseHint ?? '';
        if (hint) next.base_url = hint;
      }
      if (!f.id && (!f.models_path.trim())) {
        next.models_path = MODELS_PATH_HINT[kind] ?? '/models';
      }
      return next;
    });
  };

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

  const handleConnectSelected = async (): Promise<void> => {
    if (!selectedSaved) {
      onToast('Save this provider first, then Connect');
      return;
    }
    await handleConnect(selectedSaved);
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
      if (selected === p.id || selected === p.name) {
        setSelected('');
        setForm(null);
      }
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
      setSelected(p.id);
      setEditingSavedId(p.id);
      setForm(formFromSaved(p));
    } else {
      setSelected(ADD_NEW_VALUE);
      setForm({ ...EMPTY_FORM });
    }
  };

  const startEditSelected = (): void => {
    if (!selectedSaved || !form) return;
    // Edit mode: every field (name, kind, URL, models path, key) is editable
    // so a rotated key or changed endpoint can be updated in place.
    setEditingSavedId(selectedSaved.id);
    setForm(formFromSaved(selectedSaved));
    onToast(`Editing ${selectedSaved.name} — change any field, then Save`);
  };

  const saveForm = async (): Promise<void> => {
    if (!form) return;
    if (!form.name.trim()) {
      onToast('Provider name is required');
      return;
    }
    if (!form.base_url.trim()) {
      onToast('V1 endpoint URL is required');
      return;
    }
    setBusy('form');
    try {
      const saved = await upsertProvider({
        // When editing a saved row keep its id so the keyring entry is reused;
        // otherwise create a new row (preset or Add New becomes a saved provider).
        id: form.id ?? editingSavedId ?? undefined,
        name: form.name.trim(),
        kind: form.kind,
        base_url: form.base_url.trim(),
        models_path: form.models_path.trim() || '/models',
        api_key: form.api_key || undefined,
      });
      onToast(
        form.id || editingSavedId
          ? `Saved ${saved.name}${form.api_key ? ' (key updated in keyring)' : ' (key kept in keyring — just hit Connect)'}`
          : `Added ${saved.name} — saved securely. Just hit Connect next time.`,
      );
      setSelected(saved.id);
      setForm(formFromSaved(saved));
      setEditingSavedId(null);
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
  const handshakePreview = form ? `${form.base_url.replace(/\/$/, '')}${form.models_path || '/models'}` : '';
  const activeTemplateHelp = selectedTemplate?.help
    ?? PROVIDER_TEMPLATES.find((tpl) => tpl.name === form?.name)?.help;

  return (
    <div className="gf-providers">
      <div className="gf-providers-head">
        <h2>Providers</h2>
        <button className="gf-btn primary" onClick={() => openForm()}>
          Add Provider
        </button>
      </div>

      {/* ---- Quick-connect dropdown: saved + presets + Add New last ---- */}
      <div className="gf-provider-form" style={{ maxWidth: 640 }}>
        <h3>Provider connection</h3>
        <div>
          <label className="gf-label" htmlFor="pf-provider-select">Provider</label>
          <select
            id="pf-provider-select"
            className="gf-select"
            value={selected}
            onChange={(e) => handleSelect(e.target.value)}
          >
            <option value="">— Choose a provider —</option>
            {providers.length > 0 && (
              <optgroup label="Saved providers (keys in keyring — just hit Connect)">
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name} — {p.has_key ? 'key saved' : 'no key'} — {p.status}
                  </option>
                ))}
              </optgroup>
            )}
            <optgroup label="Presets (pre-filled V1 endpoint)">
              {PROVIDER_TEMPLATES.map((tpl) => (
                <option key={tpl.id} value={tpl.name}>
                  {tpl.name}
                </option>
              ))}
            </optgroup>
            <option value={ADD_NEW_VALUE}>Add New…</option>
          </select>
        </div>

        {form && (
          <>
            <div>
              <label className="gf-label" htmlFor="pf-name">Provider name</label>
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
                  onChange={(e) => handleKindChange(e.target.value as ProviderKind)}
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
              <label className="gf-label" htmlFor="pf-base">V1 endpoint URL (base URL)</label>
              <input
                id="pf-base"
                className="gf-input"
                value={form.base_url}
                onChange={(e) => setForm({ ...form, base_url: e.target.value })}
                placeholder={kindMeta?.baseHint ?? 'https://'}
                inputMode="url"
                autoComplete="off"
              />
              {handshakePreview && (
                <div className="gf-form-hint" style={{ marginTop: 4 }}>
                  Handshake URL: <span style={{ fontFamily: 'var(--gf-mono)' }}>{handshakePreview}</span>
                </div>
              )}
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
                    selectedTemplate?.keyPlaceholder
                    ?? (form.id || editingSavedId
                      ? 'Leave blank to keep the stored key — or paste a new key to rotate it'
                      : 'Stored in the system keyring — never shown again')
                  }
                />
                {(form.id || editingSavedId) && selectedSaved?.has_key && !form.api_key && (
                  <div className="gf-form-hint">A key is already stored in the keyring — just hit Connect.</div>
                )}
              </div>
            )}
            {activeTemplateHelp && (
              <div className="gf-keyring-note">{activeTemplateHelp}</div>
            )}
            <div className="gf-keyring-note">
              Credentials are encrypted at rest in the system keyring, masked in
              the UI, and never written to the app database or logs. Saved providers
              reconnect with Connect — no re-entry needed.
            </div>
            <div className="gf-row" style={{ flexWrap: 'wrap' }}>
              <button
                className="gf-btn primary"
                disabled={busy === 'form'}
                onClick={() => void saveForm()}
              >
                {busy === 'form' ? 'Saving…' : 'Save'}
              </button>
              {selectedSaved && (
                <>
                  <button
                    className="gf-btn"
                    disabled={busy !== null}
                    onClick={startEditSelected}
                    title="Edit name, URL, models path, or rotate the API key"
                  >
                    Edit
                  </button>
                  <button
                    className="gf-btn primary"
                    disabled={busy !== null}
                    onClick={() => void handleConnectSelected()}
                    title="Handshake with the saved key — no re-entry needed"
                  >
                    Connect
                  </button>
                </>
              )}
              <button className="gf-btn" onClick={() => { setForm(null); setSelected(''); setEditingSavedId(null); }}>
                Cancel
              </button>
            </div>
          </>
        )}
      </div>

      {loading && <span className="gf-spin">Loading providers…</span>}

      {!loading && providers.length === 0 && !form && (
        <div className="gf-empty">
          No providers yet. Choose a preset above (or Add New…) to connect a model service —
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

      {form && selected === '' && null}
    </div>
  );
}
