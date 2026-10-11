// SPDX-License-Identifier: Apache-2.0
// Phase 4 — Advanced file ops: move/rename across directories, unified diff
// between two files, and applying a unified patch. Wires the previously
// uncalled `ws_move`, `ws_diff`, `ws_patch` backend commands.

import React, { useState } from 'react';
import { invoke } from '../../lib/api';
import { audit } from '../../lib/audit';

export function FileOpsPanel({
  projectRoot,
  selected,
  onToast,
  onRefresh,
}: {
  projectRoot: string;
  selected: string | null;
  onToast: (t: string) => void;
  onRefresh: () => void;
}): React.ReactElement {
  const [moveDest, setMoveDest] = useState('');
  const [diffB, setDiffB] = useState('');
  const [diffOut, setDiffOut] = useState('');
  const [patchText, setPatchText] = useState('');
  const [busy, setBusy] = useState(false);

  const src = selected ?? '';

  const doMove = async () => {
    if (!src) {
      onToast('Select a file first');
      return;
    }
    if (!moveDest.trim()) {
      onToast('Enter a destination path');
      return;
    }
    setBusy(true);
    try {
      await invoke('ws_move', { project_root: projectRoot, rel: src, dest: moveDest.trim() });
      audit('INFO', 'filesystem', `moved ${src} to ${moveDest.trim()}`, { src, dest: moveDest.trim() });
      onToast('Moved');
      onRefresh();
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const doDiff = async () => {
    if (!src) {
      onToast('Select the first file in the tree, then enter the second path');
      return;
    }
    if (!diffB.trim()) {
      onToast('Enter the second file path');
      return;
    }
    setBusy(true);
    try {
      const out = await invoke<string>('ws_diff', {
        project_root: projectRoot,
        rel_a: src,
        rel_b: diffB.trim(),
      });
      setDiffOut(out || '(no differences)');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const doPatch = async () => {
    if (!src) {
      onToast('Select the file to patch first');
      return;
    }
    if (!patchText.trim()) {
      onToast('Paste a unified diff first');
      return;
    }
    setBusy(true);
    try {
      await invoke('ws_patch', { project_root: projectRoot, rel: src, patch: patchText });
      audit('INFO', 'filesystem', `patched ${src}`, { rel: src });
      onToast('Patch applied (checkpoint saved)');
      setPatchText('');
      onRefresh();
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ padding: 8, borderTop: '1px solid var(--gf-border)' }}>
      <span className="gf-label">Move / Diff / Patch</span>
      <div className="gf-muted" style={{ fontSize: 11, marginBottom: 4 }}>
        {src ? `Selected: ${src}` : 'Select a file to enable move/diff/patch.'}
      </div>
      <div className="gf-row" style={{ gap: 6 }}>
        <input
          className="gf-input gf-grow"
          placeholder="destination, e.g. src/moved.ts"
          value={moveDest}
          onChange={(e) => setMoveDest(e.target.value)}
        />
        <button className="gf-btn" disabled={busy || !src} onClick={() => void doMove()}>
          Move
        </button>
      </div>
      <div className="gf-row" style={{ gap: 6, marginTop: 6 }}>
        <input
          className="gf-input gf-grow"
          placeholder="compare with, e.g. src/other.ts"
          value={diffB}
          onChange={(e) => setDiffB(e.target.value)}
        />
        <button className="gf-btn" disabled={busy || !src} onClick={() => void doDiff()}>
          Diff
        </button>
      </div>
      {diffOut && (
        <pre style={{ fontSize: 11, whiteSpace: 'pre-wrap', maxHeight: 140, overflowY: 'auto' }}>{diffOut.slice(0, 6000)}</pre>
      )}
      <textarea
        className="gf-textarea"
        style={{ width: '100%', minHeight: 60, marginTop: 6, fontSize: 11 }}
        placeholder="Paste unified diff to apply to selected file…"
        value={patchText}
        onChange={(e) => setPatchText(e.target.value)}
      />
      <button className="gf-btn" style={{ marginTop: 4 }} disabled={busy || !src} onClick={() => void doPatch()}>
        Apply patch
      </button>
    </div>
  );
}
