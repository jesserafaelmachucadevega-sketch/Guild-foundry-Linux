// SPDX-License-Identifier: Apache-2.0
// Phase 14 — Scheduled tasks UI: list, create, edit, enable/disable, run now.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';

export interface ScheduledTask {
  id: string;
  name: string;
  task_prompt: string;
  provider_id: string;
  model_id: string;
  schedule_kind: string;
  interval_minutes: number;
  daily_at: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
  last_status: string | null;
  last_summary: string | null;
  auto_approve: boolean;
}

interface Provider {
  id: string;
  name: string;
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function fmtWhen(iso: string | null): string {
  if (!iso) return '—';
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

function scheduleLabel(t: ScheduledTask): string {
  if (t.schedule_kind === 'daily') return `Daily at ${t.daily_at}`;
  const m = t.interval_minutes;
  if (m < 60) return `Every ${m} min`;
  if (m % 60 === 0) return `Every ${m / 60}h`;
  return `Every ${m} min`;
}

const BLANK = {
  name: '',
  task_prompt: '',
  provider_id: '',
  model_id: '',
  schedule_kind: 'interval',
  interval_minutes: 60,
  daily_at: '08:00',
  auto_approve: false,
};

export function ScheduledPanel({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [models, setModels] = useState<string[]>([]);
  const [editing, setEditing] = useState<string | 'new' | null>(null);
  const [form, setForm] = useState({ ...BLANK });
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setTasks(await invoke<ScheduledTask[]>('scheduler_list'));
    } catch (e) {
      onToast(`Scheduler: ${errText(e)}`);
    }
  }, [onToast]);

  useEffect(() => {
    void refresh();
    void invoke<Provider[]>('provider_list')
      .then((ps) => setProviders(ps.map((p) => ({ id: p.id, name: p.name }))))
      .catch(() => {});
  }, [refresh]);

  useEffect(() => {
    if (!form.provider_id) {
      setModels([]);
      return;
    }
    void invoke<string[]>('provider_list_models', { providerId: form.provider_id })
      .then((m) => setModels(m))
      .catch(() => setModels([]));
  }, [form.provider_id]);

  const openNew = () => {
    setForm({ ...BLANK });
    setEditing('new');
  };

  const openEdit = (t: ScheduledTask) => {
    setForm({
      name: t.name,
      task_prompt: t.task_prompt,
      provider_id: t.provider_id,
      model_id: t.model_id,
      schedule_kind: t.schedule_kind,
      interval_minutes: t.interval_minutes,
      daily_at: t.daily_at,
      auto_approve: t.auto_approve,
    });
    setEditing(t.id);
  };

