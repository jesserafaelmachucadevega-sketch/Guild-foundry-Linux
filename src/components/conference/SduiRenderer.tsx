// SPDX-License-Identifier: Apache-2.0
// Phase 3 — SDUI artifact renderer. Parses ```sdui fenced JSON blocks embedded
// in model text and renders them as interactive widgets (polls, checklists,
// sliders, cards, tables, forms, approval dialogs). All state is local.

import React, { useMemo, useState } from 'react';

export interface SduiPoll {
  type: 'poll';
  title?: string;
  options?: string[];
}
export interface SduiChecklist {
  type: 'checklist';
  title?: string;
  items?: string[];
}
export interface SduiSlider {
  type: 'slider';
  title?: string;
  min?: number;
  max?: number;
  value?: number;
  step?: number;
}
export interface SduiCard {
  type: 'card';
  title?: string;
  body?: string;
}
export interface SduiTable {
  type: 'table';
  title?: string;
  columns?: string[];
  rows?: string[][];
}
export interface SduiFormField {
  name: string;
  label?: string;
  type?: string;
  value?: string;
  placeholder?: string;
}
export interface SduiForm {
  type: 'form';
  title?: string;
  fields?: SduiFormField[];
  submit?: string;
}
export interface SduiApproval {
  type: 'approval';
  title?: string;
  body?: string;
  approveLabel?: string;
  rejectLabel?: string;
}

export type SduiWidget =
  | SduiPoll
  | SduiChecklist
  | SduiSlider
  | SduiCard
  | SduiTable
  | SduiForm
  | SduiApproval;

export type SduiPart =
  | { kind: 'text'; text: string }
  | { kind: 'widget'; widget: SduiWidget }
  | { kind: 'bad'; raw: string };

/** Split raw model text into plain-text parts and SDUI widget parts. */
export function parseSdui(text: string): SduiPart[] {
  const fence = /```sdui[ \t]*\r?\n([\s\S]*?)```/g;
  const parts: SduiPart[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = fence.exec(text)) !== null) {
    if (m.index > last) parts.push({ kind: 'text', text: text.slice(last, m.index) });
    try {
      const w = JSON.parse(m[1]) as SduiWidget;
      if (w && typeof w === 'object' && typeof w.type === 'string') {
        parts.push({ kind: 'widget', widget: w });
      } else {
        parts.push({ kind: 'bad', raw: m[1] });
      }
    } catch {
      parts.push({ kind: 'bad', raw: m[1] });
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) parts.push({ kind: 'text', text: text.slice(last) });
  return parts;
}

/** All successfully parsed widgets in a text, in order. */
export function extractSduiWidgets(text: string): SduiWidget[] {
  return parseSdui(text)
    .filter((p): p is { kind: 'widget'; widget: SduiWidget } => p.kind === 'widget')
    .map((p) => p.widget);
}

/** Model text with every ```sdui block removed. */
export function stripSdui(text: string): string {
  return parseSdui(text)
    .filter((p): p is { kind: 'text'; text: string } => p.kind === 'text')
    .map((p) => p.text)
    .join('')
    .trim();
}

function PollWidget({ widget, onToast }: { widget: SduiPoll; onToast: (t: string) => void }): React.ReactElement {
  const [voted, setVoted] = useState<number | null>(null);
  const options = Array.isArray(widget.options) ? widget.options : [];
  return (
    <div className="gf-sdui">
      {widget.title && <span className="gf-label">{widget.title}</span>}
      {options.map((opt, i) => (
        <div key={i} className="gf-row" style={{ marginTop: 4 }}>
          <button
            className="gf-btn"
            style={{ padding: '4px 10px' }}
            onClick={() => {
              setVoted(i);
              onToast(`Voted: ${opt}`);
            }}
          >
            {voted === i ? '[x]' : '[ ]'}
          </button>
          <span>{opt}</span>
        </div>
      ))}
      {options.length === 0 && <span className="gf-muted">No poll options provided.</span>}
    </div>
  );
}

function ChecklistWidget({ widget }: { widget: SduiChecklist }): React.ReactElement {
  const items = Array.isArray(widget.items) ? widget.items : [];
  const [checked, setChecked] = useState<boolean[]>(items.map(() => false));
  return (
    <div className="gf-sdui">
      {widget.title && <span className="gf-label">{widget.title}</span>}
      {items.map((label, i) => (
        <label key={i} className="gf-row" style={{ marginTop: 4, cursor: 'pointer' }}>
          <input
            type="checkbox"
            checked={checked[i] ?? false}
            onChange={() => {
              const next = [...checked];
              next[i] = !next[i];
              setChecked(next);
            }}
          />
          <span>{label}</span>
        </label>
      ))}
      {items.length === 0 && <span className="gf-muted">No checklist items provided.</span>}
    </div>
  );
}

function SliderWidget({ widget }: { widget: SduiSlider }): React.ReactElement {
  const min = typeof widget.min === 'number' ? widget.min : 0;
  const max = typeof widget.max === 'number' ? widget.max : 100;
  const [value, setValue] = useState<number>(
    typeof widget.value === 'number' ? widget.value : min,
  );
  return (
    <div className="gf-sdui">
      <span className="gf-label">
        {widget.title ?? 'Slider'}: {value}
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={widget.step ?? 1}
        value={value}
        onChange={(e) => setValue(Number(e.target.value))}
        style={{ width: '100%' }}
      />
    </div>
  );
}

function CardWidget({ widget }: { widget: SduiCard }): React.ReactElement {
  return (
    <div className="gf-sdui">
      {widget.title && <span className="gf-label">{widget.title}</span>}
      <div style={{ whiteSpace: 'pre-wrap', fontSize: 13 }}>{widget.body ?? ''}</div>
    </div>
  );
}

