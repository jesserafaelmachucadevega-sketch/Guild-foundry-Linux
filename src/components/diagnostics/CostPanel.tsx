// SPDX-License-Identifier: Apache-2.0
// CostPanel — token usage ledger + editable per-million pricing.
//
// Numbers are never invented: a model with no saved price shows its token
// counts with a "—" cost and is flagged as unpriced until the user sets a
// price they trust.

import React, { useEffect, useState } from 'react';
import { invoke } from '../../lib/api';

interface UsageRow {
  day: string;
  providerId: string;
  model: string;
  promptTokens: number;
  completionTokens: number;
  cost: number;
  priced: boolean;
}

interface Summary {
  days: number;
  rows: UsageRow[];
  totalCost: number;
  totalPrompt: number;
  totalCompletion: number;
  unpricedModels: string[];
}

interface PriceRow {
  model: string;
  inputPerMtok: number;
  outputPerMtok: number;
}

function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const cellStyle: React.CSSProperties = { padding: '2px 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' };

export function CostPanel({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [days, setDays] = useState(30);
  const [sum, setSum] = useState<Summary | null>(null);
  const [prices, setPrices] = useState<PriceRow[]>([]);
  const [model, setModel] = useState('');
  const [inPrice, setInPrice] = useState('');
  const [outPrice, setOutPrice] = useState('');
  const [busy, setBusy] = useState(false);

  const refresh = async (d: number = days): Promise<void> => {
    try {
      const [s, p] = await Promise.all([
        invoke<Summary>('cost_summary', { days: d }),
        invoke<PriceRow[]>('cost_prices'),
      ]);
      setSum(s);
      setPrices(p ?? []);
    } catch (e) {
      onToast(errText(e));
    }
  };

  useEffect(() => {
    void refresh(days);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [days]);

  const savePrice = async (): Promise<void> => {
    if (!model.trim()) {
      onToast('Enter the model id first (as the provider reports it)');
      return;
    }
    const i = Number.parseFloat(inPrice || '0');
    const o = Number.parseFloat(outPrice || '0');
    if (Number.isNaN(i) || Number.isNaN(o)) {
      onToast('Prices must be numbers');
      return;
    }
    setBusy(true);
    try {
      await invoke('cost_price_set', {
        model: model.trim(),
        input_per_mtok: i,
        output_per_mtok: o,
      });
      onToast(`Price saved for ${model.trim()}`);
      setModel('');
      setInPrice('');
      setOutPrice('');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const removePrice = async (m: string): Promise<void> => {
    setBusy(true);
    try {
      await invoke('cost_price_remove', { model: m });
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const clearUsage = async (): Promise<void> => {
    if (!window.confirm('Delete the entire usage ledger? Model prices are kept.')) return;
    setBusy(true);
    try {
      await invoke('cost_usage_clear');
      onToast('Usage ledger cleared');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="gf-pane" style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: 16 }}>
      <div className="gf-row" style={{ justifyContent: 'space-between', flexWrap: 'wrap' }}>
        <span className="gf-label">Usage &amp; cost — per-million-token prices, in your own currency units</span>
        <span className="gf-view-switcher" role="tablist" aria-label="Window">
          {[7, 30, 90].map((d) => (
            <button
              key={d}
              role="tab"
              aria-selected={days === d}
              className={days === d ? 'active' : ''}
              onClick={() => setDays(d)}
            >
              {d}d
            </button>
          ))}
        </span>
      </div>

      {sum && (
        <div className="gf-row" style={{ flexWrap: 'wrap', marginTop: 8 }}>
          <span className="gf-badge ok">spend {sum.totalCost.toFixed(4)}</span>
          <span className="gf-badge">in {fmtNum(sum.totalPrompt)}</span>
          <span className="gf-badge">out {fmtNum(sum.totalCompletion)}</span>
          {sum.unpricedModels.length > 0 && (
            <span className="gf-badge err">unpriced: {sum.unpricedModels.join(', ')}</span>
          )}
        </div>
      )}
      {sum && sum.unpricedModels.length > 0 && (
        <p className="gf-muted" style={{ fontSize: 12 }}>
          Unpriced models show tokens but count as 0 spend. Add their prices below to get real numbers.
        </p>
      )}

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '90px 1fr 1.5fr 70px 70px 80px',
          gap: 8,
          fontSize: 12,
          marginTop: 12,
        }}
      >
        <span className="gf-muted">Day</span>
        <span className="gf-muted">Provider</span>
        <span className="gf-muted">Model</span>
        <span className="gf-muted" style={{ textAlign: 'right' }}>In</span>
        <span className="gf-muted" style={{ textAlign: 'right' }}>Out</span>
        <span className="gf-muted" style={{ textAlign: 'right' }}>Cost</span>
        {(sum?.rows ?? []).map((r, idx) => (
          <React.Fragment key={`${r.day}:${r.providerId}:${r.model}:${idx}`}>
            <span style={cellStyle}>{r.day}</span>
            <span style={cellStyle}>{r.providerId}</span>
            <span style={cellStyle} title={r.model}>
              {r.model}
              {!r.priced && ' *'}
            </span>
            <span style={{ ...cellStyle, textAlign: 'right' }}>{fmtNum(r.promptTokens)}</span>
            <span style={{ ...cellStyle, textAlign: 'right' }}>{fmtNum(r.completionTokens)}</span>
            <span style={{ ...cellStyle, textAlign: 'right' }}>
              {r.priced ? r.cost.toFixed(4) : '—'}
            </span>
          </React.Fragment>
        ))}
        {sum && sum.rows.length === 0 && (
          <span className="gf-muted" style={{ gridColumn: '1 / -1' }}>
            No usage recorded in this window yet — send a message to any model.
          </span>
        )}
      </div>

      <hr className="gf-divider" />
      <span className="gf-label">Model prices</span>
      <div className="gf-row" style={{ flexWrap: 'wrap', gap: 8, marginTop: 6 }}>
        <input
          className="gf-input"
          style={{ minWidth: 220 }}
          value={model}
          onChange={(e) => setModel(e.target.value)}
          placeholder="model id as the provider reports it"
        />
        <input
          className="gf-input"
          style={{ width: 110 }}
          value={inPrice}
          onChange={(e) => setInPrice(e.target.value)}
          placeholder="in / M"
        />
        <input
          className="gf-input"
          style={{ width: 110 }}
          value={outPrice}
          onChange={(e) => setOutPrice(e.target.value)}
          placeholder="out / M"
        />
        <button className="gf-btn primary" disabled={busy} onClick={() => void savePrice()}>
          Save price
        </button>
        <button className="gf-btn danger" disabled={busy} onClick={() => void clearUsage()}>
          Clear ledger
        </button>
      </div>
      <div className="gf-row" style={{ flexWrap: 'wrap', gap: 6, marginTop: 8 }}>
        {prices.map((p) => (
          <span
            key={p.model}
            className="gf-badge"
            title={`in ${p.inputPerMtok}/M · out ${p.outputPerMtok}/M`}
          >
            {p.model}: in {p.inputPerMtok} / out {p.outputPerMtok}
            <button
              className="gf-btn gf-btn-sm"
              style={{ marginLeft: 6 }}
              onClick={() => void removePrice(p.model)}
            >
              ×
            </button>
          </span>
        ))}
        {prices.length === 0 && (
          <span className="gf-muted" style={{ fontSize: 12 }}>
            No prices set yet — usage above still shows raw token counts.
          </span>
        )}
      </div>
    </div>
  );
}
