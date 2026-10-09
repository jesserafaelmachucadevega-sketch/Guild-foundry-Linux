// SPDX-License-Identifier: Apache-2.0
// Connections tab — the agent's friendly surface for account and app access.
// Builders keep the raw MCP panel; this tab answers "give the agent access to
// my Gmail / Outlook / GitHub / terminal" in one click per service.
//
// Everything connected here becomes an MCP server under the hood, which means
// its tools flow into the agent's tool registry automatically. Credentials go
// to the OS keychain only — the agent sees tool names, never tokens.

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '../../lib/api';
import type { McpServerView } from '../mcp/types';
import './connections.css';

interface Props {
  onToast: (msg: string) => void;
}

type AuthKind = 'oauth' | 'bearer' | 'builtin' | 'none';

type ConnectorCategory = 'Google' | 'Microsoft 365' | 'Social media' | 'More services';

const CATEGORY_ORDER: ConnectorCategory[] = ['Google', 'Microsoft 365', 'Social media', 'More services'];

interface ConnectorTemplate {
  id: string;
  name: string;
  tagline: string;
  category: ConnectorCategory;
  auth: AuthKind;
  /** Prefilled server URL; empty means the user pastes it from a registry. */
  urlHint: string;
  urlPlaceholder: string;
  oauth?: {
    authorizationUrl: string;
    tokenUrl: string;
    scopes: { value: string; label: string; recommended?: boolean }[];
    redirectUri: string;
  };
  bearerLabel?: string;
  bearerPlaceholder?: string;
  note?: string;
}

const GOOGLE_OAUTH = {
  authorizationUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
  tokenUrl: 'https://oauth2.googleapis.com/token',
  redirectUri: 'http://127.0.0.1:18793/oauth/callback',
};
const MS_OAUTH = {
  authorizationUrl: 'https://login.microsoftonline.com/common/oauth2/v2.0/authorize',
  tokenUrl: 'https://login.microsoftonline.com/common/oauth2/v2.0/token',
  redirectUri: 'http://127.0.0.1:18793/oauth/callback',
};

