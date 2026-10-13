// SPDX-License-Identifier: Apache-2.0
// Local GGUF models — point the app at .gguf files and it runs them itself
// (llama.cpp compiled in). No server, no local-model manager required.
//
// Scanned models appear under the built-in "Local GGUF files" provider, so
// every model picker in the app (agent, conference, builder, coding
// workspace) can use them with zero extra setup.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke, isDesktop } from '../../lib/api';
import { open } from '@tauri-apps/plugin-dialog';

interface GgufFile {
  path: string;
  name: string;
  size: number;
}

interface ScanResult {
  count: number;
  files: GgufFile[];
  folders: string[];
}

interface GgufStatus {
  loaded: string | null;
  context: number;
  engine: string;
}

function fmtGB(bytes: number): string {
  return `${(bytes / 1073741824).toFixed(2)} GB`;
}

function basename(p: string): string {
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
}

export function LocalModels({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [scan, setScan] = useState<ScanResult>({ count: 0, files: [], folders: [] });
  const [status, setStatus] = useState<GgufStatus | null>(null);
  const [ctx, setCtx] = useState('8192');
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!isDesktop()) return;
    try {
      const s = await invoke<ScanResult>('gguf_scan');
      setScan(s ?? { count: 0, files: [], folders: [] });
      const st = await invoke<GgufStatus>('gguf_status');
      setStatus(st);
      setCtx(String(st?.context ?? 8192));
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  }, [onToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const saveFolders = async (folders: string[]) => {
    try {
      await invoke('gguf_set_folders', { folders });
      await refresh();
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const addFolder = async () => {
    try {
      const dir = await open({ directory: true, multiple: false, title: 'Add a folder containing .gguf files' });
      if (!dir || typeof dir !== 'string') return;
      await saveFolders([...scan.folders, dir]);
      onToast('Folder added');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const addModelFile = async () => {
    try {
      const file = await open({
        multiple: false,
        title: 'Pick a .gguf model',
        filters: [{ name: 'GGUF model', extensions: ['gguf'] }],
      });
      if (!file || typeof file !== 'string') return;
      const i = Math.max(file.lastIndexOf('/'), file.lastIndexOf('\\'));
      const dir = i >= 0 ? file.slice(0, i) : file;
      await saveFolders([...scan.folders, dir]);
      onToast('Model folder added — the file is ready to use');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const removeFolder = async (folder: string) => {
    await saveFolders(scan.folders.filter((f) => f !== folder));
  };

  const load = async (path: string) => {
    setBusy(path);
    try {
      await invoke('gguf_load', { path });
      onToast('Model loaded — pick it in any chat');
      await refresh();
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const unload = async () => {
    try {
      await invoke('gguf_unload');
      onToast('Model unloaded');
      await refresh();
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const saveCtx = async () => {
    const n = Number(ctx);
    if (!Number.isFinite(n)) {
      onToast('Context size must be a number');
      return;
    }
    try {
      const applied = await invoke<number>('gguf_set_ctx', { ctx: Math.round(n) });
      onToast(`Context size set to ${applied} tokens`);
      await refresh();
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  if (!isDesktop()) {
    return <div className="gf-pwa-notice">Local models need the desktop app.</div>;
  }

  return (
    <div className="gf-pane" style={{ padding: 14, overflowY: 'auto' }}>
      <p className="gf-muted" style={{ marginTop: 0 }}>
        Point this at your <strong>.gguf</strong> files — the app runs them itself (llama.cpp compiled in). No server,
        no local-model manager. Scanned models show up under <strong>Local GGUF files</strong> in every model picker,
        and temperature / top-p / top-k settings apply to them too.
      </p>

      <div className="gf-pane" style={{ padding: 10, marginBottom: 12 }}>
        <div className="gf-row" style={{ gap: 16, flexWrap: 'wrap', alignItems: 'center' }}>
          <span>
            Engine: <strong>{status?.engine ?? 'llama.cpp (in-process)'}</strong>
          </span>
          <span>
            Loaded:{' '}
            <strong>{status?.loaded ? basename(status.loaded) : 'none'}</strong>
          </span>
          {status?.loaded && (
            <button className="gf-btn gf-btn-sm" onClick={() => void unload()}>
              Unload
            </button>
          )}
          <label className="gf-row" style={{ gap: 6, alignItems: 'center' }}>
            <span className="gf-muted">Context (tokens)</span>
            <input
              className="gf-input"
              style={{ width: 110 }}
              value={ctx}
              onChange={(e) => setCtx(e.target.value)}
            />
            <button className="gf-btn gf-btn-sm" onClick={() => void saveCtx()}>
              Save
            </button>
          </label>
        </div>
        <p className="gf-muted" style={{ marginBottom: 0, marginTop: 6, fontSize: 12 }}>
          Bigger context = more memory. Loading happens on first use; switching models swaps automatically.
        </p>
      </div>

      <div className="gf-row" style={{ gap: 8, marginBottom: 10 }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void addModelFile()}>
          Add model file…
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void addFolder()}>
          Add folder…
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void refresh()}>
          Rescan
        </button>
      </div>

      <div className="gf-label" style={{ marginTop: 8 }}>
        Folders ({scan.folders.length})
      </div>
      <div style={{ marginBottom: 12 }}>
        {scan.folders.map((f) => (
          <div key={f} className="gf-row" style={{ gap: 8, marginTop: 4, alignItems: 'center' }}>
            <span className="gf-muted" style={{ flex: 1, wordBreak: 'break-all' }}>
              {f}
            </span>
            <button className="gf-icon-btn" onClick={() => void removeFolder(f)} title="Remove this folder">
              Remove
            </button>
          </div>
        ))}
      </div>

      <div className="gf-label">Models ({scan.files.length})</div>
      {scan.files.length === 0 && (
        <p className="gf-muted">
          No .gguf files found yet. Use <strong>Add model file…</strong> and pick one — for example from your models
          drive.
        </p>
      )}
      {scan.files.map((f) => {
        const isLoaded = status?.loaded === f.path;
        return (
          <div
            key={f.path}
            className="gf-row"
            style={{ gap: 10, marginTop: 6, alignItems: 'center', flexWrap: 'wrap' }}
          >
            <span style={{ minWidth: 220 }}>
              <strong>{f.name}</strong> <span className="gf-muted">· {fmtGB(f.size)}</span>
            </span>
            <span className="gf-muted" style={{ flex: 1, fontSize: 12, wordBreak: 'break-all' }}>
              {f.path}
            </span>
            {isLoaded ? (
              <span className="conn-badge conn-badge-ok">Loaded</span>
            ) : (
              <button className="gf-btn gf-btn-sm" disabled={busy === f.path} onClick={() => void load(f.path)}>
                {busy === f.path ? 'Loading…' : 'Load'}
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
