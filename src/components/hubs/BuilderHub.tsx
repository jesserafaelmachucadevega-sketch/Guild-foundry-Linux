// SPDX-License-Identifier: Apache-2.0
// App Builder Studio hub — the multi-agent developer workbench with interior
// sub-tabs. Workspace (orchestrator), Runs (history), Scheduled (background
// tasks), and Artifacts (builds/templates/releases) share one hub instead of
// four sparse sidebar entries.

import React, { useState } from 'react';
import BuilderStudio from '../builder/BuilderStudio';
import RunHistory from '../builder/RunHistory';
import { ScheduledPanel } from '../scheduler/ScheduledPanel';
import { ArtifactsTabs } from '../artifacts/ArtifactsTabs';
import './hub.css';

interface Props {
  onToast: (t: string) => void;
}

type BuilderTab = 'workspace' | 'runs' | 'scheduled' | 'artifacts';

export function BuilderHub({ onToast }: Props): React.ReactElement {
  const [tab, setTab] = useState<BuilderTab>('workspace');
  const [runsTick, setRunsTick] = useState(0);
  const [selectedRunId, setSelectedRunId] = useState<string | undefined>(undefined);

  const openTab = (t: BuilderTab) => {
    if (t === 'runs') setRunsTick((n) => n + 1);
    setTab(t);
  };

  return (
    <div className="gf-workspace">
      <div className="gf-pane" style={{ flex: 1, minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        <div className="gf-pane-header">
          <span>App Builder Studio</span>
          <span className="gf-view-switcher" role="tablist" aria-label="Builder views">
            {(
              [
                ['workspace', 'Workspace'],
                ['runs', 'Runs'],
                ['scheduled', 'Scheduled'],
                ['artifacts', 'Artifacts'],
              ] as [BuilderTab, string][]
            ).map(([id, label]) => (
              <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? 'active' : ''} onClick={() => openTab(id)}>
                {label}
              </button>
            ))}
          </span>
        </div>
        <div className="hub-fill">
          {tab === 'workspace' && <BuilderStudio />}
          {tab === 'runs' && (
            <div className="gf-pane" style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
              <RunHistory
                refreshKey={runsTick}
                selectedId={selectedRunId}
                onSelect={(r) => {
                  setSelectedRunId(r.id);
                  setTab('workspace');
                  onToast(`Opened run ${r.id.slice(0, 8)} in the workspace`);
                }}
              />
            </div>
          )}
          {tab === 'scheduled' && <ScheduledPanel onToast={onToast} />}
          {tab === 'artifacts' && <ArtifactsTabs onToast={onToast} />}
        </div>
      </div>
    </div>
  );
}
