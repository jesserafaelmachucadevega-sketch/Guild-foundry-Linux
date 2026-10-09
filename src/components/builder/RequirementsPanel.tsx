// SPDX-License-Identifier: Apache-2.0
// Phase 5 — Requirements engine panel: Q&A with the Supervisor refines the
// requirements document, which is saved via requirements_upsert; the
// architecture sign-off flips architecture_approved; ADRs are listed and can
// be added manually.

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';
import { streamChat } from '../../lib/chat';
import type { AdrRow, ModelChoice, RequirementsDoc } from './types';

interface QA {
  q: string;
  a: string;
}

export default function RequirementsPanel({
  runId,
  model,
  onChanged,
}: {
  runId: string;
  model: ModelChoice;
  onChanged: () => void;
}) {
  const [doc, setDoc] = useState('');
  const [approved, setApproved] = useState(false);
  const [qa, setQa] = useState<QA[]>([]);
  const [question, setQuestion] = useState('');
  const [busy, setBusy] = useState(false);
  const [adrs, setAdrs] = useState<AdrRow[]>([]);
  const [adrTitle, setAdrTitle] = useState('');
  const [adrDecision, setAdrDecision] = useState('');
  const [error, setError] = useState('');

  const load = useCallback(() => {
    invoke<RequirementsDoc>('requirements_get', { run_id: runId })
      .then((d) => {
        setDoc(d.doc_json);
        setApproved(d.architecture_approved);
      })
      .catch(() => setDoc(JSON.stringify({ summary: '' }, null, 2)));
    invoke<AdrRow[]>('adr_list', { run_id: runId })
      .then(setAdrs)
      .catch(() => setAdrs([]));
  }, [runId]);

  useEffect(() => {
    load();
  }, [load]);

  const ask = async () => {
    const q = question.trim();
    if (!q || busy) return;
    setBusy(true);
    setError('');
    setQuestion('');
    try {
      let a = '';
      await streamChat(
        {
          providerId: model.providerId,
          modelId: model.modelId,
          messages: [
            {
              role: 'system',
              content:
                'You are the Supervisor helping the user refine build requirements. Answer concisely. When the requirements look complete, say so and summarize them.',
            },
            { role: 'user', content: `Current requirements draft:\n${doc}\n\nUser question: ${q}` },
          ],
          params: { temperature: 0.4 },
        },
        {
          onDelta: (d) => {
            a += d;
          },
          onDone: () => {
            setQa((prev) => [...prev, { q, a }]);
            setBusy(false);
          },
          onError: (e) => {
            setError(e.message);
            setBusy(false);
          },
        },
      );
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const save = async () => {
    setError('');
    try {
      JSON.parse(doc);
    } catch {
      setError('Document is not valid JSON.');
      return;
    }
    try {
      await invoke('requirements_upsert', { run_id: runId, doc_json: doc });
      onChanged();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const signoff = async () => {
    setError('');
    try {
      await invoke('requirements_signoff', { run_id: runId });
      setApproved(true);
      onChanged();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const addAdr = async () => {
    const title = adrTitle.trim();
    if (!title) return;
    setError('');
    try {
      await invoke('adr_create', {
        run_id: runId,
        title,
        context: '',
        decision: adrDecision,
        consequences: '',
      });
      setAdrTitle('');
      setAdrDecision('');
      const rows = await invoke<AdrRow[]>('adr_list', { run_id: runId });
      setAdrs(rows);
      onChanged();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="bld-panel">
      <h3>Requirements &amp; architecture</h3>

      <div className="gf-label">Refine with the Supervisor</div>
      {qa.map((item, i) => (
        <div key={i} className="bld-qa">
          <div className="q">Q: {item.q}</div>
          <div className="a">{item.a}</div>
        </div>
      ))}
      <div className="bld-form-row">
        <input
          className="gf-input"
          placeholder="Ask about the requirements…"
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void ask();
          }}
        />
        <button className="gf-btn" disabled={busy} onClick={() => void ask()}>
          {busy ? '…' : 'Ask'}
        </button>
      </div>

      <div className="gf-label">Requirements document (JSON)</div>
      <textarea
        className="gf-textarea"
        style={{ width: '100%', minHeight: 140, fontFamily: 'var(--gf-mono)', fontSize: 12 }}
        value={doc}
        onChange={(e) => setDoc(e.target.value)}
      />
      <div className="bld-form-row" style={{ marginTop: 8 }}>
        <button className="gf-btn" onClick={() => void save()}>
          Save requirements
        </button>
        <button className="gf-btn primary" onClick={() => void signoff()} disabled={approved}>
          {approved ? 'Architecture signed off ✓' : 'Sign off architecture'}
        </button>
      </div>

      <div className="gf-label" style={{ marginTop: 12 }}>
        Architecture decision records ({adrs.length})
      </div>
      {adrs.map((a) => (
        <div key={a.id} className="bld-adr">
          <h4>{a.title}</h4>
          {a.context && <p><span className="lbl">Context: </span>{a.context}</p>}
          {a.decision && <p><span className="lbl">Decision: </span>{a.decision}</p>}
          {a.consequences && <p><span className="lbl">Consequences: </span>{a.consequences}</p>}
        </div>
      ))}
      <div className="bld-form-row">
        <input
          className="gf-input"
          placeholder="ADR title"
          value={adrTitle}
          onChange={(e) => setAdrTitle(e.target.value)}
        />
        <input
          className="gf-input"
          placeholder="Decision"
          value={adrDecision}
          onChange={(e) => setAdrDecision(e.target.value)}
        />
        <button className="gf-btn" onClick={() => void addAdr()}>
          Add ADR
        </button>
      </div>

      {error && <div className="bld-error">{error}</div>}
    </div>
  );
}
