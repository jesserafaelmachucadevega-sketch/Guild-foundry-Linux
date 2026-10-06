import React, { useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../lib/api';

// Master spec Sections 11 + 57: secure credential inputs (masked) + editable,
// versioned system prompt fields per room/agent.

const PROMPT_FIELDS = [
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

const CREDENTIAL_FIELDS = [
  { id: 'openai', label: 'OpenAI API key' },
  { id: 'anthropic', label: 'Anthropic API key' },
  { id: 'google', label: 'Google AI API key' },
  { id: 'openrouter', label: 'OpenRouter API key' },
  { id: 'ollama', label: 'Ollama base URL' },
];

interface Props {
  open: boolean;
  onClose: () => void;
  onToast: (text: string) => void;
}

export function SettingsDrawer({ open, onClose, onToast }: Props): React.ReactElement | null {
  const [prompts, setPrompts] = useState<Record<string, string>>({});
  const [secrets, setSecrets] = useState<Record<string, string>>({});
  const [connState, setConnState] = useState<Record<string, string>>({});

  useEffect(() => {
    if (!open || !isDesktop()) return;
    void invoke<Record<string, string>>('settings_load_prompts')
      .then((p) => setPrompts(p ?? {}))
      .catch(() => undefined);
  }, [open ]);

  if (!open) return null;

  const savePrompts = async (): Promise<void> => {
    try {
      await invoke('settings_save_prompts', { prompts });
      onToast('System prompts saved');
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : 'Save failed',
      );
    }
  };

  const connectProvider = async (id: string): Promise<void> => {
    const value = secrets[id];
    if (!value) {
      onToast('Enter a value first');
      return;
    }
    try {
      await invoke('secret_set', { key: `provider:${id}`, value });
      const res = await invoke<{ ok: boolean; code?: number | string; detail?: string }>(
        'handshake_provider',
        { provider_id: id },
      );
      if (res.ok) {
        setConnState((s) => ({ ...s, [id]: 'CONNECTED' }));
        onToast(`${id}: CONNECTED`);
      } else {
        setConnState((s) => ({ ...s, [id]: `ERROR ${res.code ?? ''}` }));
        onToast(`${id}: ERROR ${res.code ?? ''} — ${res.detail ?? ''}`);
      }
      setSecrets((s) => ({ ...s, [id]: '' }));
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : 'Connect failed',
      );
    }
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
          <span className="gf-label">Credentials (masked, keyring-backed)</span>
          {CREDENTIAL_FIELDS.map((f) => (
            <div key={f.id} style={{ marginBottom: 8 }}>
              <label className="gf-label" htmlFor={`cred-${f.id}`}>
                {f.label}{' '}
                {connState[f.id] && (
                  <span
                    className={`gf-badge ${connState[f.id] === 'CONNECTED' ? 'ok' : 'err'}`}
                    style={{ marginLeft: 6 }}
                  >
                    {connState[f.id]}
                  </span>
                )}
              </label>
              <div className="gf-row">
                <input
                  id={`cred-${f.id}`}
                  className="gf-input"
                  type="password"
                  autoComplete="off"
                  placeholder="••••••••"
                  value={secrets[f.id] ?? ''}
                  onChange={(e) => setSecrets((s) => ({ ...s, [f.id]: e.target.value }))}
                />
                <button className="gf-btn" onClick={() => void connectProvider(f.id)}>
                  Connect
                </button>
              </div>
            </div>
          ))}
          <p className="gf-muted" style={{ fontSize: 12 }}>
            Keys are stored in the system keyring on desktop, never in plain text.
            Tap Connect — no re-entry needed afterwards.
          </p>
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
          <button className="gf-btn primary" onClick={() => void savePrompts()}>
            Save prompts
          </button>
        </section>

        <button className="gf-btn" onClick={onClose}>
          Done
        </button>
      </div>
    </aside>
  );
}
