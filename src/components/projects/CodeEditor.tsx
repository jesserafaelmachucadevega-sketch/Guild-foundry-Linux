// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: embedded code editor (Monaco).
// Tabs, diff view, diagnostics (Monaco language services), and agent-change
// line highlighting via deltaDecorations.
import React, { useEffect, useRef, useState } from 'react';
import * as monaco from 'monaco-editor';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';

interface Tab {
  rel: string;
  dirty: boolean;
}

function languageFor(rel: string): string {
  const ext = rel.split('.').pop()?.toLowerCase() ?? '';
  switch (ext) {
    case 'ts': return 'typescript';
    case 'tsx': return 'typescript';
    case 'js': return 'javascript';
    case 'jsx': return 'javascript';
    case 'json': return 'json';
    case 'html': return 'html';
    case 'css': return 'css';
    case 'scss': return 'scss';
    case 'md': return 'markdown';
    case 'py': return 'python';
    case 'rs': return 'rust';
    case 'go': return 'go';
    case 'java': return 'java';
    case 'toml': return 'ini';
    case 'yaml': case 'yml': return 'yaml';
    case 'sh': return 'shell';
    case 'sql': return 'sql';
    default: return 'plaintext';
  }
}

function setupWorkers(): void {
  const g = self as unknown as { MonacoEnvironment?: unknown };
  if (g.MonacoEnvironment) return;
  const workerUrl = (p: string) => new URL(p, import.meta.url);
  g.MonacoEnvironment = {
    getWorker(_moduleId: unknown, label: string) {
      if (label === 'typescript' || label === 'javascript') {
        return new Worker(workerUrl('monaco-editor/esm/vs/language/typescript/ts.worker?worker'), { type: 'module' });
      }
      if (label === 'json') {
        return new Worker(workerUrl('monaco-editor/esm/vs/language/json/json.worker?worker'), { type: 'module' });
      }
      if (label === 'css' || label === 'scss' || label === 'less') {
        return new Worker(workerUrl('monaco-editor/esm/vs/language/css/css.worker?worker'), { type: 'module' });
      }
      if (label === 'html' || label === 'handlebars' || label === 'razor') {
        return new Worker(workerUrl('monaco-editor/esm/vs/language/html/html.worker?worker'), { type: 'module' });
      }
      return new Worker(workerUrl('monaco-editor/esm/vs/editor/editor.worker?worker'), { type: 'module' });
    },
  };
}

function defineTheme(): void {
  monaco.editor.defineTheme('gf-dark', {
    base: 'vs-dark',
    inherit: true,
    rules: [
      { token: 'comment', foreground: '8f8a7d' },
      { token: 'keyword', foreground: 'c9a227' },
      { token: 'string', foreground: 'd2b48c' },
    ],
    colors: {
      'editor.background': '#0a0a0a',
      'editor.foreground': '#f2ede1',
      'editor.lineHighlightBackground': '#161616',
      'editorLineNumber.foreground': '#8f8a7d',
      'editorLineNumber.activeForeground': '#c9a227',
      'editorCursor.foreground': '#c9a227',
      'editor.selectionBackground': '#3d3d3d',
      'editor.inactiveSelectionBackground': '#2b2b2b',
      'diffEditor.insertedTextBackground': '#1d3a2410',
      'diffEditor.removedTextBackground': '#4a1d1610',
    },
  });
}

export interface AgentHighlight {
  rel: string;
  lines: number[]; // 1-based line numbers changed by an agent
}

