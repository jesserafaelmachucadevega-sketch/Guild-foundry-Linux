// SPDX-License-Identifier: Apache-2.0
// Phase 9 — Security classifier demo: classify a shell command and preview
// content labeling. Wires the previously uncalled `sec_classify_command`,
// `sec_wrap_content`, and `sec_build_prompt` backend commands.

import React, { useState } from 'react';
import { invoke } from '../../lib/api';

export function ClassifierPanel({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [cmd, setCmd] = useState('');
  const [classification, setClassification] = useState('');
  const [source, setSource] = useState('USER');
  const [content, setContent] = useState('');
  const [wrapped, setWrapped] = useState('');

  const classify = async () => {
    if (!cmd.trim()) return;
    try {
      const r = await invoke<string>('sec_classify_command', { cmd: cmd.trim() });
      setClassification(r);
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const wrap = async () => {
    if (!content.trim()) return;
    try {
      const r = await invoke<string>('sec_wrap_content', { source, content });
      setWrapped(r);
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const buildPrompt = async () => {
    if (!content.trim()) return;
    try {
      const r = await invoke<string>('sec_build_prompt', {
        parts: [{ source, content }],
      });
      setWrapped(r);
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div style={{ padding: 12 }}>
      <span className="gf-label">Command classifier</span>
      <div className="gf-row" style={{ gap: 6 }}>
        <input
          className="gf-input gf-grow"
          placeholder="e.g. rm -rf /tmp/cache"
          value={cmd}
          onChange={(e) => setCmd(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void classify();
          }}
        />
        <button className="gf-btn" onClick={() => void classify()}>
          Classify
        </button>
      </div>
      {classification && (
        <p className="gf-muted" style={{ fontSize: 12 }}>
          Verdict: <strong>{classification}</strong>
        </p>
      )}
      <hr className="gf-divider" />
      <span className="gf-label">Content labeling (prompt-injection defense)</span>
      <div className="gf-row" style={{ gap: 6 }}>
        <select className="gf-input" value={source} onChange={(e) => setSource(e.target.value)}>
          {['USER', 'SYSTEM', 'TOOL RESULT', 'EXTERNAL', 'MODEL-GENERATED'].map((s) => (
            <option key={s} value={s}>{s}</option>
          ))}
        </select>
      </div>
      <textarea
        className="gf-textarea"
        style={{ width: '100%', minHeight: 70, marginTop: 6 }}
        placeholder="External content to label…"
        value={content}
        onChange={(e) => setContent(e.target.value)}
      />
      <div className="gf-row" style={{ gap: 6, marginTop: 6 }}>
        <button className="gf-btn" onClick={() => void wrap()}>
          Label content
        </button>
        <button className="gf-btn" onClick={() => void buildPrompt()}>
          Build prompt
        </button>
      </div>
      {wrapped && (
        <pre style={{ fontSize: 11, whiteSpace: 'pre-wrap', marginTop: 8 }}>{wrapped.slice(0, 4000)}</pre>
      )}
    </div>
  );
}
