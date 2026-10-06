// SPDX-License-Identifier: Apache-2.0
// Phase 2 — Model catalog: every model across all providers, sorted
// VERIFIED FREE → VERIFIED PAID → UNKNOWN (master spec section 13).
// Favorites, hidden models, and custom aliases persist through the Phase 1
// settings commands. Includes the model router (spec section 14): describe a
// task, get a suggestion with the reason; automatic routing can be disabled.

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import './models.css';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import {
  getProviders,
  listModels,
  suggestModel,
  type ModelInfo,
  type ProviderSummary,
  type PricingVerified,
  type RouterSuggestion,
} from '../../lib/providers';

interface Props {
  onToast: (text: string) => void;
}

type PriceFilter = 'all' | PricingVerified;
type CapFilter = 'all' | 'tools' | 'vision' | 'reasoning';

const PRICE_RANK: Record<PricingVerified, number> = { free: 0, paid: 1, unknown: 2 };

const PRICE_LABEL: Record<PricingVerified, string> = {
  free: 'VERIFIED FREE',
  paid: 'VERIFIED PAID',
  unknown: 'UNKNOWN',
};

async function loadSetting<T>(key: string, fallback: T): Promise<T> {
  try {
    const v = await invoke<T | null>('settings_get', { key });
    return (v ?? fallback) as T;
  } catch {
    return fallback;
  }
}

async function saveSetting(key: string, value: unknown): Promise<void> {
  await invoke('settings_set', { key, value });
}

interface CatalogEntry extends ModelInfo {
  providerName: string;
  display: string; // alias ?? name
}

