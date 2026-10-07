// SPDX-License-Identifier: Apache-2.0
// Phase 14 — headless runner for scheduled tasks. Mounted once in App.
// Listens for `scheduler-task-due`, runs the tool loop without UI, records
// the result, and notifies the user.

import React, { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '../../lib/api';
import { runToolLoop, type ToolLoopEvent } from '../../lib/toolLoop';

export interface DueTask {
  id: string;
  name: string;
  task_prompt: string;
  provider_id: string;
  model_id: string;
  auto_approve: boolean;
}

const MAX_TURNS = 25;

function newSessionId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID();
  return `sched-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
}

async function runOnce(task: DueTask, onToast: (t: string) => void): Promise<void> {
  const sessionId = newSessionId();

  if (!task.provider_id || !task.model_id) {
    await invoke('scheduler_complete', {
      id: task.id,
      status: 'error',
      summary: 'No provider/model configured for this task.',
      sessionId,
    });
    return;
  }

  // Auto-approve grant: only when the user explicitly enabled it for this task.
  if (task.auto_approve) {
    try {
      await invoke('scheduler_grant_session', { sessionId, taskId: task.id });
    } catch (e) {
      await invoke('scheduler_complete', {
        id: task.id,
        status: 'error',
        summary: `Auto-approve grant failed: ${e instanceof Error ? e.message : String(e)}`,
        sessionId,
      });
      return;
    }
  }

  const systemPrompt =
    'You are a scheduled background agent in Guild Foundry AI. ' +
    'Complete the task below using your tools. Be concise and factual. ' +
    'End with a short summary of what you did and what you found — ' +
    'that summary is what the user will see in their notification. ' +
    'Do not ask questions; if you need something you cannot get, say so in the summary.';

  const result = await new Promise<{ status: string; finalText: string }>((resolve) => {
    let done = false;
    const finish = (status: string, finalText: string) => {
      if (done) return;
      done = true;
      resolve({ status, finalText });
    };
    runToolLoop({
      providerId: task.provider_id,
      modelId: task.model_id,
      systemPrompt,
      sessionId,
      agentId: `scheduler:${task.id}`,
      maxTurns: MAX_TURNS,
      userMessage: task.task_prompt,
      onEvent: (e: ToolLoopEvent) => {
        if (e.type !== 'loop_end') return;
        let text = e.finalText || '';
        let status = e.status;
        if (e.status === 'approval_required') {
          text =
            `Stopped: a tool needed approval and auto-approve is off for this task. ` +
            (e.detail ? `Detail: ${e.detail}` : '');
        } else if (e.status === 'user_input_required') {
          text = `Stopped: the task asked the user a question. ${e.detail ?? ''}`;
        } else if (e.status === 'error') {
          text = `Error: ${e.detail ?? e.finalText}`;
        } else if (e.status === 'max_turns') {
          text = `${text}\n\n(Stopped at ${MAX_TURNS} turns.)`;
        }
        finish(status, text);
      },
    });
    // Safety net: never let a headless run hang forever.
    setTimeout(() => finish('error', 'Timed out after 20 minutes.'), 20 * 60 * 1000);
  });

  const summary = result.finalText.trim().slice(0, 2000) || '(no output)';
  try {
    await invoke('scheduler_complete', {
      id: task.id,
      status: result.status,
      summary,
      sessionId,
    });
    await invoke('scheduler_notify', {
      title: `Scheduled: ${task.name}`,
      body:
        result.status === 'done'
          ? summary.slice(0, 300)
          : `Finished with status: ${result.status}. ${summary.slice(0, 200)}`,
    });
  } catch (e) {
    onToast(`Scheduler report failed: ${e instanceof Error ? e.message : String(e)}`);
  }
}

export function SchedulerRunner({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const busyRef = useRef<Set<string>>(new Set());
  const toastRef = useRef(onToast);
  toastRef.current = onToast;

  useEffect(() => {
    let unlisten: () => void = () => undefined;
    let live = true;
    void listen<DueTask>('scheduler-task-due', (event) => {
      const task = event.payload;
      if (!task || !live) return;
      if (busyRef.current.has(task.id)) return; // already running this task
      busyRef.current.add(task.id);
      void runOnce(task, (t) => toastRef.current(t)).finally(() => {
        busyRef.current.delete(task.id);
      });
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      live = false;
      unlisten();
    };
  }, []);

  return <></>;
}
