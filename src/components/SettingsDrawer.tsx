import React, { useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../lib/api';
import {
  getProviders,
  handshakeProvider,
  upsertProvider,
  type ProviderSummary,
} from '../lib/providers';
import { PROVIDER_TEMPLATES } from '../lib/providerTemplates';

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

export type ThemeMode = 'light' | 'dark';

interface Props {
  open: boolean;
  onClose: () => void;
  onToast: (text: string) => void;
  theme: ThemeMode;
  onTheme: (t: ThemeMode) => void;
}

export function SettingsDrawer({ open, onClose, onToast, theme, onTheme }: Props): React.ReactElement | null {
  const [prompts, setPrompts] = useState<Record<string, string>>({});
  const [promptsSaved, setPromptsSaved] = useState<Record<string, string>>({});
  const [secrets, setSecrets] = useState<Record<string, string>>({});
  const [showKeys, setShowKeys] = useState<Record<string, boolean>>({});
  const [connState, setConnState] = useState<Record<string, string>>({});
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [busyId, setBusyId] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !isDesktop()) return;
    void invoke<Record<string, string>>('settings_load_prompts')
      .then((p) => {
        setPrompts(p ?? {});
        setPromptsSaved(p ?? {});
      })
      .catch(() => undefined);
    void getProviders()
      .then((list) => {
        setProviders(list);
        const states: Record<string, string> = {};
        for (const p of list) {
          if (p.status === 'connected') states[p.name] = 'CONNECTED';
          else if (p.status === 'error') states[p.name] = `ERROR ${p.last_error_code ?? ''}`.trim();
        }
        setConnState((s) => ({ ...s, ...states }));
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

  const setThemeAndPersist = async (t: ThemeMode): Promise<void> => {
    onTheme(t);
    try {
      await invoke('settings_set', { key: 'ui.theme', value: t });
    } catch {
      // Theme still applies for this session; persistence is best-effort.
    }
  };

  const connectProvider = async (templateName: string): Promise<void> => {
    const value = (secrets[templateName] ?? '').trim();
    const tpl = PROVIDER_TEMPLATES.find((p) => p.name === templateName);
    if (!tpl) {
      onToast('Unknown provider');
      return;
    }
    // If a saved row already exists, reuse it so the keyring entry is kept
    // and future reconnects are just Connect — no re-entry needed.
    const existing = providers.find((p) => p.name.toLowerCase() === templateName.toLowerCase());
    if (!value && !existing?.has_key) {
      onToast('Enter the API key first (or Connect a saved provider with a stored key)');
      return;
    }
    setBusyId(templateName);
    try {
      const saved = await upsertProvider({
        id: existing?.id,
        name: tpl.name,
        kind: tpl.kind,
        base_url: existing?.base_url ?? tpl.base_url,
        models_path: existing?.models_path ?? tpl.models_path,
        api_key: value || undefined,
      });
      const report = await handshakeProvider(saved.id);
      if (report.ok) {
        setConnState((s) => ({ ...s, [templateName]: 'CONNECTED' }));
        onToast(`${templateName}: CONNECTED — ${report.models_found} models`);
      } else {
        setConnState((s) => ({ ...s, [templateName]: `ERROR [${report.raw_code}]` }));
        onToast(`${templateName}: ERROR [${report.raw_code}] — ${report.layman}`);
      }
      setSecrets((s) => ({ ...s, [templateName]: '' }));
      const list = await getProviders().catch(() => null);
      if (list) setProviders(list);
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : `Connect failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setBusyId(null);
    }
  };

  const disconnectProvider = async (templateName: string): Promise<void> => {
    const existing = providers.find((p) => p.name.toLowerCase() === templateName.toLowerCase());
    if (!existing) {
      setConnState((s) => {
        const next = { ...s };
        delete next[templateName];
        return next;
      });
      return;
    }
    try {
      // Clear the keyring entry but keep the endpoint row, so reconnecting
      // later only needs a fresh key — the URL stays saved.
      await invoke('secret_delete', { key: `gfa-provider:${existing.id}` }).catch(() => undefined);
      setConnState((s) => ({ ...s, [templateName]: 'DISCONNECTED' }));
      onToast(`${templateName}: disconnected (endpoint kept, key cleared)`);
      const list = await getProviders().catch(() => null);
      if (list) setProviders(list);
    } catch (err) {
      onToast(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : 'Disconnect failed');
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
          <span className="gf-label">Appearance (default: Light)</span>
          <div className="gf-view-switcher" role="tablist" aria-label="Theme">
            <button
              role="tab"
              aria-selected={theme === 'light'}
              className={theme === 'light' ? 'active' : ''}
              onClick={() => void setThemeAndPersist('light')}
            >
              Light
            </button>
            <button
              role="tab"
              aria-selected={theme === 'dark'}
              className={theme === 'dark' ? 'active' : ''}
              onClick={() => void setThemeAndPersist('dark')}
            >
              Dark
            </button>
          </div>
          <p className="gf-muted" style={{ fontSize: 12 }}>
            Light is the default: tans, warm greys and light browns with black text on light surfaces.
          </p>
        </section>

        <hr className="gf-divider" />

        <section>
          <span className="gf-label">Credentials (masked, keyring-backed)</span>
          {PROVIDER_TEMPLATES.map((tpl) => {
            const busy = busyId === tpl.name;
            const state = connState[tpl.name];
            const show = showKeys[tpl.name] === true;
            return (
              <div key={tpl.id} style={{ marginBottom: 8 }}>
                <label className="gf-label" htmlFor={`cred-${tpl.id}`}>
                  {tpl.name}{' '}
                  {state && (
                    <span
                      className={`gf-badge ${state === 'CONNECTED' ? 'ok' : state === 'DISCONNECTED' ? 'idle' : 'err'}`}
                      style={{ marginLeft: 6 }}
                    >
                      {state}
                    </span>
                  )}
                </label>
                <div className="gf-row">
                  <input
                    id={`cred-${tpl.id}`}
                    className="gf-input"
                    type={show ? 'text' : 'password'}
                    autoComplete="off"
                    placeholder={tpl.keyPlaceholder}
                    value={secrets[tpl.name] ?? ''}
                    onChange={(e) => setSecrets((s) => ({ ...s, [tpl.name]: e.target.value }))}
                  />
                  <button
                    className="gf-icon-btn"
                    title={show ? 'Hide key' : 'Show key'}
                    onClick={() => setShowKeys((s) => ({ ...s, [tpl.name]: !show }))}
                  >
                    {show ? 'Hide' : 'Show'}
                  </button>
                  <button className="gf-btn" disabled={busy} onClick={() => void connectProvider(tpl.name)}>
                    {busy ? '…' : 'Connect'}
                  </button>
                  <button className="gf-btn" disabled={busy} onClick={() => void disconnectProvider(tpl.name)}>
                    Disconnect
                  </button>
                </div>
                <div className="gf-muted" style={{ fontSize: 11 }}>{tpl.handshakeUrl}</div>
              </div>
            );
          })}
          <p className="gf-muted" style={{ fontSize: 12 }}>
            Keys are stored in the system keyring on desktop, never in plain text.
            Saved providers reconnect with Connect — no re-entry needed. Full
            endpoint editing lives in the Providers panel.
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
