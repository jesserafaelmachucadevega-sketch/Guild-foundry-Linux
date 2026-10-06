import React, { useState } from 'react';
import type { Artifact } from '../lib/types';

// Phase 1 shell of the SDUI engine (polls, checklists, sliders, cards).
// Section III extends this with the full interactive renderer.
export function ArtifactRenderer({ artifact }: { artifact: Artifact }): React.ReactElement {
  const [checked, setChecked] = useState<boolean[]>(
    (artifact.items ?? []).map((i) => i.checked),
  );
  const [slider, setSlider] = useState<number>(artifact.value ?? artifact.min ?? 0);
  const [voted, setVoted] = useState<number | null>(null);

  if (artifact.kind === 'card') {
    return (
      <div className="gf-bubble" style={{ marginTop: 8 }}>
        <div className="gf-bubble-header">
          <span className="gf-bubble-role">{artifact.title}</span>
        </div>
        <div className="gf-bubble-body">{artifact.body}</div>
      </div>
    );
  }

  if (artifact.kind === 'poll') {
    return (
      <div style={{ marginTop: 8, border: '1px solid var(--gf-border)', padding: 8 }}>
        <div className="gf-label">{artifact.title}</div>
        {(artifact.options ?? []).map((opt, i) => (
          <div key={i} className="gf-row" style={{ marginTop: 4 }}>
            <button
              className="gf-btn"
              style={{ padding: '4px 10px' }}
              onClick={() => setVoted(i)}
            >
              {voted === i ? '[x]' : '[ ]'}
            </button>
            <span>{opt}</span>
          </div>
        ))}
      </div>
    );
  }

  if (artifact.kind === 'checklist') {
    return (
      <div style={{ marginTop: 8, border: '1px solid var(--gf-border)', padding: 8 }}>
        <div className="gf-label">{artifact.title}</div>
        {(artifact.items ?? []).map((item, i) => (
          <label key={i} className="gf-row" style={{ marginTop: 4, cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={checked[i] ?? false}
              onChange={() => {
                const next = [...checked];
                next[i] = !next[i];
                setChecked(next);
              }}
            />
            <span>{item.label}</span>
          </label>
        ))}
      </div>
    );
  }

  // slider
  return (
    <div style={{ marginTop: 8, border: '1px solid var(--gf-border)', padding: 8 }}>
      <div className="gf-label">
        {artifact.title}: {slider}
      </div>
      <input
        type="range"
        min={artifact.min ?? 0}
        max={artifact.max ?? 100}
        value={slider}
        onChange={(e) => setSlider(Number(e.target.value))}
        style={{ width: '100%' }}
      />
    </div>
  );
}
