// SPDX-License-Identifier: Apache-2.0
// appearance.ts — the user's own theme, layered over the light/dark base.
//
// Everything is a CSS custom property applied to the document root. An unset
// value is REMOVED rather than replaced, so the base theme keeps owning every
// default: switching light/dark never fights the user's overrides, and
// "reset" on any single token is exact.

import { invoke } from './api';

import type { CSSProperties } from 'react';

export interface SectionType {
  /** FONT_STACKS id, or undefined for the app default. */
  family?: string;
  /** px; undefined keeps the element's own size. */
  size?: number;
  bold?: boolean;
  italic?: boolean;
  strike?: boolean;
}

export interface Appearance {
  baseFont?: string;
  baseSize?: number;
  colors: Record<string, string>;
  sections: Record<string, SectionType>;
}

export function emptyAppearance(): Appearance {
  return { colors: {}, sections: {} };
}

/** Bundled open-source families (see src/fonts.css) plus honest system stacks. */
export const FONT_STACKS: { id: string; label: string; css: string }[] = [
  { id: 'system', label: 'App default', css: '' },
  { id: 'forum', label: 'Forum', css: "'Forum', Georgia, serif" },
  { id: 'inter', label: 'Inter', css: "'Inter', system-ui, sans-serif" },
  { id: 'lora', label: 'Lora', css: "'Lora', Georgia, serif" },
  { id: 'merriweather', label: 'Merriweather', css: "'Merriweather', Georgia, serif" },
  { id: 'crimson-pro', label: 'Crimson Pro', css: "'Crimson Pro', Georgia, serif" },
  { id: 'libre-baskerville', label: 'Libre Baskerville', css: "'Libre Baskerville', Georgia, serif" },
  { id: 'eb-garamond', label: 'EB Garamond', css: "'EB Garamond', Georgia, serif" },
  { id: 'source-sans-3', label: 'Source Sans 3', css: "'Source Sans 3', system-ui, sans-serif" },
  { id: 'ibm-plex-sans', label: 'IBM Plex Sans', css: "'IBM Plex Sans', system-ui, sans-serif" },
  { id: 'atkinson-hyperlegible', label: 'Atkinson Hyperlegible', css: "'Atkinson Hyperlegible', system-ui, sans-serif" },
  { id: 'nunito', label: 'Nunito', css: "'Nunito', system-ui, sans-serif" },
  { id: 'playfair-display', label: 'Playfair Display', css: "'Playfair Display', Georgia, serif" },
  { id: 'jetbrains-mono', label: 'JetBrains Mono', css: "'JetBrains Mono', ui-monospace, monospace" },
  { id: 'fira-code', label: 'Fira Code', css: "'Fira Code', ui-monospace, monospace" },
  { id: 'georgia', label: 'Georgia (system)', css: "Georgia, 'Times New Roman', serif" },
  { id: 'system-mono', label: 'System monospace', css: 'ui-monospace, SFMono-Regular, Menlo, monospace' },
];

export const COLOR_TOKENS: { key: string; label: string; cssVar: string; sample: string }[] = [
  { key: 'headerBg', label: 'Header background', cssVar: '--ap-header-bg', sample: 'Guild Foundry AI · Model & API Hub' },
  { key: 'railBg', label: 'Left panel background', cssVar: '--ap-rail-bg', sample: 'Personal Agent' },
  { key: 'paneBg', label: 'Main pane background', cssVar: '--ap-pane-bg', sample: 'chat · lists · editors' },
  { key: 'outline', label: 'Box outlines', cssVar: '--ap-outline', sample: 'borders everywhere' },
  { key: 'buttonBg', label: 'Buttons — background', cssVar: '--ap-btn-bg', sample: 'Connect' },
  { key: 'buttonText', label: 'Buttons — text', cssVar: '--ap-btn-text', sample: 'Connect' },
  { key: 'accent', label: 'Accent (active, links, primary)', cssVar: '--ap-accent', sample: 'open · active tab' },
  { key: 'chatText', label: 'Chat text', cssVar: '--ap-chat-text', sample: 'Here is what I found…' },
  { key: 'mutedText', label: 'Muted text', cssVar: '--ap-muted', sample: 'secondary notes' },
];

