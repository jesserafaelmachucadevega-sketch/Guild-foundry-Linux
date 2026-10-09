// SPDX-License-Identifier: Apache-2.0
// Phase 3 — Debate controls: topic input, Start, persistent Pause/Resume and
// Stop buttons, round indicator, and the post-debate broadcast input.

import React from 'react';

export type DebatePhase = 'idle' | 'running' | 'paused' | 'done' | 'discussion';

export interface DebateRoundDef {
  id: string;
  label: string;
  short: string;
}

export const DEBATE_ROUNDS: DebateRoundDef[] = [
  { id: 'independent', label: 'R1 · Independent answers', short: 'R1' },
  { id: 'cross-critique', label: 'R2 · Cross-critique', short: 'R2' },
  { id: 'rebuttal', label: 'R3 · Rebuttal', short: 'R3' },
  { id: 'evidence', label: 'R4 · Evidence review', short: 'R4' },
  { id: 'final', label: 'R5 · Final positions', short: 'R5' },
  { id: 'judge', label: 'R6 · Judge / synthesis', short: 'R6' },
];

interface DebateControlsProps {
  phase: DebatePhase;
  roundIndex: number;
  topic: string;
  onTopicChange: (t: string) => void;
  onStart: () => void;
  onPauseToggle: () => void;
  onStop: () => void;
  onReset: () => void;
  participantCount: number;
  discussion: boolean;
  broadcast: string;
  onBroadcastChange: (t: string) => void;
  onBroadcastSend: () => void;
}

export function DebateControls(props: DebateControlsProps): React.ReactElement {
  const { phase, roundIndex } = props;
  const active = phase === 'running' || phase === 'paused';
  const finished = phase === 'done' || phase === 'discussion';

  const status =
    phase === 'running' && roundIndex >= 0
      ? `Round ${roundIndex + 1}/6 — ${DEBATE_ROUNDS[roundIndex].label}`
      : phase === 'paused'
        ? 'Paused — resumes between turns'
        : phase === 'discussion'
          ? 'Post-debate discussion'
          : phase === 'done'
            ? 'Debate complete'
            : 'Ready';

  return (
    <div className="gf-debate-controls">
      <div className="gf-row" style={{ gap: 8, flexWrap: 'wrap' }}>
        <input
          className="gf-input"
          style={{ flex: 1, minWidth: 200 }}
          placeholder="Debate topic…"
          value={props.topic}
          disabled={active}
          onChange={(e) => props.onTopicChange(e.target.value)}
        />
        <button
          className="gf-btn primary"
          disabled={active || props.participantCount < 1}
          onClick={props.onStart}
          title="Run the structured 6-round debate"
        >
          Start debate
        </button>
        <button className="gf-btn" disabled={!active} onClick={props.onPauseToggle} title="Pause between turns">
          {phase === 'paused' ? 'Resume' : 'Pause'}
        </button>
        <button className="gf-btn danger" disabled={!active} onClick={props.onStop} title="Stop the debate and declare a winner">
          Stop
        </button>
        <button className="gf-btn" disabled={active} onClick={props.onReset} title="Clear the room">
          New conference
        </button>
        <span className="gf-muted" style={{ fontSize: 12 }}>
          {status}
        </span>
      </div>
      <div className="gf-row" style={{ gap: 6, marginTop: 6, flexWrap: 'wrap' }}>
        {DEBATE_ROUNDS.map((r, i) => (
          <span
            key={r.id}
            className={`gf-round-chip${i === roundIndex && active ? ' active' : ''}${i < roundIndex || finished ? ' done' : ''}`}
            title={r.label}
          >
            {r.short}
          </span>
        ))}
      </div>
      {props.discussion && (
        <div className="gf-row" style={{ gap: 8, marginTop: 6 }}>
          <input
            className="gf-input"
            style={{ flex: 1 }}
            placeholder="Message all models…"
            value={props.broadcast}
            onChange={(e) => props.onBroadcastChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') props.onBroadcastSend();
            }}
          />
          <button className="gf-btn primary" onClick={props.onBroadcastSend} disabled={!props.broadcast.trim()}>
            Send to all
          </button>
          <button className="gf-btn" onClick={() => props.onBroadcastChange('')} disabled={!props.broadcast}>
            Clear
          </button>
        </div>
      )}
    </div>
  );
}
