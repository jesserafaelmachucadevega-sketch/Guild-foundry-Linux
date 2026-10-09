// SPDX-License-Identifier: Apache-2.0
// Phase 8 — Tabbed panel composing the build, artifact, and template views.
// Intended wiring: App.tsx renders <ArtifactsTabs> for the `artifacts` nav
// section (see .integration/phase-8.md).

import React, { useState } from 'react';
import './artifacts.css';
import { BuildPanel } from './BuildPanel';
import { ArtifactPanel } from './ArtifactPanel';
import { TemplateGallery } from './TemplateGallery';

type Tab = 'build' | 'artifacts' | 'templates';

const TABS: { id: Tab; label: string }[] = [
  { id: 'build', label: 'Build & Fix' },
  { id: 'artifacts', label: 'Artifacts' },
  { id: 'templates', label: 'Templates' },
];

export function ArtifactsTabs({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [tab, setTab] = useState<Tab>('build');
  return (
    <div className="gf-workspace">
      <div className="gf-pane gf-artifacts" style={{ flex: 1 }}>
        <div className="gf-tab-bar" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              className={`gf-tab ${tab === t.id ? 'active' : ''}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="gf-artifacts-body">
          {tab === 'build' && <BuildPanel onToast={onToast} />}
          {tab === 'artifacts' && <ArtifactPanel onToast={onToast} />}
          {tab === 'templates' && <TemplateGallery onToast={onToast} />}
        </div>
      </div>
    </div>
  );
}