function TableWidget({ widget }: { widget: SduiTable }): React.ReactElement {
  const columns = Array.isArray(widget.columns) ? widget.columns : [];
  const rows = Array.isArray(widget.rows) ? widget.rows : [];
  return (
    <div className="gf-sdui">
      {widget.title && <span className="gf-label">{widget.title}</span>}
      <table className="gf-sdui-table">
        <thead>
          <tr>
            {columns.map((c, i) => (
              <th key={i}>{c}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i}>
              {(Array.isArray(row) ? row : []).map((cell, j) => (
                <td key={j}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {columns.length === 0 && <span className="gf-muted">No table columns provided.</span>}
    </div>
  );
}

function FormWidget({ widget, onToast }: { widget: SduiForm; onToast: (t: string) => void }): React.ReactElement {
  const fields = Array.isArray(widget.fields) ? widget.fields : [];
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(fields.map((f) => [f.name, f.value ?? ''])),
  );
  const [submitted, setSubmitted] = useState(false);
  const submit = () => {
    setSubmitted(true);
    onToast(`Form submitted${widget.title ? `: ${widget.title}` : ''}`);
  };
  return (
    <div className="gf-sdui">
      {widget.title && <span className="gf-label">{widget.title}</span>}
      {fields.map((f, i) => (
        <div key={i} style={{ marginTop: 6 }}>
          <span className="gf-label">{f.label ?? f.name}</span>
          <input
            className="gf-input"
            style={{ width: '100%' }}
            type={f.type ?? 'text'}
            value={values[f.name] ?? ''}
            placeholder={f.placeholder ?? ''}
            onChange={(e) => setValues((v) => ({ ...v, [f.name]: e.target.value }))}
          />
        </div>
      ))}
      <div className="gf-row" style={{ marginTop: 8 }}>
        <button className="gf-btn primary" onClick={submit}>
          {widget.submit ?? 'Submit'}
        </button>
      </div>
      {submitted && (
        <div className="gf-muted" style={{ marginTop: 6, fontSize: 12 }}>
          Submitted:{' '}
          {fields.map((f) => `${f.label ?? f.name}=${values[f.name] ?? ''}`).join(' · ')}
        </div>
      )}
    </div>
  );
}

function ApprovalWidget({ widget, onToast }: { widget: SduiApproval; onToast: (t: string) => void }): React.ReactElement {
  const [decision, setDecision] = useState<'approved' | 'rejected' | null>(null);
  const decide = (d: 'approved' | 'rejected') => {
    setDecision(d);
    onToast(d === 'approved' ? 'Approved' : 'Rejected');
  };
  return (
    <div className="gf-sdui gf-sdui-approval">
      <span className="gf-label">{widget.title ?? 'Approval requested'}</span>
      {widget.body && <div style={{ whiteSpace: 'pre-wrap', fontSize: 13, marginBottom: 8 }}>{widget.body}</div>}
      {decision === null ? (
        <div className="gf-row" style={{ gap: 8 }}>
          <button className="gf-btn primary" onClick={() => decide('approved')}>
            {widget.approveLabel ?? 'Approve'}
          </button>
          <button className="gf-btn danger" onClick={() => decide('rejected')}>
            {widget.rejectLabel ?? 'Reject'}
          </button>
        </div>
      ) : (
        <div className={decision === 'approved' ? 'gf-gold-text' : ''} style={{ fontSize: 13 }}>
          {decision === 'approved' ? 'Approved' : 'Rejected'}
        </div>
      )}
    </div>
  );
}

function WidgetView({ widget, onToast }: { widget: SduiWidget; onToast: (t: string) => void }): React.ReactElement {
  switch (widget.type) {
    case 'poll':
      return <PollWidget widget={widget} onToast={onToast} />;
    case 'checklist':
      return <ChecklistWidget widget={widget} />;
    case 'slider':
      return <SliderWidget widget={widget} />;
    case 'card':
      return <CardWidget widget={widget} />;
    case 'table':
      return <TableWidget widget={widget} />;
    case 'form':
      return <FormWidget widget={widget} onToast={onToast} />;
    case 'approval':
      return <ApprovalWidget widget={widget} onToast={onToast} />;
    default: {
      const unknownType = (widget as unknown as { type?: string }).type ?? '?';
      return (
        <div className="gf-sdui">
          <span className="gf-muted">Unknown SDUI widget type &ldquo;{unknownType}&rdquo;.</span>
        </div>
      );
    }
  }
}

interface SduiRendererProps {
  text: string;
  onToast: (t: string) => void;
  /** Render only the widgets, skipping the plain-text parts. */
  widgetsOnly?: boolean;
}

/**
 * Render model text: plain-text parts as-is plus interactive widgets for
 * every ```sdui fenced JSON block. Malformed blocks surface an honest note
 * instead of failing silently.
 */
export function SduiRenderer({ text, onToast, widgetsOnly = false }: SduiRendererProps): React.ReactElement {
  const parts = useMemo(() => parseSdui(text), [text]);
  return (
    <>
      {parts.map((p, i) => {
        if (p.kind === 'text') {
          return widgetsOnly ? null : (
            <span key={i} style={{ whiteSpace: 'pre-wrap' }}>
              {p.text}
            </span>
          );
        }
        if (p.kind === 'widget') {
          return <WidgetView key={i} widget={p.widget} onToast={onToast} />;
        }
        return (
          <div key={i} className="gf-sdui">
            <span className="gf-muted">An SDUI block could not be parsed as JSON and was skipped.</span>
          </div>
        );
      })}
    </>
  );
}
