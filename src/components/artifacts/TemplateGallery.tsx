// SPDX-License-Identifier: Apache-2.0
// Phase 8 — Template gallery: browse embedded project templates and
// instantiate one into a chosen project root.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';

interface TemplateMeta {
  id: string;
  name: string;
  stack: string;
  description: string;
  file_count: number;
}

function fmtErr(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

export function TemplateGallery({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [templates, setTemplates] = useState<TemplateMeta[]>([]);
  const [destRoot, setDestRoot] = useState('');
  const [appName, setAppName] = useState('');

  useEffect(() => {
    void invoke<TemplateMeta[]>('template_list')
      .then(setTemplates)
      .catch((e: unknown) => onToast(`Desktop Capability Required: ${fmtErr(e)}`));
  }, [onToast]);

  const instantiate = useCallback(
    async (id: string) => {
      if (!destRoot.trim() || !appName.trim()) {
        onToast('Choose a destination root and an app name first.');
        return;
      }
      try {
        const path = await invoke<string>('template_instantiate', {
          templateId: id,
          destRoot: destRoot.trim(),
          appName: appName.trim(),
        });
        onToast(`Template created at ${path}`);
      } catch (e) {
        onToast(`Desktop Capability Required: ${fmtErr(e)}`);
      }
    },
    [destRoot, appName, onToast],
  );

  return (
    <div>
      <div className="gf-pane-header">
        <span>Project Templates</span>
      </div>
      <p className="gf-muted">
        Embedded, offline-capable starters. Each is a minimal but genuinely
        working project for its stack.
      </p>

      <div className="gf-build-controls">
        <label className="gf-field">
          <span className="gf-label">Destination root</span>
          <input
            className="gf-input"
            value={destRoot}
            onChange={(e) => setDestRoot(e.target.value)}
            placeholder="/path/to/projects"
          />
        </label>
        <label className="gf-field" style={{ minWidth: 160 }}>
          <span className="gf-label">App name</span>
          <input
            className="gf-input"
            value={appName}
            onChange={(e) => setAppName(e.target.value)}
            placeholder="my-app"
          />
        </label>
      </div>

      <div className="gf-template-grid">
        {templates.map((t) => (
          <div className="gf-template-card" key={t.id}>
            <h4>{t.name}</h4>
            <span className="gf-badge">{t.stack}</span>
            <p className="gf-muted" style={{ fontSize: 13, margin: 0 }}>
              {t.description}
            </p>
            <span className="gf-muted" style={{ fontSize: 12 }}>
              {t.file_count} files
            </span>
            <button className="gf-btn primary" onClick={() => void instantiate(t.id)}>
              Create project
            </button>
          </div>
        ))}
      </div>
      {templates.length === 0 && (
        <p className="gf-muted">Loading templates…</p>
      )}
    </div>
  );
}
