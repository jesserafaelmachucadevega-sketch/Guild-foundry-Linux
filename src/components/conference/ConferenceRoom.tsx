// SPDX-License-Identifier: Apache-2.0
// Phase 3 — Conference Room: 1-4 model grid, participant configuration,
// structured 6-round debate engine (independent → cross-critique → rebuttal →
// evidence → final → judge/synthesis) with pause/stop protocol, winner
// selection, post-debate discussion mode, per-round voting, the knowledge
// bridge panel, and SDUI widget rendering inside model messages.

import React, { useCallback, useRef, useState } from 'react';
import { streamChat, type ChatMsg } from '../../lib/chat';
import { DesktopCapabilityRequired } from '../../lib/api';
import type { ChatMessage } from '../../lib/types';
import { ChatBubble } from '../ChatBubble';
import {
  ParticipantBar,
  type DebateRole,
  type ParticipantConfig,
} from './ParticipantBar';
import { DebateControls, DEBATE_ROUNDS, type DebatePhase } from './DebateControls';
import { WinnerDialog } from './WinnerDialog';
import { SynthesisPanel } from './SynthesisPanel';
import { SduiRenderer, extractSduiWidgets, stripSdui } from './SduiRenderer';
import './conference.css';

// Every system prompt carries the human-speech rule (SDUI widgets are the one
// sanctioned exception to the no-code-blocks rule).
const HUMAN_SPEECH_RULE =
  'Speak in clear, direct, natural human language. No code blocks unless discussing code or emitting an ```sdui interactive widget.';

// Vote weights by debate role when weighted voting is on.
const ROLE_WEIGHTS: Record<DebateRole, number> = {
  proposer: 1,
  critic: 2,
  judge: 3,
  observer: 1,
};

const VOTE_RE = /VOTE:\s*(\d{1,2})/;
const STOPPED_BY_USER = 'stopped-by-user';

type PaneStatus = 'idle' | 'streaming' | 'waiting' | 'error';

interface PaneState {
  messages: ChatMessage[];
  input: string;
  status: PaneStatus;
  streamText: string;
}

interface VoteState {
  /** roundIndex -> participant slot -> voted model number (1..N) */
  rounds: Record<number, Record<number, number>>;
  /** roundIndex -> user vote (model number) or null */
  user: Record<n
umber, number | null>;
}

function blankPane(): PaneState {
  return { messages: [], input: '', status: 'idle', streamText: '' };
}

function blankParticipant(slot: number): ParticipantConfig {
  return {
    slot,
    providerId: '',
    modelId: '',
    modelName: '',
    systemPrompt: '',
    role: 'proposer',
    temperature: 0.7,
    reasoningEffort: 'medium',
    tools: { web: false, filesystem: false, shell: false, mcp: false },
    memoryScope: 'conversation',
  };
}

function makeMsg(
  role: ChatMessage['role'],
  content: string,
  slot: number,
  parts: ParticipantConfig[],
): ChatMessage {
  return {
    id: `m-${Date.now()}-${slot}-${Math.floor(Math.random() * 1e6)}`,
    role,
    content,
    modelId: parts[slot]?.modelId || undefined,
    modelName: parts[slot]?.modelName || `Model ${slot + 1}`,
    createdAt: Date.now(),
  };
}

function buildSystemPrompt(p: ParticipantConfig): string {
  const enabled = (Object.keys(p.tools) as Array<keyof typeof p.tools>).filter(
    (k) => p.tools[k],
  );
  return [
    p.systemPrompt.trim(),
    `Your debate role is "${p.role}".`,
    `Enabled tools: ${enabled.length ? enabled.join(', ') : 'none'}. Memory scope: ${p.memoryScope}.`,
    HUMAN_SPEECH_RULE,
  ]
    .filter(Boolean)
    .join('\n\n');
}

