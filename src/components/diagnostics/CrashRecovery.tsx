// SPDX-License-Identifier: Apache-2.0
// Phase 11 — Crash recovery: on mount, load any persisted snapshot. If one
// exists, show a modal offering Resume / Inspect / Roll Back / Discard.
// Destructive options (Roll Back, Discard) require typing CONFIRM — nothing is
// ever auto-resumed destructively.
//
// The actual resume/rollback mechanics belong to the phases that own the
// state (builder, agents, artifacts); this component surfaces the decision
// and hands the parsed snapshot to the parent via callbacks. Also exports
// `saveRecoveryPoint` so the parent's heartbeat can persist state.

import React, { useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import '../../a11y.css';
import './diagnostics.css';

export interface RecoveryDecision {
  action: 'resume' | 'inspect' | 'rollback' | 'discard';
  /** Parsed snapshot payload, if it was valid JSON. */
  state: unknown;
}

interface Props {
  /** Called with the user's decision and the parsed snapshot. */
  onDecide: (decision: RecoveryDecision) => void;
  onToast?: (text: string) => void;
}

/** Persist a recovery snapshot (call from a heartbeat / before risky ops). */
export async function saveRecoveryPoint(state: unknown): Promise<void> {
  if (!isDesktop()) throw new DesktopCapabilityRequired('crash_state_save');
  const state_json = JSON.stringify({
    saved_at: new Date().toISOString(),
    app: 'guild-foundry-ai',
    state,
  });
  await invoke('crash_state_save', { state_json });
}

function pretty(state: unknown): string {
  try {
    return JSON.stringify(state, null, 2);
  } catch {
    return String(state);
  }
}

export function CrashRecovery({ onDecide, onToast }: Props): React.ReactElement | null {
  const [open, setOpen] = useState(false);
  const [raw, setRaw] = useState<string | null>(null);
  const [parsed, setParsed] = useState<unknown>(null);
  const [inspecting, setInspecting] = useState(false);
  const [pendingDestructive, setPendingDestructive] = useState<'rollback' | 'discard' | null>(null);
  const [confirmText, setConfirmText] = useState('');

  useEffect(() => {
    if (!isDesktop()) return;
    invoke<string | null>('crash_state_load')
      .then((res) => {
        if (res == null) return; // Clean start — nothing to recover.
        setRaw(res);
        try {
          setParsed(JSON.parse(res));
        } catch {
          setParsed(res); // Non-JSON snapshot: still offer Discard/Inspect.
        }
        setOpen(true);
      })
      .catch((err) => {
        if (!(err instanceof DesktopCapabilityRequired)) onToast?.('Crash-state check failed');
      });
  }, [onToast]);

  if (!open) return null;

  const clearAndDecide = async (action: RecoveryDecision['action']): Promise<void> => {
    try {
      await invoke('crash_state_clear');
    } catch {
      // Clearing is best-effort; the decision still stands.
    }
    setOpen(false);
    onDecide({ action, state: parsed });
  };

  const startDestructive = (kind: 'rollback' | 'discard'): void => {
    setPendingDestructive(kind);
    setConfirmText('');
  };

  const confirmDestructive = (): void => {
    if (!pendingDestructive || confirmText.trim().toUpperCase() !== 'CONFIRM') return;
    void clearAndDecide(pendingDestructive);
  };

  return (
    <div className="gf-modal-backdrop" role="dialog" aria-modal="true" aria-label="Crash recovery">
      <div className="gf-modal" style={{ maxWidth: 640 }}>
        <h2 style={{ margin: '0 0 8px', fontSize: 16, color: 'var(--gf-gold)' }}>
          Recovered previous session
        </h2>
        <p className="gf-muted" style={{ fontSize: 13, margin: '0 0 14px' }}>
          The app closed unexpectedly and left a recovery snapshot. Nothing has
          been resumed automatically — choose what to do with it.
        </p>

        <div className="gf-row" style={{ flexWrap: 'wrap', marginBottom: 12 }}>
          <button className="gf-btn primary" onClick={() => void clearAndDecide('resume')}>
            Resume
          </button>
          <button className="gf-btn" onClick={() => setInspecting((v) => !v)}>
            {inspecting ? 'Hide snapshot' : 'Inspect'}
          </button>
          <button className="gf-btn danger" onClick={() => startDestructive('rollback')}>
            Roll Back
          </button>
          <button className="gf-btn danger" onClick={() => startDestructive('discard')}>
            Discard
          </button>
        </div>

        {inspecting && (
          <div
            className="gf-json-inspect"
            tabIndex={0}
            aria-label="Recovery snapshot contents"
            style={{ marginBottom: 12 }}
          >
            {pretty(parsed ?? raw)}
          </div>
        )}

        {pendingDestructive && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <p className="gf-offline-note" role="alert" style={{ margin: 0 }}>
              {pendingDestructive === 'rollback'
                ? 'Roll Back reverts to the snapshot state, discarding any work done after it was saved.'
                : 'Discard permanently deletes the recovery snapshot.'}{' '}
              This cannot be undone.
            </p>
            <label className="gf-label" htmlFor="crash-confirm">
              Type CONFIRM to {pendingDestructive === 'rollback' ? 'roll back' : 'discard'}
            </label>
            <div className="gf-row">
              <input
                id="crash-confirm"
                className="gf-input"
                value={confirmText}
                onChange={(e) => setConfirmText(e.target.value)}
                placeholder="Type CONFIRM"
                autoComplete="off"
              />
              <button
                className="gf-btn danger"
                disabled={confirmText.trim().toUpperCase() !== 'CONFIRM'}
                onClick={confirmDestructive}
              >
                Confirm
              </button>
              <button className="gf-btn" onClick={() => setPendingDestructive(null)}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
