import React, { useEffect, useState } from 'react';
import './theme.css';
import './a11y.css';
import type { ViewMode } from './lib/types';
import { AppShell, useToasts, type NavSection } from './components/AppShell';
import { SettingsDrawer } from './components/SettingsDrawer';
import { AgentHarness } from './components/agent/AgentHarness';
import { BuilderHub } from './components/hubs/BuilderHub';
import { ModelsApiHub } from './components/hubs/ModelsApiHub';
import { SystemHub, type BubbleStyle } from './components/hubs/SystemHub';
import { ProjectsPanel } from './components/projects/ProjectsPanel';
import { ConferenceRoom } from './components/conference/ConferenceRoom';
import { SchedulerRunner } from './components/scheduler/SchedulerRunner';
import { CrashRecovery } from './components/diagnostics/CrashRecovery';
import { invoke, isDesktop, DesktopCapabilityRequired } from './lib/api';

// Six primary hubs. Former single-purpose views live as interior tabs inside
// their hub; every section below renders its real implementation.

const SECTION_PHASE: Record<NavSection, number> = {
  agent: 15,
  builder: 5,
  conference: 3,
  projects: 4,
  'models-api': 2,
  'settings-system': 11,
};

const SECTION_TITLE: Record<NavSection, string> = {
  agent: 'Personal Agent',
  builder: 'App Builder Studio',
  conference: 'Conference Room',
  projects: 'Projects IDE',
  'models-api': 'Model & API Hub',
  'settings-system': 'System & Settings',
};

export default function App(): React.ReactElement {
  const [section, setSection] = useState<NavSection>('agent');
  const [viewMode, setViewMode] = useState<ViewMode>('desktop');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [connectionState, setConnectionState] = useState('IDLE');
  // Light is the default theme (white and blues on deep blue-black).
  const [theme, setTheme] = useState<'light' | 'dark'>('light');
  // Chat bubble aesthetic (Settings → Appearance); Minimal is the owner's
  // preferred default, Classic and Card remain one click away.
  const [bubbleStyle, setBubbleStyle] = useState<BubbleStyle>('minimal');
  const { toasts, pushToast } = useToasts();

  const openSection = (s: NavSection): void => {
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
    // Restore saved theme; default stays light when nothing is stored.
    void invoke<string | null>('settings_get', { key: 'ui.theme' })
      .then((v) => {
        if (v === 'dark' || v === 'light') setTheme(v);
      })
      .catch(() => undefined);
    void invoke<string | null>('settings_get', { key: 'ui.bubble_style' })
      .then((v) => {
        if (v === 'classic' || v === 'minimal' || v === 'card') setBubbleStyle(v);
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
  }, [theme]);

  useEffect(() => {
    document.documentElement.setAttribute('data-bubbles', bubbleStyle);
  }, [bubbleStyle]);

  return (
    <div data-theme={theme} data-bubbles={bubbleStyle} style={{ height: '100%', width: '100%' }}>
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
        }}
      />
      {settingsOpen ? (
        <div className="gf-workspace">
          <div style={{ flex: 1 }} />
          <SettingsDrawer
            open={settingsOpen}
            onClose={() => setSettingsOpen(false)}
            onToast={pushToast}
            onNavigate={(s) => {
              setSettingsOpen(false);
              openSection(s);
            }}
          />
        </div>
      ) : section === 'agent' ? (
        <AgentHarness onToast={pushToast} onNavigate={openSection} />
      ) : section === 'builder' ? (
        <BuilderHub onToast={pushToast} />
      ) : section === 'conference' ? (
        <ConferenceRoom onToast={pushToast} />
      ) : section === 'projects' ? (
        <ProjectsPanel onToast={pushToast} />
      ) : section === 'models-api' ? (
        <ModelsApiHub onToast={pushToast} />
      ) : section === 'settings-system' ? (
        <SystemHub
          onToast={pushToast}
          onNavigate={openSection}
          theme={theme}
          onTheme={setTheme}
          bubbleStyle={bubbleStyle}
          onBubbleStyle={setBubbleStyle}
          onEditPrompts={() => setSettingsOpen(true)}
        />
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
    </div>
  );
}
