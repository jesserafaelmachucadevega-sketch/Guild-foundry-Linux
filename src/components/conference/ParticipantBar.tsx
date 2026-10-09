// SPDX-License-Identifier: Apache-2.0
// Phase 3 — Conference Room participant configuration bar. One config card per
// model slot: provider, model (from catalog), system prompt (defaults from the
// settings prompt fields conference-1..4), role, temperature, reasoning effort,
// tool permission toggles, memory scope.

import React, { useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';
import {
  getProviders,
  listModels,
  type ModelInfo,
  type ProviderSummary,
} from '../../lib/providers';

export type DebateRole = 'proposer' | 'critic' | 'judge' | 'observer';
export type MemoryScope =
  | 'conversation'
  | 'model'
  | 'agent'
  | 'project'
  | 'workspace'
  | 'global';
export type ReasoningEffort = 'low' | 'medium' | 'high';

export interface ToolPermissions {
  web: boolean;
  filesystem: boolean;
  shell: boolean;
  mcp: boolean;
}

export interface ParticipantConfig {
  slot: number;
  providerId: string;
  modelId: string;
  modelName: string;
  systemPrompt: string;
  role: DebateRole;
  temperature: number;
  reasoningEffort: ReasoningEffort;
  tools: ToolPermissions;
  memoryScope: MemoryScope;
}

const ROLES: DebateRole[] = ['proposer', 'critic', 'judge', 'observer'];
const SCOPES: MemoryScope[] = ['conversation', 'model', 'agent', 'project', 'workspace', 'global'];
const EFFORTS: ReasoningEffort[] = ['low', 'medium', 'high'];
const TOOL_KEYS: Array<{ key: keyof ToolPermissions; label: string }> = [
  { key: 'web', label: 'Web' },
  { key: 'filesystem', label: 'Filesystem' },
  { key: 'shell', label: 'Shell' },
  { key: 'mcp', label: 'MCP' },
];

interface ParticipantBarProps {
  participants: ParticipantConfig[];
  onChange: (slot: number, patch: Partial<ParticipantConfig>) => void;
  onAdd: () => void;
  onRemove: (slot: number) => void;
  onToast: (t: string) => void;
  disabled?: boolean;
}

export function ParticipantBar({
  participants,
  onChange,
  onAdd,
  onRemove,
  onToast,
  disabled = false,
}: ParticipantBarProps): React.ReactElement {
  const [open, setOpen] = useState(true);
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [modelCache, setModelCache] = useState<Record<string, ModelInfo[]>>({});
  const [promptDefaults, setPromptDefaults] = useState<Record<string, string>>({});

  // Provider catalog (Phase 2 owns the Rust side; tolerate absence until then).
  useEffect(() => {
    let alive = true;
    void getProviders()
      .then((ps) => {
        if (alive) setProviders(ps ?? []);
      })
      .catch((err) => {
        if (alive) {
          onToast(
            err instanceof DesktopCapabilityRequired
              ? 'Desktop Capability Required'
              : 'Could not load providers',
          );
        }
      });
    return () => {
      alive = false;
    };
  }, [onToast]);

  // System prompt defaults from the settings prompt fields conference-1..4.
  useEffect(() => {
    let alive = true;
    void invoke<Record<string, string> | null>('settings_get', { key: 'system_prompts' })
      .then((v) => {
        if (alive && v) setPromptDefaults(v);
      })
      .catch((err) => {
        if (alive && err instanceof DesktopCapabilityRequired) {
          onToast('Desktop Capability Required');
        }
      });
    return () => {
      alive = false;
    };
  }, [onToast]);

  // Fill empty per-slot prompts from the settings defaults, lazily.
  useEffect(() => {
    participants.forEach((p) => {
      if (!p.systemPrompt) {
        const d = promptDefaults[`conference-${p.slot + 1}`];
        if (d) onChange(p.slot, { systemPrompt: d });
      }
    });
  }, [participants, promptDefaults, onChange]);

  const ensureModels = async (providerId: string): Promise<void> => {
    if (modelCache[providerId]) return;
    try {
      const models = await listModels(providerId);
      setModelCache((c) => ({ ...c, [providerId]: models ?? [] }));
    } catch (err) {
      onToast(
        err instanceof DesktopCapabilityRequired
          ? 'Desktop Capability Required'
          : 'Could not load models for provider',
      );
    }
  };

  return (
    <div className="gf-participant-bar">
      <div className="gf-row" style={{ justifyContent: 'space-between', padding: '6px 8px' }}>
        <button className="gf-btn" onClick={() => setOpen((o) => !o)}>
          {open ? '▾' : '▸'} Participants ({participants.length})
        </button>
        <button
          className="gf-btn"
          disabled={disabled || participants.length >= 4}
          onClick={onAdd}
          title="Add a model (up to 4)"
        >
          + Add model
        </button>
      </div>
      {open && (
        <div className="gf-participant-grid">
          {participants.map((p) => {
            const models = modelCache[p.providerId] ?? [];
            const defaultKey = `conference-${p.slot + 1}`;
            return (
              <div className="gf-participant" key={p.slot}>
                <div className="gf-row" style={{ justifyContent: 'space-between' }}>
                  <span className="gf-label">Model {p.slot + 1}</span>
                  {participants.length > 1 && (
                    <button
                      className="gf-icon-btn"
                      disabled={disabled}
                      onClick={() => onRemove(p.slot)}
                      title="Remove this participant"
                    >
                      Remove
                    </button>
                  )}
                </div>

                <span className="gf-label">Provider</span>
                <select
                  className="gf-select"
                  disabled={disabled}
                  value={p.providerId}
                  onChange={(e) => {
                    const pid = e.target.value;
                    onChange(p.slot, { providerId: pid, modelId: '', modelName: '' });
                    if (pid) void ensureModels(pid);
                  }}
                >
                  <option value="">Select provider…</option>
                  {providers.map((pr) => (
                    <option key={pr.id} value={pr.id}>
                      {pr.name}
                    </option>
                  ))}
                </select>

                <span className="gf-label">Model</span>
                <select
                  className="gf-select"
                  disabled={disabled || !p.providerId}
                  value={p.modelId}
                  onChange={(e) => {
                    const mid = e.target.value;
                    const mi = models.find((m) => m.model_id === mid);
                    onChange(p.slot, { modelId: mid, modelName: mi?.name ?? mid });
                  }}
                >
                  <option value="">Select model…</option>
                  {models.map((m) => (
                    <option key={m.id} value={m.model_id}>
                      {m.name}
                    </option>
                  ))}
                </select>

                <div className="gf-row" style={{ gap: 8 }}>
                  <div style={{ flex: 1 }}>
                    <span className="gf-label">Role</span>
                    <select
                      className="gf-select"
                      disabled={disabled}
                      value={p.role}
                      onChange={(e) => onChange(p.slot, { role: e.target.value as DebateRole })}
                    >
                      {ROLES.map((r) => (
                        <option key={r} value={r}>
                          {r}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div style={{ flex: 1 }}>
                    <span className="gf-label">Memory scope</span>
                    <select
                      className="gf-select"
                      disabled={disabled}
                      value={p.memoryScope}
                      onChange={(e) =>
                        onChange(p.slot, { memoryScope: e.target.value as MemoryScope })
                      }
                    >
                      {SCOPES.map((s) => (
                        <option key={s} value={s}>
                          {s}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>

                <div className="gf-row" style={{ gap: 8 }}>
                  <div style={{ flex: 1 }}>
                    <span className="gf-label">Reasoning</span>
                    <select
                      className="gf-select"
                      disabled={disabled}
                      value={p.reasoningEffort}
                      onChange={(e) =>
                        onChange(p.slot, { reasoningEffort: e.target.value as ReasoningEffort })
                      }
                    >
                      {EFFORTS.map((r) => (
                        <option key={r} value={r}>
                          {r}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div style={{ flex: 1 }}>
                    <span className="gf-label">Temperature: {p.temperature.toFixed(1)}</span>
                    <input
                      type="range"
                      min={0}
                      max={2}
                      step={0.1}
                      disabled={disabled}
                      value={p.temperature}
                      onChange={(e) =>
                        onChange(p.slot, { temperature: Number(e.target.value) })
                      }
                      style={{ width: '100%' }}
                    />
                  </div>
                </div>

                <span className="gf-label">Tool permissions</span>
                <div className="gf-row" style={{ gap: 10, flexWrap: 'wrap' }}>
                  {TOOL_KEYS.map(({ key, label }) => (
                    <label key={key} className="gf-row" style={{ gap: 4, fontSize: 12 }}>
                      <input
                        type="checkbox"
                        disabled={disabled}
                        checked={p.tools[key]}
                        onChange={() =>
                          onChange(p.slot, { tools: { ...p.tools, [key]: !p.tools[key] } })
                        }
                      />
                      {label}
                    </label>
                  ))}
                </div>

                <span className="gf-label">System prompt</span>
                <textarea
                  className="gf-textarea"
                  disabled={disabled}
                  rows={3}
                  value={p.systemPrompt}
                  placeholder={promptDefaults[defaultKey] ?? 'System prompt for this participant…'}
                  onChange={(e) => onChange(p.slot, { systemPrompt: e.target.value })}
                />
                {promptDefaults[defaultKey] && (
                  <button
                    className="gf-icon-btn"
                    disabled={disabled}
                    onClick={() =>
                      onChange(p.slot, { systemPrompt: promptDefaults[defaultKey] ?? '' })
                    }
                    title="Restore the settings default for this slot"
                  >
                    Restore settings default
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