/** Assistant message view: ChatBubble for text plus interactive SDUI widgets. */
function MessageView({
  message,
  onToast,
}: {
  message: ChatMessage;
  onToast: (t: string) => void;
}): React.ReactElement {
  if (message.role === 'assistant' && extractSduiWidgets(message.content).length > 0) {
    const textOnly = stripSdui(message.content);
    return (
      <>
        {textOnly && (
          <ChatBubble message={{ ...message, content: textOnly }} onToast={onToast} />
        )}
        <div className="gf-bubble">
          <div className="gf-bubble-body">
            <SduiRenderer text={message.content} widgetsOnly onToast={onToast} />
          </div>
        </div>
      </>
    );
  }
  return <ChatBu
bble message={message} onToast={onToast} />;
}

export function ConferenceRoom({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [participants, setParticipants] = useState<ParticipantConfig[]>([
    blankParticipant(0),
    blankParticipant(1),
  ]);
  const [panes, setPanes] = useState<PaneState[]>([blankPane(), blankPane()]);
  const [phase, setPhase] = useState<DebatePhase>('idle');
  const [roundIndex, setRoundIndex] = useState(-1);
  const [topic, setTopic] = useState('');
  const [votes, setVotes] = useState<VoteState>({ rounds: {}, user: {} });
  const [weighted, setWeighted] = useState(true);
  const [voteRound, setVoteRound] = useState(0);
  const [winnerOpen, setWinnerOpen] = useState(false);
  const [winner, setWinner] = useState<number | null>(null);
  const [broadcast, setBroadcast] = useState('');
  const [claimTexts, setClaimTexts] = useState<string[]>([]);

  const ctrl = useRef({ paused: false, stopped: false });
  const cancels = useRef<Array<(() => void) | null>>([]);
  const stopRejects = useRef<Array<(() => void) | null>>([]);
  const roundTexts = useRef<Record<number, string[]>>({});
  const participantsRef = useRef(participants);
  participantsRef.current = participants;
  const topicRef = useRef(topic);
  topicRef.current = topic;

  const updatePane = useCallback((slot: number, patch: Partial<PaneState>) => {
    setPanes((prev) => prev.map((p, i) => (i === slot ? { ...p, ...patch } : p)));
  }, []);

  const appendMessage = useCallback((slot: number, msg: ChatMessage) => {
    setPanes((prev) =>
      prev.map((p, i) =>
        i === slot ? { ...p, messages: [...p.messages, msg], streamText: '' } : p,
      ),
    );
  }, []);

  const patchParticipant = useCallback((slot: number, patch: Partial<ParticipantConfig>) => {
    setParticipants((prev) => prev.map((p, i) => (i === slot ? { ...p, ...patch } : p)));
  }, []);

  const addParticipant = useCallback(() => {
    setParticipants((prev) => {
      if (prev.leng
th >= 4) {
        onToast('Maximum 4 models in the Conference Room');
        return prev;
      }
      return [...prev, blankParticipant(prev.length)];
    });
    setPanes((prev) => (prev.length >= 4 ? prev : [...prev, blankPane()]));
  }, [onToast]);

  const removeParticipant = useCallback((slot: number) => {
    setParticipants((prev) => {
      if (prev.length <= 1) return prev;
      return prev
        .filter((_, i) => i !== slot)
        .map((p, i) => ({ ...p, slot: i }));
    });
    setPanes((prev) => prev.filter((_, i) => i !== slot));
  }, []);

  /** Stream one model turn; resolves with the full text. */
  const speak = useCallback(
    (slot: number, history: ChatMsg[], systemPrompt: string, p: ParticipantConfig): Promise<string> => {
      return new Promise<string>((resolve, reject) => {
        let full = '';
        let settled = false;
        const finish = (fn: () => void): void => {
          if (settled) return;
          settled = true;
          stopRejects.current[slot] = null;
          fn();
        };
        stopRejects.current[slot] = () => finish(() => reject(new Error(STOPPED_BY_USER)));
        updatePane(slot, { status: 'streaming', streamText: '' });
        void streamChat(
          {
            providerId: p.providerId,
            modelId: p.modelId,
            messages: history,
            params: {
              systemPrompt,
              temperature: p.temperature,
              reasoningEffort: p.reasoningEffort,
            },
          },
          {
            onDelta: (d) => {
              full += d;
              if (!ctrl.current.stopped) updatePane(slot, { streamText: full });
            },
            onDone: () => {
              updatePane(slot, { status: 'idle', streamText: '' });
              finish(() => resolve(full));
            },
            onError: (e) => {
              updatePane(slot, { status: 'error', streamText: '' });
              finish(() => reject(e));
            },
          },

        )
          .then((cancel) => {
            cancels.current[slot] = cancel;
            if (ctrl.current.stopped) cancel();
          })
          .catch((e: unknown) => {
            finish(() => reject(e instanceof Error ? e : new Error(String(e))));
          });
      });
    },
    [updatePane],
  );

  const waitIfPaused = async (): Promise<void> => {
    while (ctrl.current.paused && !ctrl.current.stopped) {
      await new Promise((res) => setTimeout(res, 150));
    }
  };

  const recordModelVote = useCallback((r: number, slot: number, text: string) => {
    const m = VOTE_RE.exec(text);
    if (!m) return;
    const v = parseInt(m[1], 10);
    const n = participantsRef.current.length;
    if (v < 1 || v > n) return;
    setVotes((prev) => ({
      ...prev,
      rounds: { ...prev.rounds, [r]: { ...(prev.rounds[r] ?? {}), [slot]: v } },
    }));
  }, []);

  function roundPrompt(
    roundId: string,
    slot: number,
    parts: ParticipantConfig[],
  ): string {
    const n = parts.length;
    const topicText = topicRef.current.trim();
    const name = (i: number): string => parts[i].modelName || `Model ${i + 1}`;
    const prior = roundTexts.current;
    switch (roundId) {
      case 'independent':
        return (
          `Debate topic:\n${topicText}\n\n` +
          'Give your independent, well-reasoned answer to the topic. ' +
          'Do not reference other participants; they have not spoken yet.'
        );
      case 'cross-critique': {
        const others = (prior[0] ?? [])
          .map((t, i) =>
            i === slot ? null : `--- Model ${i + 1} (${name(i)}) ---\n${t ?? '(no answer recorded)'}`,
          )
          .filter(Boolean)
          .join('\n\n');
        return (
          `Debate topic:\n${topicText}\n\n` +
          `The other participants gave these independent answers:\n\n${others}\n\n` +
          'Critique each of the other answers: identify strengths, weaknesses, and ' +
          'factual or logical problems. B
e direct and specific.'
        );
      }
      case 'rebuttal': {
        const critiques = (prior[1] ?? [])
          .map((t, i) =>
            i === slot ? null : `--- Model ${i + 1} (${name(i)}) critique ---\n${t ?? '(none recorded)'}`,
          )
          .filter(Boolean)
          .join('\n\n');
        return (
          `Debate topic:\n${topicText}\n\n` +
          `Other participants critiqued your answer:\n\n${critiques || '(no critiques recorded)'}\n\n` +
          'Rebut the critiques of your position. Defend what holds, concede what ' +
          'does not, and sharpen your argument.'
        );
      }
      case 'evidence':
        return (
          `Debate topic:\n${topicText}\n\n` +
          'Provide concrete supporting evidence for your position: facts, examples, ' +
          'or data you are confident about. Mark anything you are unsure of as ' +
          'uncertain rather than inventing it.'
        );
      case 'final':
        return (
          `Debate topic:\n${topicText}\n\n` +
          'State your final position concisely, incorporating what survived ' +
          'critique and rebuttal.\n\n' +
          'On its own line at the very end, cast your vote in exactly this form:\n' +
          `VOTE: <n>\nwhere <n> is the model number (1-${n}) whose final position is ` +
          'strongest. You may vote for yourself.'
        );
      case 'judge': {
        const finals = (prior[4] ?? [])
          .map((t, i) => `--- Model ${i + 1} (${name(i)}) final position ---\n${t ?? '(none recorded)'}`)
          .join('\n\n');
        return (
          `Debate topic:\n${topicText}\n\n` +
          'You are the judge for this debate. Here are the final positions:\n\n' +
          `${finals}\n\n` +
          'Evaluate them on reasoning quality, evidence, and responsiveness to ' +
          'critique. Declare which model argued best and why, then write a balanced ' +
          'synthesis of the strongest points from all sides. End with your own
 ' +
          'vote on its own line:\nVOTE: <n>'
        );
      }
      default:
        return topicText;
    }
  }

  async function runRound(r: number, parts: ParticipantConfig[]): Promise<void> {
    const roundId = DEBATE_ROUNDS[r].id;
    const speakers =
      roundId === 'judge'
        ? (() => {
            const judges = parts
              .map((p, i) => (p.role === 'judge' ? i : -1))
              .filter((i) => i >= 0);
            return judges.length > 0 ? judges : [0];
          })()
        : parts.map((_, i) => i);
    const texts: string[] = [];
    parts.forEach((_, i) => {
      if (speakers.includes(i)) updatePane(i, { status: 'waiting' });
    });
    // The independent round has no inter-speaker dependency (no one has spoken
    // yet), so all panes stream simultaneously. All later rounds depend on
    // earlier speakers' answers and stay sequential.
    const independent = roundId === 'independent';
    if (independent) {
      await Promise.all(
        speakers.map(async (s) => {
          if (ctrl.current.stopped) return;
          await waitIfPaused();
          if (ctrl.current.stopped) return;
          const p = parts[s];
          const history: ChatMsg[] = [{ role: 'user', content: roundPrompt(roundId, s, parts) }];
          const full = await speak(s, history, buildSystemPrompt(p), p);
          texts[s] = full;
          appendMessage(s, makeMsg('assistant', full, s, parts));
          recordModelVote(r, s, full);
        }),
      );
    } else {
      for (const s of speakers) {
        if (ctrl.current.stopped) return;
        await waitIfPaused();
        if (ctrl.current.stopped) return;
        const p = parts[s];
        const history: ChatMsg[] = [{ role: 'user', content: roundPrompt(roundId, s, parts) }];
        const full = await speak(s, history, buildSystemPrompt(p), p);
        texts[s] = full;
        appendMessage(s, makeMsg('assistant', full, s, parts));
        recordModelVote(r, s, full);
      }
    }
    roundTexts.current[r] = texts;
    parts.forEach((_, i) => updatePane(i, { status: 'idle' }));
  }

  async function runDebate(): Promise<void> {
    const parts = participantsRef.current;
    const n = parts.length;
    if (n < 2) {
      onToast('Add at least 2 models to start a debate');
      return;
    }
    if (!topicRef.current.trim()) {
      onToast('Enter a debate topic first');
      return;
    }
    if (parts.some((p) => !p.providerId || !p.modelId)) {
      onToast('Choose a provider and model for every participant first');
      return;
    }
    ctrl.current = { paused: false, stopped: false };
    roundTexts.current = {};
    setVotes({ rounds: {}, user: {} });
    setVoteRound(0);
    setWinner(null);
    setClaimTexts([]);
    setPhase('running');
    setRoundIndex(-1);
    try {
      for (let r = 0; r < DEBATE_ROUNDS.length;
 r++) {
        if (ctrl.current.stopped) break;
        await waitIfPaused();
        if (ctrl.current.stopped) break;
        setRoundIndex(r);
        await runRound(r, parts);
      }
    } catch (err) {
      if (!(err instanceof Error && err.message === STOPPED_BY_USER)) {
        onToast(
          err instanceof DesktopCapabilityRequired
            ? 'Desktop Capability Required'
            : `Debate ended early: ${err instanceof Error ? err.message : String(err)}`,
        );
      }
    } finally {
      parts.forEach((_, i) => updatePane(i, { status: 'idle', streamText: '' }));
      if (!ctrl.current.stopped) setPhase('done');
      const finals =
        roundTexts.current[4] ??
        roundTexts.current[DEBATE_ROUNDS.length - 1] ??
        [];
      setClaimTexts(finals.filter(Boolean));
      setWinnerOpen(true);
    }
  }

  function onPauseToggle(): void {
    if (phase === 'running') {
      ctrl.current.paused = true;
      setPhase('paused');
      onToast('Debate paused — resumes between turns');
    } else if (phase === 'paused') {
      ctrl.current.paused = false;
      setPhase('running');
    }
  }

  function onStop(): void {
    if (phase !== 'running' && phase !== 'paused') return;
    ctrl.current.stopped = true;
    ctrl.current.paused = false;
    cancels.current.forEach((c) => {
      try {
        c?.();
      } catch {
        /* cancel is best-effort */
      }
    });
    stopRejects.current.forEach((rej) => {
      try {
        rej?.();
      } catch {
        /* already settled */
      }
    });
    setPhase('idle');
    onToast('Debate stopped');
    // runDebate's finally block opens the winner dialog.
  }

  function resetRoom(): void {
    if (phase === 'running' || phase === 'paused') {
      onToast('Stop the debate before starting a new conference');
      return;
    }
    setPanes(participants.map(() => blankPane()));
    setPhase('idle');
    setRoundIndex(-1);
    setVotes({ rounds: {}, user: {} });
    setVoteR
ound(0);
    setWinner(null);
    setTopic('');
    setBroadcast('');
    setClaimTexts([]);
  }

  function declareWinner(modelNumber: number): void {
    setWinnerOpen(false);
    setWinner(modelNumber);
    const parts = participantsRef.current;
    const name = parts[modelNumber - 1]?.modelName || `Model ${modelNumber}`;
    const card: ChatMessage = {
      id: `w-${Date.now()}`,
      role: 'winner',
      content: `🎉 Winner: Model ${modelNumber} — ${name}!`,
      modelName: '🎉 Winner',
      createdAt: Date.now(),
    };
    setPanes((prev) => prev.map((p) => ({ ...p, messages: [...p.messages, card] })));
    setPhase('discussion');
    setRoundIndex(-1);
    onToast(`Winner declared: Model ${modelNumber} — ${name}`);
  }

  function skipWinner(): void {
    setWinnerOpen(false);
    setPhase('discussion');
    setRoundIndex(-1);
  }

  async function sendPane(slot: number): Promise<void> {
    const parts = participantsRef.current;
    const p = parts[slot];
    const text = panes[slot]?.input.trim() ?? '';
    if (!text || !p) return;
    if (!p.providerId || !p.modelId) {
      onToast('Choose a provider and model for this participant first');
      return;
    }
    if (phase === 'running' || phase === 'paused') {
      onToast('Debate in progress — pause or stop it to chat freely');
      return;
    }
    updatePane(slot, { input: '' });
    appendMessage(slot, makeMsg('user', text, slot, parts));
    const history: ChatMsg[] = [
      ...(panes[slot]?.messages ?? []).map((m) => ({
        role: (m.role === 'winner' ? 'assistant' : m.role) as 'user' | 'assistant' | 'system',
        content: m.content,
      })),
      { role: 'user' as const, content: text },
    ];
    try {
      const full = await speak(slot, history, buildSystemPrompt(p), p);
      appendMessage(slot, makeMsg('assistant', full, slot, parts));
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : `S
end failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }

  async function sendBroadcast(): Promise<void> {
    const text = broadcast.trim();
    if (!text) return;
    setBroadcast('');
    const parts = participantsRef.current;
    for (let s = 0; s < parts.length; s++) {
      const p = parts[s];
      if (!p.providerId || !p.modelId) continue;
      appendMessage(s, makeMsg('user', text, s, parts));
      const history: ChatMsg[] = [
        ...(panes[s]?.messages ?? []).map((m) => ({
          role: (m.role === 'winner' ? 'assistant' : m.role) as 'user' | 'assistant' | 'system',
          content: m.content,
        })),
        { role: 'user' as const, content: text },
      ];
      try {
        const full = await speak(s, history, buildSystemPrompt(p), p);
        appendMessage(s, makeMsg('assistant', full, s, parts));
      } catch (err) {
        onToast(
          err instanceof DesktopCapabilityRequired
            ? 'Desktop Capability Required'
            : `Broadcast failed: ${err instanceof Error ? err.message : String(err)}`,
        );
        break;
      }
    }
  }

  // ---- voting ----
  function setUserVote(r: number, modelNumber: number): void {
    setVotes((prev) => ({
      ...prev,
      user: { ...prev.user, [r]: prev.user[r] === modelNumber ? null : modelNumber },
    }));
  }

  const voteRounds = Array.from(
    new Set([
      ...Object.keys(votes.rounds).map(Number),
      ...Object.keys(votes.user).map(Number),
      ...(roundIndex >= 0 ? [roundIndex] : []),
    ]),
  ).sort((a, b) => a - b);
  const effectiveVoteRound = voteRounds.includes(voteRound)
    ? voteRound
    : (voteRounds[voteRounds.length - 1] ?? 0);

  const tally = (() => {
    const r = effectiveVoteRound;
    const modelVotes = votes.rounds[r] ?? {};
    const rows: Array<{ label: string; vote: number; weight: number }> = [];
    participants.forEach((p, s) => {
      const v = modelVotes[s];
      if (v) {
        rows.push({
     
     label: `Model ${s + 1}${p.role === 'judge' ? ' (judge)' : ''}`,
          vote: v,
          weight: weighted ? ROLE_WEIGHTS[p.role] : 1,
        });
      }
    });
    const uv = votes.user[r];
    if (uv) rows.push({ label: 'You', vote: uv, weight: 1 });
    const totals: Record<number, number> = {};
    rows.forEach((row) => {
      totals[row.vote] = (totals[row.vote] ?? 0) + row.weight;
    });
    const totalWeight = rows.reduce((a, row) => a + row.weight, 0);
    let leader = 0;
    let leaderWeight = 0;
    Object.entries(totals).forEach(([k, w]) => {
      if (w > leaderWeight) {
        leaderWeight = w;
        leader = Number(k);
      }
    });
    return {
      rows,
      leader,
      consensus: totalWeight > 0 ? Math.round((leaderWeight / totalWeight) * 100) : 0,
    };
  })();

  const judgeSlot = participants.findIndex((p) => p.role === 'judge');
  const judgeVote =
    judgeSlot >= 0 ? votes.rounds[effectiveVoteRound]?.[judgeSlot] : undefined;

  const debateLocked = phase === 'running' || phase === 'paused';

  return (
    <div className="gf-conf">
      <DebateControls
        phase={phase}
        roundIndex={roundIndex}
        topic={topic}
        onTopicChange={setTopic}
        onStart={() => void runDebate()}
        onPauseToggle={onPauseToggle}
        onStop={onStop}
        onReset={resetRoom}
        participantCount={participants.length}
        discussion={phase === 'discussion'}
        broadcast={broadcast}
        onBroadcastChange={setBroadcast}
        onBroadcastSend={() => void sendBroadcast()}
      />
      <ParticipantBar
        participants={participants}
        onChange={patchParticipant}
        onAdd={addParticipant}
        onRemove={removeParticipant}
        onToast={onToast}
        disabled={debateLocked}
      />
      <div className={`gf-model-grid cols-${participants.length} gf-conf-grid`}>
        {participants.map((p, s) => {
          const pane = panes[s] ?? blankPane();
          return (
     
       <div className="gf-pane" key={s}>
              <div className="gf-pane-header">
                <span className="gf-row" style={{ gap: 6 }}>
                  <span
                    className={`gf-status-dot ${pane.status}`}
                    title={`Status: ${pane.status}`}
                  />
                  <span className="gf-pane-title">{p.modelName || `Model ${s + 1}`}</span>
                </span>
                <span className="gf-row" style={{ gap: 6 }}>
                  {winner === s + 1 && <span className="gf-role-badge gf-winner-badge">winner</span>}
                  <span className="gf-role-badge">{p.role}</span>
                </span>
              </div>
              <div className="gf-pane-msgs">
                {pane.messages.map((m) => (
                  <MessageView key={m.id} message={m} onToast={onToast} />
                ))}
                {pane.streamText && (
                  <div className="gf-bubble gf-streaming">
                    <div className="gf-bubble-body">
                      <span style={{ whiteSpace: 'pre-wrap' }}>{pane.streamText}</span>
                      <span className="gf-cursor">▌</span>
                    </div>
                  </div>
                )}
                {pane.status === 'waiting' && !pane.streamText && (
                  <div className="gf-muted" style={{ fontSize: 12 }}>
                    Waiting to speak…
                  </div>
                )}
              </div>
              <div className="gf-pane-input">
                <input
                  className="gf-input"
                  style={{ flex: 1 }}
                  placeholder={
                    debateLocked ? 'Debate in progress…' : `Message ${p.modelName || `Model ${s + 1}`}…`
                  }
                  value={pane.input}
                  disabled={debateLocked}
                  onChange={(e) => updatePane(s, { input: e.target.value })}
                  onKeyDown={(e) => {
           
         if (e.key === 'Enter') void sendPane(s);
                  }}
                />
                <button
                  className="gf-btn primary"
                  disabled={debateLocked}
                  onClick={() => void sendPane(s)}
                >
                  Send
                </button>
              </div>
            </div>
          );
        })}
      </div>
      <div className="gf-conf-bottom">
        <div className="gf-vote-panel">
          <div className="gf-row" style={{ justifyContent: 'space-between' }}>
            <span className="gf-label">Voting</span>
            <label className="gf-row" style={{ gap: 4, fontSize: 12 }}>
              <input
                type="checkbox"
                checked={weighted}
                onChange={(e) => setWeighted(e.target.checked)}
              />
              Weighted
            </label>
          </div>
          {voteRounds.length === 0 ? (
            <span className="gf-muted" style={{ fontSize: 12 }}>
              No votes yet. Models vote with <code>VOTE: &lt;n&gt;</code> in their
              final positions; you can vote below per round.
            </span>
          ) : (
            <>
              <div className="gf-row" style={{ gap: 6, marginBottom: 6, flexWrap: 'wrap' }}>
                <span className="gf-muted" style={{ fontSize: 12 }}>
                  Round:
                </span>
                {voteRounds.map((r) => (
                  <button
                    key={r}
                    className={`gf-btn${r === effectiveVoteRound ? ' primary' : ''}`}
                    style={{ padding: '2px 8px' }}
                    onClick={() => setVoteRound(r)}
                  >
                    R{r + 1}
                  </button>
                ))}
              </div>
              <div className="gf-row" style={{ gap: 6, flexWrap: 'wrap' }}>
                <span className="gf-muted" style={{ fontSize: 12 }}>
                  Your vote:
    
            </span>
                {participants.map((_, i) => (
                  <button
                    key={i}
                    className={`gf-btn${votes.user[effectiveVoteRound] === i + 1 ? ' primary' : ''}`}
                    style={{ padding: '2px 8px' }}
                    onClick={() => setUserVote(effectiveVoteRound, i + 1)}
                    title={`Vote for Model ${i + 1}`}
                  >
                    {i + 1}
                  </button>
                ))}
              </div>
              {tally.rows.length > 0 && (
                <div style={{ marginTop: 6, fontSize: 12 }}>
                  {tally.rows.map((row, i) => (
                    <div key={i} className="gf-row" style={{ justifyContent: 'space-between' }}>
                      <span>
                        {row.label} → Model {row.vote}
                      </span>
                      <span className="gf-muted">weight {row.weight}</span>
                    </div>
                  ))}
                  <div className="gf-row" style={{ marginTop: 6, gap: 8 }}>
                    <span>Consensus</span>
                    <div className="gf-meter" style={{ flex: 1 }}>
                      <div style={{ width: `${tally.consensus}%` }} />
                    </div>
                    <span>
                      {tally.consensus}%
                      {tally.leader > 0 ? ` · Model ${tally.leader} leads` : ''}
                    </span>
                  </div>
                  {judgeVote !== undefined && (
                    <div className="gf-muted" style={{ marginTop: 4 }}>
                      Judge vote: Model {judgeVote}
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>
        <SynthesisPanel topic={topic} claims={claimTexts} onToast={onToast} />
      </div>
      <WinnerDialog
        open={winnerOpen}
        count={participants.length}
        onSubmit={declareWinne
r}
        onCancel={skipWinner}
      />
    </div>
  );
}
