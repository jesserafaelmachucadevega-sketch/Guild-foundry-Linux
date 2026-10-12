import React, { useCallback, useState } from 'react';
import type { ToastMsg, ViewMode } from '../lib/types';
import { invoke, isDesktop } from '../lib/api';

import builderIcon from '../assets/nav/builder.png';
import conferenceIcon from '../assets/nav/conference.png';
import projectsIcon from '../assets/nav/projects.png';
import settingsIcon from '../assets/nav/settings.png';
import agentIcon from '../assets/nav/agent.png';
import modelsIcon from '../assets/nav/models.png';
import appLogo from '../assets/logo/foundry-logo.png';

// Master spec Section 5: six primary hubs. Former single-purpose views live
// as interior tabs inside their hub (Builder, Model & API, System).
export type NavSection =
  | 'agent'
  | 'builder'
  | 'conference'
  | 'projects'
  | 'models-api'
  | 'settings-system';

export const NAV_ITEMS: { id: NavSection; label: string; fullName: string; detail: string; icon: string; phase: number }[] = [
  { id: 'agent', label: 'Agent', fullName: 'Personal Agent', detail: 'Daily companion: chat, tasks, mail, reminders, media.', icon: agentIcon, phase: 15 },
  { id: 'builder', label: 'Builder', fullName: 'App Builder Studio', detail: 'Multi-agent builds: workspace, runs, scheduled, artifacts.', icon: builderIcon, phase: 5 },
  { id: 'conference', label: 'Conf.', fullName: 'Conference Room', detail: '1–4 models debate, vote, and synthesize answers.', icon: conferenceIcon, phase: 3 },
  { id: 'projects', label: 'Proj.', fullName: 'Projects IDE', detail: 'Files, editor, terminal, Git, and checkpoints.', icon: projectsIcon, phase: 4 },
  { id: 'models-api', label: 'Models', fullName: 'Model & API Hub', detail: 'Providers, keyring credentials, catalog, router, aliases.', icon: modelsIcon, phase: 2 },
  { id: 'settings-system', label: 'System', fullName: 'System & Settings', detail: 'Tools, registry, security, diagnostics, updates, theme.', icon: settingsIcon, phase: 11 },
];

interface Props {
  section: NavSection;
  onSection: (s: NavSection) => void;
  viewMode: ViewMode;
  onViewMode: (v: ViewMode) => void;
  toasts: ToastMsg[];
  pushToast: (text: string) => void;
  connectionState: string;
  children: React.ReactNode;
}

const VIEW_MODES: { id: ViewMode; label: string }[] = [
  { id: 'desktop', label: 'Desktop' },
  { id: 'compact', label: 'Compact Window' },
  { id: 'pwa', label: 'PWA' },
];

const PALETTE_COMMANDS: { label: string; target: NavSection }[] = [
  { label: 'Open Personal Agent', target: 'agent' },
  { label: 'Open Project', target: 'projects' },
  { label: 'New Conference', target: 'conference' },
  { label: 'Connect Provider', target: 'models-api' },
  { label: 'Run Build', target: 'builder' },
  { label: 'Open Settings', target: 'settings-system' },
];

export function AppShell(props: Props): React.ReactElement {
  const { section, onSection, viewMode, onViewMode, connectionState } = props;
  const [paletteOpen, setPaletteOpen] = useState(false);

  const switchView = useCallback(
    async (mode: ViewMode) => {
      onViewMode(mode);
      if (isDesktop() && mode !== 'pwa') {
        try {
          await invoke('window_set_compact', { compact: mode === 'compact' });
        } catch {
          /* window control unavailable — CSS still adapts */
        }
      }
    },
    [onViewMode],
  );

  const activeItem = NAV_ITEMS.find((i) => i.id === section);

  return (
    <div className={`gf-app view-${viewMode}`}>
      <nav className="gf-nav-rail" aria-label="Sections">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            className={`gf-nav-btn ${section === item.id ? 'active' : ''}`}
            onClick={() => onSection(item.id)}
            title={`${item.fullName} — ${item.detail} (Phase ${item.phase})`}
            aria-label={item.fullName}
          >
            <span className="gf-nav-glyph"><img src={item.icon} alt="" width={28} height={28} draggable={false} /></span>
            <span className="gf-nav-text">
              <span className="gf-nav-label">{item.fullName}</span>
              <span className="gf-nav-detail">{item.detail}</span>
            </span>
          </button>
        ))}
      </nav>

      <div className="gf-main">
        <header className="gf-header">
          <div className="gf-row">
            <img src={appLogo} alt="Agent Guild Foundry" className="gf-logo" draggable={false} />
            <strong className="gf-gold-text">Guild Foundry AI</strong>
            <span className="gf-muted" style={{ fontSize: 12 }}>
              {activeItem?.fullName ?? ''}
            </span>
            <span
              className={`gf-badge ${connectionState === 'CONNECTED' ? 'ok' : 'idle'}`}
              style={{ marginLeft: 8 }}
            >
              {connectionState}
            </span>
          </div>
          <div className="gf-row">
            <button
              className="gf-icon-btn"
              title="Command palette"
              onClick={() => setPaletteOpen((o) => !o)}
            >
              Cmd
            </button>
            <div className="gf-view-switcher" role="tablist" aria-label="View mode">
              {VIEW_MODES.map((m) => (
                <button
                  key={m.id}
                  role="tab"
                  aria-selected={viewMode === m.id}
                  className={viewMode === m.id ? 'active' : ''}
                  onClick={() => void switchView(m.id)}
                >
                  {m.label}
                </button>
              ))}
            </div>
          </div>
        </header>
        {paletteOpen && (
          <div className="gf-pane" style={{ borderBottom: '1px solid var(--gf-border)', padding: 8 }}>
            <span className="gf-label">Command palette</span>
            <div className="gf-row" style={{ flexWrap: 'wrap' }}>
              {PALETTE_COMMANDS.map((c) => (
                <button
                  key={c.label}
                  className="gf-btn"
                  onClick={() => {
                    onSection(c.target);
                    setPaletteOpen(false);
                  }}
                >
                  {c.label}
                </button>
              ))}
            </div>
          </div>
        )}
        {props.children}
      </div>

      <div className="gf-toasts">
        {props.toasts.map((t) => (
          <div key={t.id} className="gf-toast">
            {t.text}
          </div>
        ))}
      </div>
    </div>
  );
}

export function useToasts(): { toasts: ToastMsg[]; pushToast: (text: string) => void } {
  const [toasts, setToasts] = useState<ToastMsg[]>([]);
  const pushToast = useCallback((text: string) => {
    const id = Date.now() + Math.random();
    setToasts((prev) => [...prev, { id, text }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 2400);
  }, []);
  return { toasts, pushToast };
}