/** Sections with real sample text and an honest note about the current weight. */
export const TEXT_SECTIONS: { key: string; label: string; sample: string; note: string }[] = [
  { key: 'appTitle', label: 'App title (header)', sample: 'Guild Foundry AI', note: 'header brand — currently bold' },
  { key: 'headerSection', label: 'Header — section name', sample: 'Model & API Hub', note: 'header side label — currently regular' },
  { key: 'railLabel', label: 'Left panel labels', sample: 'Personal Agent', note: 'currently bold' },
  { key: 'paneTitle', label: 'Pane titles', sample: 'PROVIDERS', note: 'currently regular, shown uppercase' },
  { key: 'listStrong', label: 'Names & headings in lists', sample: 'OpenRouter', note: 'usually bold (provider names, card heads)' },
  { key: 'formLabel', label: 'Form labels', sample: 'KIND', note: 'currently regular, shown uppercase' },
  { key: 'buttons', label: 'Button labels', sample: 'Connect', note: 'currently regular' },
  { key: 'inputs', label: 'Text inside inputs', sample: 'https://openrouter.ai/api/v1', note: 'currently regular' },
  { key: 'chatText', label: 'Chat message text', sample: 'Here is what I found in your project…', note: 'currently regular' },
  { key: 'chatRole', label: 'Chat model name', sample: 'qwen3.5-4b', note: 'bubble header — currently bold' },
  { key: 'muted', label: 'Muted / secondary text', sample: 'Model & API Hub', note: 'currently regular' },
];

const SETTINGS_KEY = 'ui.appearance';

function fontCss(id: string | undefined): string {
  if (!id) return '';
  return FONT_STACKS.find((f) => f.id === id)?.css ?? '';
}

function setVar(root: HTMLElement, name: string, value: string): void {
  if (value) root.style.setProperty(name, value);
  else root.style.removeProperty(name);
}

/** Apply (or clear) every override on the document root. Never throws: a bad
 *  override must not be able to take the view down. */
export function applyAppearance(a: Appearance): void {
  try {
    const root = document.documentElement;
    setVar(root, '--ap-font', fontCss(a.baseFont));
    setVar(root, '--ap-font-size', a.baseSize ? `${a.baseSize}px` : '');
    for (const t of COLOR_TOKENS) {
      const value = a.colors[t.key] ?? '';
      // Guard against half-typed hex: an invalid custom property would make the
      // consuming declaration invalid-at-computed-value-time and flash.
      const ok =
        value === '' || /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(value);
      setVar(root, t.cssVar, ok ? value : '');
    }
    for (const s of TEXT_SECTIONS) {
      const v = a.sections[s.key] ?? {};
      setVar(root, `--ap-${s.key}-family`, fontCss(v.family));
      setVar(root, `--ap-${s.key}-size`, v.size ? `${v.size}px` : '');
      setVar(root, `--ap-${s.key}-weight`, v.bold === undefined ? '' : v.bold ? '700' : '400');
      setVar(root, `--ap-${s.key}-style`, v.italic === undefined ? '' : v.italic ? 'italic' : 'normal');
      setVar(root, `--ap-${s.key}-deco`, v.strike === undefined ? '' : v.strike ? 'line-through' : 'none');
    }
  } catch {
    // The theme stays exactly as it was.
  }
}

export async function loadAppearance(): Promise<Appearance> {
  try {
    const raw = await invoke<Appearance | null>('settings_get', { key: SETTINGS_KEY });
    if (!raw || typeof raw !== 'object') return emptyAppearance();
    return {
      baseFont: typeof raw.baseFont === 'string' ? raw.baseFont : undefined,
      baseSize: typeof raw.baseSize === 'number' ? raw.baseSize : undefined,
      colors: raw.colors && typeof raw.colors === 'object' ? raw.colors : {},
      sections: raw.sections && typeof raw.sections === 'object' ? raw.sections : {},
    };
  } catch {
    return emptyAppearance();
  }
}

export async function saveAppearance(a: Appearance): Promise<void> {
  try {
    await invoke('settings_set', { key: SETTINGS_KEY, value: a });
  } catch {
    /* applies for this session; persistence is best-effort */
  }
}

/** Inline style for the live sample preview in the editor. */
export function sectionPreviewStyle(v: SectionType): CSSProperties {
  return {
    fontFamily: fontCss(v.family) || undefined,
    fontSize: v.size ? `${v.size}px` : undefined,
    fontWeight: v.bold === undefined ? undefined : v.bold ? 700 : 400,
    fontStyle: v.italic ? 'italic' : undefined,
    textDecorationLine: v.strike ? 'line-through' : undefined,
  };
}
