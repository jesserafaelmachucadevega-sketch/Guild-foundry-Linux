// SPDX-License-Identifier: Apache-2.0
// Native account cards — real email (IMAP/SMTP) and calendar (CalDAV)
// accounts, not MCP servers. Host/user config lives in the settings store;
// passwords live ONLY in the OS keychain.
//
// These accounts back the email.* and calendar.* agent tools. Google and
// Microsoft calendar accounts need OAuth — connect their connectors under
// "Connect a service" instead; this card is for CalDAV providers
// (Nextcloud, Fastmail, Radicale, iCloud with an app password, and similar).

import React, { useEffect, useState } from 'react';
import { invoke } from '../../lib/api';

export interface AccountTest {
  ok: boolean;
  layman: string;
  detail: string;
  latency_ms: number;
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function Badge({ test, busy }: { test: AccountTest | null; busy: boolean }): React.ReactElement {
  if (busy) return <span className="conn-badge">Testing…</span>;
  if (!test) return <span className="conn-badge">Not tested</span>;
  if (test.ok) return <span className="conn-badge conn-badge-ok">Connected · {test.latency_ms} ms</span>;
  return (
    <span className="conn-badge" title={test.detail}>
      {test.layman}
    </span>
  );
}

const labelStyle: React.CSSProperties = { display: 'flex', flexDirection: 'column', gap: 4, marginTop: 6 };

interface EmailPreset {
  id: string;
  label: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  security: 'tls' | 'starttls';
  note: string;
}

const EMAIL_PRESETS: EmailPreset[] = [
  {
    id: 'gmail',
    label: 'Gmail',
    imap_host: 'imap.gmail.com',
    imap_port: 993,
    smtp_host: 'smtp.gmail.com',
    smtp_port: 587,
    security: 'tls',
    note: 'Use a Google app password (requires 2-Step Verification): Google Account → Security → App passwords.',
  },
  {
    id: 'outlook',
    label: 'Outlook / Microsoft 365',
    imap_host: 'outlook.office365.com',
    imap_port: 993,
    smtp_host: 'smtp.office365.com',
    smtp_port: 587,
    security: 'tls',
    note: 'Microsoft is retiring basic auth on many accounts — if sign-in fails, use an app password or connect the Microsoft connector under "Connect a service".',
  },
  {
    id: 'custom',
    label: 'Custom IMAP/SMTP',
    imap_host: '',
    imap_port: 993,
    smtp_host: '',
    smtp_port: 587,
    security: 'tls',
    note: 'Any standard IMAP + SMTP account works. IMAP usually 993 (TLS); SMTP usually 587 (STARTTLS).',
  },
];

interface EmailForm {
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  security: 'tls' | 'starttls';
  user: string;
  from_name: string;
}

function EmailCard({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [presetId, setPresetId] = useState('gmail');
  const [form, setForm] = useState<EmailForm>({
    imap_host: 'imap.gmail.com',
    imap_port: 993,
    smtp_host: 'smtp.gmail.com',
    smtp_port: 587,
    security: 'tls',
    user: '',
    from_name: '',
  });
  const [password, setPassword] = useState('');
  const [hasPass, setHasPass] = useState(false);
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<AccountTest | null>(null);

  useEffect(() => {
    let live = true;
    void invoke<Record<string, unknown> | null>('settings_get', { key: 'email.account' })
      .then((v) => {
        if (!live || !v || typeof v !== 'object') return;
        setForm((f) => ({
          imap_host: typeof v.imap_host === 'string' ? v.imap_host : f.imap_host,
          imap_port: typeof v.imap_port === 'number' ? v.imap_port : f.imap_port,
          smtp_host: typeof v.smtp_host === 'string' ? v.smtp_host : f.smtp_host,
          smtp_port: typeof v.smtp_port === 'number' ? v.smtp_port : f.smtp_port,
          security: v.security === 'starttls' ? 'starttls' : 'tls',
          user: typeof v.user === 'string' ? v.user : f.user,
          from_name: typeof v.from_name === 'string' ? v.from_name : f.from_name,
        }));
      })
      .catch(() => {});
    void invoke<string[]>('secret_list_keys')
      .then((keys) => {
        if (live) setHasPass(keys.includes('gfa-email:default'));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const preset = EMAIL_PRESETS.find((p) => p.id === presetId) ?? EMAIL_PRESETS[0];

  const applyPreset = (id: string): void => {
    const p = EMAIL_PRESETS.find((x) => x.id === id);
    if (!p) return;
    setPresetId(id);
    setForm((f) => ({
      ...f,
      imap_host: p.imap_host || f.imap_host,
      imap_port: p.imap_port,
      smtp_host: p.smtp_host || f.smtp_host,
      smtp_port: p.smtp_port,
      security: p.security,
    }));
    setTest(null);
  };

  const save = async (): Promise<void> => {
    if (!form.user.trim()) {
      onToast('Enter the email username first');
      return;
    }
    setBusy(true);
    try {
      await invoke('settings_set', {
        key: 'email.account',
        value: { label: preset.label, ...form },
      });
      if (password.trim()) {
        await invoke('secret_set', { key: 'gfa-email:default', value: password.trim() });
        setHasPass(true);
        setPassword('');
      }
      setTest(null);
      onToast(hasPass || password.trim() ? 'Email account saved' : 'Email account saved — now add the app password');
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const testConn = async (): Promise<void> => {
    setBusy(true);
    setTest(null);
    try {
      const r = await invoke<AccountTest>('email_test_connection');
      setTest(r);
      onToast(r.ok ? `Email: ${r.detail}` : `Email: ${r.detail}`);
    } catch (e) {
      setTest({ ok: false, layman: 'Test failed', detail: errText(e), latency_ms: 0 });
    } finally {
      setBusy(false);
    }
  };

  const remove = async (): Promise<void> => {
    if (!window.confirm('Remove the saved email account? The email tools will stop working.')) return;
    try {
      await invoke('secret_delete', { key: 'gfa-email:default' }).catch(() => {});
      await invoke('settings_set', { key: 'email.account', value: null });
      setHasPass(false);
      setTest(null);
      onToast('Email account removed');
    } catch (e) {
      onToast(errText(e));
    }
  };

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>Email (IMAP/SMTP)</strong>
        <Badge test={test} busy={busy} />
      </div>
      <p className="conn-tagline">The agent reads, lists, and sends mail from your real account — email.list, email.read, email.send.</p>
      <select className="gf-input" value={presetId} onChange={(e) => applyPreset(e.target.value)}>
        {EMAIL_PRESETS.map((p) => (
          <option key={p.id} value={p.id}>
            {p.label}
          </option>
        ))}
      </select>
      <p className="conn-note">{preset.note}</p>
      <label className="gf-label" style={labelStyle}>
        IMAP host
        <input className="gf-input" value={form.imap_host} onChange={(e) => setForm((f) => ({ ...f, imap_host: e.target.value }))} placeholder="imap.example.com" />
      </label>
      <label className="gf-label" style={labelStyle}>
        SMTP host
        <input className="gf-input" value={form.smtp_host} onChange={(e) => setForm((f) => ({ ...f, smtp_host: e.target.value }))} placeholder="smtp.example.com" />
      </label>
      <label className="gf-label" style={labelStyle}>
        Username (full email address)
        <input className="gf-input" value={form.user} onChange={(e) => setForm((f) => ({ ...f, user: e.target.value }))} placeholder="you@example.com" />
      </label>
      <label className="gf-label" style={labelStyle}>
        From name (optional)
        <input className="gf-input" value={form.from_name} onChange={(e) => setForm((f) => ({ ...f, from_name: e.target.value }))} placeholder="Jesse" />
      </label>
      <label className="gf-label" style={labelStyle}>
        App password {hasPass ? '(saved — type to replace)' : ''}
        <input className="gf-input" type="password" value={password} onChange={(e) => setPassword(e.target.value)} placeholder={hasPass ? '••••••••' : 'app password'} />
      </label>
      <div className="gf-row" style={{ marginTop: 8 }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void save()} disabled={busy}>
          Save
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void testConn()} disabled={busy || !hasPass} title={hasPass ? 'Verify sign-in now' : 'Save the app password first'}>
          Test
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void remove()} disabled={busy}>
          Remove
        </button>
      </div>
      <p className="conn-note">Passwords go to your OS keychain — the agent sees tool names, never credentials.</p>
    </div>
  );
}

function CalendarCard({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [baseUrl, setBaseUrl] = useState('');
  const [user, setUser] = useState('');
  const [label, setLabel] = useState('');
  const [password, setPassword] = useState('');
  const [hasPass, setHasPass] = useState(false);
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<AccountTest | null>(null);

  useEffect(() => {
    let live = true;
    void invoke<Record<string, unknown> | null>('settings_get', { key: 'calendar.account' })
      .then((v) => {
        if (!live || !v || typeof v !== 'object') return;
        if (typeof v.base_url === 'string') setBaseUrl(v.base_url);
        if (typeof v.user === 'string') setUser(v.user);
        if (typeof v.label === 'string') setLabel(v.label);
      })
      .catch(() => {});
    void invoke<string[]>('secret_list_keys')
      .then((keys) => {
        if (live) setHasPass(keys.includes('gfa-calendar:default'));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const save = async (): Promise<void> => {
    if (!baseUrl.trim() || !user.trim()) {
      onToast('CalDAV URL and username are required');
      return;
    }
    setBusy(true);
    try {
      await invoke('settings_set', {
        key: 'calendar.account',
        value: { label: label.trim() || 'CalDAV', base_url: baseUrl.trim(), user: user.trim() },
      });
      if (password.trim()) {
        await invoke('secret_set', { key: 'gfa-calendar:default', value: password.trim() });
        setHasPass(true);
        setPassword('');
      }
      setTest(null);
      onToast(hasPass || password.trim() ? 'Calendar account saved' : 'Calendar account saved — now add the password');
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const testConn = async (): Promise<void> => {
    setBusy(true);
    setTest(null);
    try {
      const r = await invoke<AccountTest>('calendar_test_connection');
      setTest(r);
      onToast(r.ok ? `Calendar: ${r.detail}` : `Calendar: ${r.detail}`);
    } catch (e) {
      setTest({ ok: false, layman: 'Test failed', detail: errText(e), latency_ms: 0 });
    } finally {
      setBusy(false);
    }
  };

  const remove = async (): Promise<void> => {
    if (!window.confirm('Remove the saved calendar? The calendar tools will stop working.')) return;
    try {
      await invoke('secret_delete', { key: 'gfa-calendar:default' }).catch(() => {});
      await invoke('settings_set', { key: 'calendar.account', value: null });
      setHasPass(false);
      setTest(null);
      onToast('Calendar account removed');
    } catch (e) {
      onToast(errText(e));
    }
  };

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>Calendar (CalDAV)</strong>
        <Badge test={test} busy={busy} />
      </div>
      <p className="conn-tagline">The agent lists, creates, updates, and deletes events on your real calendar.</p>
      <p className="conn-note">
        CalDAV providers only (Nextcloud, Fastmail, Radicale, iCloud with an app password, and similar). Google and Outlook
        calendars require OAuth — connect their connectors under "Connect a service" and the agent calls those tools instead.
      </p>
      <label className="gf-label" style={labelStyle}>
        Display name (optional)
        <input className="gf-input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Personal" />
      </label>
      <label className="gf-label" style={labelStyle}>
        CalDAV collection URL
        <input className="gf-input" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://cloud.example.com/remote.php/dav/calendars/you/personal/" />
      </label>
      <label className="gf-label" style={labelStyle}>
        Username
        <input className="gf-input" value={user} onChange={(e) => setUser(e.target.value)} placeholder="you@example.com" />
      </label>
      <label className="gf-label" style={labelStyle}>
        App password {hasPass ? '(saved — type to replace)' : ''}
        <input className="gf-input" type="password" value={password} onChange={(e) => setPassword(e.target.value)} placeholder={hasPass ? '••••••••' : 'app password'} />
      </label>
      <div className="gf-row" style={{ marginTop: 8 }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void save()} disabled={busy}>
          Save
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void testConn()} disabled={busy || !hasPass} title={hasPass ? 'Verify sign-in now' : 'Save the password first'}>
          Test
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void remove()} disabled={busy}>
          Remove
        </button>
      </div>
    </div>
  );
}

function PhoneCard({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [accountSid, setAccountSid] = useState('');
  const [fromNumber, setFromNumber] = useState('');
  const [label, setLabel] = useState('');
  const [token, setToken] = useState('');
  const [hasToken, setHasToken] = useState(false);
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<AccountTest | null>(null);
  const [recordCalls, setRecordCalls] = useState(false);

  useEffect(() => {
    let live = true;
    void invoke<Record<string, unknown> | null>('settings_get', { key: 'phone.twilio' })
      .then((v) => {
        if (!live || !v || typeof v !== 'object') return;
        if (typeof v.account_sid === 'string') setAccountSid(v.account_sid);
        if (typeof v.from_number === 'string') setFromNumber(v.from_number);
        if (typeof v.label === 'string') setLabel(v.label);
      })
      .catch(() => {});
    void invoke<string[]>('secret_list_keys')
      .then((keys) => {
        if (live) setHasToken(keys.includes('gfa-twilio:auth'));
      })
      .catch(() => {});
    void invoke<boolean | null>('settings_get', { key: 'phone.record_calls' })
      .then((v) => {
        if (live && typeof v === 'boolean') setRecordCalls(v);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const toggleRecord = async (next: boolean): Promise<void> => {
    setRecordCalls(next);
    try {
      await invoke('settings_set', { key: 'phone.record_calls', value: next });
      onToast(
        next
          ? 'Call recording enabled for new calls — make sure you have consent to record in your jurisdiction.'
          : 'Call recording disabled',
      );
    } catch (e) {
      onToast(errText(e));
    }
  };

  const transcribeLast = async (): Promise<void> => {
    setBusy(true);
    try {
      const r = await invoke<{ transcript: string; memory_key: string; stored_in_memory: boolean; engine: string }>(
        'phone_call_transcribe',
        {},
      );
      const head = r.transcript.slice(0, 100) + (r.transcript.length > 100 ? '…' : '');
      onToast(
        r.stored_in_memory
          ? `Transcript stored in memory (${r.memory_key}): ${head}`
          : `Transcript (${r.engine}): ${head}`,
      );
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const save = async (): Promise<void> => {
    if (!accountSid.trim() || !fromNumber.trim()) {
      onToast('Account SID and your Twilio phone number are required');
      return;
    }
    setBusy(true);
    try {
      await invoke('settings_set', {
        key: 'phone.twilio',
        value: {
          label: label.trim() || 'Twilio',
          account_sid: accountSid.trim(),
          from_number: fromNumber.trim(),
        },
      });
      if (token.trim()) {
        await invoke('secret_set', { key: 'gfa-twilio:auth', value: token.trim() });
        setHasToken(true);
        setToken('');
      }
      setTest(null);
      onToast(hasToken || token.trim() ? 'Phone provider saved' : 'Phone provider saved — now add the auth token');
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const testConn = async (): Promise<void> => {
    setBusy(true);
    setTest(null);
    try {
      const r = await invoke<AccountTest>('twilio_test_connection');
      setTest(r);
      onToast(r.ok ? `Phone: ${r.detail}` : `Phone: ${r.detail}`);
    } catch (e) {
      setTest({ ok: false, layman: 'Test failed', detail: errText(e), latency_ms: 0 });
    } finally {
      setBusy(false);
    }
  };

  const remove = async (): Promise<void> => {
    if (!window.confirm('Remove the phone provider? Calling and texting will stop working.')) return;
    try {
      await invoke('secret_delete', { key: 'gfa-twilio:auth' }).catch(() => {});
      await invoke('settings_set', { key: 'phone.twilio', value: null });
      setHasToken(false);
      setTest(null);
      onToast('Phone provider removed');
    } catch (e) {
      onToast(errText(e));
    }
  };

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>Phone &amp; SMS</strong>
        <Badge test={test} busy={busy} />
      </div>
      <p className="conn-tagline">The agent sends texts and places calls from your number — phone.sms_send, phone.call_start, phone.call_status.</p>
      <p className="conn-note">
        Requires a Twilio account with a phone number. Texts and calls cost real money per use and are always approval-gated before
        anything is sent.
      </p>
      <label className="gf-label" style={labelStyle}>
        Display name (optional)
        <input className="gf-input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="My phone" />
      </label>
      <label className="gf-label" style={labelStyle}>
        Account SID
        <input className="gf-input" value={accountSid} onChange={(e) => setAccountSid(e.target.value)} placeholder="AC…" />
      </label>
      <label className="gf-label" style={labelStyle}>
        Your phone number (E.164)
        <input className="gf-input" value={fromNumber} onChange={(e) => setFromNumber(e.target.value)} placeholder="+15551234567" />
      </label>
      <label className="gf-label" style={labelStyle}>
        Auth token {hasToken ? '(saved — type to replace)' : ''}
        <input className="gf-input" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder={hasToken ? '••••••••' : 'auth token'} />
      </label>
      <div className="gf-row" style={{ marginTop: 8 }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void save()} disabled={busy}>
          Save
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void testConn()} disabled={busy || !hasToken} title={hasToken ? 'Verify credentials now' : 'Save the auth token first'}>
          Test
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void remove()} disabled={busy}>
          Remove
        </button>
        <button
          className="gf-btn gf-btn-sm"
          onClick={() => void transcribeLast()}
          disabled={busy || !hasToken}
          title="Download the newest call recording, transcribe it offline, and store it in memory"
        >
          Transcribe last call
        </button>
      </div>
      <label className="gf-label" style={{ ...labelStyle, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <input
          type="checkbox"
          checked={recordCalls}
          onChange={(e) => void toggleRecord(e.target.checked)}
        />
        Record calls (enables transcripts; you are responsible for consent laws)
      </label>
    </div>
  );
}

interface LocalVoiceStatus {
  whisper_available: boolean;
  whisper_model: string | null;
  kokoro_available: boolean;
  kokoro_note: string;
  kokoro: {
    available: boolean;
    binary: string | null;
    dir: string | null;
    missing: string[];
    voices: { id: number; name: string }[];
    default_voice: string;
    note: string;
  } | null;
  engine: string;
}

function LocalVoiceCard({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [status, setStatus] = useState<LocalVoiceStatus | null>(null);
  const [modelPath, setModelPath] = useState('');
  const [kokoroDir, setKokoroDir] = useState('');
  const [voice, setVoice] = useState('af_bella');
  const [busy, setBusy] = useState(false);

  const refresh = async (): Promise<void> => {
    try {
      const st = await invoke<LocalVoiceStatus>('media_local_voice_status');
      setStatus(st);
      setModelPath(st.whisper_model ?? '');
      setKokoroDir(st.kokoro?.dir ?? '');
      const savedVoice = await invoke<string | null>('settings_get', { key: 'media.tts_voice' });
      if (savedVoice) setVoice(savedVoice);
    } catch {
      setStatus(null);
    }
  };

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = async (): Promise<void> => {
    if (!modelPath.trim()) {
      onToast('Enter the path to a whisper.cpp ggml model first');
      return;
    }
    setBusy(true);
    try {
      await invoke('media_local_set_model', { kind: 'whisper', path: modelPath.trim() });
      onToast('Local Whisper model set — the mic now transcribes offline');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const saveKokoro = async (): Promise<void> => {
    if (!kokoroDir.trim()) {
      onToast('Enter the path to a Kokoro model directory first');
      return;
    }
    setBusy(true);
    try {
      await invoke('media_local_set_model', { kind: 'kokoro', path: kokoroDir.trim() });
      onToast('Kokoro set — replies can now be spoken fully offline');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const saveVoice = async (): Promise<void> => {
    setBusy(true);
    try {
      await invoke('settings_set', { key: 'media.tts_voice', value: voice });
      onToast(`Voice set to ${voice} — used for both offline and cloud speech`);
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>Local voice (offline)</strong>
        {status?.whisper_available ? (
          <span className="conn-badge conn-badge-ok">Whisper ready</span>
        ) : (
          <span className="conn-badge">No model yet</span>
        )}
        {status?.kokoro?.available ? (
          <span className="conn-badge conn-badge-ok">Kokoro ready</span>
        ) : (
          <span className="conn-badge">Kokoro off</span>
        )}
      </div>
      <p className="conn-tagline">
        Speech-to-text runs fully offline with a whisper.cpp model — no cloud call, no API key.
      </p>
      <p className="conn-note">
        Point this at any whisper.cpp ggml model (for example <code>ggml-base.en.bin</code> or{' '}
        <code>ggml-small.bin</code>). Drop it in <code>app-data/models/whisper/whisper-model.bin</code> or paste a full
        path below. The mic prefers this model whenever it is available; otherwise it falls back to the cloud service.
      </p>
      <label className="gf-label" style={labelStyle}>
        Whisper model file
        <input
          className="gf-input"
          value={modelPath}
          onChange={(e) => setModelPath(e.target.value)}
          placeholder="/path/to/ggml-base.en.bin"
        />
      </label>
      <div className="gf-row" style={{ marginTop: 8 }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void save()} disabled={busy}>
          Save
        </button>
        <button className="gf-btn gf-btn-sm" onClick={() => void refresh()} disabled={busy}>
          Refresh
        </button>
      </div>
      <p className="conn-tagline" style={{ marginTop: 12 }}>
        Speech output runs fully offline with Kokoro. It is used automatically whenever it is ready — the cloud
        voice only takes over if the local engine is missing or fails.
      </p>
      <label className="gf-label" style={labelStyle}>
        Kokoro model directory
        <input
          className="gf-input"
          value={kokoroDir}
          onChange={(e) => setKokoroDir(e.target.value)}
          placeholder="/path/to/kokoro-en-v0_19"
        />
      </label>
      <div className="gf-row" style={{ marginTop: 8 }}>
        <button className="gf-btn gf-btn-sm" onClick={() => void saveKokoro()} disabled={busy}>
          Save Kokoro
        </button>
      </div>
      {status?.kokoro && status.kokoro.voices.length > 0 && (
        <div className="gf-row" style={{ marginTop: 8 }}>
          <select className="gf-select" value={voice} onChange={(e) => setVoice(e.target.value)}>
            {status.kokoro.voices.map((v) => (
              <option key={v.id} value={v.name}>
                {v.name}
              </option>
            ))}
          </select>
          <button className="gf-btn gf-btn-sm" onClick={() => void saveVoice()} disabled={busy}>
            Use voice
          </button>
        </div>
      )}
      <p className="conn-note" title={status?.kokoro_note}>
        {status?.kokoro?.note ??
          status?.kokoro_note ??
          'Kokoro status unknown yet — use Refresh.'}
      </p>
    </div>
  );
}

interface MobileStatus {
  running: boolean;
  url?: string | null;
  token?: string | null;
  port: number;
  lan_ip?: string | null;
}

function MobileCard({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [status, setStatus] = useState<MobileStatus | null>(null);
  const [port, setPort] = useState('8787');
  const [busy, setBusy] = useState(false);

  const refresh = async (): Promise<void> => {
    try {
      const st = await invoke<MobileStatus>('mobile_status');
      setStatus(st);
      if (st.port) setPort(String(st.port));
    } catch {
      setStatus(null);
    }
  };

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const start = async (): Promise<void> => {
    setBusy(true);
    try {
      const p = Number.parseInt(port, 10);
      if (Number.isNaN(p)) {
        onToast('Port must be a number');
        return;
      }
      const st = await invoke<{ url: string }>('mobile_start', { port: p });
      await refresh();
      onToast(`Mobile companion running: ${st.url}`);
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async (): Promise<void> => {
    setBusy(true);
    try {
      await invoke('mobile_stop');
      await refresh();
      onToast('Mobile companion stopped');
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const copy = async (): Promise<void> => {
    if (!status?.url) return;
    try {
      await navigator.clipboard.writeText(status.url);
      onToast('Link copied — open it on your phone (same Wi-Fi)');
    } catch (e) {
      onToast(`${errText(e)} — select the link field and copy it manually`);
    }
  };

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>Mobile companion (LAN)</strong>
        {status?.running ? (
          <span className="conn-badge conn-badge-ok">Running</span>
        ) : (
          <span className="conn-badge">Stopped</span>
        )}
      </div>
      <p className="conn-tagline">
        Your phone talks straight to this machine over Wi-Fi — no cloud, no relay. It chats on the same
        thread as the Agent tab, so the conversation follows you between desktop and phone.
      </p>
      {status?.running && status.url ? (
        <>
          <label className="gf-label" style={labelStyle}>
            Open on your phone
            <input
              className="gf-input"
              readOnly
              value={status.url}
              onFocus={(e) => e.currentTarget.select()}
            />
          </label>
          <div className="gf-row" style={{ marginTop: 8 }}>
            <button className="gf-btn gf-btn-sm" onClick={() => void copy()} disabled={busy}>
              Copy link
            </button>
            <button className="gf-btn gf-btn-sm" onClick={() => void stop()} disabled={busy}>
              Stop
            </button>
          </div>
          <p className="conn-note">
            Anyone on your Wi-Fi who has this link can talk to your assistant — treat it like a password.
            A fresh token is generated every time you start the companion.
          </p>
        </>
      ) : (
        <>
          <label className="gf-label" style={labelStyle}>
            Port
            <input className="gf-input" value={port} onChange={(e) => setPort(e.target.value)} placeholder="8787" />
          </label>
          <div className="gf-row" style={{ marginTop: 8 }}>
            <button className="gf-btn gf-btn-sm" onClick={() => void start()} disabled={busy}>
              Start
            </button>
          </div>
          <p className="conn-note">
            Pick a provider and model in the Agent tab first — the phone uses that same selection.
          </p>
        </>
      )}
    </div>
  );
}

export function AccountCards({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  return (
    <>
      <h3 className="conn-section">Accounts (native)</h3>
      <div className="conn-grid">
        <EmailCard onToast={onToast} />
        <CalendarCard onToast={onToast} />
        <PhoneCard onToast={onToast} />
        <LocalVoiceCard onToast={onToast} />
        <MobileCard onToast={onToast} />
      </div>
    </>
  );
}
