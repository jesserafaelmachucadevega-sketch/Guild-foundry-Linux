// SPDX-License-Identifier: Apache-2.0
// Phase 3 — Winner selection dialog shown when a debate ends. Accepts a model
// number 1..N with validation; the parent emits the congratulatory card.

import React, { useEffect, useState } from 'react';

interface WinnerDialogProps {
  open: boolean;
  count: number;
  onSubmit: (n: number) => void;
  onCancel: () => void;
}

export function WinnerDialog({ open, count, onSubmit, onCancel }: WinnerDialogProps): React.ReactElement | null {
  const [value, setValue] = useState('');
  const [error, setError] = useState('');

  useEffect(() => {
    if (open) {
      setValue('');
      setError('');
    }
  }, [open ]);

  if (!open) return null;

  const submit = (): void => {
    const raw = value.trim();
    const n = Number(raw);
    if (!/^\d+$/.test(raw) || !Number.isInteger(n) || n < 1 || n > count) {
      setError(`Enter a whole number from 1 to ${count}.`);
      return;
    }
    onSubmit(n);
  };

  return (
    <div className="gf-modal-backdrop">
      <div className="gf-modal" role="dialog" aria-label="Declare debate winner">
        <div className="gf-pane-header" style={{ marginBottom: 12 }}>
          <span>Declare a winner</span>
        </div>
        <p className="gf-muted" style={{ marginTop: 0 }}>
          The debate has ended. Which model won? Enter its number (1–{count}).
        </p>
        <div className="gf-row" style={{ gap: 8 }}>
          <input
            className="gf-input"
            style={{ width: 120 }}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit();
            }}
            placeholder={`1–${count}`}
            aria-label="Winning model number"
          />
          <button className="gf-btn primary" onClick={submit}>
            Declare winner
          </button>
          <button className="gf-btn" onClick={onCancel}>
            Skip
          </button>
        </div>
        {error && <p style={{ color: 'var(--gf-danger)', fontSize: 12 }}>{error}</p>}
      </div>
    </div>
  );
}
