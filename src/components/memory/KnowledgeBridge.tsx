// SPDX-License-Identifier: Apache-2.0
import React, { useState } from 'react';
import { invoke } from '../../lib/api';
import './memory.css';

// Phase 10 — KnowledgeBridge: the cross-model shared knowledge bridge.
// memory_synthesize(topic) returns exactly five buckets; Phase 3's
// SynthesisPanel consumes the same shape from the Rust side.

interface SynthesisEntry {
  key: string;
  content: string;
  scope: string;
  provider: string;
  model: string;
  updatedAt: string;
  flagged: string;
}

interface SynthesisView {
  verifiedFacts: SynthesisEntry[];
  modelClaims: SynthesisEntry[];
  userDecisions: SynthesisEntry[];
  inferences: SynthesisEntry[];
  unresolved: SynthesisEntry[];
}

const BUCKETS: { id: keyof SynthesisView; label: string; hint: string }[] = [
  { id: 'verifiedFacts', label: 'Verified facts', hint: 'confirmed true' },
  { id: 'modelClaims', label: 'Model claims', hint: 'asserted by a model, unverified' },
  { id: 'userDecisions', label: 'User decisions', hint: 'explicitly decided by the user' },
  { id: 'inferences', label: 'Inferences', hint: 'derived, not directly observed' },
  { id: 'unresolved', label: 'Unresolved', hint: 'open questions / unclassified' },
];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function KnowledgeBridge(): React.ReactElement {
  const [topic, setTopic] = useState('');
  const [view, setView] = useState<SynthesisView | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);

  async function synthesize(): Promise<void> {
    setBusy(true);
    setStatus('');
    try {
      const v = await invoke<SynthesisView>('memory_synthesize', { topic });
      setView(v);
      const total = BUCKETS.reduce((n, b) => n + v[b.id].length, 0);
      setStatus(`${total} entr${total === 1 ? 'y' : 'ies'} across 5 categories`);
    } catch (e) {
      setStatus(`Desktop Capability Required — ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="gf-pane mem-panel">
      <div className="gf-pane-header">
        <span>Knowledge Bridge</span>
        <span className="gf-badge">cross-model synthesis</span>
      </div>

      <div className="mem-search">
        <input
          className="gf-input"
          value={topic}
          onChange={(e) => setTopic(e.target.value)}
          placeholder="topic, e.g. deployment strategy…"
          onKeyDown={(e) => { if (e.key === 'Enter') void synthesize(); }}
        />
        <button className="gf-btn primary" disabled={busy || topic.trim().length < 2} onClick={() => void synthesize()}>
          Synthesize
        </button>
      </div>

      {status && <div className="mem-status">{status}</div>}

      {view && (
        <div className="mem-bridge">
          {BUCKETS.map((b) => (
            <section key={b.id} className="mem-bucket">
              <h4>
                {b.label} <span className="gf-badge idle">{view[b.id].length}</span>
                <span className="mem-hint">{b.hint}</span>
              </h4>
              {view[b.id].length === 0 && <div className="mem-empty">—</div>}
              {view[b.id].map((e, i) => (
                <div key={`${b.id}-${i}`} className="mem-entry">
                  <div className="mem-entry-key">{e.key}</div>
                  <div className="mem-entry-body">{e.content}</div>
                  <div className="mem-entry-meta">
                    <span className="gf-badge idle">{e.scope} · {e.provider} · {e.model || '—'}</span>
                    {e.flagged && <span className="gf-badge err">flagged: {e.flagged}</span>}
                  </div>
                </div>
              ))}
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
