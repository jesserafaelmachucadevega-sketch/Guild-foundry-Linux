import React, { useCallback, useState } from 'react';
import type { ToastMsg, ViewMode } from '../lib/types';
import { invoke, isDesktop } from '../lib/api';

// Master spec Section 5: left navigation across all product areas.
export type NavSection =
  | 'home'
  | 'builder'
  | 'conference'
  | 'projects'
  | 'providers'
  | 'models'
  | 'tools'
  | 'mcp'
  | 'connections'
  | 'scheduled'
  | 'memory'
  | 'artifacts'
  | 'runs'
  | 'security'
  | 'diagnostics'
  | 'updates'
  | 'settings';

export const NAV_ITEMS: { id: NavSection; label: string; fullName: string; detail: string; glyph: string; phase: number }[] = [
  { id: 'home', label: 'Home', fullName: 'Home', detail: 'Start here: environment status and quick entry points.', glyph: 'H', phase: 1 },
  { id: 'builder', label: 'Builder', fullName: 'App Builder Studio', detail: 'Multi-agent builds with approval gates and task graph.', glyph: 'B', phase: 5 },
  { id: 'conference', label: 'Conf.', fullName: 'Conference Room', detail: '1–4 models debate, vote, and synthesize answers.', glyph: 'C', phase: 3 },
  { id: 'projects', label: 'Proj.', fullName: 'Projects', detail: 'Files, editor, terminal, Git, and checkpoints.', glyph: 'P', phase: 4 },
  { id: 'providers', label: 'Prov.', fullName: 'Providers', detail: 'LLM API connections, keys in OS keyring, handshake.', glyph: 'V', phase: 2 },
  { id: 'models', label: 'Models', fullName: 'Models', detail: 'Catalog, favorites, aliases, and model router.', glyph: 'M', phase: 2 },
  { id: 'tools', label: 'Tools', fullName: 'Tools', detail: 'Tool execution loop and capability handshake.', glyph: 'T', phase: 6 },
  { id: 'mcp', label: 'MCP', fullName: 'MCP Servers', detail: 'Raw MCP servers, tools, auth, and test calls.', glyph: 'X', phase: 7 },
  { id: 'connections', label: 'Conn.', fullName: 'Connections', detail: 'One-click Gmail, Drive, GitHub, and app access.', glyph: 'N', phase: 7 },
  { id: 'scheduled', label: 'Sched.', fullName: 'Scheduled', detail: 'Scheduled agent runs and background jobs.', glyph: 'W', phase: 14 },
  { id: 'memory', label: 'Mem.', fullName: 'Memory', detail: 'Model-isolated memory, bridge, and research.', glyph: 'E', phase: 10 },
  { id: 'artifacts', label: 'Artif.', fullName: 'Artifacts', detail: 'Builds, templates, releases, and exports.', glyph: 'A', phase: 8 },
  { id: 'runs', label: 'Runs', fullName: 'Runs', detail: 'Agent run history and recovery.', glyph: 'R', phase: 5 },
  { id: 'security', label: 'Sec.', fullName: 'Security', detail: 'Constitution, audit log, and safety checks.', glyph: 'S', phase: 9 },
  { id: 'diagnostics', label: 'Diag.', fullName: 'Diagnostics', detail: 'System health, tokens, network, and workers.', glyph: 'D', phase: 11 },
  { id: 'updates', label: 'Upd.', fullName: 'Updates', detail: 'Update feed, staging, and explicit install.', glyph: 'U', phase: 11 },
  { id: 'settings', label: 'Setup', fullName: 'Settings', detail: 'Theme, credentials, and system prompts.', glyph: '*', phase: 1 },
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
  { label: 'Open Project', target: 'projects' },
  { label: 'New Conference', target: 'conference' },
  { label: 'Connect Provider', target: 'providers' },
  { label: 'Run Build', target: 'builder' },
  { label: 'Open Settings', target: 'settings' },
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
            <span className="gf-nav-glyph" style={{ fontSize: 18 }}>{item.glyph}</span>
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
