// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: projects panel.
// Composes explorer + code editor + terminal + git + checkpoints over one
// sandboxed project root.
import React, { useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';
import { ProjectExplorer } from './ProjectExplorer';
import { CodeEditor, type AgentHighlight } from './CodeEditor';
import { Terminal } from './Terminal';
import { GitPanel } from './GitPanel';
import { CheckpointPanel } from './CheckpointPanel';
import './projects.css';

export function ProjectsPanel({
  onToast,
}: {
  onToast: (t: string) => void;
}): React.ReactElement {
  const [projectRoot, setProjectRoot] = useState<string>('');
  const [rootInput, setRootInput] = useState<string>('');
  const [openRequest, setOpenRequest] = useState<{ rel: string; nonce: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [agentHighlight, setAgentHighlight] = useState<AgentHighlight | null>(null);

  const loadDefaultRoot = async () => {
    try {
      const r = await invoke<string>('ws_project_root', { root: null });
      setProjectRoot(r);
      setRootInput(r);
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : err instanceof Error
            ? err.message
            : String(err),
      );
    }
  };

  useEffect(() => {
    void loadDefaultRoot();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const applyRoot = async () => {
    try {
      const r = await invoke<string>('ws_project_root', {
        root: rootInput.trim() || null,
      });
      setProjectRoot(r);
      setRootInput(r);
      setOpenRequest(null);
      setSelected(null);
      onToast('Project root set');
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  const openFile = (rel: string) => {
    setSelected(rel);
    setOpenRequest((prev) => ({ rel, nonce: (prev?.nonce ?? 0) + 1 }));
    // Agent-change highlights are cleared when the user opens another file;
    // Phase 6 agents set them via this panel's prop.
    setAgentHighlight(null);
  };

  if (!projectRoot) {
    return (
      <div className="gf-workspace">
        <div className="gf-pane" style={{ flex: 1, padding: 16 }}>
          <div className="gf-pane-header">
            <span>Projects</span>
          </div>
          <p className="gf-muted">Resolving project workspace…</p>
        </div>
      </div>
    );
  }

  return (
    <div className="gf-workspace" style={{ flexDirection: 'column' }}>
      <div
        className="gf-row"
        style={{
          padding: '6px 12px',
          borderBottom: '1px solid var(--gf-border)',
          background: 'var(--gf-panel)',
        }}
      >
        <span className="gf-label">Project root</span>
        <input
          className="gf-input gf-grow"
          style={{ fontFamily: 'var(--gf-mono)', fontSize: 12 }}
          value={rootInput}
          onChange={(e) => setRootInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void applyRoot();
          }}
          title="Absolute path on this machine. Everything (files, terminal, git) is sandboxed inside it."
        />
        <button className="gf-btn" onClick={() => void applyRoot()}>
          Set
        </button>
      </div>
      <div className="gf-projects" style={{ flex: 1, minHeight: 0 }}>
        <ProjectExplorer
          projectRoot={projectRoot}
          selected={selected}
          onOpenFile={openFile}
          onToast={onToast}
        />
        <div className="gf-projects-col" style={{ flex: 1 }}>
          <div style={{ flex: 2, display: 'flex', minHeight: 0 }}>
            <CodeEditor
              projectRoot={projectRoot}
              openRequest={openRequest}
              agentHighlight={agentHighlight}
              onToast={onToast}
            />
          </div>
          <div
            style={{
              flex: 1,
              display: 'flex',
              minHeight: 180,
              borderTop: '1px solid var(--gf-border)',
            }}
          >
            <Terminal projectRoot={projectRoot} onToast={onToast} />
          </div>
        </div>
        <div className="gf-projects-col" style={{ width: 380, minWidth: 300 }}>
          <div style={{ flex: 1, display: 'flex', minHeight: 0, borderBottom: '1px solid var(--gf-border)' }}>
            <GitPanel projectRoot={projectRoot} onToast={onToast} />
          </div>
          <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
            <CheckpointPanel projectRoot={projectRoot} onToast={onToast} />
          </div>
        </div>
      </div>
    </div>
  );
}
