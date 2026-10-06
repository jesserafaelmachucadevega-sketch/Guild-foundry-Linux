// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: Git panel.
// Status, diff, log, branches, commit, stash, pull. Push is never automatic:
// the push button opens a confirmation modal (remote, branch, outgoing
// commits, changed files, diff summary) and only then calls git_push with
// confirmed=true.
import React, { useCallback, useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';

interface GitFileStatus {
  path: string;
  staged: string;
  worktree: string;
}

interface GitCommit {
  hash: string;
  short: string;
  author: string;
  date: string;
  message: string;
}

interface GitBranch {
  name: string;
  current: boolean;
  upstream: string;
}

interface GitRemote {
  name: string;
  url: string;
}

function errMsg(err: unknown): string {
  return err instanceof DesktopCapabilityRequired
    ? 'Desktop Capability Required'
    : err instanceof Error
      ? err.message
      : String(err);
}

export function GitPanel({
  projectRoot,
  onToast,
}: {
  projectRoot: string;
  onToast: (t: string) => void;
}): React.ReactElement {
  const [status, setStatus] = useState<GitFileStatus[]>([]);
  const [log, setLog] = useState<GitCommit[]>([]);
  const [branches, setBranches] = useState<GitBranch[]>([]);
  const [remotes, setRemotes] = useState<GitRemote[]>([]);
  const [selFile, setSelFile] = useState<string | null>(null);
  const [diff, setDiff] = useState('');
  const [message, setMessage] = useState('');
  const [pushOpen, setPushOpen] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);
  const [pushRemote, setPushRemote] = useState('origin');
  const [pushBranch, setPushBranch] = useState('');
  const [outgoing, setOutgoing] = useState<GitCommit[]>([]);
  const [pushDiffSummary, setPushDiffSummary] = useState('');

  const refresh = useCallback(async () => {
    try {
      const [st, lg, br, rm] = await Promise.all([
        invoke<GitFileStatus[]>('git_status', { project_root: projectRoot }),
        invoke<GitCommit[]>('git_log', { project_root: projectRoot, limit: 30 }),
        invoke<GitBranch[]>('git_branch', { project_root: projectRoot }),
        invoke<GitRemote[]>('git_remote', { project_root: projectRoot }).catch(() => [] as GitRemote[]),
      ]);
      setStatus(st);
      setLog(lg);
      setBranches(br);
      setRemotes(rm);
      const cur = br.find((b) => b.current);
      if (cur) setPushBranch((prev) => prev || cur.name);
    } catch (err) {
      onToast(errMsg(err));
    }
  }, [projectRoot, onToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const loadDiff = async (path: string, staged: boolean) => {
    setSelFile(path);
    try {
      const d = await invoke<string>('git_diff', {
        project_root: projectRoot,
        staged,
        path,
      });
      setDiff(d || '(no textual diff)');
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const doCommit = async () => {
    if (!message.trim()) {
      onToast('Commit message is empty');
      return;
    }
    try {
      const hash = await invoke<string>('git_commit', {
        project_root: projectRoot,
        message: message.trim(),
      });
      onToast(`Committed ${hash.slice(0, 8)}`);
      setMessage('');
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const doStash = async () => {
    try {
      const out = await invoke<string>('git_stash', {
        project_root: projectRoot,
        message: null,
        include_untracked: false,
      });
      onToast(out || 'Stashed');
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const doPull = async () => {
    try {
      const out = await invoke<string>('git_pull', {
        project_root: projectRoot,
        remote: pushRemote,
        branch: pushBranch,
      });
      onToast('Pulled');
      setPullOut(out);
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const [pullOut, setPullOut] = useState('');

  // Pre-push modal: gather remote, branch, outgoing commits, changed files,
  // and a diff summary before anything is pushed.
  const openPushModal = async () => {
    try {
      const branch =
        pushBranch || branches.find((b) => b.current)?.name || 'main';
      setPushBranch(branch);
      // Outgoing commits: commits not on the upstream (best effort).
      let commits: GitCommit[] = [];
      try {
        commits = await invoke<GitCommit[]>('git_log', {
          project_root: projectRoot,
          limit: 10,
        });
      } catch {
        commits = [];
      }
      setOutgoing(commits);
      const d = await invoke<string>('git_diff', {
        project_root: projectRoot,
        staged: false,
        path: null,
      }).catch(() => '');
      const changedLines = d
        .split('\n')
        .filter((l) => l.startsWith('+') || l.startsWith('-'))
        .length;
      setPushDiffSummary(
        `${status.length} changed file(s), ~${changedLines} changed line(s) in working tree`,
      );
      setPushOpen(true);
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const confirmPush = async () => {
    setPushBusy(true);
    try {
      const out = await invoke<string>('git_push', {
        project_root: projectRoot,
        remote: pushRemote,
        branch: pushBranch,
        confirmed: true,
      });
      onToast('Pushed');
      setPullOut(out);
      setPushOpen(false);
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    } finally {
      setPushBusy(false);
    }
  };

  const checkout = async (name: string) => {
    try {
      await invoke('git_checkout', { project_root: projectRoot, target: name });
      onToast(`Checked out ${name}`);
      void refresh();
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  return (
    <div className="gf-projects-col" style={{ width: 380, minWidth: 300 }}>
      <div className="gf-pane-header">
        <span>Source Control</span>
        <button className="gf-btn" style={{ padding: '0 8px' }} onClick={() => void refresh()}>
          ⟳
        </button>
      </div>
      <div className="gf-scroll-list">
        <div className="gf-section-title">Branches</div>
        <div className="gf-row" style={{ padding: '0 10px', flexWrap: 'wrap' }}>
          {branches.map((b) => (
            <button
              key={b.name}
              className={`gf-btn${b.current ? ' primary' : ''}`}
              style={{ padding: '2px 8px', fontSize: 11 }}
              onClick={() => !b.current && void checkout(b.name)}
              title={b.upstream ? `upstream: ${b.upstream}` : 'no upstream'}
            >
              {b.current ? '● ' : ''}{b.name}
            </button>
          ))}
          {branches.length === 0 && <span className="gf-muted" style={{ fontSize: 12 }}>No branches (is this a git repo?)</span>}
        </div>

        <div className="gf-section-title">Changes ({status.length})</div>
        {status.map((f) => (
          <div
            key={f.path}
            className={`gf-git-status-row${selFile === f.path ? ' selected' : ''}`}
            onClick={() => void loadDiff(f.path, false)}
          >
            <span className="gf-git-xy">{`${f.staged}${f.worktree}`}</span>
            <span className="gf-grow" style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{f.path}</span>
          </div>
        ))}
        {status.length === 0 && (
          <p className="gf-muted" style={{ padding: '0 10px', fontSize: 12 }}>Working tree clean.</p>
        )}

        {selFile && (
          <>
            <div className="gf-section-title">Diff — {selFile}</div>
            <pre
              style={{
                margin: '0 10px',
                padding: 8,
                background: 'var(--gf-black)',
                border: '1px solid var(--gf-border)',
                fontSize: 11,
                fontFamily: 'var(--gf-mono)',
                overflow: 'auto',
                maxHeight: 240,
                whiteSpace: 'pre-wrap',
              }}
            >
              {diff}
            </pre>
          </>
        )}

        <div className="gf-section-title">Commit</div>
        <div style={{ padding: '0 10px' }}>
          <textarea
            className="gf-textarea"
            rows={3}
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="Commit message…"
          />
          <div className="gf-row" style={{ marginTop: 8, flexWrap: 'wrap' }}>
            <button className="gf-btn primary" onClick={() => void doCommit()}>
              Commit
            </button>
            <button className="gf-btn" onClick={() => void doStash()}>
              Stash
            </button>
            <button className="gf-btn" onClick={() => void doPull()}>
              Pull
            </button>
            <button className="gf-btn danger" onClick={() => void openPushModal()}>
              Push…
            </button>
          </div>
          {pullOut && (
            <pre
              style={{
                marginTop: 8,
                padding: 8,
                background: 'var(--gf-black)',
                border: '1px solid var(--gf-border)',
                fontSize: 11,
                fontFamily: 'var(--gf-mono)',
                whiteSpace: 'pre-wrap',
              }}
            >
              {pullOut}
            </pre>
          )}
        </div>

        <div className="gf-section-title">Log</div>
        {log.map((c) => (
          <div key={c.hash} style={{ padding: '6px 10px', borderBottom: '1px solid var(--gf-border)' }}>
            <div className="gf-row">
              <span style={{ fontFamily: 'var(--gf-mono)', color: 'var(--gf-gold)', fontSize: 12 }}>
                {c.short}
              </span>
              <span className="gf-grow" style={{ fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {c.message}
              </span>
            </div>
            <div className="gf-muted" style={{ fontSize: 11 }}>
              {c.author} · {c.date}
            </div>
          </div>
        ))}
      </div>

      {pushOpen && (
        <div className="gf-modal-backdrop" onClick={() => !pushBusy && setPushOpen(false)}>
          <div className="gf-modal" onClick={(e) => e.stopPropagation()}>
            <div className="gf-pane-header">
              <span>Confirm push</span>
              <span className="gf-badge high">high risk</span>
            </div>
            <div className="gf-modal-body">
              <div className="gf-row" style={{ marginBottom: 8 }}>
                <span className="gf-label">Remote</span>
                <select
                  className="gf-select"
                  value={pushRemote}
                  onChange={(e) => setPushRemote(e.target.value)}
                >
                  {(remotes.length ? remotes : [{ name: 'origin', url: '' }]).map((r) => (
                    <option key={r.name} value={r.name}>
                      {r.name}{r.url ? ` (${r.url})` : ''}
                    </option>
                  ))}
                </select>
                <span className="gf-label">Branch</span>
                <input
                  className="gf-input"
                  value={pushBranch}
                  onChange={(e) => setPushBranch(e.target.value)}
                  style={{ width: 140 }}
                />
              </div>
              <span className="gf-label">Recent commits</span>
              <pre>
                {outgoing.length
                  ? outgoing.map((c) => `${c.short} ${c.message}`).join('\n')
                  : '(no commit info)'}
              </pre>
              <span className="gf-label">Changed files</span>
              <pre>
                {status.length
                  ? status.map((f) => `${f.staged}${f.worktree} ${f.path}`).join('\n')
                  : '(working tree clean)'}
              </pre>
              <span className="gf-label">Diff summary</span>
              <pre>{pushDiffSummary}</pre>
              <p className="gf-muted">
                Push is never automatic. Confirming here sends the branch to the
                remote.
              </p>
            </div>
            <div className="gf-modal-footer">
              <button className="gf-btn" onClick={() => setPushOpen(false)} disabled={pushBusy}>
                Cancel
              </button>
              <button className="gf-btn primary" onClick={() => void confirmPush()} disabled={pushBusy}>
                {pushBusy ? 'Pushing…' : `Push ${pushBranch} → ${pushRemote}`}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
