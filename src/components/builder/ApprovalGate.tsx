// SPDX-License-Identifier: Apache-2.0
// Phase 5 — Human approval gate modal. Listens for the `agent://approval`
// Tauri event; Approve/Deny resolves via `approval_resolve`, which emits
// `agent://approval-resolved` for any waiting orchestration loop.

import { useEffect, useState } from 'react';
import { invoke } from '../../lib/api';
import { listen } from '@tauri-apps/api/event';
import type { ApprovalEvent } from './types';

export default function ApprovalGate() {
  const [pending, setPending] = useState<ApprovalEvent[]>([]);
  const [reason, setReason] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<ApprovalEvent>('agent://approval', (event) => {
      setPending((p) => [...p, event.payload]);
    }).then((u) => {
      unlisten = u;
    });
    return () => unlisten?.();
  }, []);

  if (pending.length === 0) return null;
  const current = pending[0];

  const resolve = async (decision: 'approved' | 'denied') => {
    setBusy(true);
    setError('');
    try {
      await invoke('approval_resolve', {
        approval_id: current.approval_id,
        decision,
        reason,
      });
      setPending((p) => p.slice(1));
      setReason('');
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  let payloadPretty = current.payload_json;
  try {
    payloadPretty = JSON.stringify(JSON.parse(current.payload_json), null, 2);
  } catch {
    /* show raw */
  }

  return (
    <div className="gf-modal-backdrop" role="dialog" aria-modal="true" aria-label="Approval gate">
      <div className="gf-modal" style={{ maxWidth: 560 }}>
        <h3 style={{ color: 'var(--gf-gold)', marginTop: 0 }}>Approval required</h3>
        <div className="bld-approval-kind">{current.kind.replace(/_/g, ' ')}</div>
        <p style={{ fontSize: 13 }}>{current.summary}</p>
        <pre
          className="bld-log"
          style={{ height: 120, fontSize: 11 }}
        >
          {payloadPretty}
        </pre>
        <div className="gf-muted" style={{ fontSize: 11, marginBottom: 8 }}>
          requested {current.requested_at}
          {pending.length > 1 && ` · ${pending.length - 1} more waiting`}
        </div>
        <input
          className="gf-input"
          placeholder="Reason (optional)"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          style={{ width: '100%', marginBottom: 8 }}
        />
        {error && <div className="bld-error">{error}</div>}
        <div className="gf-row" style={{ justifyContent: 'flex-end', gap: 8 }}>
          <button className="gf-btn danger" disabled={busy} onClick={() => void resolve('denied')}>
            Deny
          </button>
          <button className="gf-btn primary" disabled={busy} onClick={() => void resolve('approved')}>
            Approve
          </button>
        </div>
      </div>
    </div>
  );
}