export function ModelCatalog({ onToast }: Props): React.ReactElement {
  const desktop = isDesktop();
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [entries, setEntries] = useState<CatalogEntry[]>([]);
  const [loading, setLoading] = useState(true);

  const [favorites, setFavorites] = useState<string[]>([]);
  const [hidden, setHidden] = useState<string[]>([]);
  const [aliases, setAliases] = useState<Record<string, string>>({});
  const [showHidden, setShowHidden] = useState(false);

  const [search, setSearch] = useState('');
  const [priceFilter, setPriceFilter] = useState<PriceFilter>('all');
  const [providerFilter, setProviderFilter] = useState('all');
  const [capFilter, setCapFilter] = useState<CapFilter>('all');
  const [favsOnly, setFavsOnly] = useState(false);

  const [editingAlias, setEditingAlias] = useState<string | null>(null);
  const [aliasDraft, setAliasDraft] = useState('');

  const [routerEnabled, setRouterEnabled] = useState(true);
  const [task, setTask] = useState('');
  const [suggestion, setSuggestion] = useState<RouterSuggestion | null>(null);
  const [routing, setRouting] = useState(false);

  const reload = useCallback(async (): Promise<void> => {
    if (!isDesktop()) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const [provs, favs, hid, als, sh, re] = await Promise.all([
        getProviders(),
        loadSetting<string[]>('models.favorites', []),
        loadSetting<string[]>('models.hidden', []),
        loadSetting<Record<string, string>>('models.aliases', {}),
        loadSetting<boolean>('models.show_hidden', false),
        loadSetting<boolean>('router.enabled', true),
      ]);
      setProviders(provs);
      setFavorites(favs);
      setHidden(hid);
      setAliases(als);
      setShowHidden(sh);
      setRouterEnabled(re);

      const settled = await Promise.allSettled(
        provs.map((p) => listModels(p.id)),
      );
      const all: CatalogEntry[] = [];
      settled.forEach((r, i) => {
        if (r.status === 'fulfilled') {
          for (const m of r.value) {
            all.push({
              ...m,
              providerName: provs[i].name,
              display: als[m.id] && als[m.id].trim() ? als[m.id].trim() : m.name,
            });
          }
        }
      });
      setEntries(all);
    } catch (err) {
      if (!(err instanceof DesktopCapabilityRequired)) {
        onToast('Could not load the model catalog');
      }
    } finally {
      setLoading(false);
    }
  }, [onToast]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const persist = useCallback(
    async (key: string, value: unknown, apply: () => void): Promise<void> => {
      apply();
      try {
        await saveSetting(key, value);
      } catch (err) {
        onToast(
          err instanceof DesktopCapabilityRequired
            ? 'Desktop Capability Required'
            : 'Could not save preference',
        );
      }
    },
    [onToast],
  );

  const toggleFavorite = (id: string): void => {
    const next = favorites.includes(id)
      ? favorites.filter((f) => f !== id)
      : [...favorites, id];
    void persist('models.favorites', next, () => setFavorites(next));
  };

  const toggleHidden = (id: string): void => {
    const next = hidden.includes(id)
      ? hidden.filter((h) => h !== id)
      : [...hidden, id];
    void persist('models.hidden', next, () => setHidden(next));
  };

  const saveAlias = (id: string): void => {
    const next = { ...aliases };
    if (aliasDraft.trim()) {
      next[id] = aliasDraft.trim();
    } else {
      delete next[id];
    }
    setEditingAlias(null);
    void persist('models.aliases', next, () => {
      setAliases(next);
      setEntries((es) =>
        es.map((e) =>
          e.id === id
            ? { ...e, display: next[id]?.trim() ? next[id].trim() : e.name }
            : e,
        ),
      );
    });
  };

  const toggleRouter = (): void => {
    const next = !routerEnabled;
    void persist('router.enabled', next, () => setRouterEnabled(next));
    setSuggestion(null);
  };

  const runRouter = async (): Promise<void> => {
    if (!task.trim()) {
      onToast('Describe the task first');
      return;
    }
    setRouting(true);
    try {
      setSuggestion(await suggestModel(task.trim()));
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : `Router: ${err instanceof Error ? err.message : String(err)}`,
      );
      setSuggestion(null);
    } finally {
      setRouting(false);
    }
  };

  const visible = useMemo(() => {
    const q = search.trim().toLowerCase();
    return entries
      .filter((e) => (showHidden ? true : !hidden.includes(e.id)))
      .filter((e) => priceFilter === 'all' || e.pricing_verified === priceFilter)
      .filter((e) => providerFilter === 'all' || e.provider_id === providerFilter)
      .filter((e) => {
        if (capFilter === 'all') return true;
        if (capFilter === 'tools') return e.supports_tools === true;
        if (capFilter === 'vision') return e.supports_vision === true;
        return e.supports_reasoning === true;
      })
      .filter((e) => !favsOnly || favorites.includes(e.id))
      .filter((e) =>
        q === ''
          ? true
          : e.display.toLowerCase().includes(q) ||
            e.name.toLowerCase().includes(q) ||
            e.providerName.toLowerCase().includes(q),
      )
      .sort((a, b) => {
        const favA = favorites.includes(a.id) ? 0 : 1;
        const favB = favorites.includes(b.id) ? 0 : 1;
        if (favA !== favB) return favA - favB;
        const pa = PRICE_RANK[a.pricing_verified] ?? 2;
        const pb = PRICE_RANK[b.pricing_verified] ?? 2;
        if (pa !== pb) return pa - pb;
        return a.display.localeCompare(b.display);
      });
  }, [entries, hidden, showHidden, priceFilter, providerFilter, capFilter, favsOnly, favorites, search]);

  if (!desktop) {
    return (
      <div className="gf-models">
        <div className="gf-pwa-notice">
          The model catalog needs the desktop app — providers and their
          catalogs live behind native keyring credentials.{' '}
          <strong>Desktop Capability Required.</strong>
        </div>
      </div>
    );
  }

  const priceChip = (id: PriceFilter, label: string): React.ReactElement => (
    <button
      key={id}
      className={`gf-chip${priceFilter === id ? ' active' : ''}`}
      onClick={() => setPriceFilter(id)}
    >
      {label}
    </button>
  );

  const capChip = (id: CapFilter, label: string): React.ReactElement => (
    <button
      key={id}
      className={`gf-chip${capFilter === id ? ' active' : ''}`}
      onClick={() => setCapFilter(id)}
    >
      {label}
    </button>
  );

  return (
    <div className="gf-models">
      <div className="gf-models-head">
        <h2>Model Catalog</h2>
        <span className="gf-models-count">
          {visible.length} of {entries.length} models
        </span>
      </div>

      <div className="gf-router-box">
        <div className="gf-row" style={{ justifyContent: 'space-between' }}>
          <span className="gf-label" style={{ margin: 0 }}>Model router</span>
          <button
            className={`gf-chip${routerEnabled ? ' active' : ''}`}
            onClick={toggleRouter}
            title="Toggle automatic model routing"
          >
            {routerEnabled ? 'Automatic: ON' : 'Automatic: OFF'}
          </button>
        </div>
        <div className="gf-row">
          <input
            className="gf-input"
            value={task}
            onChange={(e) => setTask(e.target.value)}
            placeholder="Describe the task — e.g. “summarize this long document”"
            onKeyDown={(e) => {
              if (e.key === 'Enter') void runRouter();
            }}
          />
          <button
            className="gf-btn primary"
            disabled={routing}
            onClick={() => void runRouter()}
            style={{ whiteSpace: 'nowrap' }}
          >
            {routing ? 'Routing…' : 'Suggest'}
          </button>
        </div>
        {suggestion && (
          <div className="gf-router-result">
            <div>
              <strong>{suggestion.model_id}</strong>{' '}
              <span className="gf-muted">
                ({providers.find((p) => p.id === suggestion.provider_id)?.name ?? suggestion.provider_id})
              </span>
            </div>
            <div className="why">{suggestion.reason}</div>
          </div>
        )}
        {!routerEnabled && (
          <span className="gf-muted" style={{ fontSize: 12 }}>
            Automatic routing is off — suggestions use your configured default model.
          </span>
        )}
      </div>

      <div className="gf-filter-bar">
        <input
          className="gf-input gf-search"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search models…"
        />
        {priceChip('all', 'All prices')}
        {priceChip('free', 'Verified free')}
        {priceChip('paid', 'Verified paid')}
        {priceChip('unknown', 'Unknown')}
        <select
          className="gf-select"
          style={{ width: 'auto' }}
          value={providerFilter}
          onChange={(e) => setProviderFilter(e.target.value)}
        >
          <option value="all">All providers</option>
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        {capChip('all', 'Any capability')}
        {capChip('tools', 'Tool calling')}
        {capChip('vision', 'Vision')}
        {capChip('reasoning', 'Reasoning')}
        <button
          className={`gf-chip${favsOnly ? ' active' : ''}`}
          onClick={() => setFavsOnly((v) => !v)}
        >
          ★ Favorites
        </button>
        <button
          className={`gf-chip${showHidden ? ' active' : ''}`}
          onClick={() => {
            const next = !showHidden;
            void persist('models.show_hidden', next, () => setShowHidden(next));
          }}
        >
          {showHidden ? 'Hide hidden' : 'Show hidden'}
        </button>
      </div>

      {loading && <span className="gf-spin">Loading catalog…</span>}

      {!loading && entries.length === 0 && (
        <div className="gf-empty">
          No models cataloged yet. Connect a provider first — the handshake
          stores its model list here.
        </div>
      )}

      {visible.map((e) => {
        const isFav = favorites.includes(e.id);
        const isHidden = hidden.includes(e.id);
        return (
          <div
            key={e.id}
            className={`gf-model-row${isHidden ? ' is-hidden' : ''}${isFav ? ' is-fav' : ''}`}
          >
            <button
              className={`gf-star${isFav ? ' on' : ''}`}
              title={isFav ? 'Remove from favorites' : 'Add to favorites'}
              onClick={() => toggleFavorite(e.id)}
            >
              ★
            </button>
            <div className="gf-model-main">
              {editingAlias === e.id ? (
                <input
                  className="gf-alias-input"
                  value={aliasDraft}
                  autoFocus
                  onChange={(ev) => setAliasDraft(ev.target.value)}
                  onBlur={() => saveAlias(e.id)}
                  onKeyDown={(ev) => {
                    if (ev.key === 'Enter') saveAlias(e.id);
                    if (ev.key === 'Escape') setEditingAlias(null);
                  }}
                  placeholder={e.name}
                />
              ) : (
                <span className="gf-model-name" title={e.name}>
                  {e.display}
                  {aliases[e.id] && (
                    <span className="gf-muted" style={{ fontWeight: 400 }}>
                      {' '}
                      aka {e.name}
                    </span>
                  )}
                </span>
              )}
              <span className="gf-model-sub">
                {e.providerName} · {e.id}
              </span>
            </div>
            <div className="gf-model-tags">
              <span className={`gf-price ${e.pricing_verified}`}>
                {PRICE_LABEL[e.pricing_verified] ?? 'UNKNOWN'}
              </span>
              {e.context_length != null && (
                <span className="gf-ctx">{Math.round(e.context_length / 1000)}k ctx</span>
              )}
              {e.supports_tools === true && <span className="gf-cap">tools</span>}
              {e.supports_vision === true && <span className="gf-cap">vision</span>}
              {e.supports_reasoning === true && <span className="gf-cap">reasoning</span>}
            </div>
            <div className="gf-row">
              <button
                className="gf-icon-btn"
                title={aliases[e.id] ? 'Edit alias' : 'Set alias'}
                onClick={() => {
                  setEditingAlias(e.id);
                  setAliasDraft(aliases[e.id] ?? '');
                }}
              >
                ✎
              </button>
              <button
                className="gf-icon-btn"
                title={isHidden ? 'Unhide model' : 'Hide model'}
                onClick={() => toggleHidden(e.id)}
              >
                {isHidden ? '◉' : '◌'}
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}