const TEMPLATES: ConnectorTemplate[] = [
  {
    id: 'gmail',
    name: 'Gmail',
    tagline: 'Let the agent read and send your email.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Gmail MCP server URL (see starter pack doc)',
    oauth: {
      authorizationUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
      tokenUrl: 'https://oauth2.googleapis.com/token',
      scopes: [
        { value: 'https://www.googleapis.com/auth/gmail.readonly', label: 'Read mail', recommended: true },
        { value: 'https://www.googleapis.com/auth/gmail.send', label: 'Send mail' },
        { value: 'https://www.googleapis.com/auth/gmail.modify', label: 'Read + modify (labels, archive)' },
      ],
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
    },
    note: 'Start with Read-only. You need a Google Cloud OAuth client ID (Desktop app type).',
  },
  {
    id: 'outlook',
    name: 'Outlook',
    tagline: 'Let the agent read and send your Outlook mail.',
    category: 'Microsoft 365',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Outlook MCP server URL (see starter pack doc)',
    oauth: {
      authorizationUrl: 'https://login.microsoftonline.com/common/oauth2/v2.0/authorize',
      tokenUrl: 'https://login.microsoftonline.com/common/oauth2/v2.0/token',
      scopes: [
        { value: 'Mail.Read', label: 'Read mail', recommended: true },
        { value: 'Mail.Send', label: 'Send mail' },
      ],
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
    },
    note: 'Start with Mail.Read. Register an app in Azure Entra ID to get a client ID.',
  },
  {
    id: 'gcal',
    name: 'Google Calendar',
    tagline: 'Let the agent see and manage your schedule.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Calendar MCP server URL',
    oauth: {
      authorizationUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
      tokenUrl: 'https://oauth2.googleapis.com/token',
      scopes: [
        { value: 'https://www.googleapis.com/auth/calendar.readonly', label: 'Read calendars', recommended: true },
        { value: 'https://www.googleapis.com/auth/calendar', label: 'Read + write calendars' },
      ],
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
    },
    note: 'Same Google Cloud OAuth client as Gmail works here.',
  },
  {
    id: 'github',
    name: 'GitHub',
    tagline: 'Repos, issues, PRs, and code search for the agent.',
    category: 'More services',
    auth: 'bearer',
    urlHint: 'https://api.githubcopilot.com/mcp/',
    urlPlaceholder: 'MCP server URL (verify it is current)',
    bearerLabel: 'Personal access token',
    bearerPlaceholder: 'ghp_… / github_pat_…',
    note: 'Use a fine-grained PAT scoped to the repos the agent needs. Verify the server URL in the starter pack doc before connecting.',
  },
  {
    id: 'websearch',
    name: 'Web Search',
    tagline: 'Fresh information on demand — how the agent stays current.',
    category: 'More services',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your search MCP server URL (Brave, Firecrawl…)',
    bearerLabel: 'API key',
    bearerPlaceholder: 'Brave / Firecrawl API key',
    note: 'This is what keeps a local model current: it looks things up instead of guessing from weights.',
  },
  {
    id: 'youtube',
    name: 'YouTube',
    tagline: 'Search videos and pull transcripts — look up anything, read it instead of watching.',
    category: 'More services',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your YouTube MCP server URL (see starter pack doc)',
    bearerLabel: 'API key',
    bearerPlaceholder: 'Server API key (if required)',
    note: 'API-key servers like transcriptapi.com or web-data-toolkit work here. Some hosted servers use their own OAuth sign-in instead — check the starter pack doc.',
  },
  {
    id: 'music',
    name: 'Music Generation',
    tagline: 'Have the agent compose full songs — vocals, lyrics, any genre.',
    category: 'More services',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your music MCP server URL (see starter pack doc)',
    bearerLabel: 'API key',
    bearerPlaceholder: 'AIMLAPI / PiAPI key',
    note: 'AIMLAPI wraps Suno, Udio and more behind one key. Suno is the strongest for full songs with vocals (v6, downloads on paid plans).',
  },
  {
    id: 'browser',
    name: 'Browser Automation',
    tagline: 'Give the agent a real Chromium: click, type, and read any website — no API needed.',
    category: 'More services',
    auth: 'none',
    urlHint: '',
    urlPlaceholder: 'Your Playwright MCP server URL (http://localhost:PORT/mcp)',
    note: 'Run the Playwright MCP server with HTTP transport (keeps the app install lean — no bundled Chromium), then paste its URL. The agent can use any site, and you watch via screenshots in the activity trace.',
  },
  {
    id: 'drive',
    name: 'Google Drive',
    tagline: 'Let the agent read and organize your Drive files.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Drive MCP server URL',
    oauth: { ...GOOGLE_OAUTH, scopes: [
      { value: 'https://www.googleapis.com/auth/drive.readonly', label: 'Read files', recommended: true },
      { value: 'https://www.googleapis.com/auth/drive', label: 'Read + write files' },
    ] },
    note: 'Same Google Cloud OAuth client as Gmail works here. Start read-only.',
  },
  {
    id: 'photos',
    name: 'Google Photos',
    tagline: 'Let the agent find and describe your photos.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Photos MCP server URL',
    oauth: { ...GOOGLE_OAUTH, scopes: [
      { value: 'https://www.googleapis.com/auth/photoslibrary.readonly', label: 'View library', recommended: true },
      { value: 'https://www.googleapis.com/auth/photoslibrary.appendonly', label: 'Add photos' },
    ] },
  },
  {
    id: 'docs',
    name: 'Google Docs',
    tagline: 'Let the agent read and draft documents.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Docs MCP server URL',
    oauth: { ...GOOGLE_OAUTH, scopes: [
      { value: 'https://www.googleapis.com/auth/documents.readonly', label: 'Read documents', recommended: true },
      { value: 'https://www.googleapis.com/auth/documents', label: 'Read + write documents' },
    ] },
  },
  {
    id: 'slides',
    name: 'Google Slides',
    tagline: 'Let the agent read and build presentations.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Slides MCP server URL',
    oauth: { ...GOOGLE_OAUTH, scopes: [
      { value: 'https://www.googleapis.com/auth/presentations.readonly', label: 'Read presentations', recommended: true },
      { value: 'https://www.googleapis.com/auth/presentations', label: 'Read + write presentations' },
    ] },
  },
  {
    id: 'sheets',
    name: 'Google Sheets',
    tagline: 'Let the agent read and update spreadsheets.',
    category: 'Google',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Google Sheets MCP server URL',
    oauth: { ...GOOGLE_OAUTH, scopes: [
      { value: 'https://www.googleapis.com/auth/spreadsheets.readonly', label: 'Read spreadsheets', recommended: true },
      { value: 'https://www.googleapis.com/auth/spreadsheets', label: 'Read + write spreadsheets' },
    ] },
  },
  {
    id: 'maps',
    name: 'Google Maps',
    tagline: 'Places, directions, and local search for the agent.',
    category: 'Google',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your Maps MCP server URL',
    bearerLabel: 'Google Maps Platform API key',
    bearerPlaceholder: 'Maps API key',
    note: 'Key from Google Cloud Console with the Maps APIs enabled.',
  },
  {
    id: 'onedrive',
    name: 'OneDrive & Office',
    tagline: 'Word, Excel, and PowerPoint files via Microsoft Graph.',
    category: 'Microsoft 365',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Microsoft 365 MCP server URL',
    oauth: { ...MS_OAUTH, scopes: [
      { value: 'Files.Read', label: 'Read files', recommended: true },
      { value: 'Files.ReadWrite', label: 'Read + write files' },
    ] },
    note: 'One connector covers Word, Excel, and PowerPoint — they are all Graph files. Same Entra app as Outlook works.',
  },
  {
    id: 'outlook-calendar',
    name: 'Outlook Calendar',
    tagline: 'Let the agent see and manage your Outlook calendar.',
    category: 'Microsoft 365',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Outlook Calendar MCP server URL',
    oauth: { ...MS_OAUTH, scopes: [
      { value: 'Calendars.Read', label: 'Read calendar', recommended: true },
      { value: 'Calendars.ReadWrite', label: 'Read + write calendar' },
    ] },
  },
  {
    id: 'onenote',
    name: 'OneNote',
    tagline: 'Let the agent read and write your notebooks.',
    category: 'Microsoft 365',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your OneNote MCP server URL',
    oauth: { ...MS_OAUTH, scopes: [
      { value: 'Notes.Read', label: 'Read notebooks', recommended: true },
      { value: 'Notes.ReadWrite', label: 'Read + write notebooks' },
    ] },
  },
  {
    id: 'x',
    name: 'X',
    tagline: 'Let the agent read timelines and post.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your X MCP server URL',
    oauth: {
      authorizationUrl: 'https://twitter.com/i/oauth2/authorize',
      tokenUrl: 'https://api.twitter.com/2/oauth2/token',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'tweet.read users.read', label: 'Read posts and profiles', recommended: true },
        { value: 'tweet.write', label: 'Post' },
        { value: 'offline.access', label: 'Stay signed in' },
      ],
    },
    note: 'Needs an app in the X Developer Portal.',
  },
  {
    id: 'facebook',
    name: 'Facebook',
    tagline: 'Let the agent read and post on your behalf.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Facebook MCP server URL',
    oauth: {
      authorizationUrl: 'https://www.facebook.com/dialog/oauth',
      tokenUrl: 'https://graph.facebook.com/oauth/access_token',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'public_profile', label: 'Basic profile', recommended: true },
        { value: 'email', label: 'Email address' },
      ],
    },
    note: 'App ID from Meta for Developers.',
  },
  {
    id: 'instagram',
    name: 'Instagram',
    tagline: 'Let the agent read your feed and publish.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Instagram MCP server URL',
    oauth: {
      authorizationUrl: 'https://www.facebook.com/dialog/oauth',
      tokenUrl: 'https://graph.facebook.com/oauth/access_token',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'instagram_basic', label: 'Read profile and media', recommended: true },
        { value: 'instagram_content_publish', label: 'Publish posts' },
      ],
    },
    note: 'Runs through Facebook Login; publishing needs a business or creator account.',
  },
  {
    id: 'linkedin',
    name: 'LinkedIn',
    tagline: 'Let the agent read your network and post updates.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your LinkedIn MCP server URL',
    oauth: {
      authorizationUrl: 'https://www.linkedin.com/oauth/v2/authorization',
      tokenUrl: 'https://www.linkedin.com/oauth2/v2/accessToken',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'openid profile email', label: 'Sign in and read profile', recommended: true },
        { value: 'w_member_social', label: 'Post updates' },
      ],
    },
    note: 'Client ID from a LinkedIn Developer app.',
  },
  {
    id: 'tiktok',
    name: 'TikTok',
    tagline: 'Let the agent read your profile and upload videos.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your TikTok MCP server URL',
    oauth: {
      authorizationUrl: 'https://www.tiktok.com/v2/auth/authorize/',
      tokenUrl: 'https://open.tiktokapis.com/v2/auth/token/',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'user.info.basic', label: 'Read basic profile', recommended: true },
        { value: 'video.upload', label: 'Upload videos' },
      ],
    },
    note: 'Client key from the TikTok Developer portal.',
  },
  {
    id: 'reddit',
    name: 'Reddit',
    tagline: 'Let the agent read subreddits and post.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Reddit MCP server URL',
    oauth: {
      authorizationUrl: 'https://www.reddit.com/api/v1/authorize',
      tokenUrl: 'https://www.reddit.com/api/v1/access_token',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'read identity', label: 'Read posts and profile', recommended: true },
        { value: 'submit', label: 'Post and comment' },
      ],
    },
    note: 'Create an app at reddit.com/prefs/apps (web or script type).',
  },
  {
    id: 'discord',
    name: 'Discord',
    tagline: 'Let the agent read servers and send messages.',
    category: 'Social media',
    auth: 'oauth',
    urlHint: '',
    urlPlaceholder: 'Paste your Discord MCP server URL',
    oauth: {
      authorizationUrl: 'https://discord.com/oauth2/authorize',
      tokenUrl: 'https://discord.com/api/oauth2/token',
      redirectUri: 'http://127.0.0.1:18793/oauth/callback',
      scopes: [
        { value: 'identify guilds', label: 'Profile and servers', recommended: true },
      ],
    },
    note: 'For automating a server, a bot token through a Discord MCP server is the usual path.',
  },
  {
    id: 'adobe-pdf',
    name: 'Adobe PDF',
    tagline: 'Let the agent read, split, merge, and convert PDFs.',
    category: 'More services',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your Adobe PDF MCP server URL',
    bearerLabel: 'Adobe PDF Services API key',
    bearerPlaceholder: 'Adobe API key',
    note: 'Key from the Adobe Developer Console (PDF Services API).',
  },
  {
    id: 'huggingface',
    name: 'Hugging Face',
    tagline: 'Models, datasets, and inference for the agent.',
    category: 'More services',
    auth: 'bearer',
    urlHint: '',
    urlPlaceholder: 'Paste your Hugging Face MCP server URL',
    bearerLabel: 'Hugging Face token',
    bearerPlaceholder: 'hf_…',
    note: 'Token from huggingface.co/settings/tokens. Read-only scopes to start.',
  },
];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

