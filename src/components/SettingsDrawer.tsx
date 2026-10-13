import React, { useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../lib/api';
import type { NavSection } from './AppShell';

// Master spec Sections 11 + 57: editable, versioned system prompt fields per
// room/agent. Appearance lives in the System hub; credentials live in the
// Model & API hub — this drawer keeps the prompts only.

const PROMPT_FIELDS = [
  { id: 'agent-harness', label: 'Personal Agent (Agent tab)' },
  { id: 'conference-1', label: 'Conference Model 1' },
  { id: 'conference-2', label: 'Conference Model 2' },
  { id: 'conference-3', label: 'Conference Model 3' },
  { id: 'conference-4', label: 'Conference Model 4' },
  { id: 'supervisor', label: 'Supervisor' },
  { id: 'architect', label: 'Architect' },
  { id: 'frontend', label: 'Frontend Engineer' },
  { id: 'backend', label: 'Backend Engineer' },
  { id: 'qa', label: 'QA Engineer' },
  { id: 'security', label: 'Security Reviewer' },
  { id: 'synthesizer', label: 'Synthesizer' },
  { id: 'researcher', label: 'Researcher' },
];

export type ThemeMode = 'light' | 'dark';

interface Props {
  open: boolean;
  onClose: () => void;
  onToast: (text: string) => void;
  onNavigate: (s: NavSection) => void;
}

export function SettingsDrawer({ open, onClose, onToast, onNavigate }: Props): React.ReactElement | null {
  const [prompts, setPrompts] = useState<Record<string, string>>({});
  const [promptsSaved, setPromptsSaved] = useState<Record<string, string>>({});

  useEffect(() => {
    if (!open || !isDesktop()) return;
    void invoke<Record<string, string>>('settings_load_prompts')
      .then((p) => {
        setPrompts(p ?? {});
        setPromptsSaved(p ?? {});
      })
      .catch(() => undefined);
  }, [open]);

  if (!open) return null;

  const savePrompts = async (): Promise<void> => {
    try {
      await invoke('settings_save_prompts', { prompts });
      setPromptsSaved({ ...prompts });
      onToast('System prompts saved');
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : 'Save failed',
      );
    }
  };

  const discardPrompts = (): void => {
    setPrompts({ ...promptsSaved });
    onToast('Prompt edits discarded');
  };

  return (
    <aside className="gf-drawer" aria-label="Settings">
      <div className="gf-pane-header">
        <span>Settings</span>
        <button className="gf-icon-btn" onClick={onClose}>
          Close
        </button>
      </div>
      <div style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 14 }}>
        <section>
          <span className="gf-label">Moved to hubs</span>
          <p className="gf-muted" style={{ fontSize: 12 }}>
            Appearance lives in System & Settings → Appearance. API keys live in
            the Model & API Hub.
          </p>
          <div className="gf-row" style={{ flexWrap: 'wrap' }}>
            <button
              className="gf-btn"
              onClick={() => {
                onClose();
                onNavigate('models-api');
              }}
            >
              Open Model & API Hub
            </button>
            <button
              className="gf-btn"
              onClick={() => {
                onClose();
                onNavigate('settings-system');
              }}
            >
              Open System & Settings
            </button>
          </div>
        </section>

        <hr className="gf-divider" />

        <section>
          <span className="gf-label">System prompts (editable, versioned)</span>
          {PROMPT_FIELDS.map((f) => (
            <div key={f.id} style={{ marginBottom: 8 }}>
              <label className="gf-label" htmlFor={`prompt-${f.id}`}>
                {f.label}
              </label>
              <textarea
                id={`prompt-${f.id}`}
                className="gf-textarea"
                rows={3}
                value={prompts[f.id] ?? ''}
                onChange={(e) => setPrompts((p) => ({ ...p, [f.id]: e.target.value }))}
              />
            </div>
          ))}
          <div className="gf-row">
            <button className="gf-btn primary" onClick={() => void savePrompts()}>
              Save prompts
            </button>
            <button className="gf-btn" onClick={discardPrompts}>
              Discard changes
            </button>
          </div>
        </section>

        <button className="gf-btn" onClick={onClose}>
          Done
        </button>
      </div>
    </aside>
  );
}
