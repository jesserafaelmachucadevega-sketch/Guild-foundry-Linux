// SPDX-License-Identifier: Apache-2.0
// Phase 8 — Artifact panel: list artifacts, show/verify SHA-256, export to
// zip, create artifacts and releases.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';

interface ArtifactMeta {
  id: string;
  build_id: string | null;
  kind: string;
  path: string | null;
  sha256: string | null;
  created_at: string;
}

interface VerifyResult {
  ok: boolean;
  expected: string | null;
  actual: string | null;
}

interface ReleaseMeta {
  id: string;
  build_id: string;
  version: string;
  notes: string;
  created_at: string;
}

function fmtErr(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

function shortHash(h: string | null): string {
  if (!h) return '—';
  return h.length > 24 ? `${h.slice(0, 24)}…` : h;
}

export function ArtifactPanel({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [artifacts, setArtifacts] = useState<ArtifactMeta[]>([]);
  const [filterBuild, setFilterBuild] = useState('');
  const [verify, setVerify] = useState<Record<string, VerifyResult>>({});
  const [kind, setKind] = useState('package');
  const [filePath, setFilePath] = useState('');
  const [relBuild, setRelBuild] = useState('');
  const [relVersion, setRelVersion] = useState('');
  const [relNotes, setRelNotes] = useState('');

  const load = useCallback(async () => {
    try {
      const list = await invoke<ArtifactMeta[]>('artifact_list', {
        buildId: filterBuild.trim() || null,
      });
      setArtifacts(list);
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
    }
  }, [filterBuild, onToast]);

  useEffect(() => {
    void load();
  }, [load]);

  const verifyOne = useCallback(
    async (id: string) => {
      try {
        const r = await invoke<VerifyResult>('artifact_verify', { artifactId: id });
        setVerify((prev) => ({ ...prev, [id]: r }));
        onToast(r.ok ? 'SHA-256 matches.' : 'SHA-256 MISMATCH.');
      } catch (e) {
        onToast(`Desktop Capability Required: ${fmtErr(e)}`);
      }
    },
    [onToast],
  );

  const exportOne = useCallback(
    async (id: string) => {
      try {
        const r = await invoke<{ path: string }>('artifact_export', { artifactId: id });
        onToast(`Exported zip: ${r.path}`);
      } catch (e) {
        onToast(`Desktop Capability Required: ${fmtErr(e)}`);
      }
    },
    [onToast],
  );

  const create = useCallback(async () => {
    if (!relBuild.trim() || !filePath.trim()) {
      onToast('Build id and file path are required.');
      return;
    }
    try {
      const id = await invoke<string>('artifact_create', {
        buildId: relBuild.trim(),
        kind,
        path: filePath.trim(),
      });
      onToast(`Artifact recorded: ${id.slice(0, 8)}`);
      setFilterBuild(relBuild.trim());
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
    }
  }, [relBuild, kind, filePath, onToast]);

  const createRelease = useCallback(async () => {
    if (!relBuild.trim() || !relVersion.trim()) {
      onToast('Build id and version are required.');
      return;
    }
    try {
      const r = await invoke<ReleaseMeta>('release_create', {
        buildId: relBuild.trim(),
        version: relVersion.trim(),
        notes: relNotes,
      });
      onToast(`Release ${r.version} created (${r.id.slice(0, 8)}).`);
    } catch (e) {
      onToast(`Desktop Capability Required: ${fmtErr(e)}`);
    }
  }, [relBuild, relVersion, relNotes, onToast]);

  return (
    <div>
      <div className="gf-pane-header">
        <span>Artifacts &amp; Releases</span>
      </div>

      <div className="gf-build-controls">
        <label className="gf-field">
          <span className="gf-label">Filter by build id</span>
          <input
            className="gf-input"
            value={filterBuild}
            onChange={(e) => setFilterBuild(e.target.value)}
            placeholder="(all)"
          />
        </label>
        <button className="gf-btn" onClick={load}>
          Refresh
        </button>
      </div>

      {artifacts.length === 0 && (
        <p className="gf-muted">No artifacts recorded yet.</p>
      )}
      {artifacts.map((a) => {
        const v = verify[a.id];
        return (
          <div className="gf-artifact-row" key={a.id}>
            <span className="gf-badge">{a.kind}</span>
            <span className="gf-hash" title={a.sha256 ?? ''}>
              sha256: {shortHash(a.sha256)}
            </span>
            <span className="gf-muted">{a.path ?? '—'}</span>
            {v && (
              <span className={v.ok ? 'gf-verify-ok' : 'gf-verify-bad'}>
                {v.ok ? 'verified' : 'MISMATCH'}
              </span>
            )}
            <span style={{ flex: 1 }} />
            <button className="gf-btn" onClick={() => void verifyOne(a.id)}>
              Verify
            </button>
            <button className="gf-btn" onClick={() => void exportOne(a.id)}>
              Export zip
            </button>
          </div>
        );
      })}

      <hr className="gf-divider" />
      <div className="gf-pane-header">
        <span>Record artifact</span>
      </div>
      <div className="gf-build-controls">
        <label className="gf-field">
          <span className="gf-label">Build id</span>
          <input
            className="gf-input"
            value={relBuild}
            onChange={(e) => setRelBuild(e.target.value)}
          />
        </label>
        <label className="gf-field" style={{ minWidth: 120 }}>
          <span className="gf-label">Kind</span>
          <select className="gf-input" value={kind} onChange={(e) => setKind(e.target.value)}>
            <option value="package">package</option>
            <option value="log">log</option>
            <option value="failure-report">failure-report</option>
            <option value="patch-suggestion">patch-suggestion</option>
          </select>
        </label>
        <label className="gf-field">
          <span className="gf-label">File path</span>
          <input
            className="gf-input"
            value={filePath}
            onChange={(e) => setFilePath(e.target.value)}
            placeholder="/path/to/file"
          />
        </label>
        <button className="gf-btn primary" onClick={create}>
          Record
        </button>
      </div>

      <div className="gf-pane-header">
        <span>Create release</span>
      </div>
      <div className="gf-build-controls">
        <label className="gf-field" style={{ minWidth: 120 }}>
          <span className="gf-label">Version</span>
          <input
            className="gf-input"
            value={relVersion}
            onChange={(e) => setRelVersion(e.target.value)}
            placeholder="0.1.0"
          />
        </label>
        <label className="gf-field">
          <span className="gf-label">Notes</span>
          <input
            className="gf-input"
            value={relNotes}
            onChange={(e) => setRelNotes(e.target.value)}
            placeholder="Release notes"
          />
        </label>
        <button className="gf-btn primary" onClick={createRelease}>
          Create release
        </button>
      </div>
    </div>
  );
}
