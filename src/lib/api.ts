// Typed wrapper around the Tauri 2 IPC bridge (@tauri-apps/api/core).
// In PWA fallback mode (plain browser, no Tauri runtime), calls reject with a
// clear "Desktop Capability Required" error so UI degrades gracefully instead
// of failing silently.

import { invoke as tauriInvoke } from '@tauri-apps/api/core';

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isDesktop(): boolean {
  return (
    typeof window !== 'undefined' && typeof window.__TAURI_INTERNALS__ !== 'undefined'
  );
}

export class DesktopCapabilityRequired extends Error {
  constructor(channel: string) {
    super(`Desktop Capability Required (invoke "${channel}" needs the native app)`);
    this.name = 'DesktopCapabilityRequired';
  }
}

export async function invoke<T>(channel: string, payload?: unknown): Promise<T> {
  if (!isDesktop()) {
    throw new DesktopCapabilityRequired(channel);
  }
  return (await tauriInvoke(channel, payload as Record<string, unknown>)) as T;
}

export async function copyToClipboard(text: string): Promise<void> {
  const errors: string[] = [];
  if (isDesktop()) {
    // Native clipboard via the Tauri clipboard-manager plugin. Requires the
    // `clipboard-manager:allow-write-text` capability — the plugin's default
    // permission set is deliberately EMPTY, so without it every call is denied.
    try {
      const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
      await writeText(text);
      return;
    } catch (e) {
      errors.push(`native: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
  } catch (e) {
    errors.push(`web: ${e instanceof Error ? e.message : String(e)}`);
  }
  // Legacy fallback: a hidden textarea + execCommand works in WebKitGTK even
  // when the async clipboard APIs are unavailable or denied.
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    if (ok) return;
    errors.push('fallback: the browser refused execCommand("copy")');
  } catch (e) {
    errors.push(`fallback: ${e instanceof Error ? e.message : String(e)}`);
  }
  throw new Error(errors.join('; ') || 'clipboard-unavailable');
}
