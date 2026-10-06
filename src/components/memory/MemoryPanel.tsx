// SPDX-License-Identifier: Apache-2.0
import React, { useState } from 'react';
import { invoke } from '../../lib/api';
import './memory.css';

// Phase 10 — MemoryPanel: model-isolated memory with composite-key display.
// Composite key = scope + provider + model + version + key. Every query
// carries all five parts; the backend enforces isolation at DB + app level.

const SCOPES = ['conversation', 'model', 'agent', 'project', 'workspace', 'global'] as const;

const CLAIM_TYPES = ['verified_fact', 'model_claim', 'user_decision', 'inference', 'unresolved'] as const;

interface MemoryEntry {
  scope: string;
  providerId: string;
  modelName: string;
  version: string;
  scopeId: string | null;
  key: string;
  content: string;
  metadata: unknown;
  claimType: string;
  importance: number;
  accessCount: number;
  createdAt: string;
  updatedAt: string;
  flagged: string;
}

interface MemoryHit {
  entry: MemoryEntry;
  score: number;
}

interface ConsolidationReport {
  deduped: number;
  prunedStale: number;
  contradictionsFlagged: number;
  rescored: number;
}

function compositeKey(e: MemoryEntry): string {
  return `${e.scope} / ${e.providerId} / ${e.modelName} / ${e.version || '—'} / ${e.key}`;
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function MemoryPanel(): React.ReactElement {
  const [scope, setScope] = useState<string>('project');
  const [providerId, setProviderId] = useState('local');
  const [modelName, setModelName] = useState('');
  const [version, setVersion] = useState('');
  const [key, setKey] = useState('');
  const [content, setContent] = useState('');
  const [claimType, setClaimType] = useState<string>('unresolved');
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<MemoryHit[]>([]);
  const [report, setReport] = useState<ConsolidationReport | null>(null);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);

  async function put(): Promise<void> {
    setBusy(true);
    setStatus('');
    try {
      await invoke<void>('memory_put', {
        scope,
        providerId,
        modelName,
        version,
        scopeId: null,
        key,
        content,
        metadataJson: null,
        claimType,
      });
      setStatus(`stored under ${scope} / ${providerId} / ${modelName || '—'} / ${version || '—'} / ${key}`);
      setKey('');
      setContent('');
    } catch (e) {
      setStatus(`Desktop Capability Required — ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function get(): Promise<void> {
    setBusy(true);
    setStatus('');
    try {
      const value = await invoke<string | null>('memory_get', {
        scope,
        providerId,
        modelName,
        version,
        key,
      });
      if (value == null) {
        setStatus('no entry under that composite key');
      } else {
        setContent(value);
        setStatus('loaded');
      }
    } catch (e) {
      setStatus(`Desktop Capability Required — ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function search(): Promise<void> {
    setBusy(true);
    setStatus('');
    try {
      const r = await invoke<MemoryHit[]>('memory_search', {
        query,
        scope,
        providerId,
        modelName,
        version,
      });
      setHits(r);
      setStatus(`${r.length} hit(s)`);
    } catch (e) {
      setStatus(`Desktop Capability Required — ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function consolidate(): Promise<void> {
    setBusy(true);
    setStatus('');
    try {
      const r = await invoke<ConsolidationReport>('memory_consolidate');
      setReport(r);
      setStatus('consolidation complete');
    } catch (e) {
      setStatus(`Desktop Capability Required — ${errText(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="gf-pane mem-panel">
      <div className="gf-pane-header">
        <span>Memory</span>
        <span className="gf-badge">keyword search — semantic search not implemented</span>
      </div>

      <div className="mem-tabs">
        {SCOPES.map((s) => (
          <button
            key={s}
            className={`gf-btn${scope === s ? ' primary' : ''}`}
            onClick={() => setScope(s)}
          >
            {s}
          </button>
        ))}
      </div>

      <div className="mem-grid">
        <label>Provider <input className="gf-input" value={providerId} onChange={(e) => setProviderId(e.target.value)} /></label>
        <label>Model <input className="gf-input" value={modelName} onChange={(e) => setModelName(e.target.value)} placeholder="model name" /></label>
        <label>Version <input className="gf-input" value={version} onChange={(e) => setVersion(e.target.value)} placeholder="optional" /></label>
        <label>Key <input className="gf-input" value={key} onChange={(e) => setKey(e.target.value)} placeholder="memory key" /></label>
        <label className="mem-span2">Content
          <textarea className="gf-textarea" value={content} onChange={(e) => setContent(e.target.value)} rows={3} placeholder="memory content" />
        </label>
        <label>Claim type
          <select className="gf-select" value={claimType} onChange={(e) => setClaimType(e.target.value)}>
            {CLAIM_TYPES.map((c) => <option key={c} value={c}>{c}</option>)}
          </select>
        </label>
      </div>

      <div className="mem-actions">
        <button className="gf-btn primary" disabled={busy || !key} onClick={() => void put()}>Store</button>
        <button className="gf-btn" disabled={busy || !key} onClick={() => void get()}>Load</button>
        <button className="gf-btn danger" disabled={busy} onClick={() => void consolidate()}>Consolidate now</button>
      </div>

      <div className="mem-search">
        <input className="gf-input" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="search within this isolation domain…" />
        <button className="gf-btn" disabled={busy || query.trim().length < 2} onClick={() => void search()}>Search</button>
      </div>

      {status && <div className="mem-status">{status}</div>}

      {report && (
        <div className="mem-report">
          <span className="gf-badge">deduped {report.deduped}</span>
          <span className="gf-badge">pruned stale {report.prunedStale}</span>
          <span className="gf-badge err">contradictions {report.contradictionsFlagged}</span>
          <span className="gf-badge ok">rescored {report.rescored}</span>
        </div>
      )}

      <div className="mem-list">
        {hits.map((h) => (
          <div key={`${h.entry.scope}:${h.entry.providerId}:${h.entry.modelName}:${h.entry.version}:${h.entry.key}`} className="mem-entry">
            <div className="mem-entry-key">{compositeKey(h.entry)}</div>
            <div className="mem-entry-body">{h.entry.content}</div>
            <div className="mem-entry-meta">
              <span className="gf-badge">{h.entry.claimType}</span>
              <span className="gf-badge idle">score {h.score.toFixed(2)}</span>
              <span className="gf-badge idle">importance {h.entry.importance.toFixed(2)}</span>
              {h.entry.flagged && <span className="gf-badge err">flagged: {h.entry.flagged}</span>}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
