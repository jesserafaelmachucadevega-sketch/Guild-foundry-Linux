// SPDX-License-Identifier: Apache-2.0
// Phase 9 — Tabbed host for the audit log and the constitution, so a single
// nav section can surface the whole security layer.

import React, { useState } from 'react';
import { AuditPanel } from './AuditPanel';
import { Constitution } from './Constitution';
import './security.css';

type Tab = 'audit' | 'constitution';

export function SecurityView(): React.ReactElement {
  const [tab, setTab] = useState<Tab>('audit');
  return (
    <div className="gf-workspace" style={{ flexDirection: 'column' }}>
      <div className="gf-row" style={{ gap: 8, padding: '8px 12px' }}>
        <button
          type="button"
          className={`gf-btn${tab === 'audit' ? ' primary' : ''}`}
          onClick={() => setTab('audit')}
        >
          Audit Log
        </button>
        <button
          type="button"
          className={`gf-btn${tab === 'constitution' ? ' primary' : ''}`}
          onClick={() => setTab('constitution')}
        >
          Constitution
        </button>
      </div>
      {tab === 'audit' ? <AuditPanel /> : <Constitution />}
    </div>
  );
}