  const save = async () => {
    if (!form.name.trim() || !form.task_prompt.trim()) {
      onToast('Name and task prompt are required');
      return;
    }
    if (!form.provider_id || !form.model_id) {
      onToast('Pick a provider and model');
      return;
    }
    setBusy(true);
    try {
      if (editing === 'new') {
        await invoke('scheduler_create', { input: form });
        onToast('Scheduled task created');
      } else if (editing) {
        await invoke('scheduler_update', { id: editing, input: form });
        onToast('Scheduled task updated');
      }
      setEditing(null);
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = async (t: ScheduledTask) => {
    try {
      await invoke('scheduler_set_enabled', { id: t.id, enabled: !t.enabled });
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const remove = async (t: ScheduledTask) => {
    if (!window.confirm(`Delete scheduled task "${t.name}"?`)) return;
    try {
      await invoke('scheduler_delete', { id: t.id });
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const runNow = async (t: ScheduledTask) => {
    try {
      await invoke('scheduler_run_now', { id: t.id });
      onToast(`"${t.name}" started`);
    } catch (e) {
      onToast(errText(e));
    }
  };

  return (
    <div className="gf-panel">
      <div className="gf-panel-head">
        <div>
          <h2>Scheduled tasks</h2>
          <p className="gf-muted">
            Agents that run on their own — watches, digests, reminders. Results
            arrive as desktop notifications.
          </p>
        </div>
        <button className="gf-btn primary" onClick={openNew}>
          New task
        </button>
      </div>

      {tasks.length === 0 && !editing && (
        <p className="gf-muted" style={{ padding: 16 }}>
          Nothing scheduled yet. Create a task like "check my inbox every morning
          and brief me" or "watch this repo for new releases".
        </p>
      )}

      <div className="conn-grid">
        {tasks.map((t) => (
          <div className="conn-card" key={t.id}>
            <div className="conn-card-head">
              <strong>{t.name}</strong>
              <span className={`conn-badge${t.enabled ? ' conn-badge-ok' : ''}`}>
                {t.enabled ? 'On' : 'Off'}
              </span>
            </div>
            <p className="conn-tagline">
              {scheduleLabel(t)} · {t.provider_id}/{t.model_id}
              {t.auto_approve ? ' · auto-approve' : ''}
            </p>
            {t.last_summary && (
              <p className="conn-note" style={{ whiteSpace: 'pre-wrap' }}>
                Last run ({fmtWhen(t.last_run_at)}, {t.last_status}): {t.last_summary.slice(0, 280)}
              </p>
            )}
            {!t.last_summary && (
              <p className="conn-note">Next run: {fmtWhen(t.next_run_at)}</p>
            )}
            <div className="gf-row" style={{ gap: 6, marginTop: 8, flexWrap: 'wrap' }}>
              <button className="gf-icon-btn" onClick={() => void toggle(t)}>
                {t.enabled ? 'Pause' : 'Resume'}
              </button>
              <button className="gf-icon-btn" onClick={() => void runNow(t)}>
                Run now
              </button>
              <button className="gf-icon-btn" onClick={() => openEdit(t)}>
                Edit
              </button>
              <button className="gf-icon-btn" onClick={() => void remove(t)}>
                Delete
              </button>
            </div>
          </div>
        ))}
      </div>

      {editing && (
        <div className="conn-card" style={{ marginTop: 16 }}>
          <div className="conn-card-head">
            <strong>{editing === 'new' ? 'New scheduled task' : 'Edit scheduled task'}</strong>
          </div>
          <label className="gf-label">
            <span className="gf-muted conn-small">Name</span>
            <input
              className="gf-input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder="Morning inbox brief"
            />
          </label>
          <label className="gf-label">
            <span className="gf-muted conn-small">Task prompt (what the agent should do)</span>
            <textarea
              className="gf-input"
              rows={4}
              value={form.task_prompt}
              onChange={(e) => setForm({ ...form, task_prompt: e.target.value })}
              placeholder="Check Gmail for unread mail from the last 12 hours and summarize anything important."
            />
          </label>
          <div className="gf-row" style={{ gap: 8 }}>
            <label className="gf-label" style={{ flex: 1 }}>
              <span className="gf-muted conn-small">Provider</span>
              <select
                className="gf-input"
                value={form.provider_id}
                onChange={(e) => setForm({ ...form, provider_id: e.target.value, model_id: '' })}
              >
                <option value="">Select…</option>
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="gf-label" style={{ flex: 1 }}>
              <span className="gf-muted conn-small">Model</span>
              <select
                className="gf-input"
                value={form.model_id}
                onChange={(e) => setForm({ ...form, model_id: e.target.value })}
              >
                <option value="">Select…</option>
                {models.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className="gf-row" style={{ gap: 8 }}>
            <label className="gf-label" style={{ flex: 1 }}>
              <span className="gf-muted conn-small">Schedule</span>
              <select
                className="gf-input"
                value={form.schedule_kind}
                onChange={(e) => setForm({ ...form, schedule_kind: e.target.value })}
              >
                <option value="interval">Repeat every…</option>
                <option value="daily">Daily at…</option>
              </select>
            </label>
            {form.schedule_kind === 'interval' ? (
              <label className="gf-label" style={{ flex: 1 }}>
                <span className="gf-muted conn-small">Minutes</span>
                <input
                  className="gf-input"
                  type="number"
                  min={5}
                  value={form.interval_minutes}
                  onChange={(e) =>
                    setForm({ ...form, interval_minutes: Math.max(5, Number(e.target.value) || 5) })
                  }
                />
              </label>
            ) : (
              <label className="gf-label" style={{ flex: 1 }}>
                <span className="gf-muted conn-small">Time (local)</span>
                <input
                  className="gf-input"
                  type="time"
                  value={form.daily_at}
                  onChange={(e) => setForm({ ...form, daily_at: e.target.value })}
                />
              </label>
            )}
          </div>
          <label className="gf-row" style={{ gap: 8, marginTop: 8, alignItems: 'flex-start' }}>
            <input
              type="checkbox"
              checked={form.auto_approve}
              onChange={(e) => setForm({ ...form, auto_approve: e.target.checked })}
            />
            <span className="conn-small">
              <strong>Auto-approve tools for this task.</strong>{' '}
              <span className="gf-muted">
                Scheduled runs are unattended — without this, the run stops the
                first time a tool needs approval. Only enable for tasks you trust.
                An explicit Deny always wins.
              </span>
            </span>
          </label>
          <div className="gf-row" style={{ gap: 8, marginTop: 12 }}>
            <button className="gf-btn primary" disabled={busy} onClick={() => void save()}>
              {editing === 'new' ? 'Create' : 'Save'}
            </button>
            <button className="gf-btn" disabled={busy} onClick={() => setEditing(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
