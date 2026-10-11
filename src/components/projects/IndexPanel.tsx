// SPDX-License-Identifier: Apache-2.0
// Phase 10 — Project index panel: rebuild the filename/symbol/text index,
// keyword search it, and start/stop the file watcher. Wires the previously
// uncalled `index_rebuild`, `index_search`, `index_watch_start`,
// `index_watch_stop` backend commands.

import React, { useState } from 'react';
import { invoke } from '../../lib/api';

interface IndexHit {
  path: string;
  kind: string;
  snippet?: string;
}

export function IndexPanel({
  projectRoot,
  onToast,
}: {
  projectRoot: string;
  onToast: (t: string) => void;
}): React.ReactElement {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<IndexHit[]>([]);
  const [watching, setWatching] = useState(false);
  const [busy, setBusy] = useState(false);

  const rebuild = async () => {
    setBusy(true);
    try {
      await invoke('index_rebuild', { project_root: projectRoot });
      onToast('Index rebuilt');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const search = async () => {
    if (query.trim().length < 2) {
      onToast('Enter at least 2 characters');
      return;
    }
    try {
      const r = await invoke<IndexHit[]>('index_search', { query: query.trim(), project_root: projectRoot });
      setHits(r ?? []);
      if ((r ?? []).length === 0) onToast('No index hits');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const toggleWatch = async () => {
    try {
      if (watching) {
        await invoke('index_watch_stop', { project_root: projectRoot });
        setWatching(false);
        onToast('Watcher stopped');
      } else {
        await invoke('index_watch_start', { project_root: projectRoot });
        setWatching(true);
        onToast('Watcher started');
      }
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div style={{ padding: 8, borderTop: '1px solid var(--gf-border)' }}>
      <div className="gf-row" style={{ justifyContent: 'space-between' }}>
        <span className="gf-label" style={{ margin: 0 }}>Project index</span>
        <span className="gf-badge idle">{watching ? 'watching' : 'idle'}</span>
      </div>
      <div className="gf-row" style={{ gap: 6, marginTop: 4 }}>
        <button className="gf-btn" disabled={busy} onClick={() => void rebuild()}>
          Rebuild
        </button>
        <button className="gf-btn" onClick={() => void toggleWatch()}>
          {watching ? 'Stop watch' : 'Watch'}
        </button>
      </div>
      <div className="gf-row" style={{ gap: 6, marginTop: 6 }}>
        <input
          className="gf-input gf-grow"
          placeholder="Search index…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void search();
          }}
        />
        <button className="gf-btn" onClick={() => void search()}>
          Search
        </button>
      </div>
      {hits.length > 0 && (
        <div style={{ marginTop: 6, maxHeight: 160, overflowY: 'auto' }}>
          {hits.slice(0, 50).map((h, i) => (
            <div key={i} style={{ fontSize: 12 }}>
              <span className="gf-badge idle">{h.kind}</span> {h.path}
              {h.snippet && <div className="gf-muted" style={{ fontSize: 11 }}>{h.snippet.slice(0, 160)}</div>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
