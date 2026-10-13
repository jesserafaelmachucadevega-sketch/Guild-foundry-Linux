// SPDX-License-Identifier: Apache-2.0
// FloatingShell — lets a chat/panel detach into a movable, resizable,
// minimizable, maximizable window (and dock back). Bounds persist per
// storageKey. Dependency-free: pointer events + fixed positioning.
//
// Usage: wrap any flex-column content; pass `floating` to switch modes.

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '../../lib/api';

interface Bounds {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface Props {
  floating: boolean;
  onClose: () => void;
  title: string;
  storageKey: string;
  children: React.ReactNode;
}

function defaultBounds(): Bounds {
  const w = Math.min(460, window.innerWidth - 40);
  const h = Math.min(640, window.innerHeight - 80);
  return {
    x: Math.max(12, window.innerWidth - w - 24),
    y: 72,
    w,
    h,
  };
}

export function FloatingShell({ floating, onClose, title, storageKey, children }: Props): React.ReactElement {
  const [bounds, setBounds] = useState<Bounds | null>(null);
  const [minimized, setMinimized] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const dragRef = useRef<{ dx: number; dy: number } | null>(null);
  const sizeRef = useRef<{ sw: number; sh: number } | null>(null);

  const storageId = `ui.float.${storageKey}`;

  useEffect(() => {
    if (!floating) return;
    let live = true;
    void invoke<Partial<Bounds> | null>('settings_get', { key: storageId })
      .then((v) => {
        if (!live || !v || typeof v !== 'object') return;
        const base = defaultBounds();
        setBounds({
          x: typeof v.x === 'number' ? v.x : base.x,
          y: typeof v.y === 'number' ? v.y : base.y,
          w: typeof v.w === 'number' ? Math.max(320, v.w) : base.w,
          h: typeof v.h === 'number' ? Math.max(240, v.h) : base.h,
        });
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [floating, storageId]);

  const persist = useCallback(
    (b: Bounds) => {
      void invoke('settings_set', { key: storageId, value: b }).catch(() => undefined);
    },
    [storageId],
  );

  const onHeaderPointerDown = (e: React.PointerEvent<HTMLDivElement>): void => {
    if (maximized) return;
    const b = bounds ?? defaultBounds();
    dragRef.current = { dx: e.clientX - b.x, dy: e.clientY - b.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onHeaderPointerMove = (e: React.PointerEvent<HTMLDivElement>): void => {
    if (!dragRef.current) return;
    const { dx, dy } = dragRef.current;
    setBounds((prev) => {
      const b = prev ?? defaultBounds();
      return {
        ...b,
        x: Math.max(0, Math.min(window.innerWidth - 140, e.clientX - dx)),
        y: Math.max(0, Math.min(window.innerHeight - 44, e.clientY - dy)),
      };
    });
  };

  const onHeaderPointerUp = (): void => {
    if (!dragRef.current) return;
    dragRef.current = null;
    setBounds((prev) => {
      if (prev) persist(prev);
      return prev;
    });
  };

  const onResizePointerDown = (e: React.PointerEvent<HTMLDivElement>): void => {
    const b = bounds ?? defaultBounds();
    sizeRef.current = { sw: b.x + b.w - e.clientX, sh: b.y + b.h - e.clientY };
    e.currentTarget.setPointerCapture(e.pointerId);
    e.stopPropagation();
  };

  const onResizePointerMove = (e: React.PointerEvent<HTMLDivElement>): void => {
    if (!sizeRef.current) return;
    const { sw, sh } = sizeRef.current;
    setBounds((prev) => {
      const b = prev ?? defaultBounds();
      return {
        ...b,
        w: Math.max(320, e.clientX + sw - b.x),
        h: Math.max(240, e.clientY + sh - b.y),
      };
    });
  };

  const onResizePointerUp = (): void => {
    if (!sizeRef.current) return;
    sizeRef.current = null;
    setBounds((prev) => {
      if (prev) persist(prev);
      return prev;
    });
  };

  if (!floating) {
    return <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>{children}</div>;
  }

  const b = bounds ?? defaultBounds();
  const style: React.CSSProperties = maximized
    ? { left: 16, top: 56, width: window.innerWidth - 32, height: window.innerHeight - 72 }
    : { left: b.x, top: b.y, width: b.w, height: minimized ? 38 : b.h };

  return (
    <div className="agent-window" style={style}>
      <div
        className="agent-window-header"
        onPointerDown={onHeaderPointerDown}
        onPointerMove={onHeaderPointerMove}
        onPointerUp={onHeaderPointerUp}
      >
        <span>{title}</span>
        <div className="agent-window-controls">
          <button onClick={() => setMinimized((m) => !m)} title={minimized ? 'Restore' : 'Minimize'}>
            {minimized ? '▢' : '–'}
          </button>
          <button onClick={() => setMaximized((m) => !m)} title={maximized ? 'Restore size' : 'Maximize'}>
            {maximized ? '❐' : '□'}
          </button>
          <button onClick={onClose} title="Dock back into the page">
            ✕
          </button>
        </div>
      </div>
      {!minimized && (
        <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
          {children}
        </div>
      )}
      {!minimized && !maximized && (
        <div
          className="agent-window-resize"
          onPointerDown={onResizePointerDown}
          onPointerMove={onResizePointerMove}
          onPointerUp={onResizePointerUp}
        />
      )}
    </div>
  );
}
