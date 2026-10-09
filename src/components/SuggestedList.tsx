// SPDX-License-Identifier: Apache-2.0
// Suggested to-do list rendered under the Agent tab conversation.
// One tap runs the suggestion as a normal user message (permission gates
// apply); the X dismisses it and teaches generation not to repeat it.

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import {
  dismissSuggestion,
  listSuggestions,
  type Suggestion,
} from '../lib/suggestions';

interface Props {
  conversationId: string;
  /** Host sends the suggestion's prompt as a new user message. */
  onRun: (prompt: string) => void;
  /** Increment to force a refresh (e.g. after generateSuggestions). */
  refreshToken: number;
}

export function SuggestedList({ conversationId, onRun, refreshToken }: Props): React.ReactElement | null {
  const [items, setItems] = useState<Suggestion[]>([]);

  useEffect(() => {
    let live = true;
    void listSuggestions(conversationId)
      .then((all) => {
        if (live) setItems(all.filter((s) => s.status === 'pending'));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [conversationId, refreshToken]);

  if (items.length === 0) return null;

  const run = async (s: Suggestion) => {
    try {
      await invoke('suggestions_accept', { id: s.id });
    } catch {
      // Non-fatal: the run matters more than the bookkeeping.
    }
    setItems((prev) => prev.filter((x) => x.id !== s.id));
    onRun(s.prompt);
  };

  const dismiss = async (s: Suggestion) => {
    setItems((prev) => prev.filter((x) => x.id !== s.id));
    try {
      await dismissSuggestion(s.id);
    } catch {
      // Already removed from view; the status write is best-effort.
    }
  };

  return (
    <div className="gf-suggested" aria-label="Suggested to-do">
      <div className="gf-suggested-title">Suggested to-do</div>
      <ul className="gf-suggested-list">
        {items.map((s) => (
          <li key={s.id} className="gf-suggested-item">
            <button
              className="gf-suggested-run"
              title={s.prompt}
              onClick={() => void run(s)}
            >
              {s.label}
            </button>
            <button
              className="gf-suggested-dismiss"
              aria-label={`Dismiss: ${s.label}`}
              onClick={() => void dismiss(s)}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
