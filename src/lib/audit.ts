// SPDX-License-Identifier: Apache-2.0
// Best-effort audit writer: wraps the Phase 9 `sec_audit` command so UI
// actions leave a trail. Never throws — audit failure must not break UX.

import { invoke } from './api';

export function audit(
  level: 'DEBUG' | 'INFO' | 'WARN' | 'ERROR' | 'SECURITY',
  category: string,
  message: string,
  meta?: Record<string, unknown>,
): void {
  void invoke('sec_audit', {
    level,
    category,
    message,
    meta_json: JSON.stringify(meta ?? {}),
  }).catch(() => undefined);
}
