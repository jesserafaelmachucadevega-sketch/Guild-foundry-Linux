// SPDX-License-Identifier: Apache-2.0
// Model & API Hub — providers (keyring credentials + handshake), the model
// catalog + router, and model aliases in one hub with three inner tabs.
// IPC contract: Tauri `provider_upsert` expects snake_case `base_url` /
// `models_path` (see src-tauri/src/providers.rs). Never invoke it directly
// with camelCase `baseUrl` / `modelsPath` — route through `upsertProvider`
// from ../../lib/providers, which maps baseUrl -> base_url and
// modelsPath -> models_path.

import React, { useEffect, useState } from 'react';
import { invoke, isDesktop } from '../../lib/api';
import { getProviders, type ModelInfo, type ProviderSummary } from '../../lib/providers';
import { ProvidersPanel } from '../providers/ProvidersPanel';
import { ModelCatalog } from '../models/ModelCatalog';
import './hub.css';

interface Props {
  onToast: (t: string) => void;
}

type HubTab = 'providers' | 'catalog' | 'aliases';

function AliasesPanel({ onToast }: Props): React.ReactElement {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [modelsByProvider, setModelsByProvider] = useState<Record<string, ModelInfo[]>>({});
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saved, setSaved] = useState<Record<string, string>>({});
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (!isDesktop()) {
      setLoaded(true);
      return;
    }
    let live = true;
    void (async () => {
      try {
        const [list, aliases] = await Promise.all([
          getProviders(),
          invoke<Record<string, string> | null>('settings_get', { key: 'models.aliases' }),
        ]);
        if (!live) return;
        setProviders(list ?? []);
        const a = aliases ?? {};
        setDraft(a);
        setSaved(a);
        const grouped: Record<string, ModelInfo[]> = {};
        await Promise.all(
          (list ?? []).map(async (p) => {
            try {
              const ms = await invoke<ModelInfo[]>('provider_list_models', { provider_id: p.id });
              if (live) grouped[p.id] = ms ?? [];
            } catch {
              if (live) grouped[p.id] = [];
            }
          }),
        );
        if (live) {
          setModelsByProvider(grouped);
          setLoaded(true);
        }
      } catch {
        if (live) setLoaded(true);
      }
    })();
    return () => {
      live = false;
    };
  }, []);

  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);

  const save = async () => {
    const cleaned: Record<string, string> = {};
    for (const [k, v] of Object.entries(draft)) {
      const t = v.trim();
      if (t) cleaned[k] = t;
    }
    try {
      await invoke('settings_set', { key: 'models.aliases', value: cleaned });
      setDraft(cleaned);
      setSaved(cleaned);
      onToast('Aliases saved');
    } catch (e) {
      onToast(`Save failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  if (!isDesktop()) {
    return (
      <div className="gf-models">
        <div className="gf-pwa-notice">
          Model aliases need the desktop app. <strong>Desktop Capability Required.</strong>
        </div>
      </div>
    );
  }
  if (!loaded) return <div className="gf-models"><p className="gf-muted">Loading models…</p></div>;

  return (
    <div className="gf-models">
      <div className="gf-models-head">
        <h2>Model Aliases</h2>
        <button className="gf-btn primary" disabled={!dirty} onClick={() => void save()}>
          Save aliases
        </button>
      </div>
      <p className="gf-muted" style={{ margin: 0 }}>
        Short names for models (e.g. <span style={{ fontFamily: 'var(--gf-mono)' }}>workhorse</span> for a long model id).
        Used across chat, routing, and favorites.
      </p>
      {providers.length === 0 && <p className="gf-muted">No providers yet — add one under API Keyring & Providers first.</p>}
      {providers.map((p) => (
        <section key={p.id}>
          <h3 className="conn-category">{p.name}</h3>
          {(modelsByProvider[p.id] ?? []).length === 0 && <p className="gf-muted">No cataloged models.</p>}
          {(modelsByProvider[p.id] ?? []).map((m) => (
            <div className="gf-row" key={m.id} style={{ gap: 10, marginBottom: 6 }}>
              <span className="gf-muted conn-small" style={{ minWidth: 220, fontFamily: 'var(--gf-mono)', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                {m.model_id}
              </span>
              <input
                className="gf-input"
                style={{ maxWidth: 280 }}
                placeholder="Alias (optional)"
                value={draft[m.id] ?? ''}
                onChange={(e) => setDraft((d) => ({ ...d, [m.id]: e.target.value }))}
              />
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

export function ModelsApiHub({ onToast }: Props): React.ReactElement {
  const [tab, setTab] = useState<HubTab>('providers');
  return (
    <div className="gf-workspace">
      <div className="gf-pane" style={{ flex: 1, minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        <div className="gf-pane-header">
          <span>Model & API Hub</span>
          <span className="gf-view-switcher" role="tablist" aria-label="Model and API views">
            {(
              [
                ['providers', 'API Keyring & Providers'],
                ['catalog', 'Model Catalog & Router'],
                ['aliases', 'Model Aliases'],
              ] as [HubTab, string][]
            ).map(([id, label]) => (
              <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? 'active' : ''} onClick={() => setTab(id)}>
                {label}
              </button>
            ))}
          </span>
        </div>
        <div className="hub-fill">
          {tab === 'providers' && <ProvidersPanel onToast={onToast} />}
          {tab === 'catalog' && <ModelCatalog onToast={onToast} />}
          {tab === 'aliases' && <AliasesPanel onToast={onToast} />}
        </div>
      </div>
    </div>
  );
}