type PermLevel = 'always_allow' | 'ask_every_time' | 'deny';

const PERM_OPTIONS: { id: PermLevel; label: string }[] = [
  { id: 'ask_every_time', label: 'Ask every time' },
  { id: 'always_allow', label: 'Always allow' },
  { id: 'deny', label: 'Deny' },
];

function PermSelect({
  domain,
  onToast,
}: {
  domain: string;
  onToast: (msg: string) => void;
}): React.ReactElement {
  const [level, setLevel] = useState<PermLevel>('ask_every_time');
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let live = true;
    void invoke<string>('perm_get', { domain })
      .then((l) => {
        if (!live) return;
        if (l === 'always_allow' || l === 'ask_every_time' || l === 'deny') setLevel(l);
        setLoaded(true);
      })
      .catch(() => setLoaded(true));
    return () => {
      live = false;
    };
  }, [domain]);

  const change = async (next: PermLevel) => {
    const prev = level;
    setLevel(next);
    try {
      await invoke('perm_set', { domain, level: next });
      onToast(
        next === 'always_allow'
          ? 'Always allowed — the agent will not ask first'
          : next === 'deny'
            ? 'Denied — the agent cannot use this at all'
            : 'The agent will ask for approval each time',
      );
    } catch (e) {
      setLevel(prev);
      onToast(errText(e));
    }
  };

  return (
    <label className="conn-perm">
      <span className="gf-muted conn-small">Permission</span>{' '}
      <select
        className="gf-input conn-perm-select"
        value={level}
        disabled={!loaded}
        onChange={(e) => void change(e.target.value as PermLevel)}
      >
        {PERM_OPTIONS.map((o) => (
          <option key={o.id} value={o.id}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

const FAL_KEYRING_KEY = 'gfa-media-fal-key';
const IMAGE_MODELS = [
  { id: 'fal-ai/flux-2-dev', label: 'FLUX 2 Dev — ~$0.025/image, best value' },
  { id: 'fal-ai/flux-2-pro', label: 'FLUX 2 Pro — ~$0.05/image, highest quality' },
  { id: 'fal-ai/stable-diffusion-xl', label: 'SDXL — ~$0.003/image, cheap drafts' },
];
const VIDEO_MODELS = [
  { id: 'fal-ai/wan/v2.7/text-to-video', label: 'Wan 2.7 — ~$0.05/sec, budget' },
  { id: 'fal-ai/veo3.1', label: 'Veo 3.1 — premium, native audio' },
  { id: 'fal-ai/kling-video/v3/pro/text-to-video', label: 'Kling v3 Pro — premium, camera control' },
];
const TTS_VOICES = [
  { id: 'af_heart', label: 'Heart — warm, natural' },
  { id: 'af_bella', label: 'Bella — bright, natural' },
  { id: 'am_adam', label: 'Adam — deep American male, YouTube narrator' },
  { id: 'bf_emma', label: 'Emma — British female, natural' },
];
const TTS_VOICES_ES = [
  { id: 'ef_dora', label: 'Dora — Spanish female' },
  { id: 'em_alex', label: 'Alex — Spanish male' },
];
const TTS_LANGUAGES = [
  { id: 'en', label: 'English' },
  { id: 'es', label: 'Español' },
  { id: 'fr', label: 'Français' },
];

function ImageGenCard({ onToast }: { onToast: (msg: string) => void }): React.ReactElement {
  const [key, setKey] = useState('');
  const [hasKey, setHasKey] = useState(false);
  const [model, setModel] = useState(IMAGE_MODELS[0].id);
  const [videoModel, setVideoModel] = useState(VIDEO_MODELS[0].id);
  const [voice, setVoice] = useState(TTS_VOICES[0].id);
  const [ttsLang, setTtsLang] = useState('en');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    void invoke<string[]>('secret_list_keys')
      .then((keys) => {
        if (live) setHasKey(keys.includes(FAL_KEYRING_KEY));
      })
      .catch(() => {});
    void invoke<string | null>('settings_get', { key: 'media.image_model' })
      .then((v) => {
        if (live && v && IMAGE_MODELS.some((m) => m.id === v)) setModel(v);
      })
      .catch(() => {});
    void invoke<string | null>('settings_get', { key: 'media.video_model' })
      .then((v) => {
        if (live && v && VIDEO_MODELS.some((m) => m.id === v)) setVideoModel(v);
      })
      .catch(() => {});
    void invoke<string | null>('settings_get', { key: 'media.tts_voice' })
      .then((v) => {
        if (live && v && TTS_VOICES.some((m) => m.id === v)) setVoice(v);
      })
      .catch(() => {});
    void invoke<string | null>('settings_get', { key: 'media.tts_language' })
      .then((v) => {
        if (live && v && TTS_LANGUAGES.some((m) => m.id === v)) setTtsLang(v);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  const saveKey = async () => {
    if (!key.trim()) {
      onToast('Paste your Fal API key first (fal.ai dashboard)');
      return;
    }
    setBusy(true);
    try {
      await invoke('secret_set', { key: FAL_KEYRING_KEY, value: key.trim() });
      setHasKey(true);
      setKey('');
      onToast('Fal API key saved to the OS keychain');
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const removeKey = async () => {
    if (!window.confirm('Remove the Fal API key? Image generation will stop working.')) return;
    try {
      await invoke('secret_delete', { key: FAL_KEYRING_KEY });
      setHasKey(false);
      onToast('Fal API key removed');
    } catch (e) {
      onToast(errText(e));
    }
  };

  const changeModel = async (id: string) => {
    const prev = model;
    setModel(id);
    try {
      await invoke('settings_set', { key: 'media.image_model', value: id });
    } catch (e) {
      setModel(prev);
      onToast(errText(e));
    }
  };

  const changeVideoModel = async (id: string) => {
    const prev = videoModel;
    setVideoModel(id);
    try {
      await invoke('settings_set', { key: 'media.video_model', value: id });
    } catch (e) {
      setVideoModel(prev);
      onToast(errText(e));
    }
  };

  const changeVoice = async (id: string) => {
    const prev = voice;
    setVoice(id);
    try {
      await invoke('settings_set', { key: 'media.tts_voice', value: id });
      onToast('Voice updated');
    } catch (e) {
      setVoice(prev);
      onToast(errText(e));
    }
  };

  const changeTtsLang = async (id: string) => {
    const prev = ttsLang;
    const prevVoice = voice;
    setTtsLang(id);
    // Keep a valid voice for the language (fr falls back to English voices —
    // there is no separate French voice list in this build).
    const voices = id === 'es' ? TTS_VOICES_ES : TTS_VOICES;
    const nextVoice = voices.some((m) => m.id === voice) ? voice : voices[0].id;
    if (nextVoice !== voice) setVoice(nextVoice);
    try {
      await invoke('settings_set', { key: 'media.tts_language', value: id });
      await invoke('settings_set', { key: 'media.tts_voice', value: nextVoice });
    } catch (e) {
      setTtsLang(prev);
      setVoice(prevVoice);
      onToast(errText(e));
    }
  };

  const ttsVoices = ttsLang === 'es' ? TTS_VOICES_ES : TTS_VOICES;

  return (
    <div className="conn-card">
      <div className="conn-card-head">
        <strong>Media generation</strong>
        {hasKey ? (
          <span className="conn-badge conn-badge-ok">Key saved</span>
        ) : (
          <span className="conn-badge">No key</span>
        )}
      </div>
      <p className="conn-tagline">
        Any model — local or frontier — can generate images and video through one
        API. The agent calls <span className="conn-mono">media.generate_image</span> or{' '}
        <span className="conn-mono">media.generate_video</span> with a prompt; Fal
        renders it and the result appears in the conversation.
      </p>
      {hasKey ? (
        <button className="gf-btn gf-btn-sm conn-danger" onClick={() => void removeKey()}>
          Remove key
        </button>
      ) : (
        <div className="conn-keyrow">
          <input
            type="password"
            className="gf-input conn-mono"
            value={key}
            placeholder="Fal API key (fal.ai)"
            onChange={(e) => setKey(e.target.value)}
          />
          <button className="gf-btn gf-btn-sm" onClick={() => void saveKey()} disabled={busy}>
            Save
          </button>
        </div>
      )}
      <label className="gf-label conn-modellabel">
        <span className="gf-muted conn-small">Image model</span>
        <select
          className="gf-input"
          value={model}
          onChange={(e) => void changeModel(e.target.value)}
        >
          {IMAGE_MODELS.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </label>
      <label className="gf-label conn-modellabel">
        <span className="gf-muted conn-small">Video model</span>
        <select
          className="gf-input"
          value={videoModel}
          onChange={(e) => void changeVideoModel(e.target.value)}
        >
          {VIDEO_MODELS.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </label>
      <label className="gf-label conn-modellabel">
        <span className="gf-muted conn-small">Voice language</span>
        <select
          className="gf-input"
          value={ttsLang}
          onChange={(e) => void changeTtsLang(e.target.value)}
        >
          {TTS_LANGUAGES.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </label>
      <label className="gf-label conn-modellabel">
        <span className="gf-muted conn-small">Voice</span>
        <select
          className="gf-input"
          value={voice}
          onChange={(e) => void changeVoice(e.target.value)}
        >
          {ttsVoices.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </label>
      <PermSelect domain="media" onToast={onToast} />
      <p className="conn-note">
        Every generation costs real money — video is per-second, so a 5s premium
        clip can cost over $1. Default is Ask every time, so the agent confirms
        with you before spending.
      </p>
    </div>
  );
}

export function ConnectionsPanel({ onToast }: Props): React.ReactElement {
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [connecting, setConnecting] = useState<ConnectorTemplate | null>(null);
  const [url, setUrl] = useState('');
  const [clientId, setClientId] = useState('');
  const [token, setToken] = useState('');
  const [scopes, setScopes] = useState<string[]>([]);
  const [oauth, setOauth] = useState<{ serverId: string; authUrl: string; state: string; code: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<McpServerView[]>('mcp_server_list');
      setServers(list);
    } catch (e) {
      onToast(errText(e));
    }
  }, [onToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const openConnect = (t: ConnectorTemplate) => {
    setConnecting(t);
    setUrl(t.urlHint);
    setClientId('');
    setToken('');
    setScopes(t.oauth ? t.oauth.scopes.filter((s) => s.recommended).map((s) => s.value) : []);
    setOauth(null);
  };

  const toggleScope = (v: string) => {
    setScopes((prev) => (prev.includes(v) ? prev.filter((s) => s !== v) : [...prev, v]));
  };

  const doConnect = async () => {
    if (!connecting) return;
    if (!url.trim()) {
      onToast('Enter the MCP server URL first');
      return;
    }
    if (connecting.auth === 'oauth' && !clientId.trim()) {
      onToast('Enter your OAuth client ID first');
      return;
    }
    if (connecting.auth === 'bearer' && !token.trim()) {
      onToast('Enter your API token first');
      return;
    }
    setBusy(true);
    try {
      const input = {
        name: connecting.name,
        transport: 'streamable_http' as const,
        url: url.trim(),
        auth_type: connecting.auth,
        oauth_client_id: clientId.trim(),
        oauth_authorization_url: connecting.oauth?.authorizationUrl ?? '',
        oauth_token_url: connecting.oauth?.tokenUrl ?? '',
        oauth_scopes: scopes.join(' '),
        oauth_redirect_uri: connecting.oauth?.redirectUri ?? '',
        allowed_tools: [] as string[],
        denied_tools: [] as string[],
        bearer_present: connecting.auth === 'bearer' && token.length > 0,
      };
      const id = await invoke<string>('mcp_server_add', { input });
      if (connecting.auth === 'bearer' && token) {
        // Tokens go to the OS keyring only — never the database, never the model.
        await invoke('secret_set', { key: `gfa-mcp-bearer:${id}`, value: token });
      }
      if (connecting.auth === 'oauth') {
        const res = await invoke<{ auth_url: string; state: string }>('mcp_oauth_start', {
          server_id: id,
        });
        setOauth({ serverId: id, authUrl: res.auth_url, state: res.state, code: '' });
        onToast('Server added — complete sign-in below');
      } else {
        onToast(`${connecting.name} connected`);
        setConnecting(null);
      }
      await refresh();
    } catch (e) {
      onToast(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const completeOAuth = async () => {
    if (!oauth || !oauth.code.trim()) {
      onToast('Paste the authorization code first');
      return;
    }
    try {
      await invoke('mcp_oauth_callback', { state: oauth.state, code: oauth.code.trim() });
      setOauth(null);
      setConnecting(null);
      onToast('Signed in');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const disconnect = async (s: McpServerView) => {
    const label = s.oauth_connected ? 'Disconnect (revoke access)' : 'Remove server';
    if (!window.confirm(`${label} "${s.name}"?`)) return;
    try {
      if (s.oauth_connected) {
        await invoke('mcp_oauth_revoke', { server_id: s.id });
      } else {
        await invoke('mcp_server_remove', { server_id: s.id });
      }
      onToast('Done');
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  const toggleEnabled = async (s: McpServerView) => {
    try {
      await invoke(s.enabled ? 'mcp_server_disable' : 'mcp_server_enable', { server_id: s.id });
      await refresh();
    } catch (e) {
      onToast(errText(e));
    }
  };

  return (
    <div className="gf-workspace">
      <div className="gf-pane conn-pane">
        <div className="gf-pane-header">
          <span>Connections</span>
          <span className="gf-muted"> — give the agent access to your accounts and apps</span>
        </div>

        <p className="conn-intro">
          Everything you connect here becomes a tool your agent can use — email, calendar,
          code, web search. Tokens are stored in your OS keychain; the agent sees tool
          names, never your credentials. Builders keep using the raw MCP panel; this tab
          is the agent's front door.
        </p>

        <h3 className="conn-section">Built-in</h3>
        <div className="conn-grid">
          <div className="conn-card">
            <div className="conn-card-head">
              <strong>Terminal</strong>
              <span className="conn-badge conn-badge-ok">Available</span>
            </div>
            <p className="conn-tagline">
              Full Linux shell for the agent: download files, install programs,
              run commands, clean up install files, uninstall programs — the whole
              lifecycle, one command at a time.
            </p>
            <p className="conn-note">
              Gated by the constitution: shell commands are classified by risk and
              consequential ones require your approval. Fine-grained per-tool rules
              live in the Permission Center.
            </p>
            <PermSelect domain="shell" onToast={onToast} />
          </div>
          <ImageGenCard onToast={onToast} />
        </div>

        <h3 className="conn-section">Connect a service</h3>
        {CATEGORY_ORDER.map((cat) => (
          <div key={cat}>
            <h4 className="conn-category">{cat}</h4>
            <div className="conn-grid">
              {TEMPLATES.filter((t) => t.category === cat).map((t) => {
            const existing = servers.find(
              (s) => s.name.toLowerCase() === t.name.toLowerCase(),
            );
            // A server counts as connected only with real credentials:
            // OAuth signed in, bearer key present, or no-auth built-in enabled.
            // `enabled` alone (without credentials) is not a connection.
            const connected = existing && (existing.oauth_connected || existing.bearer_present || (t.auth === 'none' && existing.enabled));
            return (
              <div className="conn-card" key={t.id}>
                <div className="conn-card-head">
                  <strong>{t.name}</strong>
                  {connected ? (
                    <span className="conn-badge conn-badge-ok">Connected</span>
                  ) : (
                    <span className="conn-badge">
                      {t.auth === 'oauth' ? 'OAuth' : t.auth === 'bearer' ? 'API key' : t.auth === 'none' ? 'No sign-in' : ''}
                    </span>
                  )}
                </div>
                <p className="conn-tagline">{t.tagline}</p>
                {t.note && <p className="conn-note">{t.note}</p>}
                {!connected && (
                  <button
                    className="gf-btn conn-connect"
                    onClick={() => openConnect(t)}
                    disabled={busy}
                  >
                    Connect
                  </button>
                )}
                {connected && existing && (
                  <div className="gf-row" style={{ marginTop: 8 }}>
                    <button
                      className="gf-btn gf-btn-sm"
                      onClick={() => void toggleEnabled(existing)}
                    >
                      {existing.enabled ? 'Disable' : 'Enable'}
                    </button>
                    <button
                      className="gf-btn gf-btn-sm conn-danger"
                      onClick={() => void disconnect(existing)}
                    >
                      {existing.oauth_connected ? 'Disconnect' : 'Remove'}
                    </button>
                  </div>
                )}
                  </div>
                );
              })}
            </div>
          </div>
        ))}

        <h3 className="conn-section">Connected ({servers.length})</h3>
        <div className="conn-mcp-perm">
          <span className="gf-muted conn-small">
            Permission for all connected service tools:
          </span>{' '}
          <PermSelect domain="mcp" onToast={onToast} />
        </div>
        {servers.length === 0 ? (
          <p className="gf-muted">Nothing connected yet. Pick a service above.</p>
        ) : (
          <div className="conn-list">
            {servers.map((s) => (
              <div className="conn-row" key={s.id}>
                <div>
                  <strong>{s.name}</strong>{' '}
                  <span className="gf-muted conn-small">
                    {s.tool_count} tools · {s.enabled ? 'enabled' : 'disabled'}
                    {s.oauth_connected ? ' · signed in' : ''}
                  </span>
                </div>
                <div className="conn-row-actions">
                  <button className="gf-btn gf-btn-sm" onClick={() => void toggleEnabled(s)}>
                    {s.enabled ? 'Disable' : 'Enable'}
                  </button>
                  <button
                    className="gf-btn gf-btn-sm conn-danger"
                    onClick={() => void disconnect(s)}
                  >
                    {s.oauth_connected ? 'Disconnect' : 'Remove'}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {connecting && (
          <div className="conn-modal-backdrop" onClick={() => !busy && setConnecting(null)}>
            <div className="conn-modal" onClick={(e) => e.stopPropagation()}>
              <h3>Connect {connecting.name}</h3>
              <label className="gf-label">
                MCP server URL
                <input
                  className="gf-input conn-mono"
                  value={url}
                  placeholder={connecting.urlPlaceholder}
                  onChange={(e) => setUrl(e.target.value)}
                />
              </label>
              {connecting.auth === 'oauth' && (
                <>
                  <label className="gf-label">
                    OAuth client ID
                    <input
                      className="gf-input conn-mono"
                      value={clientId}
                      placeholder="Your app's client ID"
                      onChange={(e) => setClientId(e.target.value)}
                    />
                  </label>
                  {connecting.oauth && (
                    <fieldset className="conn-scopes">
                      <legend className="gf-label">Scopes (least privilege first)</legend>
                      {connecting.oauth.scopes.map((sc) => (
                        <label key={sc.value} className="conn-scope">
                          <input
                            type="checkbox"
                            checked={scopes.includes(sc.value)}
                            onChange={() => toggleScope(sc.value)}
                          />
                          <span>
                            {sc.label}
                            {sc.recommended && <em> — recommended</em>}
                          </span>
                        </label>
                      ))}
                    </fieldset>
                  )}
                  {connecting.note && <p className="conn-note">{connecting.note}</p>}
                </>
              )}
              {connecting.auth === 'bearer' && (
                <label className="gf-label">
                  {connecting.bearerLabel ?? 'API token'}
                  <input
                    type="password"
                    className="gf-input conn-mono"
                    value={token}
                    placeholder={connecting.bearerPlaceholder}
                    onChange={(e) => setToken(e.target.value)}
                  />
                </label>
              )}
              <div className="conn-modal-actions">
                <button className="gf-btn" onClick={() => setConnecting(null)} disabled={busy}>
                  Cancel
                </button>
                <button className="gf-btn gf-btn-primary" onClick={() => void doConnect()} disabled={busy}>
                  {busy ? 'Working…' : connecting.auth === 'oauth' ? 'Add & sign in' : 'Connect'}
                </button>
              </div>

              {oauth && (
                <div className="conn-oauth">
                  <p>
                    <strong>Step 1:</strong> open this sign-in URL, approve access, then
                    paste the authorization code below.
                  </p>
                  <textarea className="gf-textarea conn-mono" readOnly rows={4} value={oauth.authUrl} />
                  <button
                    className="gf-btn gf-btn-sm"
                    onClick={() => {
                      void navigator.clipboard?.writeText(oauth.authUrl).then(
                        () => onToast('Sign-in URL copied'),
                        () => onToast('Copy failed — select the URL manually'),
                      );
                    }}
                  >
                    Copy sign-in URL
                  </button>
                  <label className="gf-label">
                    <strong>Step 2:</strong> authorization code
                    <input
                      className="gf-input conn-mono"
                      value={oauth.code}
                      onChange={(e) => setOauth({ ...oauth, code: e.target.value })}
                      placeholder="Paste code here"
                    />
                  </label>
                  <button className="gf-btn gf-btn-primary" onClick={() => void completeOAuth()}>
                    Finish sign-in
                  </button>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
