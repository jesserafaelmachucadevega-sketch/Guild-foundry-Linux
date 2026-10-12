// SPDX-License-Identifier: Apache-2.0
// Memory & research views (model-isolated memory, knowledge bridge, web
// research). Rendered as a tab inside the Personal Agent harness — a
// companion needs its memory next to its chat.

import React, { useState } from 'react';
import { MemoryPanel } from './MemoryPanel';
import { KnowledgeBridge } from './KnowledgeBridge';
import { ResearchPanel } from './ResearchPanel';

export function MemoryTabs(): React.ReactElement {
  const [tab, setTab] = useState<'memory' | 'bridge' | 'research'>('memory');
  return (
    <div className="gf-workspace">
      <div className="gf-pane" style={{ flex: 1, minWidth: 0 }}>
        <div className="gf-pane-header">
          <span>Memory</span>
          <span className="gf-view-switcher" role="tablist" aria-label="Memory views">
            <button
              role="tab"
              aria-selected={tab === 'memory'}
              className={tab === 'memory' ? 'active' : ''}
              onClick={() => setTab('memory')}
            >
              Memory
            </button>
            <button
              role="tab"
              aria-selected={tab === 'bridge'}
              className={tab === 'bridge' ? 'active' : ''}
              onClick={() => setTab('bridge')}
            >
              Knowledge Bridge
            </button>
            <button
              role="tab"
              aria-selected={tab === 'research'}
              className={tab === 'research' ? 'active' : ''}
              onClick={() => setTab('research')}
            >
              Research
            </button>
          </span>
        </div>
        {tab === 'memory' && <MemoryPanel />}
        {tab === 'bridge' && <KnowledgeBridge />}
        {tab === 'research' && <ResearchPanel />}
      </div>
    </div>
  );
}
