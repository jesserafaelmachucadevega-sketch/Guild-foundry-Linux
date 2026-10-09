// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: checkpoints panel.
// List / preview / restore / compare snapshots. Snapshots are independent of
// Git: full file copies under <root>/.gf/checkpoints, with an automatic
// pre-restore checkpoint so restores are themselves reversible.
import React, { useCallback, useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';

interface SnapshotInfo {
  id: string;
  label: string;
  created_at: string;
  file_count: number;
}

interface SnapshotFile {
  rel: string;
  size: number;
  sha256: string;
}

interface SnapshotChange {
  rel: string;
  kind: 'added' | 'removed' | 'modified';
}

function errMsg(err: unknown): string {
  return err instanceof DesktopCapabilityRequired
    ? 'Desktop Capability Required'
    : err instanceof Error
      ? err.message
      : String(err);
}

export function CheckpointPanel({
  projectRoot,
  onToast,
}: {
  projectRoot: string;
  onToast: (t: string) => void;
}): React.ReactElement {
  const [snaps, setSnaps] = useState<SnapshotInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<SnapshotFile[] | null>(null);
  const [changes, setChanges] = useState<SnapshotChange[] | null>(null);
  const [view, setView] = useState<'preview' | 'compare'>('preview');

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<SnapshotInfo[]>('ws_checkpoints', {
        project_root: projectRoot,
      });
      setSnaps(list);
    } catch (err) {
      onToast(errMsg(err));
    }
  }, [projectRoot, onToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const takeSnapshot = async () => {
    const label = window.prompt('Checkpoint label:', `manual ${new Date().toLocaleString()}`);
    if (label === null) return;
    try {
      await invoke('ws_snapshot', {
        project_root: projectRoot,
        label: label || 'manual',
      });
      onToast('Checkpoint created');
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const select = async (id: string, v: 'preview' | 'compare') => {
    setSelected(id);
    setView(v);
    setPreview(null);
    setChanges(null);
    try {
      if (v === 'preview') {
        const files = await invoke<SnapshotFile[]>('ws_checkpoint_preview', {
          project_root: projectRoot,
          snapshot_id: id,
        });
        setPreview(files);
      } else {
        const diff = await invoke<SnapshotChange[]>('ws_checkpoint_diff', {
          project_root: projectRoot,
          snapshot_id: id,
        });
        setChanges(diff);
      }
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const restore = async (id: string, label: string) => {
    if (
      !window.confirm(
        `Restore checkpoint "${label}"? Current files will be replaced (an automatic pre-restore checkpoint is taken first).`,
      )
    ) {
      return;
    }
    try {
      await invoke('ws_restore', { project_root: projectRoot, snapshot_id: id });
      onToast('Restored (pre-restore checkpoint saved)');
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  return (
    <div className="gf-projects-col" style={{ width: 340, minWidth: 280 }}>
      <div className="gf-pane-header">
        <span>Checkpoints</span>
        <span className="gf-row">
          <button className="gf-btn" style={{ padding: '0 8px' }} onClick={() => void takeSnapshot()}>
            + Snapshot
          </button>
          <button className="gf-btn" style={{ padding: '0 8px' }} onClick={() => void refresh()}>
            ⟳
          </button>
        </span>
      </div>
      <div className="gf-scroll-list">
        {snaps.map((s) => (
          <div className="gf-checkpoint-card" key={s.id}>
            <div className="gf-title">{s.label}</div>
            <div className="gf-meta">
              {s.created_at} · {s.file_count} file(s)
              <br />
              <span style={{ fontFamily: 'var(--gf-mono)' }}>{s.id.slice(0, 8)}</span>
            </div>
            <div className="gf-row" style={{ marginTop: 8, flexWrap: 'wrap' }}>
              <button className="gf-btn" style={{ padding: '2px 8px', fontSize: 11 }} onClick={() => void select(s.id, 'preview')}>
                Preview
              </button>
              <button className="gf-btn" style={{ padding: '2px 8px', fontSize: 11 }} onClick={() => void select(s.id, 'compare')}>
                Compare
              </button>
              <button className="gf-btn danger" style={{ padding: '2px 8px', fontSize: 11 }} onClick={() => void restore(s.id, s.label)}>
                Restore
              </button>
            </div>
            {selected === s.id && view === 'preview' && (
              <div style={{ marginTop: 8, maxHeight: 200, overflowY: 'auto' }}>
                {preview === null ? (
                  <span className="gf-muted" style={{ fontSize: 12 }}>Loading…</span>
                ) : preview.length === 0 ? (
                  <span className="gf-muted" style={{ fontSize: 12 }}>Empty snapshot.</span>
                ) : (
                  preview.slice(0, 100).map((f) => (
                    <div key={f.rel} className="gf-muted" style={{ fontSize: 11, fontFamily: 'var(--gf-mono)' }}>
                      {f.rel} <span>({f.size}b)</span>
                    </div>
                  ))
                )}
              </div>
            )}
            {selected === s.id && view === 'compare' && (
              <div style={{ marginTop: 8, maxHeight: 200, overflowY: 'auto' }}>
                {changes === null ? (
                  <span className="gf-muted" style={{ fontSize: 12 }}>Loading…</span>
                ) : changes.length === 0 ? (
                  <span className="gf-muted" style={{ fontSize: 12 }}>No differences vs current tree.</span>
                ) : (
                  changes.slice(0, 100).map((c) => (
                    <div key={c.rel} style={{ fontSize: 11, fontFamily: 'var(--gf-mono)' }}>
                      <span className={`gf-change-${c.kind}`}>[{c.kind}]</span>{' '}
                      <span className="gf-muted">{c.rel}</span>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>
        ))}
        {snaps.length === 0 && (
          <p className="gf-muted" style={{ padding: '8px 12px', fontSize: 12 }}>
            No checkpoints yet. Destructive operations create automatic
            checkpoints; take manual ones before big changes.
          </p>
        )}
      </div>
    </div>
  );
}
