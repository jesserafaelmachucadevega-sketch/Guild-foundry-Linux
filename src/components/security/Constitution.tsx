// SPDX-License-Identifier: Apache-2.0
// Phase 9 — Renders the security constitution returned by the native
// `constitution_text` command, split into readable sections.

import React, { useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';
import './security.css';

interface Section {
  title: string;
  body: string;
}

function splitSections(text: string): Section[] {
  const sections: Section[] = [];
  const lines = text.split('\n');
  let current: Section | null = null;
  for (const line of lines) {
    if (line.startsWith('## ')) {
      if (current) sections.push(current);
      current = { title: line.slice(3).trim(), body: '' };
    } else if (current) {
      current.body += line + '\n';
    }
  }
  if (current) sections.push(current);
  return sections;
}

export function Constitution(): React.ReactElement {
  const [sections, setSections] = useState<Section[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>('constitution_text')
      .then((text) => {
        setSections(splitSections(text ?? ''));
        setError(null);
      })
      .catch((err: unknown) => {
        setError(
          err instanceof DesktopCapabilityRequired
            ? 'Desktop Capability Required'
            : 'Could not load the constitution',
        );
      });
  }, []);

  return (
    <div className="gf-workspace">
      <div className="gf-pane sec-pane">
        <div className="gf-pane-header">
          <span>Security Constitution</span>
        </div>
        {error !== null ? (
          <p className="sec-error">{error}</p>
        ) : (
          <div className="sec-const">
            {sections.map((s) => (
              <section key={s.title} className="sec-const-section">
                <h2 className="sec-const-title">{s.title}</h2>
                <pre className="sec-const-body">{s.body.trim()}</pre>
              </section>
            ))}
            {sections.length === 0 && (
              <p className="gf-muted">Loading constitution…</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
