// SPDX-License-Identifier: Apache-2.0
// Phase 11 — Diagnostics dashboard: machine resources, token throughput,
// model/provider status, network state, build worker state.
//
// Offline-first (spec section 54): when diag_network() reports offline, every
// network-dependent action shows an OFFLINE badge with a plain explanation
// instead of failing ambiguously. The snapshot itself is local and always works
// on desktop. All invokes fall back to "Desktop Capability Required".

import React, { useCallback, useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import { getProviders, type ProviderSummary } from '../../lib/providers';
import '../../a11y.css';
import './diagnostics.css';

// Phase-local types — match diagnostics.rs serialisation.
interface GpuInfo {
  info?: string;
  note: string;
}

interface DiagSnapshot {
  cpu_pct: number;
  ram_used_mb: number;
  ram_total_mb: number;
  disk_free_gb: number;
  network_up: boolean;
  app_memory_mb: number;
  gpu: GpuInfo;
}

interface TokenStats {
  tokens_in: number;
  tokens_out: number;
}

interface BuildWorkerState {
  workers: unknown[];
  note: string;
}

interface Props {
  onToast?: (text: string) => void;
}

function errLabel(err: unknown): string {
  if (err instanceof DesktopCapabilityRequired) return 'Desktop Capability Required';
  return err instanceof Error ? err.message : 'unknown error';
}

function ResourceBar({
  label,
  value,
  display,
  dangerAt = 90,
}: {
  label: string;
  value: number; // 0..100
  display: string;
  dangerAt?: number;
}): React.ReactElement {
  const pct = Math.max(0, Math.min(100, value));
  const cls = pct >= dangerAt ? 'hot' : pct >= dangerAt - 15 ? 'warm' : '';
  return (
    <div className="gf-diag-row">
      <span className="gf-muted" style={{ minWidth: 110 }}>{label}</span>
      <div
        className="gf-bar-track"
        role="progressbar"
        aria-label={label}
        aria-valuenow={Math.round(pct)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuetext={display}
      >
        <div className={`gf-bar-fill ${cls}`} style={{ width: `${pct}%` }} />
      </div>
      <span className="gf-diag-value">{display}</span>
    </div>
  );
}

function NetworkBadge({ online }: { online: boolean | null }): React.ReactElement {
  if (online === null) return <span className="gf-badge idle">CHECKING</span>;
  return online ? (
    <span className="gf-badge ok">ONLINE</span>
  ) : (
    <span className="gf-badge err">OFFLINE</span>
  );
}

export function DiagnosticsPanel({ onToast }: Props): React.ReactElement {
  const [snapshot, setSnapshot] = useState<DiagSnapshot | null>(null);
  const [tokens, setTokens] = useState<TokenStats | null>(null);
  const [providers, setProviders] = useState<ProviderSummary[] | null>(null);
  const [buildWorkers, setBuildWorkers] = useState<BuildWorkerState | null>(null);
  const [loading, setLoading] = useState(false);
  const [desktop] = useState(() => isDesktop());

  const refresh = useCallback(async (): Promise<void> => {
    if (!isDesktop()) return;
    setLoading(true);
    try {
      const [snap, tok, bw] = await Promise.all([
        invoke<DiagSnapshot>('diag_snapshot'),
        invoke<TokenStats>('diag_token_stats').catch(() => null),
        invoke<BuildWorkerState>('diag_build_workers').catch(() => null),
      ]);
      setSnapshot(snap);
      setTokens(tok);
      setBuildWorkers(bw);
    } catch (err) {
      onToast?.(errLabel(err));
    } finally {
      setLoading(false);
    }
  }, [onToast]);

  const refreshProviders = useCallback(async (): Promise<void> => {
    if (!isDesktop()) return;
    try {
      // Only attempt the network-dependent provider list when the machine
      // reports online — offline means an honest OFFLINE state, not an
      // ambiguous error list.
      const net = await invoke<boolean>('diag_network');
      if (!net) {
        setProviders(null);
        return;
      }
      setProviders(await getProviders());
    } catch (err) {
      onToast?.(errLabel(err));
    }
  }, [onToast]);

  useEffect(() => {
    void refresh();
    void refreshProviders();
  }, [refresh, refreshProviders]);

  if (!desktop) {
    return (
      <div className="gf-diag">
        <p className="gf-info-note">
          Desktop Capability Required — diagnostics need the native app runtime.
        </p>
      </div>
    );
  }

  const offline = snapshot !== null && !snapshot.network_up;
  const ramPct = snapshot && snapshot.ram_total_mb > 0
    ? (snapshot.ram_used_mb / snapshot.ram_total_mb) * 100
    : 0;

  return (
    <div className="gf-diag" aria-label="Diagnostics dashboard">
      {offline && (
        <p className="gf-offline-note" role="status">
          Network is OFFLINE. AI and network features are paused — anything below
          marked OFFLINE is waiting for connectivity, not broken.
        </p>
      )}

      <section className="gf-diag-section" aria-label="Machine resources">
        <div className="gf-diag-section-head">
          <span>Resources</span>
          <button className="gf-icon-btn" onClick={() => void refresh()} disabled={loading}>
            {loading ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>
        <div className="gf-diag-section-body">
          {snapshot ? (
            <>
              <ResourceBar
                label="CPU"
                value={snapshot.cpu_pct}
                display={`${snapshot.cpu_pct.toFixed(1)} %`}
              />
              <ResourceBar
                label="Memory"
                value={ramPct}
                display={`${snapshot.ram_used_mb.toLocaleString()} / ${snapshot.ram_total_mb.toLocaleString()} MB`}
              />
              <div className="gf-diag-row">
                <span className="gf-muted" style={{ minWidth: 110 }}>Disk free</span>
                <span className="gf-diag-value">{snapshot.disk_free_gb.toFixed(1)} GB</span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted" style={{ minWidth: 110 }}>App memory</span>
                <span className="gf-diag-value">{snapshot.app_memory_mb.toLocaleString()} MB</span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted" style={{ minWidth: 110 }}>Graphics</span>
                <span className="gf-diag-value" style={{ textAlign: 'right' }}>
                  {snapshot.gpu.info ?? 'not detected'}
                  <span className="gf-muted"> ({snapshot.gpu.note})</span>
                </span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted" style={{ minWidth: 110 }}>Network</span>
                <NetworkBadge online={snapshot.network_up} />
              </div>
            </>
          ) : (
            <p className="gf-muted">{loading ? 'Reading system state…' : 'No snapshot yet.'}</p>
          )}
        </div>
      </section>

      <section className="gf-diag-section" aria-label="Token throughput">
        <div className="gf-diag-section-head">
          <span>Token throughput</span>
        </div>
        <div className="gf-diag-section-body">
          {tokens ? (
            <>
              <div className="gf-diag-row">
                <span className="gf-muted">Tokens in (session)</span>
                <span className="gf-diag-value">{tokens.tokens_in.toLocaleString()}</span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted">Tokens out (session)</span>
                <span className="gf-diag-value">{tokens.tokens_out.toLocaleString()}</span>
              </div>
            </>
          ) : (
            <p className="gf-info-note">
              Token counters are published by the provider layer. Showing 0 or
              unavailable is honest until that phase lands — nothing is estimated.
            </p>
          )}
        </div>
      </section>

      <section className="gf-diag-section" aria-label="Model and provider status">
        <div className="gf-diag-section-head">
          <span>Models</span>
          <button className="gf-icon-btn" onClick={() => void refreshProviders()}>
            Re-check
          </button>
        </div>
        <div className="gf-diag-section-body">
          {offline ? (
            <p className="gf-offline-note" role="status">
              <span className="gf-badge err" style={{ marginRight: 8 }}>OFFLINE</span>
              Model status needs a network connection to reach providers. It will
              update automatically when you are back online.
            </p>
          ) : providers === null ? (
            <p className="gf-muted">Provider list not loaded yet.</p>
          ) : providers.length === 0 ? (
            <p className="gf-muted">No providers configured.</p>
          ) : (
            <ul className="gf-provider-list">
              {providers.map((p) => (
                <li key={p.id}>
                  <span>{p.name}</span>
                  <span
                    className={`gf-badge ${p.status === 'connected' ? 'ok' : p.status === 'error' ? 'err' : 'idle'}`}
                  >
                    {p.status.toUpperCase()}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>

      <section className="gf-diag-section" aria-label="Build worker state">
        <div className="gf-diag-section-head">
          <span>Build workers</span>
        </div>
        <div className="gf-diag-section-body">
          {buildWorkers ? (
            <p className="gf-info-note">{buildWorkers.note}</p>
          ) : (
            <p className="gf-muted">Not loaded.</p>
          )}
        </div>
      </section>
    </div>
  );
}
