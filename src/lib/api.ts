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
  if (isDesktop()) {
    // Native clipboard via the Tauri clipboard-manager plugin.
    const { writeText } = await import('@tauri-apps/plugin-clipboard-manager');
    await writeText(text);
    return;
  }
  if (navigator.clipboard) {
    await navigator.clipboard.writeText(text);
    return;
  }
  throw new Error('clipboard-unavailable');
}
