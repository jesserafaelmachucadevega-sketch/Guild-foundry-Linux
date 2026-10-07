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
  | 'memory'
  | 'artifacts'
  | 'runs'
  | 'security'
  | 'diagnostics'
  | 'updates'
  | 'settings';

export const NAV_ITEMS: { id: NavSection; label: string; glyph: string; phase: number }[] = [
  { id: 'home', label: 'Home', glyph: 'H', phase: 1 },
  { id: 'builder', label: 'Builder', glyph: 'B', phase: 5 },
  { id: 'conference', label: 'Conf.', glyph: 'C', phase: 3 },
  { id: 'projects', label: 'Proj.', glyph: 'P', phase: 4 },
  { id: 'providers', label: 'Prov.', glyph: 'V', phase: 2 },
  { id: 'models', label: 'Models', glyph: 'M', phase: 2 },
  { id: 'tools', label: 'Tools', glyph: 'T', phase: 6 },
  { id: 'mcp', label: 'MCP', glyph: 'X', phase: 7 },
  { id: 'connections', label: 'Conn.', glyph: 'N', phase: 7 },
  { id: 'memory', label: 'Mem.', glyph: 'E', phase: 10 },
  { id: 'artifacts', label: 'Artif.', glyph: 'A', phase: 8 },
  { id: 'runs', label: 'Runs', glyph: 'R', phase: 5 },
  { id: 'security', label: 'Sec.', glyph: 'S', phase: 9 },
  { id: 'diagnostics', label: 'Diag.', glyph: 'D', phase: 11 },
  { id: 'updates', label: 'Upd.', glyph: 'U', phase: 11 },
  { id: 'settings', label: 'Setup', glyph: '*', phase: 1 },
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
            title={`${item.label} (Phase ${item.phase})`}
          >
            <span style={{ fontSize: 18 }}>{item.glyph}</span>
            <span>{item.label}</span>
          </button>
        ))}
      </nav>

      <div className="gf-main">
        <header className="gf-header">
          <div className="gf-row">
            <strong className="gf-gold-text">Guild Foundry AI</strong>
            <span className="gf-muted" style={{ fontSize: 12 }}>
              {activeItem?.label ?? ''}
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
            <span className="gf-label">Command palette — full command set lands in Phase 5</span>
            <div className="gf-row" style={{ flexWrap: 'wrap' }}>
              {['Open Project', 'New Conference', 'Connect Provider', 'Run Build', 'Open Settings'].map(
                (c) => (
                  <button
                    key={c}
                    className="gf-btn"
                    onClick={() => {
                      props.pushToast(`${c} — lands in its build phase`);
                      setPaletteOpen(false);
                    }}
                  >
                    {c}
                  </button>
                ),
              )}
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
