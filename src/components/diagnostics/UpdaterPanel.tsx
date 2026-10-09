// SPDX-License-Identifier: Apache-2.0
// Phase 11 — Update system UI: check, release notes, download, postpone,
// install-with-explicit-confirm.
//
// Spec section 71: never silently install. The backend's download step only
// stages the artifact; the final install here requires a typed confirmation,
// and even then the native install step is NOT VERIFIED (no signed feed in
// this build) — the UI says so plainly instead of pretending.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke, isDesktop, DesktopCapabilityRequired } from '../../lib/api';
import '../../a11y.css';
import './diagnostics.css';

interface UpdateCheckResult {
  available: boolean;
  current_version: string;
  version?: string;
  notes?: string;
  url?: string;
  reason?: string;
}

interface DownloadResult {
  status: string;
  version: string;
  asset_name: string;
  downloaded_to: string;
  size_bytes: number;
  note: string;
}

type Phase = 'idle' | 'checking' | 'result' | 'downloading' | 'staged' | 'confirm-install';

interface Props {
  onToast?: (text: string) => void;
}

export function UpdaterPanel({ onToast }: Props): React.ReactElement {
  const [desktop] = useState(() => isDesktop());
  const [phase, setPhase] = useState<Phase>('idle');
  const [check, setCheck] = useState<UpdateCheckResult | null>(null);
  const [staged, setStaged] = useState<DownloadResult | null>(null);
  const [repo, setRepo] = useState('');
  const [repoSaved, setRepoSaved] = useState('');
  const [online, setOnline] = useState<boolean | null>(null);
  const [confirmText, setConfirmText] = useState('');
  const [error, setError] = useState('');

  const loadContext = useCallback(async (): Promise<void> => {
    if (!isDesktop()) return;
    try {
      const [net, saved] = await Promise.all([
        invoke<boolean>('diag_network'),
        invoke<string | null>('settings_get', { key: 'update_repo' }).catch(() => null),
      ]);
      setOnline(net);
      setRepo(typeof saved === 'string' ? saved : '');
      setRepoSaved(typeof saved === 'string' ? saved : '');
    } catch {
      // Non-fatal: panel stays usable with manual entry.
    }
  }, []);

  useEffect(() => {
    void loadContext();
  }, [loadContext]);

  if (!desktop) {
    return (
      <div className="gf-diag">
        <p className="gf-info-note">
          Desktop Capability Required — the updater needs the native app runtime.
        </p>
      </div>
    );
  }

  const saveRepo = async (): Promise<void> => {
    setError('');
    try {
      await invoke('settings_set', { key: 'update_repo', value: repo.trim() });
      setRepoSaved(repo.trim());
      onToast?.('Update feed saved');
    } catch (err) {
      setError(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : `Save failed: ${String(err)}`);
    }
  };

  const checkUpdates = async (): Promise<void> => {
    setError('');
    setPhase('checking');
    try {
      const res = await invoke<UpdateCheckResult>('updater_check');
      setCheck(res);
      setPhase('result');
    } catch (err) {
      setPhase('idle');
      setError(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : `Check failed: ${String(err)}`);
    }
  };

  const postpone = async (): Promise<void> => {
    if (!check?.version) return;
    try {
      await invoke('updater_postpone', { version: check.version });
      onToast?.(`Update ${check.version} postponed`);
      await checkUpdates();
    } catch (err) {
      setError(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : `Postpone failed: ${String(err)}`);
    }
  };

  const clearPostpone = async (): Promise<void> => {
    setError('');
    try {
      await invoke('updater_clear_postpone');
      onToast?.('Postponed update cleared — checks run normally again');
      await checkUpdates();
    } catch (err) {
      setError(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : `Clear failed: ${String(err)}`);
    }
  };

  const dismissStaged = (): void => {
    setStaged(null);
    setConfirmText('');
    if (phase === 'staged' || phase === 'confirm-install') setPhase('result');
  };

  const download = async (): Promise<void> => {
    setError('');
    setPhase('downloading');
    try {
      const res = await invoke<DownloadResult>('updater_download_install');
      setStaged(res);
      setPhase('staged');
    } catch (err) {
      setPhase('result');
      setError(err instanceof DesktopCapabilityRequired ? 'Desktop Capability Required' : `Download failed: ${String(err)}`);
    }
  };

  const confirmInstall = (): void => {
    if (confirmText.trim().toUpperCase() !== 'INSTALL') return;
    // The native install step is NOT wired in this build (no signed feed,
    // no updater plugin). Say so plainly; never pretend to install.
    setPhase('confirm-install');
    setError('');
  };

  const renderCheckResult = (): React.ReactElement | null => {
    if (!check) return null;
    if (check.reason?.startsWith('no-update-feed-configured')) {
      return (
        <p className="gf-info-note">
          No update feed configured. Set the update feed below (e.g.
          <span className="gf-diag-value"> owner/repo</span>) to enable update checks.
        </p>
      );
    }
    if (check.reason?.includes('postponed by user')) {
      return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <p className="gf-info-note">
            Update {check.version} was postponed. Check again anytime to reconsider.
          </p>
          <div className="gf-row">
            <button className="gf-btn" onClick={() => void clearPostpone()}>
              Clear postpone
            </button>
          </div>
        </div>
      );
    }
    if (!check.available) {
      return (
        <p className="gf-info-note">
          You are up to date (v{check.current_version})
          {check.reason && check.reason !== 'up-to-date' ? ` — ${check.reason}` : ''}.
        </p>
      );
    }
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <div className="gf-diag-row">
          <span className="gf-gold-text" style={{ fontWeight: 700 }}>
            Update available: {check.version}
          </span>
          <span className="gf-muted">installed: v{check.current_version}</span>
        </div>
        {check.notes && (
          <div>
            <span className="gf-label">Release notes</span>
            <div className="gf-release-notes" tabIndex={0} aria-label="Release notes">
              {check.notes}
            </div>
          </div>
        )}
        {check.url && (
          <a href={check.url} target="_blank" rel="noreferrer" style={{ color: 'var(--gf-gold)' }}>
            View release page
          </a>
        )}
        <div className="gf-row">
          <button className="gf-btn primary" onClick={() => void download()}>
            Download update
          </button>
          <button className="gf-btn" onClick={() => void postpone()}>
            Postpone
          </button>
          <button className="gf-btn" onClick={() => void clearPostpone()} title="Clear any postponed version">
            Un-postpone
          </button>
        </div>
      </div>
    );
  };

  return (
    <div className="gf-diag" aria-label="Update system">
      {online === false && (
        <p className="gf-offline-note" role="status">
          <span className="gf-badge err" style={{ marginRight: 8 }}>OFFLINE</span>
          Update checks need a network connection. The app itself keeps working
          offline — try again when you are back online.
        </p>
      )}

      <section className="gf-diag-section" aria-label="Update feed">
        <div className="gf-diag-section-head">
          <span>Update feed</span>
        </div>
        <div className="gf-diag-section-body">
          <label className="gf-label" htmlFor="updater-repo">
            GitHub repo (owner/repo) — leave empty to disable update checks
          </label>
          <div className="gf-row">
            <input
              id="updater-repo"
              className="gf-input"
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              placeholder="owner/repo"
              autoComplete="off"
            />
            <button className="gf-btn" onClick={() => void saveRepo()}>
              Save
            </button>
          </div>
          {repoSaved && (
            <span className="gf-muted" style={{ fontSize: 12 }}>
              Feed: <span className="gf-diag-value">{repoSaved}</span>
            </span>
          )}
        </div>
      </section>

      <section className="gf-diag-section" aria-label="Check for updates">
        <div className="gf-diag-section-head">
          <span>Updates</span>
          <span className="gf-muted" style={{ fontSize: 12 }}>
            current: v{check?.current_version ?? '…'}
          </span>
        </div>
        <div className="gf-diag-section-body">
          <div className="gf-row">
            <button
              className="gf-btn primary"
              onClick={() => void checkUpdates()}
              disabled={phase === 'checking' || phase === 'downloading'}
            >
              {phase === 'checking' ? 'Checking…' : 'Check for updates'}
            </button>
            {online === false && <span className="gf-badge err">OFFLINE</span>}
          </div>
          {phase === 'downloading' && <p className="gf-muted">Downloading update…</p>}
          {renderCheckResult()}

          {staged && (phase === 'staged' || phase === 'confirm-install') && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
              <hr className="gf-divider" />
              <div className="gf-diag-row">
                <span className="gf-muted">Staged package</span>
                <span className="gf-diag-value">{staged.asset_name}</span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted">Version</span>
                <span className="gf-diag-value">{staged.version}</span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted">Size</span>
                <span className="gf-diag-value">
                  {(staged.size_bytes / 1_000_000).toFixed(1)} MB
                </span>
              </div>
              <div className="gf-diag-row">
                <span className="gf-muted">Location</span>
                <span className="gf-diag-value" style={{ wordBreak: 'break-all' }}>
                  {staged.downloaded_to}
                </span>
              </div>
              <p className="gf-info-note">{staged.note}</p>

              <div className="gf-row">
                <button className="gf-btn" onClick={dismissStaged}>
                  Dismiss
                </button>
              </div>

              {phase === 'staged' && (
                <>
                  <label className="gf-label" htmlFor="install-confirm">
                    Type INSTALL to proceed — installing replaces the running app
                  </label>
                  <div className="gf-row">
                    <input
                      id="install-confirm"
                      className="gf-input"
                      value={confirmText}
                      onChange={(e) => setConfirmText(e.target.value)}
                      placeholder="Type INSTALL"
                      autoComplete="off"
                    />
                    <button
                      className="gf-btn danger"
                      disabled={confirmText.trim().toUpperCase() !== 'INSTALL'}
                      onClick={confirmInstall}
                    >
                      Install update
                    </button>
                  </div>
                </>
              )}

              {phase === 'confirm-install' && (
                <p className="gf-offline-note" role="status">
                  Install is not wired in this build: the native install step
                  needs a signed update feed and has never been verified
                  end-to-end. The staged package above is ready for manual
                  review at the path shown — nothing was executed.
                </p>
              )}
            </div>
          )}
        </div>
      </section>

      {error && (
        <p className="gf-offline-note" role="alert" style={{ borderColor: 'var(--gf-danger)' }}>
          {error}
        </p>
      )}
    </div>
  );
}
