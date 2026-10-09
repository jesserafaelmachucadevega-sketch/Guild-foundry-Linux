// SPDX-License-Identifier: Apache-2.0
import React, { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke, isDesktop } from '../../lib/api';
import './memory.css';

// Phase 10 — ResearchPanel: web research engine UI.
// Starts a background job, follows research://update progress events,
// renders cited extractive results with provenance links.

interface CitedResult {
  index: number;
  title: string;
  url: string;
  snippet: string;
  keySentences: string[];
}

interface ResearchUpdate {
  jobId: string;
  stage: string;
  progress: number;
  message: string;
}

interface ResearchStatus {
  id: string;
  query: string;
  state: string;
  stage: string;
  progress: number;
  results: CitedResult[];
  synthesis: string;
  comparisons: string[];
  error: string;
}

const STAGES = ['search', 'collect', 'deduplicate', 'extract', 'cite', 'compare', 'synthesize'] as const;

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function ResearchPanel(): React.ReactElement {
  const [query, setQuery] = useState('');
  const [jobId, setJobId] = useState<string | null>(null);
  const [stage, setStage] = useState('');
  const [progress, setProgress] = useState(0);
  const [message, setMessage] = useState('');
  const [status, setStatus] = useState<ResearchStatus | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const activeJob = useRef<string | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    let unlisten: (() => void) | null = null;
    void listen<ResearchUpdate>('research://update', (event) => {
      const u = event.payload;
      if (u.jobId !== activeJob.current) return;
      setStage(u.stage);
      setProgress(u.progress);
      setMessage(u.message);
      if (u.stage === 'synthesize' || u.stage === 'failed') {
        void invoke<ResearchStatus>('research_status', { jobId: u.jobId })
          .then((s) => {
            setStatus(s);
            if (s.state === 'failed') setError(s.error);
            setBusy(false);
          })
          .catch((e: unknown) => {
            setError(errText(e));
            setBusy(false);
          });
      }
    }).then((u) => { unlisten = u; });
    return () => { if (unlisten) unlisten(); };
  }, []);

  async function start(): Promise<void> {
    setBusy(true);
    setError('');
    setStatus(null);
    setProgress(0);
    setStage('search');
    try {
      const id = await invoke<string>('research_start', { query });
      activeJob.current = id;
      setJobId(id);
    } catch (e) {
      setError(`Desktop Capability Required — ${errText(e)}`);
      setBusy(false);
    }
  }

  return (
    <div className="gf-pane mem-panel">
      <div className="gf-pane-header">
        <span>Research</span>
        <span className="gf-badge">extractive synthesis — not LLM-generated</span>
      </div>

      <div className="mem-search">
        <input
          className="gf-input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="research query…"
          onKeyDown={(e) => { if (e.key === 'Enter') void start(); }}
        />
        <button className="gf-btn primary" disabled={busy || query.trim().length < 3} onClick={() => void start()}>
          Research
        </button>
      </div>

      {(busy || stage) && (
        <div className="mem-progress">
          <div className="mem-progress-bar">
            <div className="mem-progress-fill" style={{ width: `${Math.round(progress * 100)}%` }} />
          </div>
          <div className="mem-status">
            {stage && <span className="gf-badge">{stage}</span>} {message}
          </div>
          <div className="mem-stages">
            {STAGES.map((s) => (
              <span key={s} className={`gf-badge${stage === s ? ' ok' : ' idle'}`}>{s}</span>
            ))}
          </div>
        </div>
      )}

      {error && <div className="mem-error">{error}</div>}

      {status && status.state === 'done' && (
        <div className="mem-research-results">
          <section className="mem-synthesis">
            <h4>Synthesis</h4>
            <pre>{status.synthesis}</pre>
          </section>

          {status.comparisons.length > 0 && (
            <section className="mem-compare">
              <h4>Source comparison</h4>
              <ul>{status.comparisons.map((c, i) => <li key={i}>{c}</li>)}</ul>
            </section>
          )}

          <section className="mem-sources">
            <h4>Sources ({status.results.length})</h4>
            {status.results.map((r) => (
              <div key={r.index} className="mem-entry">
                <div className="mem-entry-key">
                  [{r.index}] <a href={r.url} target="_blank" rel="noreferrer">{r.title || r.url}</a>
                </div>
                <div className="mem-entry-body">{r.snippet}</div>
                {r.keySentences.map((s, i) => (
                  <div key={i} className="mem-quote">“{s}” <span className="gf-badge idle">[{r.index}]</span></div>
                ))}
              </div>
            ))}
          </section>
        </div>
      )}

      {jobId && <div className="mem-hint">job {jobId}</div>}
    </div>
  );
}
