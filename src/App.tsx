import React, { useEffect, useState } from 'react';
import './theme.css';
import './a11y.css';
import type { ViewMode } from './lib/types';
import { AppShell, useToasts, type NavSection } from './components/AppShell';
import { SettingsDrawer } from './components/SettingsDrawer';
import { ProjectsPanel } from './components/projects/ProjectsPanel';
import { ProvidersPanel } from './components/providers/ProvidersPanel';
import { ModelCatalog } from './components/models/ModelCatalog';
import { ConferenceRoom } from './components/conference/ConferenceRoom';
import BuilderStudio from './components/builder/BuilderStudio';
import RunHistory from './components/builder/RunHistory';
import { ToolPanel } from './components/tools/ToolPanel';
import { McpPanel } from './components/mcp/McpPanel';
import { ConnectionsPanel } from './components/connections/ConnectionsPanel';
import { ScheduledPanel } from './components/scheduler/ScheduledPanel';
import { SchedulerRunner } from './components/scheduler/SchedulerRunner';
import { MemoryPanel } from './components/memory/MemoryPanel';
import { KnowledgeBridge } from './components/memory/KnowledgeBridge';
import { ResearchPanel } from './components/memory/ResearchPanel';
import { ArtifactsTabs } from './components/artifacts/ArtifactsTabs';
import { SecurityView } from './components/security/SecurityView';
import { DiagnosticsPanel } from './components/diagnostics/DiagnosticsPanel';
import { UpdaterPanel } from './components/diagnostics/UpdaterPanel';
import { CrashRecovery } from './components/diagnostics/CrashRecovery';
import { invoke, isDesktop, DesktopCapabilityRequired } from './lib/api';

// All 12 build phases have landed. Every section below renders its real
// implementation; nothing is mocked.

const SECTION_PHASE: Record<NavSection, number> = {
  home: 1,
  builder: 5,
  conference: 3,
  projects: 4,
  providers: 2,
  models: 2,
  tools: 6,
  mcp: 7,
  connections: 7,
  scheduled: 14,
  memory: 10,
  artifacts: 8,
  runs: 5,
  security: 9,
  diagnostics: 11,
  updates: 11,
  settings: 1,
};

const SECTION_TITLE: Record<NavSection, string> = {
  home: 'Home',
  builder: 'App Builder Studio',
  conference: 'Conference Room',
  projects: 'Projects',
  providers: 'Providers',
  models: 'Models',
  tools: 'Tools',
  mcp: 'MCP Servers',
  connections: 'Connections',
  scheduled: 'Scheduled',
  memory: 'Memory',
  artifacts: 'Artifacts',
  runs: 'Runs',
  security: 'Security',
  diagnostics: 'Diagnostics',
  updates: 'Updates',
  settings: 'Settings',
};

