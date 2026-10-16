// SPDX-License-Identifier: Apache-2.0
// System & Settings hub — overview, tools/registry, security, diagnostics,
// updates, and appearance in one multi-tab view.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke, isDesktop } from '../../lib/api';
import {
  COLOR_TOKENS,
  FONT_STACKS,
  TEXT_SECTIONS,
  applyAppearance,
  emptyAppearance,
  loadAppearance,
  saveAppearance,
  sectionPreviewStyle,
  type Appearance,
  type SectionType,
} from '../../lib/appearance';
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
  bubbleStyle: BubbleStyle;
  onBubbleStyle: (b: BubbleStyle) => void;
  onEditPrompts: () => void;
}

export type BubbleStyle = 'classic' | 'minimal' | 'card';

const BUBBLE_STYLES: { id: BubbleStyle; label: string; hint: string }[] = [
  { id: 'classic', label: 'Classic', hint: 'Colored bubble with a labeled header band (the original look).' },
  { id: 'minimal', label: 'Minimal', hint: 'Soft rounded bubble, speaker name as a quiet caption, soft shadow instead of a border.' },
  { id: 'card', label: 'Card', hint: 'Quiet surface block with a speaker-colored side rail (blue cloud, green local, grey you).' },
];

/** Three-state toggle: theme default → on → off → theme default. */
function Tri({
  value,
  onChange,
  labels,
}: {
  value?: boolean;
  onChange: (v?: boolean) => void;
  labels: [string, string, string];
}): React.ReactElement {
  return (
    <select
      className="gf-select"
      style={{ width: 'auto' }}
      value={value === undefined ? 'd' : value ? 'y' : 'n'}
      onChange={(e) => onChange(e.target.value === 'd' ? undefined : e.target.value === 'y')}
      title={labels.join(' / ')}
    >
      <option value="d">{labels[0]}</option>
      <option value="y">{labels[1]}</option>
      <option value="n">{labels[2]}</option>
    </select>
  );
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

function AppearancePanel({ theme, onTheme, bubbleStyle, onBubbleStyle, onNavigate, onEditPrompts }: {
  theme: ThemeMode;
  onTheme: (t: ThemeMode) => void;
  bubbleStyle: BubbleStyle;
  onBubbleStyle: (b: BubbleStyle) => void;
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

  const setBubbleStyleAndPersist = async (b: BubbleStyle): Promise<void> => {
    onBubbleStyle(b);
    try {
      await invoke('settings_set', { key: 'ui.bubble_style', value: b });
    } catch {
      // Applies for this session; persistence is best-effort.
    }
  };

  // ---- Theme studio: the user's own colours and type, over the base theme ----
  const [appearance, setAppearance] = useState<Appearance>(emptyAppearance());

  useEffect(() => {
    void loadAppearance().then((a) => {
      setAppearance(a);
      applyAppearance(a);
    });
  }, []);

  const updateAppearance = useCallback((next: Appearance) => {
    setAppearance(next);
    applyAppearance(next);
    void saveAppearance(next);
  }, []);

  const setColor = (key: string, value: string): void => {
    const colors = { ...appearance.colors };
    if (value) colors[key] = value;
    else delete colors[key];
    updateAppearance({ ...appearance, colors });
  };

  const setSection = (key: string, patch: Partial<SectionType>): void => {
    const merged: SectionType = { ...(appearance.sections[key] ?? {}), ...patch };
    (Object.keys(merged) as (keyof SectionType)[]).forEach((k) => {
      if (merged[k] === undefined) delete merged[k];
    });
    const sections = { ...appearance.sections };
    if (Object.keys(merged).length === 0) delete sections[key];
    else sections[key] = merged;
    updateAppearance({ ...appearance, sections });
  };

  const clearSection = (key: string): void => {
    const sections = { ...appearance.sections };
    delete sections[key];
    updateAppearance({ ...appearance, sections });
  };

  const resetAllAppearance = (): void => {
    if (!window.confirm('Reset every colour and font override back to the theme defaults?')) return;
    updateAppearance(emptyAppearance());
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
      <span className="gf-label">Chat bubbles</span>
      <div className="gf-view-switcher" role="tablist" aria-label="Chat bubble style" style={{ alignSelf: 'flex-start' }}>
        {BUBBLE_STYLES.map((s) => (
          <button
            key={s.id}
            role="tab"
            aria-selected={bubbleStyle === s.id}
            className={bubbleStyle === s.id ? 'active' : ''}
            onClick={() => void setBubbleStyleAndPersist(s.id)}
          >
            {s.label}
          </button>
        ))}
      </div>
      <p className="gf-muted" style={{ fontSize: 12 }}>
        {BUBBLE_STYLES.find((s) => s.id === bubbleStyle)?.hint ?? ''}
      </p>
      <hr className="gf-divider" />

      <span className="gf-label">Theme studio — colours (click a swatch for the full picker)</span>
      <div className="ap-grid">
        {COLOR_TOKENS.map((t) => {
          const value = appearance.colors[t.key];
          const isText =
            t.key === 'buttonText' || t.key === 'accent' || t.key === 'chatText' || t.key === 'mutedText';
          const swatch = /^#[0-9a-fA-F]{6}$/.test(value ?? '') ? (value as string) : '#2e7de9';
          return (
            <div className="ap-row" key={t.key}>
              <span className="ap-row-label" title={t.label}>
                {t.label}
                {!value && <em className="gf-muted"> · theme default</em>}
              </span>
              <span className="ap-sample" style={isText && value ? { color: value } : undefined}>
                {isText ? t.sample : ''}
                {!isText && (
                  <span
                    className="ap-swatch"
                    style={{
                      background: value || 'transparent',
                      borderColor: value || 'var(--gf-border)',
                      borderStyle: t.key === 'outline' ? 'dashed' : 'solid',
                    }}
                  />
                )}
              </span>
              <input
                className="ap-color"
                type="color"
                value={swatch}
                onChange={(e) => setColor(t.key, e.target.value)}
                title="Full colour wheel"
              />
              <input
                className="gf-input ap-hex"
                value={value ?? ''}
                placeholder="#rrggbb"
                onChange={(e) => setColor(t.key, e.target.value.trim())}
              />
              <button className="gf-btn gf-btn-sm" onClick={() => setColor(t.key, '')} disabled={!value} title="Back to the theme default">
                Reset
              </button>
            </div>
          );
        })}
      </div>

      <hr className="gf-divider" />
      <span className="gf-label">Theme studio — fonts</span>
      <div className="gf-row" style={{ flexWrap: 'wrap', gap: 10 }}>
        <label className="gf-label" style={{ margin: 0 }}>
          Base font
          <select
            className="gf-select"
            value={appearance.baseFont ?? 'system'}
            onChange={(e) =>
              updateAppearance({ ...appearance, baseFont: e.target.value === 'system' ? undefined : e.target.value })
            }
          >
            {FONT_STACKS.map((f) => (
              <option key={f.id} value={f.id}>
                {f.label}
              </option>
            ))}
          </select>
        </label>
        <label className="gf-label" style={{ margin: 0 }}>
          Base size (px)
          <input
            className="gf-input"
            type="number"
            min={10}
            max={22}
            step={0.5}
            style={{ width: 92 }}
            value={appearance.baseSize ?? ''}
            placeholder="default"
            onChange={(e) =>
              updateAppearance({ ...appearance, baseSize: e.target.value ? Number(e.target.value) : undefined })
            }
          />
        </label>
      </div>
      <p className="gf-muted" style={{ fontSize: 12 }}>
        The base font and size apply everywhere; every section below can override them. Forum is bundled, and more
        open-source families ship with the app — pick per section.
      </p>

      {TEXT_SECTIONS.map((s) => {
        const v = appearance.sections[s.key] ?? {};
        return (
          <div className="ap-section" key={s.key}>
            <div className="ap-section-head">
              <div>
                <strong>{s.label}</strong>
                <div className="gf-muted" style={{ fontSize: 11 }}>
                  {s.note}
                  {Object.keys(v).length > 0 ? ' · customised' : ''}
                </div>
              </div>
              <button className="gf-btn gf-btn-sm" onClick={() => clearSection(s.key)} disabled={Object.keys(v).length === 0}>
                Reset
              </button>
            </div>
            <div className="ap-section-preview" style={sectionPreviewStyle(v)}>
              {s.sample}
            </div>
            <div className="gf-row" style={{ gap: 6, flexWrap: 'wrap' }}>
              <select
                className="gf-select"
                value={v.family ?? 'system'}
                onChange={(e) => setSection(s.key, { family: e.target.value === 'system' ? undefined : e.target.value })}
                title="Font family for this section"
              >
                {FONT_STACKS.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.label}
                  </option>
                ))}
              </select>
              <input
                className="gf-input"
                type="number"
                min={8}
                max={34}
                step={0.5}
                style={{ width: 78 }}
                value={v.size ?? ''}
                placeholder="size"
                onChange={(e) => setSection(s.key, { size: e.target.value ? Number(e.target.value) : undefined })}
                title="Size in px (blank = theme size)"
              />
              <Tri value={v.bold} onChange={(b) => setSection(s.key, { bold: b })} labels={['weight: default', 'bold', 'regular']} />
              <Tri value={v.italic} onChange={(b) => setSection(s.key, { italic: b })} labels={['style: default', 'italic', 'upright']} />
              <Tri value={v.strike} onChange={(b) => setSection(s.key, { strike: b })} labels={['strike: default', 'struck through', 'plain']} />
            </div>
          </div>
        );
      })}

      <div className="gf-row" style={{ marginTop: 12 }}>
        <button className="gf-btn danger" onClick={resetAllAppearance}>
          Reset the whole theme
        </button>
      </div>

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
              bubbleStyle={props.bubbleStyle}
              onBubbleStyle={props.onBubbleStyle}
              onNavigate={props.onNavigate}
              onEditPrompts={props.onEditPrompts}
            />
          )}
        </div>
      </div>
    </div>
  );
}
