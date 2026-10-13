// SPDX-License-Identifier: Apache-2.0
// Personal Agent Harness — the daily companion tab (chat + tasks).
//
// Interactive driver for the Phase 6 tool loop (`runToolLoop`): one user
// message starts/resumes the loop. Tool calls dispatch in the background via
// `tool_execute` and stream back as events; media results
// (`media.generate_image` / `media.generate_video`) render inline in the
// thread per the media rendering contract (local `output.path` via
// `convertFileSrc`, remote `output.url` as fallback). `artifact.create`
// results append to the message's artifacts and render via ChatBubble.
//
// Approval resume contract: `tool_execute` with an Ask permission returns
// `needs_approval` WITHOUT executing. On Approve the harness flips that
// tool's domain to `always_allow`, re-executes the exact same call, then
// restores the previous level (approve-once), and resumes the loop with the
// result in context. Deny resumes with the denial in context. `user.ask`
// (`needs_user`) resumes with the user's answer. The backend loop is
// stateless per invocation, so resume carries the last assistant text +
// the new result as the next user message — the visible thread keeps the
// full history.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import {
  runToolLoop,
  type ToolCall,
  type ToolExecResult,
  type ToolLoopEvent,
} from '../../lib/toolLoop';
import {
  getProviders,
  listModels,
  type ModelInfo,
  type ProviderSummary,
} from '../../lib/providers';
import { generateSuggestions } from '../../lib/suggestions';
import type { ChatMsg } from '../../lib/chat';
import type { Artifact, ArtifactKind, ChatMessage } from '../../lib/types';
import type { NavSection } from '../AppShell';
import { ChatBubble } from '../ChatBubble';
import { SuggestedList } from '../SuggestedList';
import { AvatarStatus, type AvatarActivity, type AvatarGender } from '../avatar/AvatarStatus';
import { VoiceControls } from '../voice/VoiceControls';
import { getSpeechEnabled, speakText } from '../voice/speech';
import { MemoryTabs } from '../memory/MemoryTabs';
import './agent.css';

interface Props {
  onToast: (t: string) => void;
  onNavigate: (s: NavSection) => void;
}

const DEFAULT_AGENT_PROMPT = `You are the user's personal AI companion inside Guild Foundry. The user's name is Jesse.
Help with daily work: email triage, calendar and reminders, web research, creditor outreach scripts,
files, projects, and media creation. Be warm, direct, and concise.

You have tools. To call one, emit a fenced block exactly like this and nothing else on those lines:

\`\`\`tool_call
{"tool": "tool.name", "args": {...}}
\`\`\`

One call per block; you may emit several blocks in one turn. After each block you receive
TOOL RESULT and continue. Never invent tool names or file paths — only call tools from your
capability manifest. Reads run silently. Anything that sends, deletes, publishes, spends
money, or changes credentials needs the user's approval: propose the action and wait.
When asked for an image or video, call media.generate_image / media.generate_video in the
background and confirm briefly once the result is rendered — do not paste raw JSON.`;

interface MediaAtt {
  kind: 'image' | 'video';
  path: string;
  url?: string;
  model?: string;
}

interface ThreadMsg {
  id: string;
  role: 'user' | 'assistant' | 'system';
  /** Display text (tool_call fences stripped for assistant turns). */
  text: string;
  /** Full raw assistant text (kept for transcripts / resume context). */
  full?: string;
  media: MediaAtt[];
  artifacts: Artifact[];
  modelName?: string;
  ts: number;
  /** Replies to this message id — the quote is appended to the new user input. */
  replyTo?: string;
}

interface TraceEntry {
  id: string;
  kind: 'info' | 'tool_call' | 'tool_result' | 'gate';
  label: string;
  detail?: string;
  ts: number;
}

interface PendingGate {
  call: ToolCall;
  prevText: string;
}

interface PendingQuestion {
  call: ToolCall;
  question: string;
  options: string[];
  prevText: string;
  answer: string;
}

const ARTIFACT_KINDS = new Set<string>(['poll', 'checklist', 'slider', 'card', 'whiteboard', 'sticky']);

const QUICK_SEND: { label: string; prompt: string }[] = [
  {
    label: 'Check Gmail',
    prompt:
      'Check my Gmail for new unread mail from the last 24 hours and brief me on anything urgent. Use your Gmail tools; read-only unless I ask otherwise.',
  },
  {
    label: 'Reminders',
    prompt:
      'What reminders, scheduled tasks, and upcoming calendar items do I have? Check my scheduled tasks and brief me on what is next.',
  },
];

