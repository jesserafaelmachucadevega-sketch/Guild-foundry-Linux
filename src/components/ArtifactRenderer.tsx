import React, { useState } from 'react';
import type { Artifact } from '../lib/types';

// Phase 1 shell of the SDUI engine (polls, checklists, sliders, cards,
// whiteboards, stickies). Agents create these with the `artifact.create` tool;
// the Agent tab appends the returned artifact to the message's artifacts array.
export function ArtifactRenderer({ artifact }: { artifact: Artifact }): React.ReactElement {
  const [checked, setChecked] = useState<boolean[]>(
    (artifact.items ?? []).map((i) => i.checked),
  );
  const [slider, setSlider] = useState<number>(artifact.value ?? artifact.min ?? 0);
  const [voted, setVoted] = useState<number | null>(null);

  if (artifact.kind === 'sticky') {
    return (
      <div
        style={{
          marginTop: 8,
          padding: 12,
          width: 220,
          minHeight: 160,
          background: artifact.color ?? '#fff7ad',
          color: '#1a1a1a',
          borderRadius: 2,
          boxShadow: '2px 3px 8px rgba(0,0,0,0.35)',
          transform: 'rotate(-1deg)',
        }}
      >
        {artifact.title && <strong style={{ display: 'block', marginBottom: 6 }}>{artifact.title}</strong>}
        <div style={{ whiteSpace: 'pre-wrap' }}>{artifact.body}</div>
      </div>
    );
  }

  if (artifact.kind === 'whiteboard') {
    return (
      <div style={{ marginTop: 8, border: '1px solid var(--gf-border)', padding: 8 }}>
        <div className="gf-label" style={{ marginBottom: 8 }}>{artifact.title}</div>
        <div style={{ position: 'relative', minHeight: 320, background: 'var(--gf-pane-bg, #1b1b1b)', borderRadius: 4, overflow: 'hidden' }}>
          {(artifact.notes ?? []).map((n, i) => (
            <div
              key={n.id ?? i}
              style={{
                position: 'absolute',
                left: `${Math.min(100, Math.max(0, n.x))}%`,
                top: `${Math.min(100, Math.max(0, n.y))}%`,
                transform: 'translate(-8px, -8px) rotate(-1deg)',
                width: 180,
                minHeight: 110,
                padding: 10,
                background: n.color ?? '#fff7ad',
                color: '#1a1a1a',
                borderRadius: 2,
                boxShadow: '2px 3px 8px rgba(0,0,0,0.35)',
                fontSize: '0.85rem',
              }}
            >
              {n.title && <strong style={{ display: 'block', marginBottom: 4 }}>{n.title}</strong>}
              <div style={{ whiteSpace: 'pre-wrap' }}>{n.body}</div>
            </div>
          ))}
          {(artifact.notes ?? []).length === 0 && (
            <div className="gf-muted" style={{ padding: 24 }}>Empty board.</div>
          )}
        </div>
      </div>
    );
  }

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