function HomePanel({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [env, setEnv] = useState<Record<string, unknown> | null>(null);

  useEffect(() => {
    if (!isDesktop()) return;
    void invoke<Record<string, unknown>>('detect_environment')
      .then(setEnv)
      .catch(() => undefined);
  }, []);

  return (
    <div className="gf-workspace">
      <div className="gf-pane" style={{ flex: 1, padding: 16, overflowY: 'auto' }}>
        <div className="gf-pane-header">
          <span>Home</span>
        </div>
        <p>
          Guild Foundry AI — AI-native developer workstation and collaboration
          environment.
        </p>
        <div className="gf-row" style={{ marginTop: 12, flexWrap: 'wrap' }}>
          <button
            className="gf-btn primary"
            onClick={() => onToast('Use the Projects section to start a project')}
          >
            New Project
          </button>
          <button
            className="gf-btn"
            onClick={() => onToast('Use the Conference Room section')}
          >
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
    </div>
  );
}

function MemoryTabs(): React.ReactElement {
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

export default function App(): React.ReactElement {
  const [section, setSection] = useState<NavSection>('home');
  const [viewMode, setViewMode] = useState<ViewMode>('desktop');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [connectionState, setConnectionState] = useState('IDLE');
  const [runsRefresh, setRunsRefresh] = useState(0);
  const { toasts, pushToast } = useToasts();

  const openSection = (s: NavSection): void => {
    if (s === 'settings') {
      setSettingsOpen(true);
      return;
    }
    setSettingsOpen(false);
    setSection(s);
  };

  useEffect(() => {
    if (!isDesktop()) return;
    void invoke<{ connected: boolean }>('providers_any_connected')
      .then((r) => setConnectionState(r.connected ? 'CONNECTED' : 'IDLE'))
      .catch((err) => {
        if (!(err instanceof DesktopCapabilityRequired)) setConnectionState('IDLE');
      });
  }, []);

  return (
    <AppShell
      section={section}
      onSection={openSection}
      viewMode={viewMode}
      onViewMode={setViewMode}
      toasts={toasts}
      pushToast={pushToast}
      connectionState={connectionState}
    >
      <SchedulerRunner onToast={pushToast} />
      <CrashRecovery
        onToast={pushToast}
        onDecide={(d) => {
          pushToast(`Recovery: ${d.action}`);
          if (d.action === 'resume') setRunsRefresh((n) => n + 1);
        }}
      />
      {settingsOpen ? (
        <div className="gf-workspace">
          <div style={{ flex: 1 }} />
          <SettingsDrawer
            open={settingsOpen}
            onClose={() => setSettingsOpen(false)}
            onToast={pushToast}
          />
        </div>
      ) : section === 'home' ? (
        <HomePanel onToast={pushToast} />
      ) : section === 'builder' ? (
        <BuilderStudio />
      ) : section === 'conference' ? (
        <ConferenceRoom onToast={pushToast} />
      ) : section === 'projects' ? (
        <ProjectsPanel onToast={pushToast} />
      ) : section === 'providers' ? (
        <ProvidersPanel onToast={pushToast} />
      ) : section === 'models' ? (
        <ModelCatalog onToast={pushToast} />
      ) : section === 'tools' ? (
        <ToolPanel onToast={pushToast} />
      ) : section === 'mcp' ? (
        <McpPanel onToast={pushToast} />
      ) : section === 'connections' ? (
        <ConnectionsPanel onToast={pushToast} />
      ) : section === 'scheduled' ? (
        <ScheduledPanel onToast={pushToast} />
      ) : section === 'memory' ? (
        <MemoryTabs />
      ) : section === 'artifacts' ? (
        <ArtifactsTabs onToast={pushToast} />
      ) : section === 'runs' ? (
        <div className="gf-workspace">
          <div className="gf-pane" style={{ flex: 1, minWidth: 0 }}>
            <div className="gf-pane-header">
              <span>Runs</span>
            </div>
            <RunHistory
              refreshKey={runsRefresh}
              onSelect={() => setSection('builder')}
            />
          </div>
        </div>
      ) : section === 'security' ? (
        <SecurityView />
      ) : section === 'diagnostics' ? (
        <div className="gf-workspace">
          <div className="gf-pane" style={{ flex: 1, minWidth: 0 }}>
            <div className="gf-pane-header">
              <span>Diagnostics</span>
            </div>
            <DiagnosticsPanel onToast={pushToast} />
          </div>
        </div>
      ) : section === 'updates' ? (
        <div className="gf-workspace">
          <div className="gf-pane" style={{ flex: 1, minWidth: 0 }}>
            <div className="gf-pane-header">
              <span>Updates</span>
            </div>
            <UpdaterPanel onToast={pushToast} />
          </div>
        </div>
      ) : (
        <div className="gf-workspace">
          <div className="gf-pane" style={{ flex: 1, padding: 16 }}>
            <div className="gf-pane-header">
              <span>{SECTION_TITLE[section]}</span>
            </div>
            <p className="gf-muted">Unknown section.</p>
          </div>
        </div>
      )}
    </AppShell>
  );
}
