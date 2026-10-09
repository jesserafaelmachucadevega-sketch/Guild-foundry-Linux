// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: terminal panel.
// Command input + scrollback, background process list + kill, and progressive
// confirmation driven by backend safety classification
// (SAFE / MODERATE / HIGH_RISK / CRITICAL).
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';

type Safety = 'SAFE' | 'MODERATE' | 'HIGH_RISK' | 'CRITICAL';

interface ExecOut {
  stdout: string;
  stderr: string;
  exit_code: number;
  timed_out: boolean;
}

interface ProcInfo {
  id: string;
  cmdline: string;
  pid: number;
  started_at: string;
  running: boolean;
}

interface ScrollLine {
  id: number;
  kind: 'cmd' | 'out' | 'err' | 'info';
  text: string;
}

function errMsg(err: unknown): string {
  return err instanceof DesktopCapabilityRequired
    ? 'Desktop Capability Required'
    : err instanceof Error
      ? err.message
      : String(err);
}

/** Split a command line into argv the way the backend expects: no shell is
 *  involved, so this is simple whitespace splitting with quote support. */
function splitArgv(input: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: string | null = null;
  for (let i = 0; i < input.length; i++) {
    const ch = input[i];
    if (quote) {
      if (ch === quote) quote = null;
      else cur += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (/\s/.test(ch)) {
      if (cur) {
        out.push(cur);
        cur = '';
      }
    } else {
      cur += ch;
    }
  }
  if (cur) out.push(cur);
  return out;
}

export function Terminal({
  projectRoot,
  onToast,
}: {
  projectRoot: string;
  onToast: (t: string) => void;
}): React.ReactElement {
  const [lines, setLines] = useState<ScrollLine[]>([
    { id: 0, kind: 'info', text: 'Sandboxed terminal — cwd is the project root. No shell: pipes, redirects and glob expansion are unavailable by design.' },
  ]);
  const [input, setInput] = useState('');
  const [procs, setProcs] = useState<ProcInfo[]>([]);
  const [pending, setPending] = useState<{ cmd: string; args: string[]; safety: Safety } | null>(null);
  const [confirmText, setConfirmText] = useState('');
  const [ackRisk, setAckRisk] = useState(false);
  const [running, setRunning] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const idRef = useRef(1);

  const push = useCallback((kind: ScrollLine['kind'], text: string) => {
    const id = idRef.current++;
    setLines((ls) => [...ls.slice(-2000), { id, kind, text }]);
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [lines]);

  const refreshProcs = useCallback(async () => {
    try {
      const list = await invoke<ProcInfo[]>('term_list');
      setProcs(list);
    } catch {
      /* registry unavailable in preview mode; stay quiet */
    }
  }, []);

  useEffect(() => {
    void refreshProcs();
    const t = window.setInterval(() => void refreshProcs(), 5000);
    return () => window.clearInterval(t);
  }, [refreshProcs]);

  // Live output events from term_start processes.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        unlisten = await listen<{ id: string; stream: string; line: string }>(
          'terminal://output',
          (e) => {
            push(
              e.payload.stream === 'stderr' ? 'err' : e.payload.stream === 'exit' ? 'info' : 'out',
              e.payload.stream === 'exit'
                ? `[${e.payload.id.slice(0, 8)}] ${e.payload.line}`
                : `[${e.payload.id.slice(0, 8)}|${e.payload.stream}] ${e.payload.line}`,
            );
            void refreshProcs();
          },
        );
      } catch {
        /* not in desktop runtime */
      }
    })();
    return () => {
      unlisten?.();
    };
  }, [push, refreshProcs]);

  const runExec = async (cmd: string, args: string[]) => {
    setRunning(true);
    push('cmd', `$ ${[cmd, ...args].join(' ')}`);
    try {
      const out = await invoke<ExecOut>('term_exec', {
        project_root: projectRoot,
        cmd,
        args,
        timeout_secs: 60,
      });
      if (out.stdout) push('out', out.stdout);
      if (out.stderr) push('err', out.stderr);
      push(
        'info',
        out.timed_out
          ? '[timed out after 60s — process killed]'
          : `[exit ${out.exit_code}]`,
      );
    } catch (err) {
      push('err', errMsg(err));
      onToast(errMsg(err));
    } finally {
      setRunning(false);
    }
  };

  const runBackground = async (cmd: string, args: string[]) => {
    push('cmd', `$ ${[cmd, ...args].join(' ')} &`);
    try {
      const id = await invoke<string>('term_start', {
        project_root: projectRoot,
        cmd,
        args,
      });
      push('info', `[started background process ${id.slice(0, 8)}]`);
      void refreshProcs();
    } catch (err) {
      push('err', errMsg(err));
      onToast(errMsg(err));
    }
  };

  const submit = async (background: boolean) => {
    const argv = splitArgv(input.trim());
    setInput('');
    if (!argv.length) return;
    const [cmd, ...args] = argv;
    let safety: Safety = 'SAFE';
    try {
      safety = await invoke<Safety>('term_classify', { cmd, args });
    } catch (err) {
      push('err', errMsg(err));
      return;
    }
    if (safety === 'SAFE') {
      if (background) void runBackground(cmd, args);
      else void runExec(cmd, args);
    } else {
      // Progressive confirmation for anything above SAFE.
      setPending({ cmd, args, safety });
      setConfirmText('');
      setAckRisk(false);
    }
  };

  const confirmPending = () => {
    if (!pending) return;
    if (pending.safety === 'HIGH_RISK' && confirmText.trim() !== pending.cmd) return;
    if (pending.safety === 'CRITICAL' && !ackRisk) return;
    const { cmd, args } = pending;
    setPending(null);
    void runExec(cmd, args);
  };

  const killProc = async (id: string) => {
    try {
      await invoke('term_kill', { id });
      push('info', `[killed ${id.slice(0, 8)}]`);
      void refreshProcs();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const badgeClass =
    pending?.safety === 'SAFE'
      ? 'safe'
      : pending?.safety === 'MODERATE'
        ? 'moderate'
        : pending?.safety === 'HIGH_RISK'
          ? 'high'
          : 'critical';

  return (
    <div className="gf-projects-col" style={{ flex: 1 }}>
      <div className="gf-pane-header">
        <span>Terminal</span>
        <span className="gf-muted" style={{ fontSize: 11, textTransform: 'none', letterSpacing: 0 }}>
          sandboxed · no shell · 60s timeout
        </span>
      </div>
      <div className="gf-term-scroll" ref={scrollRef}>
        {lines.map((l) => (
          <div
            key={l.id}
            className={
              l.kind === 'cmd' ? 'gf-prompt' : l.kind === 'err' ? 'gf-err' : l.kind === 'info' ? 'gf-dim' : ''
            }
          >
            {l.text}
          </div>
        ))}
      </div>
      <div style={{ borderTop: '1px solid var(--gf-border)', padding: 8 }}>
        <div className="gf-row">
          <span className="gf-prompt" style={{ fontFamily: 'var(--gf-mono)', color: 'var(--gf-gold)' }}>$</span>
          <input
            className="gf-input gf-grow"
            style={{ fontFamily: 'var(--gf-mono)' }}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void submit(e.shiftKey);
            }}
            placeholder="command…  (Shift+Enter runs in background)"
            disabled={running}
          />
          <button className="gf-btn" onClick={() => void submit(false)} disabled={running || !input.trim()}>
            Run
          </button>
          <button className="gf-btn" onClick={() => void submit(true)} disabled={running || !input.trim()} title="Run as background process">
            Bg
          </button>
        </div>
      </div>
      <div style={{ borderTop: '1px solid var(--gf-border)', maxHeight: 140, overflowY: 'auto' }}>
        <div className="gf-section-title">Processes</div>
        {procs.length === 0 && (
          <p className="gf-muted" style={{ padding: '0 10px 8px', fontSize: 12 }}>
            No background processes.
          </p>
        )}
        {procs.map((p) => (
          <div className="gf-proc-row" key={p.id}>
            <span className={`gf-badge ${p.running ? 'running' : ''}`}>
              {p.running ? 'running' : 'exited'}
            </span>
            <span className="gf-cmd" title={p.cmdline}>
              {p.cmdline}
            </span>
            <span className="gf-muted" style={{ fontSize: 11 }}>
              pid {p.pid}
            </span>
            {p.running && (
              <button className="gf-btn danger" style={{ padding: '0 8px' }} onClick={() => void killProc(p.id)}>
                Kill
              </button>
            )}
          </div>
        ))}
      </div>

      {pending && (
        <div className="gf-modal-backdrop" onClick={() => setPending(null)}>
          <div className="gf-modal" onClick={(e) => e.stopPropagation()}>
            <div className="gf-pane-header">
              <span>Confirm command</span>
              <span className={`gf-badge ${badgeClass}`}>{pending.safety.replace('_', ' ')}</span>
            </div>
            <div className="gf-modal-body">
              <pre>{[pending.cmd, ...pending.args].join(' ')}</pre>
              {pending.safety === 'MODERATE' && (
                <p className="gf-muted">This command modifies files in the project. Run it?</p>
              )}
              {pending.safety === 'HIGH_RISK' && (
                <>
                  <p>
                    This command is <strong>high risk</strong> (package install,
                    network change, or destructive operation). Type the command
                    name <code>{pending.cmd}</code> to confirm.
                  </p>
                  <input
                    className="gf-input"
                    value={confirmText}
                    onChange={(e) => setConfirmText(e.target.value)}
                    placeholder={pending.cmd}
                  />
                </>
              )}
              {pending.safety === 'CRITICAL' && (
                <>
                  <p style={{ color: 'var(--gf-danger)' }}>
                    This command is <strong>critical</strong> — system-level or
                    irreversible. It will still run inside the project sandbox
                    with a timeout, but double-check what it does.
                  </p>
                  <label className="gf-row" style={{ marginTop: 8 }}>
                    <input
                      type="checkbox"
                      checked={ackRisk}
                      onChange={(e) => setAckRisk(e.target.checked)}
                    />
                    <span>I understand the risk and want to run it anyway</span>
                  </label>
                </>
              )}
            </div>
            <div className="gf-modal-footer">
              <button className="gf-btn" onClick={() => setPending(null)}>
                Cancel
              </button>
              <button
                className="gf-btn primary"
                onClick={confirmPending}
                disabled={
                  (pending.safety === 'HIGH_RISK' && confirmText.trim() !== pending.cmd) ||
                  (pending.safety === 'CRITICAL' && !ackRisk)
                }
              >
                Run command
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