export function CodeEditor({
  projectRoot,
  openRequest,
  agentHighlight,
  onToast,
  onDirtyChange,
}: {
  projectRoot: string;
  openRequest: { rel: string; nonce: number } | null;
  agentHighlight: AgentHighlight | null;
  onToast: (t: string) => void;
  onDirtyChange?: (rel: string, dirty: boolean) => void;
}): React.ReactElement {
  const containerRef = useRef<HTMLDivElement>(null);
  const diffContainerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);
  const diffEditorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const modelsRef = useRef<Map<string, monaco.editor.ITextModel>>(new Map());
  const savedRef = useRef<Map<string, string>>(new Map());
  const decorRef = useRef<string[]>([]);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [diffMode, setDiffMode] = useState(false);
  const [nonce, setNonce] = useState(0);

  const errMsg = (err: unknown): string =>
    err instanceof DesktopCapabilityRequired
      ? 'Desktop Capability Required'
      : err instanceof Error
        ? err.message
        : String(err);

  // Create the editor once.
  useEffect(() => {
    setupWorkers();
    defineTheme();
    if (!containerRef.current || editorRef.current) return;
    const ed = monaco.editor.create(containerRef.current, {
      theme: 'gf-dark',
      automaticLayout: true,
      minimap: { enabled: false },
      fontFamily: 'JetBrains Mono, Fira Code, monospace',
      fontSize: 13,
      scrollBeyondLastLine: false,
      renderWhitespace: 'selection',
    });
    ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      void saveActive();
    });
    ed.onDidChangeModelContent(() => {
      const rel = activeRef.current;
      if (!rel) return;
      const model = modelsRef.current.get(rel);
      const saved = savedRef.current.get(rel) ?? '';
      const dirty = (model?.getValue() ?? '') !== saved;
      setTabs((ts) => ts.map((t) => (t.rel === rel ? { ...t, dirty } : t)));
      onDirtyChange?.(rel, dirty);
    });
    editorRef.current = ed;
    return () => {
      modelsRef.current.forEach((m) => m.dispose());
      modelsRef.current.clear();
      ed.dispose();
      editorRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const activeRef = useRef<string | null>(null);
  useEffect(() => {
    activeRef.current = active;
  }, [active]);

  // Open a file requested by the explorer.
  useEffect(() => {
    if (!openRequest) return;
    void (async () => {
      try {
        const content = await invoke<string>('ws_read', {
          project_root: projectRoot,
          rel: openRequest.rel,
        });
        let model = modelsRef.current.get(openRequest.rel);
        if (!model) {
          model = monaco.editor.createModel(
            content,
            languageFor(openRequest.rel),
            monaco.Uri.parse(`inmemory://project/${openRequest.rel}`),
          );
          modelsRef.current.set(openRequest.rel, model);
        }
        savedRef.current.set(openRequest.rel, content);
        setTabs((ts) =>
          ts.some((t) => t.rel === openRequest.rel)
            ? ts
            : [...ts, { rel: openRequest.rel, dirty: false }],
        );
        setActive(openRequest.rel);
      } catch (err) {
        onToast(errMsg(err));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openRequest?.nonce]);

  // Switch the visible model when the active tab changes.
  useEffect(() => {
    const ed = editorRef.current;
    if (!ed || !active) return;
    const model = modelsRef.current.get(active);
    if (model) {
      ed.setModel(model);
      setDiffMode(false);
    }
  }, [active, nonce]);

  const saveActive = async () => {
    const rel = activeRef.current;
    if (!rel) return;
    const model = modelsRef.current.get(rel);
    if (!model) return;
    try {
      const content = model.getValue();
      await invoke('ws_write', { project_root: projectRoot, rel, content });
      savedRef.current.set(rel, content);
      setTabs((ts) => ts.map((t) => (t.rel === rel ? { ...t, dirty: false } : t)));
      onDirtyChange?.(rel, false);
      onToast('Saved');
    } catch (err) {
      onToast(errMsg(err));
    }
  };

  const closeTab = (rel: string) => {
    const tab = tabs.find((t) => t.rel === rel);
    if (tab?.dirty && !window.confirm(`Close "${rel}" without saving?`)) return;
    modelsRef.current.get(rel)?.dispose();
    modelsRef.current.delete(rel);
    savedRef.current.delete(rel);
    setTabs((ts) => {
      const rest = ts.filter((t) => t.rel !== rel);
      if (active === rel) setActive(rest.length ? rest[rest.length - 1].rel : null);
      return rest;
    });
    setDiffMode(false);
  };

  // Diff view: original (saved) vs current (modified).
  const toggleDiff = () => {
    const rel = active;
    const ed = editorRef.current;
    if (!rel || !ed) return;
    if (!diffMode) {
      const current = modelsRef.current.get(rel);
      const saved = savedRef.current.get(rel) ?? '';
      if (!current) return;
      const original = monaco.editor.createModel(saved, languageFor(rel));
      if (diffContainerRef.current && !diffEditorRef.current) {
        diffEditorRef.current = monaco.editor.createDiffEditor(diffContainerRef.current, {
          theme: 'gf-dark',
          automaticLayout: true,
          renderSideBySide: true,
          readOnly: true,
        });
      }
      diffEditorRef.current?.setModel({ original, modified: current });
      setDiffMode(true);
    } else {
      diffEditorRef.current?.setModel(null);
      setDiffMode(false);
    }
  };

  // Agent-change highlighting: gold background on agent-touched lines.
  useEffect(() => {
    const ed = editorRef.current;
    if (!ed) return;
    if (decorRef.current.length) {
      decorRef.current = ed.deltaDecorations(decorRef.current, []);
    }
    if (agentHighlight && agentHighlight.rel === active) {
      const model = modelsRef.current.get(active);
      if (model) {
        decorRef.current = ed.deltaDecorations(
          [],
          agentHighlight.lines.map((line) => ({
            range: new monaco.Range(line, 1, line, 1),
            options: {
              isWholeLine: true,
              className: 'gf-agent-line',
              glyphMarginClassName: 'gf-agent-glyph',
              hoverMessage: { value: 'Changed by agent' },
            },
          })),
        );
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentHighlight, active, nonce]);

  return (
    <div className="gf-projects-col" style={{ flex: 1 }}>
      <style>{`
        .gf-agent-line { background: rgba(201, 162, 39, 0.14); }
        .gf-agent-glyph { background: #c9a227; width: 4px !important; margin-left: 3px; }
      `}</style>
      <div className="gf-pane-header">
        <span>Editor</span>
        <span className="gf-row">
          <button className="gf-btn" style={{ padding: '0 8px' }} onClick={() => void saveActive()} disabled={!active}>
            Save
          </button>
          <button
            className="gf-btn"
            style={{ padding: '0 8px' }}
            onClick={toggleDiff}
            disabled={!active}
            title="Toggle diff: saved vs current"
          >
            {diffMode ? 'Edit' : 'Diff'}
          </button>
        </span>
      </div>
      <div className="gf-editor-tabs">
        {tabs.map((t) => (
          <div
            key={t.rel}
            className={`gf-editor-tab${t.rel === active ? ' active' : ''}`}
            onClick={() => {
              setActive(t.rel);
              setNonce((n) => n + 1);
            }}
          >
            <span>{t.rel.split('/').pop()}</span>
            {t.dirty && <span className="gf-dot">●</span>}
            <span
              className="gf-close"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(t.rel);
              }}
            >
              ✕
            </span>
          </div>
        ))}
        {tabs.length === 0 && (
          <span className="gf-muted" style={{ padding: '7px 12px', fontSize: 12 }}>
            Open a file from the explorer
          </span>
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0, position: 'relative' }}>
        <div ref={containerRef} style={{ position: 'absolute', inset: 0, display: diffMode ? 'none' : 'block' }} />
        <div ref={diffContainerRef} style={{ position: 'absolute', inset: 0, display: diffMode ? 'block' : 'none' }} />
      </div>
    </div>
  );
}
