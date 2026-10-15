// SPDX-License-Identifier: Apache-2.0
// System & Settings hub — overview, tools/registry, security, diagnostics,
// updates, and appearance in one multi-tab view.

import React, { useEffect, useState } from 'react';
import { invoke, isDesktop } from '../../lib/api';
import type { NavSection } from '../AppShell';
import type { ThemeMode } from '../SettingsDrawer';
import { ToolPanel } from '../tools/ToolPanel';
import { ConnectionsPanel } from '../connections/ConnectionsPanel';
import { McpPanel } from '../mcp/McpPanel';
import { SecurityView } from '../security/SecurityView';
import { DiagnosticsPanel } from '../diagnostics/DiagnosticsPanel';
import { UpdaterPanel } from '../diagnostics/UpdaterPanel';
import { CostPanel } from '../diagnostics/CostPanel';
import './hub.css';

interface Props {
  onToast: (t: string) => void;
  onNavigate: (s: NavSection) => void;
  theme: ThemeMode;
  onTheme: (t: ThemeMode) => void;
  onEditPrompts: () => void;
}

type HubTab = 'overview' | 'registry' | 'security' | 'usage' | 'diagnostics' | 'updates' | 'appearance';
type RegistryTab = 'tools' | 'connections' | 'mcp';

function OverviewPanel({ onNavigate }: { onNavigate: (s: NavSection) => void }): React.ReactElement {
  const [env, setEnv] = useState<Record<string, unknown> | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    void invoke<Record<string, unknown>>('detect_environment')
      .then(setEnv)
      .catch(() => undefined);
  }, []);

  return (
    <div className="gf-pane" style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: 16 }}>
      <p>
        Guild Foundry AI — AI-native developer workstation and collaboration
        environment.
      </p>
      <div className="gf-row" style={{ marginTop: 12, flexWrap: 'wrap' }}>
        <button className="gf-btn primary" onClick={() => onNavigate('agent')}>
          Open Personal Agent
        </button>
        <button className="gf-btn" onClick={() => onNavigate('projects')}>
          New Project
        </button>
        <button className="gf-btn" onClick={() => onNavigate('conference')}>
          New Conference
        </button>
      </div>
      <hr className="gf-divider" />
      <span className="gf-label">Environment</span>
      {!isDesktop() && (
        <p className="gf-muted">
          Browser preview mode. Native capabilities show "Desktop Capability
          Required".
        </p>
      )}
      {env ? (
        <pre
          style={{
            background: 'var(--gf-panel)',
            border: '1px solid var(--gf-border)',
            padding: 12,
            fontSize: 12,
            overflowX: 'auto',
          }}
        >
          {JSON.stringify(env, null, 2)}
        </pre>
      ) : (
        isDesktop() && <p className="gf-muted">Detecting environment…</p>
      )}
    </div>
  );
}

function AppearancePanel({ theme, onTheme, onNavigate, onEditPrompts }: {
  theme: ThemeMode;
  onTheme: (t: ThemeMode) => void;
  onNavigate: (s: NavSection) => void;
  onEditPrompts: () => void;
}): React.ReactElement {
  const setThemeAndPersist = async (t: ThemeMode): Promise<void> => {
    onTheme(t);
    try {
      await invoke('settings_set', { key: 'ui.theme', value: t });
    } catch {
      // Theme still applies for this session; persistence is best-effort.
    }
  };

  return (
    <div className="gf-pane" style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: 16 }}>
      <span className="gf-label">Appearance (default: Light)</span>
      <div className="gf-view-switcher" role="tablist" aria-label="Theme" style={{ alignSelf: 'flex-start' }}>
        <button role="tab" aria-selected={theme === 'light'} className={theme === 'light' ? 'active' : ''} onClick={() => void setThemeAndPersist('light')}>
          Light
        </button>
        <button role="tab" aria-selected={theme === 'dark'} className={theme === 'dark' ? 'active' : ''} onClick={() => void setThemeAndPersist('dark')}>
          Dark
        </button>
      </div>
      <p className="gf-muted" style={{ fontSize: 12 }}>
        White and blues, from true blue to royal blue, on deep blue-black surfaces.
      </p>
      <hr className="gf-divider" />
      <span className="gf-label">Credentials</span>
      <p className="gf-muted" style={{ fontSize: 12 }}>
        API keys live in the OS keyring under the Model & API Hub.
      </p>
      <div className="gf-row" style={{ flexWrap: 'wrap' }}>
        <button className="gf-btn" onClick={() => onNavigate('models-api')}>
          Open Model & API Hub
        </button>
        <button className="gf-btn" onClick={onEditPrompts}>
          Edit system prompts
        </button>
      </div>
    </div>
  );
}

export function SystemHub(props: Props): React.ReactElement {
  const { onToast } = props;
  const [tab, setTab] = useState<HubTab>('overview');
  const [registryTab, setRegistryTab] = useState<RegistryTab>('tools');

  return (
    <div className="gf-workspace">
      <div className="gf-pane" style={{ flex: 1, minWidth: 0, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        <div className="gf-pane-header">
          <span>System & Settings</span>
          <span className="gf-view-switcher" role="tablist" aria-label="System views">
            {(
              [
                ['overview', 'Overview'],
                ['registry', 'Tools & Registry'],
                ['security', 'Security'],
                ['usage', 'Usage & Cost'],
                ['diagnostics', 'Diagnostics'],
                ['updates', 'Updates'],
                ['appearance', 'Appearance'],
              ] as [HubTab, string][]
            ).map(([id, label]) => (
              <button key={id} role="tab" aria-selected={tab === id} className={tab === id ? 'active' : ''} onClick={() => setTab(id)}>
                {label}
              </button>
            ))}
          </span>
        </div>
        <div className="hub-fill">
          {tab === 'overview' && <OverviewPanel onNavigate={props.onNavigate} />}
          {tab === 'registry' && (
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
              <div className="hub-nested-tabs" role="tablist" aria-label="Registry views">
                {(
                  [
                    ['tools', 'Tools'],
                    ['connections', 'Connections'],
                    ['mcp', 'MCP Servers'],
                  ] as [RegistryTab, string][]
                ).map(([id, label]) => (
                  <button key={id} role="tab" aria-selected={registryTab === id} className={registryTab === id ? 'active' : ''} onClick={() => setRegistryTab(id)}>
                    {label}
                  </button>
                ))}
              </div>
              <div className="hub-fill">
                {registryTab === 'tools' && <ToolPanel onToast={onToast} />}
                {registryTab === 'connections' && <ConnectionsPanel onToast={onToast} />}
                {registryTab === 'mcp' && <McpPanel onToast={onToast} />}
              </div>
            </div>
          )}
          {tab === 'security' && <SecurityView onToast={onToast} />}
          {tab === 'usage' && <CostPanel onToast={onToast} />}
          {tab === 'diagnostics' && <DiagnosticsPanel onToast={onToast} />}
          {tab === 'updates' && <UpdaterPanel onToast={onToast} />}
          {tab === 'appearance' && (
            <AppearancePanel
              theme={props.theme}
              onTheme={props.onTheme}
              onNavigate={props.onNavigate}
              onEditPrompts={props.onEditPrompts}
            />
          )}
        </div>
      </div>
    </div>
  );
}
