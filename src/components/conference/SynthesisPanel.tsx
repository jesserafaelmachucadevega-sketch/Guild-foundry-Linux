// SPDX-License-Identifier: Apache-2.0
// Phase 3 — Cross-model knowledge synthesizer bridge panel. Calls the Phase 10
// `memory_synthesize` command honestly: if the command is missing (or the
// desktop runtime is absent), it says so instead of faking synthesis output.

import React, { useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';

interface SynthesisPanelProps {
  topic: string;
  claims: string[];
  onToast: (t: string) => void;
}

export function SynthesisPanel({ topic, claims, onToast }: SynthesisPanelProps): React.ReactElement {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState(false);

  const run = async (): Promise<void> => {
    if (claims.length === 0) {
      onToast('No final positions to synthesize yet — run a debate first');
      return;
    }
    setBusy(true);
    setUnavailable(false);
    try {
      const r = await invoke<unknown>('memory_synthesize', { topic, claims });
      setResult(typeof r === 'string' ? r : JSON.stringify(r, null, 2));
    } catch (err) {
      setUnavailable(true);
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : 'Knowledge bridge lands in Phase 10',
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="gf-synth-panel">
      <span className="gf-label">Knowledge bridge</span>
      <div className="gf-muted" style={{ fontSize: 12, marginBottom: 8 }}>
        {claims.length === 0
          ? 'Synthesizes the models\u2019 final positions into shared knowledge.'
          : `${claims.length} final position${claims.length === 1 ? '' : 's'} ready to synthesize.`}
      </div>
      <button className="gf-btn primary" disabled={busy} onClick={() => void run()}>
        {busy ? 'Synthesizing…' : 'Synthesize'}
      </button>
      {result !== null && !unavailable && (
        <pre className="gf-synth-result">{result}</pre>
      )}
      {unavailable && (
        <div className="gf-muted" style={{ fontSize: 12, marginTop: 8 }}>
          Synthesis runs on the Phase 10 memory bridge, which is not implemented
          yet — nothing was fabricated.
        </div>
      )}
    </div>
  );
}
