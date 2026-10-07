// SPDX-License-Identifier: Apache-2.0
// Suggested to-do: dynamic follow-ups generated at the end of an agent turn.
// Distinct from a feed: these live under the conversation they came from,
// read like next actions (not headlines), run with one tap through the normal
// permission gates, and never auto-run.

import { invoke } from './api';
import { completeChat, type ChatMsg } from './chat';

export interface Suggestion {
  id: string;
  conversation_id: string;
  label: string;
  prompt: string;
  status: 'pending' | 'accepted' | 'dismissed';
  created_at: string;
}

const MAX_SUGGESTIONS = 3;

export async function listSuggestions(conversationId: string): Promise<Suggestion[]> {
  return invoke<Suggestion[]>('suggestions_list', { conversation_id: conversationId });
}

export async function dismissSuggestion(id: string): Promise<void> {
  await invoke('suggestions_dismiss', { id });
}

/**
 * Generate up to 3 follow-up suggestions from the tail of the transcript.
 * Called once per agent turn end (Agent tab wiring). Best-effort: failures
 * are swallowed so a suggestion pass never breaks the chat.
 *
 * @param runSuggestion called by the host when the user taps a suggestion —
 *   typically sends `prompt` as a new user message. Marks it accepted first.
 */
export async function generateSuggestions(opts: {
  conversationId: string;
  providerId: string;
  modelId: string;
  /** Last few messages of the turn, oldest first. */
  transcript: ChatMsg[];
}): Promise<Suggestion[]> {
  try {
    const recent = await invoke<string[]>('suggestions_recent_labels', {
      conversation_id: opts.conversationId,
      limit: 12,
    });
    const tail = opts.transcript.slice(-6).map((m) => {
      const role = m.role === 'assistant' ? 'Agent' : m.role === 'user' ? 'User' : m.role;
      return `${role}: ${m.content.slice(0, 1200)}`;
    });

    const raw = await completeChat({
      providerId: opts.providerId,
      modelId: opts.modelId,
      messages: [
        {
          role: 'system',
          content:
            'You suggest follow-up actions for a personal AI agent\'s user. ' +
            'Reply with a JSON array (max 3 items) of {"label": "...", "prompt": "..."}. ' +
            'Labels are short next-actions in the user\'s voice ("Draft the follow-up to Angel"). ' +
            'Prompts are the full instruction the agent should run if the user taps the label. ' +
            'Only suggest things the agent can plausibly do with its tools. ' +
            'Skip anything already done, in progress, or listed below. ' +
            'If nothing useful follows, reply with []. JSON only, no prose.',
        },
        {
          role: 'user',
          content:
            `Already suggested (do not repeat):\n${recent.map((l) => `- ${l}`).join('\n') || '(none)'}\n\n` +
            `Recent conversation:\n${tail.join('\n\n')}`,
        },
      ],
      params: { maxTokens: 600, temperature: 0.4 },
    });

    const start = raw.indexOf('[');
    const end = raw.lastIndexOf(']');
    if (start < 0 || end <= start) return [];
    const parsed = JSON.parse(raw.slice(start, end + 1)) as Array<{
      label?: unknown;
      prompt?: unknown;
    }>;
    const seen = new Set(recent.map((l) => l.toLowerCase()));
    const out: Suggestion[] = [];
    for (const item of parsed.slice(0, MAX_SUGGESTIONS)) {
      const label = typeof item.label === 'string' ? item.label.trim() : '';
      const prompt = typeof item.prompt === 'string' ? item.prompt.trim() : '';
      if (!label || !prompt || seen.has(label.toLowerCase())) continue;
      seen.add(label.toLowerCase());
      const s = await invoke<Suggestion>('suggestions_add', {
        conversation_id: opts.conversationId,
        label,
        prompt,
      });
      out.push(s);
    }
    return out;
  } catch {
    return [];
  }
}