const QUICK_INSERT: { label: string; starter: string }[] = [
  { label: 'Creditor script', starter: 'Draft a firm but polite creditor negotiation script. The creditor is ' },
  { label: 'Research', starter: 'Research the latest on ' },
  { label: 'Image', starter: 'Generate an image of ' },
  { label: 'Video', starter: 'Generate a short video of ' },
];

function newId(prefix: string): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return `${prefix}-${crypto.randomUUID()}`;
  return `${prefix}-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
}

function stripToolFences(text: string): string {
  return text.replace(/```tool_call\s*\n[\s\S]*?```/g, '').trim();
}

function extractMedia(tool: string, output: unknown): MediaAtt | null {
  if (tool !== 'media.generate_image' && tool !== 'media.generate_video') return null;
  if (!output || typeof output !== 'object') return null;
  const o = output as Record<string, unknown>;
  if (typeof o.path !== 'string' || o.path.length === 0) return null;
  return {
    kind: tool === 'media.generate_image' ? 'image' : 'video',
    path: o.path,
    url: typeof o.url === 'string' ? o.url : undefined,
    model: typeof o.model === 'string' ? o.model : undefined,
  };
}

function extractArtifact(output: unknown): Artifact | null {
  if (!output || typeof output !== 'object') return null;
  const a = (output as Record<string, unknown>).artifact;
  if (!a || typeof a !== 'object') return null;
  const r = a as Record<string, unknown>;
  if (typeof r.kind !== 'string' || !ARTIFACT_KINDS.has(r.kind)) return null;
  if (typeof r.title !== 'string') return null;
  return { ...(r as object), kind: r.kind as ArtifactKind } as Artifact;
}

function MediaView({ media }: { media: MediaAtt }): React.ReactElement {
  const src = isDesktop() ? convertFileSrc(media.path) : (media.url ?? media.path);
  const [loaded, setLoaded] = useState(true);
  const isGenerating = !media.path && !media.url;

  if (isGenerating) {
    return (
      <figure className="agent-media agent-media-loading">
        <div style={{ background: 'var(--gf-dark-grey)', borderRadius: 4, minHeight: 160, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <div className="gf-muted">Generating…</div>
        </div>
        <figcaption className="gf-muted">
          {media.model ?? 'media'} · generating
        </figcaption>
      </figure>
    );
  }

  return (
    <figure className="agent-media">
      {!loaded && <div className="agent-media-skeleton">Loading…</div>}
      {media.kind === 'image' ? (
        // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
        <img src={src} alt={`Generated by ${media.model ?? 'agent'}`} loading="lazy" onLoad={() => setLoaded(true)} style={{ display: loaded ? 'block' : 'none' }} />
      ) : (
        <video src={src} controls preload="metadata" />
      )}
      <figcaption className="gf-muted">
        {media.model ?? 'media'} ·{' '}
        {media.url ? (
          <a href={media.url} target="_blank" rel="noreferrer">
            remote copy
          </a>
        ) : (
          'local file'
        )}
      </figcaption>
    </figure>
  );
}

export function AgentHarness({ onToast, onNavigate }: Props): React.ReactElement {
  const [tab, setTab] = useState<'chat' | 'memory'>('chat');
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [providerId, setProviderId] = useState('');
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelId, setModelId] = useState('');
  const [thread, setThread] = useState<ThreadMsg[]>([]);
  const [trace, setTrace] = useState<TraceEntry[]>([]);
  const [showTrace, setShowTrace] = useState(false);
  const [input, setInput] = useState('');
  const [running, setRunning] = useState(false);
  const [toolActive, setToolActive] = useState(false);
  const [pendingApproval, setPendingApproval] = useState<PendingGate | null>(null);
  const [pendingQuestion, setPendingQuestion] = useState<PendingQuestion | null>(null);
  const [toolCount, setToolCount] = useState(0);
  const [sessionId, setSessionId] = useState(() => newId('agent'));
  const [suggestToken, setSuggestToken] = useState(0);
  const [gender, setGender] = useState<AvatarGender>('girl');
  const [toolDomains, setToolDomains] = useState<Record<string, string>>({});
  const [agentPrompt, setAgentPrompt] = useState(DEFAULT_AGENT_PROMPT);
  const [showAgentSettings, setShowAgentSettings] = useState(false);
  const [replyTo, setReplyTo] = useState<ThreadMsg | null>(null);
  const [reactions, setReactions] = useState<Record<string, Record<string, number>>>({});
  const [showDocUpload, setShowDocUpload] = useState(false);
  const [showPreview, setShowPreview] = useState(false);
  const [docPreview, setDocPreview] = useState<{ name: string; text: string } | null>(null);

  const cancelRef = useRef<(() => void) | null>(null);
  const asstIdRef = useRef<string | null>(null);
  const asstFullRef = useRef('');
  const callArgsRef = useRef(new Map<string, ToolCall>());
  const threadEndRef = useRef<HTMLDivElement | null>(null);
  const desktop = isDesktop();

  const pushTrace = useCallback((kind: TraceEntry['kind'], label: string, detail?: string) => {
    setTrace((prev) => [...prev.slice(-199), { id: newId('tr'), kind, label, detail, ts: Date.now() }]);
  }, []);

  const loadModels = useCallback(
    async (pid: string) => {
      try {
        const list = await listModels(pid);
        setModels(list ?? []);
        setModelId((prev) => (prev && (list ?? []).some((m) => m.model_id === prev) ? prev : (list[0]?.model_id ?? '')));
      } catch {
        setModels([]);
        setModelId('');
      }
    },
    [],
  );

  const reloadProviders = useCallback(async () => {
    try {
      const list = await getProviders();
      setProviders(list ?? []);
      setProviderId((prev) => {
        if (prev && (list ?? []).some((p) => p.id === prev)) return prev;
        const connected = (list ?? []).find((p) => p.status === 'connected');
        return connected?.id ?? list[0]?.id ?? '';
      });
    } catch {
      setProviders([]);
    }
  }, []);

  useEffect(() => {
    if (!desktop) return;
    void reloadProviders();
    void invoke<Array<{ name: string; domain: string }>>('tool_list')
      .then((defs) => {
        const map: Record<string, string> = {};
        for (const d of defs ?? []) if (d?.name) map[d.name] = d.domain || 'shell';
        setToolDomains(map);
      })
      .catch(() => undefined);
     void invoke<string | null>('settings_get', { key: 'avatar.gender' })
       .then((g) => {
         if (g === 'girl' || g === 'boy') setGender(g);
       })
       .catch(() => undefined);
     void invoke<string | null>('settings_load_prompts')
       .then((prompts) => {
         const p = (prompts ?? {}) as Record<string, string>;
         if (p['agent-harness'] && p['agent-harness'].trim()) {
           setAgentPrompt(p['agent-harness']);
         }
       })
       .catch(() => undefined);
   }, [desktop, reloadProviders]);

  useEffect(() => {
    if (providerId) void loadModels(providerId);
  }, [providerId, loadModels]);

  useEffect(() => {
    threadEndRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [thread.length]);

  const toChatMessage = (m: ThreadMsg): ChatMessage => ({
    id: m.id,
    role: m.role,
    content: m.text,
    modelName: m.modelName,
    createdAt: m.ts,
    artifacts: m.artifacts.length > 0 ? m.artifacts : undefined,
  });

  const transcript = useCallback((): ChatMsg[] => {
    const out: ChatMsg[] = [];
    for (const m of thread) {
      if (m.role === 'user') out.push({ role: 'user', content: m.text });
      else if (m.role === 'assistant') out.push({ role: 'assistant', content: m.full ?? m.text });
    }
    return out.slice(-12);
  }, [thread]);

  const finishTurn = useCallback(
    (status: string, detail?: string) => {
      setRunning(false);
      setToolActive(false);
      pushTrace('info', `Turn ${status}`, detail);
      if (status === 'done') {
        const t = transcript();
        const pid = providerId;
        const mid = modelId;
        const cid = sessionId;
        void generateSuggestions({ conversationId: cid, providerId: pid, modelId: mid, transcript: t })
          .then(() => setSuggestToken((n) => n + 1))
          .catch(() => undefined);
        void getSpeechEnabled()
          .then((on) => {
            if (!on) return;
            const last = t.filter((m) => m.role === 'assistant').pop();
            if (last?.content.trim()) void speakText(stripToolFences(last.content)).catch(() => undefined);
          })
          .catch(() => undefined);
      }
    },
    [providerId, modelId, sessionId, transcript, pushTrace],
  );

  const handleEvent = useCallback(
    (e: ToolLoopEvent) => {
      switch (e.type) {
        case 'handshake':
          pushTrace('info', `Handshake — ${e.manifest.available.length} tools available`);
          break;
        case 'assistant_delta': {
          const id = asstIdRef.current;
          if (!id) break;
          asstFullRef.current += e.delta;
          const chunk = e.delta;
          setThread((prev) => prev.map((m) => (m.id === id ? { ...m, text: m.text + chunk, full: (m.full ?? '') + chunk } : m)));
          break;
        }
        case 'assistant_done': {
          const id = asstIdRef.current;
          asstFullRef.current = e.text;
          if (id) {
            const display = stripToolFences(e.text) || '(working…)';
            setThread((prev) => prev.map((m) => (m.id === id ? { ...m, text: display, full: e.text } : m)));
          }
          break;
        }
        case 'tool_call':
          callArgsRef.current.set(e.callId, e.call);
          setToolActive(true);
          setToolCount((n) => n + 1);
          pushTrace('tool_call', e.call.tool, JSON.stringify(e.call.args).slice(0, 500));
          break;
        case 'tool_result': {
          setToolActive(false);
          const r = e.result;
          pushTrace('tool_result', `${r.tool} — ${r.ok ? 'ok' : `FAILED [${r.code}]`} (${r.duration_ms}ms)`, r.ok ? undefined : (r.error ?? undefined));
          const media = extractMedia(r.tool, r.ok ? r.output : null);
          const artifact = extractArtifact(r.ok ? r.output : null);
          if ((media || artifact) && asstIdRef.current) {
            const id = asstIdRef.current;
            setThread((prev) =>
              prev.map((m) =>
                m.id === id
                  ? {
                      ...m,
                      media: media ? [...m.media, media] : m.media,
                      artifacts: artifact ? [...m.artifacts, artifact] : m.artifacts,
                    }
                  : m,
              ),
            );
          }
          if (r.needs_approval) {
            const call = callArgsRef.current.get(e.callId);
            if (call) setPendingApproval({ call, prevText: asstFullRef.current });
          }
          if (r.needs_user) {
            const call = callArgsRef.current.get(e.callId);
            let question = 'Input needed';
            let options: string[] = [];
            try {
              const o = typeof r.output === 'string' ? JSON.parse(r.output) : r.output;
              if (o && typeof o === 'object') {
                const rec = o as Record<string, unknown>;
                if (typeof rec.question === 'string') question = rec.question;
                if (Array.isArray(rec.options)) options = rec.options.filter((x): x is string => typeof x === 'string');
              }
            } catch {
              /* keep defaults */
            }
            if (call) setPendingQuestion({ call, question, options, prevText: asstFullRef.current, answer: '' });
          }
          break;
        }
        case 'loop_end':
          if (e.status === 'approval_required' || e.status === 'user_input_required') {
            setRunning(false);
            setToolActive(false);
            pushTrace('gate', `Waiting on you — ${e.status}`, e.detail);
          } else if (e.status === 'error') {
            setRunning(false);
            setToolActive(false);
            setThread((prev) => [...prev, { id: newId('sys'), role: 'system', text: `Loop error: ${e.detail ?? 'unknown'}`, media: [], artifacts: [], ts: Date.now() }]);
            pushTrace('info', 'Turn error', e.detail);
          } else {
            finishTurn(e.status, e.detail);
          }
          break;
      }
    },
    [finishTurn, pushTrace],
  );

  const startLoop = useCallback(
    (userText: string, opts?: { resume?: boolean; resumeNote?: string }) => {
      if (running) return;
      if (!desktop) {
        onToast('Desktop Capability Required');
        return;
      }
      if (!providerId || !modelId) {
        onToast('Connect a provider first — open the Model & API Hub');
        return;
      }
      setRunning(true);
      setPendingApproval(null);
      asstFullRef.current = '';
      if (!opts?.resume) {
        const uid = newId('u');
        const aid = newId('a');
        asstIdRef.current = aid;
        setThread((prev) => [
          ...prev,
          { id: uid, role: 'user', text: userText, media: [], artifacts: [], ts: Date.now() },
          { id: aid, role: 'assistant', text: '', full: '', media: [], artifacts: [], modelName: modelId, ts: Date.now() },
        ]);
      } else {
        const aid = newId('a');
        asstIdRef.current = aid;
        setThread((prev) => [...prev, { id: aid, role: 'assistant', text: '', full: '', media: [], artifacts: [], modelName: modelId, ts: Date.now() }]);
        pushTrace('info', 'Resuming loop', opts.resumeNote);
      }
      const handle = runToolLoop({
        providerId,
        modelId,
        systemPrompt: agentPrompt,
        sessionId,
        agentId: 'agent-harness',
        maxTurns: 12,
        userMessage: userText,
        onEvent: handleEvent,
      });
      cancelRef.current = handle.cancel;
    },
    [running, desktop, providerId, modelId, sessionId, handleEvent, onToast, pushTrace],
  );

  const send = useCallback(
    (text: string) => {
      const t = text.trim();
      if (!t || running || pendingApproval || pendingQuestion) return;
      setInput('');
      startLoop(t);
    },
    [running, pendingApproval, pendingQuestion, startLoop],
  );

  const sendWithReply = useCallback(
    (text: string) => {
      const t = text.trim();
      if (!t || running) return;
      setInput('');
      const prefix = replyTo ? `↳ Re: ${replyTo.text.slice(0, 200)}\n\n` : '';
      setReplyTo(null);
      startLoop(`${prefix}${t}`);
    },
    [running, replyTo, startLoop],
  );

  const handleFileUpload = useCallback(
    async (files: FileList | null, icon: string) => {
      if (!files || files.length === 0) return;
      const toProcess = Array.from(files).slice(0, 20);
      setShowDocUpload(false);

      for (const file of toProcess) {
        const preview = await file.text();
        const name = file.name;
        setThread((prev) => [
          ...prev,
          {
            id: newId('attach'),
            role: 'user',
            text: `📎 Attached ${icon}: ${name} (${file.type || 'unknown'})`,
            media: [],
            artifacts: [],
            ts: Date.now(),
          },
        ]);

        if (file.type.startsWith('image/') || ['jpg','jpeg','png','gif','webp'].includes(file.type)) {
          const url = URL.createObjectURL(file);
          setThread((prev) => {
            const last = prev[prev.length - 1];
            if (!last) return prev;
            last.media.push({ kind: 'image', path: url, url });
            return [...prev.slice(0, -1), last];
          });
        } else if (file.size < 100_000) {
          setDocPreview({ name, text: preview });
          setShowPreview(true);
        } else {
          setDocPreview({ name, text: `(File too large for inline preview: ${file.size} bytes)` });
          setShowPreview(true);
        }
      }

      onToast(`${toProcess.length} file(s) attached`);
    },
    [onToast],
  );

  const resumeLoop = useCallback(
    (note: string) => {
      const prevText = asstFullRef.current || '(no assistant text yet)';
      startLoop(`[Continuing from where you left off]\n\nLast assistant message:\n${prevText}\n\n${note}\n\nContinue.`, {
        resume: true,
        resumeNote: note.slice(0, 200),
      });
    },
    [startLoop],
  );

  const approvePending = useCallback(async () => {
    if (!pendingApproval) return;
    const { call } = pendingApproval;
    setPendingApproval(null);
    // Approve-once: flip the tool's domain to always_allow, re-execute the
    // exact gated call, then restore the previous level. Deny never touches
    // permissions — it resumes with the denial in context instead.
    const domain = toolDomains[call.tool] ?? (call.tool.includes('.') && !call.tool.startsWith('media.') && !call.tool.startsWith('user.') ? 'mcp' : 'shell');
    let prevLevel = 'ask_every_time';
    try {
      const cur = await invoke<string>('perm_get', { domain });
      if (typeof cur === 'string' && cur) prevLevel = cur;
    } catch {
      /* default stands */
    }
    try {
      await invoke('perm_set', { domain, level: 'always_allow' });
    } catch (e) {
      onToast(`Approval failed: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    let result: ToolExecResult;
    try {
      result = await invoke<ToolExecResult>('tool_execute', {
        tool: call.tool,
        args_json: JSON.stringify(call.args),
        agent_id: 'agent-harness',
        session_id: sessionId,
      });
    } catch (e) {
      result = {
        ok: false,
        output: null,
        error: e instanceof Error ? e.message : String(e),
        duration_ms: 0,
        needs_approval: false,
        needs_user: false,
        tool: call.tool,
        code: 'exec_error',
        executed_at: new Date().toISOString(),
      };
    }
    try {
      await invoke('perm_set', { domain, level: prevLevel });
    } catch {
      /* restored on next gate; non-fatal */
    }
    pushTrace('tool_result', `${call.tool} — approved, executed (${result.duration_ms}ms)`, result.ok ? undefined : (result.error ?? undefined));
    const media = extractMedia(call.tool, result.ok ? result.output : null);
    const artifact = extractArtifact(result.ok ? result.output : null);
    if ((media || artifact) && asstIdRef.current) {
      const id = asstIdRef.current;
      setThread((prev) =>
        prev.map((m) =>
          m.id === id
            ? {
                ...m,
                media: media ? [...m.media, media] : m.media,
                artifacts: artifact ? [...m.artifacts, artifact] : m.artifacts,
              }
            : m,
        ),
      );
    }
    resumeLoop(`You approved and executed ${call.tool}. TOOL RESULT [${call.tool}]:\n${JSON.stringify(result)}`);
  }, [pendingApproval, toolDomains, sessionId, onToast, pushTrace, resumeLoop]);

  const denyPending = useCallback(() => {
    if (!pendingApproval) return;
    const { call } = pendingApproval;
    setPendingApproval(null);
    pushTrace('gate', `Denied ${call.tool} by user`);
    resumeLoop(`The user DENIED the pending ${call.tool} call. Do not retry it — work around it or explain what you need instead.`);
  }, [pendingApproval, pushTrace, resumeLoop]);

  const answerPending = useCallback(() => {
    if (!pendingQuestion || !pendingQuestion.answer.trim()) return;
    const { call, answer } = pendingQuestion;
    setPendingQuestion(null);
    pushTrace('info', `Answered ${call.tool}`, answer.slice(0, 200));
    resumeLoop(`ANSWER [${call.tool}]: ${answer.trim()}`);
  }, [pendingQuestion, pushTrace, resumeLoop]);

  const stop = useCallback(() => {
    cancelRef.current?.();
    cancelRef.current = null;
    setRunning(false);
    setToolActive(false);
    pushTrace('info', 'Stopped by user');
  }, [pushTrace]);

  const newChat = useCallback(() => {
    cancelRef.current?.();
    cancelRef.current = null;
    setRunning(false);
    setToolActive(false);
    setPendingApproval(null);
    setPendingQuestion(null);
    setThread([]);
    setTrace([]);
    setToolCount(0);
    asstIdRef.current = null;
    asstFullRef.current = '';
    callArgsRef.current.clear();
    setSessionId(newId('agent'));
    setSuggestToken(0);
  }, []);

  useEffect(() => () => cancelRef.current?.(), []);

  const activity: AvatarActivity = !running ? 'idle' : toolActive ? 'waiting' : 'typing';
  const modelLabel = models.find((m) => m.model_id === modelId)?.model_id ?? modelId;

  if (!desktop) {
    return (
      <div className="agent-harness">
        <div className="gf-pwa-notice">
          The Personal Agent needs the desktop app: tools, keyring, and model calls are native capabilities.{' '}
          <strong>Desktop Capability Required.</strong>
        </div>
      </div>
    );
  }

  return (
    <div className="agent-harness">
      <div className="agent-tabs" role="tablist" aria-label="Personal agent views">
        <button role="tab" aria-selected={tab === 'chat'} className={tab === 'chat' ? 'active' : ''} onClick={() => setTab('chat')}>
          Agent Chat
        </button>
        <button role="tab" aria-selected={tab === 'memory'} className={tab === 'memory' ? 'active' : ''} onClick={() => setTab('memory')}>
          Memory & Research
        </button>
      </div>

      {tab === 'memory' ? (
        <div className="agent-fill">
          <MemoryTabs />
        </div>
      ) : (
        <>
          <div className="agent-head">
            <AvatarStatus gender={gender} activity={activity} size={56} title="Your personal agent" />
            <div className="agent-selects">
              <label className="gf-label">
                Provider
                <select className="gf-select" value={providerId} onChange={(e) => setProviderId(e.target.value)} disabled={running}>
                  {providers.length === 0 && <option value="">No providers</option>}
                  {providers.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name} — {p.status}
                    </option>
                  ))}
                </select>
              </label>
              <label className="gf-label">
                Model
                <select className="gf-select" value={modelId} onChange={(e) => setModelId(e.target.value)} disabled={running}>
                  {models.length === 0 && <option value="">No models</option>}
                  {models.map((m) => (
                    <option key={m.id} value={m.model_id}>
                      {m.model_id}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <div className="gf-row" style={{ gap: 6, flexWrap: 'wrap' }}>
              <button className="gf-icon-btn" onClick={() => setShowTrace((s) => !s)} title="Toggle activity trace">
                {showTrace ? 'Hide trace' : 'Trace'}
              </button>
              <button className="gf-icon-btn" onClick={() => void reloadProviders()} title="Reload providers">
                Reload
              </button>
              <button className="gf-icon-btn" onClick={newChat} title="Start a new conversation">
                New chat
              </button>
            </div>
          </div>

          {providers.length === 0 && (
            <div className="agent-notice">
              <span>No providers configured yet.</span>{' '}
              <button className="gf-btn gf-btn-sm" onClick={() => onNavigate('models-api')}>
                Open Model & API Hub
              </button>
            </div>
          )}

          <div className="agent-thread" aria-live="polite">
            {thread.length === 0 && (
              <div className="agent-empty">
                <p>
                  <strong>Your daily companion.</strong> Ask for email triage, reminders, research, creditor scripts —
                  or an image or video, rendered right here.
                </p>
                <p className="gf-muted">Powered by {modelLabel || 'no model selected'} · tools run in the background, consequential ones ask first.</p>
              </div>
            )}
            {thread.map((m) => (
              <div key={m.id}>
                <ChatBubble
                  message={toChatMessage(m)}
                  onToast={onToast}
                  onReply={(msg) => {
                    const found = thread.find((t) => t.id === msg.id);
                    if (found) setReplyTo(found);
                  }}
                  onEmojiReaction={(msgId, emoji) => {
                    setReactions((prev) => {
                      const existing = prev[msgId] ?? {};
                      return { ...prev, [msgId]: { ...existing, [emoji]: (existing[emoji] ?? 0) + 1 } };
                    });
                  }}
                  reactions={reactions}
                />
                {m.media.length > 0 && (
                  <div className="agent-media-row">
                    {m.media.map((md, i) => (
                      <MediaView key={`${m.id}-md-${i}`} media={md} />
                    ))}
                  </div>
                )}
              </div>
            ))}
            <div ref={threadEndRef} />
          </div>

          {showTrace && (
            <div className="agent-trace" aria-label="Activity trace">
              <div className="gf-label">Activity trace {toolCount > 0 && `· ${toolCount} tool calls`}</div>
              {trace.length === 0 && <div className="gf-muted">No activity yet.</div>}
              {trace.slice(-60).map((t) => (
                <div key={t.id} className={`agent-trace-entry trace-${t.kind}`}>
                  <span className="agent-trace-label">{t.label}</span>
                  {t.detail && <span className="agent-trace-detail">{t.detail}</span>}
                </div>
              ))}
            </div>
          )}

          {pendingApproval && (
            <div className="agent-approval" role="alertdialog" aria-label="Tool approval">
              <div>
                <strong>Approval needed: {pendingApproval.call.tool}</strong>
                <pre>{JSON.stringify(pendingApproval.call.args, null, 2).slice(0, 600)}</pre>
              </div>
              <div className="gf-row" style={{ gap: 8 }}>
                <button className="gf-btn danger" onClick={denyPending}>
                  Deny
                </button>
                <button className="gf-btn primary" onClick={() => void approvePending()}>
                  Approve & run
                </button>
              </div>
            </div>
          )}

          {pendingQuestion && (
            <div className="agent-approval" role="alertdialog" aria-label="Agent question">
              <div style={{ flex: 1 }}>
                <strong>{pendingQuestion.question}</strong>
                {pendingQuestion.options.length > 0 && (
                  <div className="gf-row" style={{ gap: 6, flexWrap: 'wrap', marginTop: 6 }}>
                    {pendingQuestion.options.map((o) => (
                      <button key={o} className="gf-btn gf-btn-sm" onClick={() => setPendingQuestion((p) => (p ? { ...p, answer: o } : p))}>
                        {o}
                      </button>
                    ))}
                  </div>
                )}
                <input
                  className="gf-input"
                  style={{ marginTop: 8 }}
                  value={pendingQuestion.answer}
                  placeholder="Your answer"
                  onChange={(e) => setPendingQuestion((p) => (p ? { ...p, answer: e.target.value } : p))}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') answerPending();
                  }}
                />
              </div>
              <div className="gf-row" style={{ gap: 8 }}>
                <button className="gf-btn primary" onClick={answerPending} disabled={!pendingQuestion.answer.trim()}>
                  Answer
                </button>
              </div>
            </div>
          )}

          <div className="agent-quick" aria-label="Quick actions">
            {QUICK_SEND.map((q) => (
              <button key={q.label} className="gf-btn gf-btn-sm" disabled={running || !providerId} onClick={() => send(q.prompt)}>
                {q.label}
              </button>
            ))}
            {QUICK_INSERT.map((q) => (
              <button key={q.label} className="gf-btn gf-btn-sm" disabled={running} onClick={() => setInput(q.starter)}>
                {q.label}…
              </button>
            ))}
          </div>

          <SuggestedList conversationId={sessionId} onRun={(prompt) => send(prompt)} refreshToken={suggestToken} />

          <div className="agent-inputrow">
            <button
              className="gf-icon-btn"
              title="Attach document, photo, or any file"
              onClick={() => setShowDocUpload(true)}
              disabled={running}
            >
              📎
            </button>
            <VoiceControls onTranscript={(t) => setInput((prev) => (prev ? `${prev} ${t}` : t))} onToast={onToast} disabled={running} />
            <input
              className="gf-input"
              value={input}
              placeholder={running ? 'Agent is working…' : replyTo ? `Replying to "${replyTo.text.slice(0, 40)}"` : 'Ask your agent anything…'}
              disabled={running}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) sendWithReply(input);
              }}
            />
            {running ? (
              <button className="gf-btn danger" onClick={stop}>
                Stop
              </button>
            ) : (
              <button className="gf-btn primary" onClick={() => sendWithReply(input)} disabled={!input.trim() || !providerId}>
                Send
              </button>
            )}
          </div>

          {replyTo && (
            <div className="gf-row" style={{ gap: 6, padding: '4px 10px', background: 'var(--gf-panel-2)', borderRadius: 4, marginTop: 4 }}>
              <span className="gf-muted">Replying to:</span>
              <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {replyTo.text.slice(0, 120)}
              </span>
              <button className="gf-icon-btn" style={{ padding: '0 6px', fontSize: 12 }} onClick={() => setReplyTo(null)}>
                ✕
              </button>
            </div>
          )}

          {showPreview && docPreview && (
            <div className="agent-preview-window" style={{ display: showPreview ? 'block' : 'none' }}>
              <div className="agent-preview-header">
                <span className="gf-label">{docPreview.name}</span>
                <div style={{ display: 'flex', gap: 4 }}>
                  <button className="gf-btn gf-btn-sm" onClick={() => setShowPreview(false)}>
                    Hide
                  </button>
                  <button className="gf-btn gf-btn-sm primary" onClick={() => setShowPreview(false)}>
                    Close
                  </button>
                </div>
              </div>
              <div className="agent-preview-body">
                <pre style={{ whiteSpace: 'pre-wrap', margin: 0 }}>{docPreview.text}</pre>
              </div>
            </div>
          )}

          {showDocUpload && (
            <div className="agent-doc-upload">
              <div className="gf-label">Attach files (up to 20)</div>
              <div className="gf-row" style={{ gap: 8, flexWrap: 'wrap', marginTop: 8 }}>
                <label className="gf-btn" style={{ cursor: 'pointer', padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 6 }}>
                  📄 Document
                  <input type="file" multiple accept=".pdf,.doc,.docx,.txt,.md,.csv,.xlsx,.pptx,.rtf,.odt" hidden onChange={(e) => handleFileUpload(e.target.files, 'doc')} />
                </label>
                <label className="gf-btn" style={{ cursor: 'pointer', padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 6 }}>
                  📸 Photo
                  <input type="file" multiple accept="image/*" hidden onChange={(e) => handleFileUpload(e.target.files, 'photo')} />
                </label>
                <label className="gf-btn" style={{ cursor: 'pointer', padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 6 }}>
                  📷 Camera
                  <input type="file" multiple accept="image/*" capture="environment" hidden onChange={(e) => handleFileUpload(e.target.files, 'camera')} />
                </label>
                <label className="gf-btn" style={{ cursor: 'pointer', padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 6 }}>
                  📁 Any
                  <input type="file" multiple hidden onChange={(e) => handleFileUpload(e.target.files, 'any')} />
                </label>
              </div>
              <div style={{ marginTop: 8, display: 'flex', gap: 8 }}>
                <button className="gf-btn gf-btn-sm" onClick={() => setShowDocUpload(false)}>
                  Cancel
                </button>
              <button
                className="gf-btn primary gf-btn-sm"
                onClick={() => setShowDocUpload(false)}
              >
                Done
              </button>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
