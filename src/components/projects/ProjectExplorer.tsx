// SPDX-License-Identifier: Apache-2.0
// Guild Foundry AI — Phase 4: project file explorer (sandboxed tree).
import React, { useCallback, useEffect, useState } from 'react';
import { invoke, DesktopCapabilityRequired } from '../../lib/api';

export interface FsEntry {
  name: string;
  rel: string;
  is_dir: boolean;
  size: number;
  modified: string | null;
}

function desktopError(err: unknown): string {
  return err instanceof DesktopCapabilityRequired
    ? 'Desktop Capability Required'
    : err instanceof Error
      ? err.message
      : String(err);
}

function TreeNode({
  entry,
  projectRoot,
  depth,
  selected,
  onSelect,
  onRefresh,
  onToast,
}: {
  entry: FsEntry;
  projectRoot: string;
  depth: number;
  selected: string | null;
  onSelect: (rel: string, isDir: boolean) => void;
  onRefresh: () => void;
  onToast: (t: string) => void;
}): React.ReactElement {
  const [expanded, setExpanded] = useState(depth < 1);
  const [children, setChildren] = useState<FsEntry[] | null>(null);

  const load = useCallback(async () => {
    try {
      const list = await invoke<FsEntry[]>('ws_list', {
        project_root: projectRoot,
        rel: entry.rel,
      });
      setChildren(list);
    } catch (err) {
      onToast(desktopError(err));
    }
  }, [entry.rel, projectRoot, onToast]);

  useEffect(() => {
    if (expanded && entry.is_dir && children === null) void load();
  }, [expanded, entry.is_dir, children, load]);

  const toggle = () => {
    if (entry.is_dir) {
      if (!expanded && children === null) void load();
      setExpanded(!expanded);
    } else {
      onSelect(entry.rel, false);
    }
  };

  const doRename = async () => {
    const name = window.prompt('Rename to:', entry.name);
    if (!name || name === entry.name) return;
    try {
      await invoke('ws_rename', { project_root: projectRoot, rel: entry.rel, new_name: name });
      onToast('Renamed');
      onRefresh();
      setChildren(null);
    } catch (err) {
      onToast(desktopError(err));
    }
  };

  const doDelete = async () => {
    if (!window.confirm(`Delete "${entry.name}"? A checkpoint is created first.`)) return;
    try {
      await invoke('ws_delete', { project_root: projectRoot, rel: entry.rel });
      onToast('Deleted (checkpoint saved)');
      onRefresh();
    } catch (err) {
      onToast(desktopError(err));
    }
  };

  return (
    <div>
      <div
        className={`gf-tree-node${selected === entry.rel ? ' selected' : ''}`}
        onClick={toggle}
        onContextMenu={(e) => {
          e.preventDefault();
          // Simple context actions via prompt-free buttons would need a menu;
          // double-click renames, alt-click deletes.
        }}
        onDoubleClick={(e) => {
          e.stopPropagation();
          void doRename();
        }}
        title="Double-click to rename. Right-click menu: use toolbar actions."
      >
        <span className="gf-caret">{entry.is_dir ? (expanded ? '▾' : '▸') : ''}</span>
        <span>{entry.is_dir ? '📁' : '📄'}</span>
        <span className="gf-fname">{entry.name}</span>
        {selected === entry.rel && entry.is_dir && (
          <button
            className="gf-btn"
            style={{ padding: '0 6px', fontSize: 10 }}
            onClick={(e) => {
              e.stopPropagation();
              void doDelete();
            }}
          >
            del
          </button>
        )}
      </div>
      {entry.is_dir && expanded && (
        <div className="gf-tree-children">
          {(children ?? []).map((c) => (
            <TreeNode
              key={c.rel}
              entry={c}
              projectRoot={projectRoot}
              depth={depth + 1}
              selected={selected}
              onSelect={onSelect}
              onRefresh={() => {
                setChildren(null);
                onRefresh();
              }}
              onToast={onToast}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function ProjectExplorer({
  projectRoot,
  selected,
  onOpenFile,
  onToast,
}: {
  projectRoot: string;
  selected: string | null;
  onOpenFile: (rel: string) => void;
  onToast: (t: string) => void;
}): React.ReactElement {
  const [root, setRoot] = useState<FsEntry[]>([]);
  const [refreshKey, setRefreshKey] = useState(0);

  const loadRoot = useCallback(async () => {
    try {
      const list = await invoke<FsEntry[]>('ws_list', {
        project_root: projectRoot,
        rel: '',
      });
      setRoot(list);
    } catch (err) {
      onToast(desktopError(err));
    }
  }, [projectRoot, onToast]);

  useEffect(() => {
    void loadRoot();
  }, [loadRoot, refreshKey]);

  const createEntry = async (isDir: boolean) => {
    const name = window.prompt(isDir ? 'New folder name:' : 'New file name (relative path):');
    if (!name) return;
    const base = selected
      ? root.find((e) => e.rel === selected)?.is_dir
        ? selected
        : selected.split('/').slice(0, -1).join('/')
      : '';
    const rel = base ? `${base}/${name}` : name;
    try {
      await invoke('ws_create', { project_root: projectRoot, rel, is_dir: isDir });
      onToast(isDir ? 'Folder created' : 'File created');
      setRefreshKey((k) => k + 1);
      if (!isDir) onOpenFile(rel);
    } catch (err) {
      onToast(desktopError(err));
    }
  };

  return (
    <div className="gf-projects-col" style={{ width: 260, minWidth: 200 }}>
      <div className="gf-pane-header">
        <span>Explorer</span>
        <span className="gf-row">
          <button className="gf-btn" style={{ padding: '0 6px' }} onClick={() => void createEntry(false)} title="New file">+</button>
          <button className="gf-btn" style={{ padding: '0 6px' }} onClick={() => void createEntry(true)} title="New folder">📁</button>
          <button className="gf-btn" style={{ padding: '0 6px' }} onClick={() => setRefreshKey((k) => k + 1)} title="Refresh">⟳</button>
        </span>
      </div>
      <div className="gf-explorer-tree">
        {root.map((e) => (
          <TreeNode
            key={e.rel}
            entry={e}
            projectRoot={projectRoot}
            depth={0}
            selected={selected}
            onSelect={(rel) => onOpenFile(rel)}
            onRefresh={() => setRefreshKey((k) => k + 1)}
            onToast={onToast}
          />
        ))}
        {root.length === 0 && (
          <p className="gf-muted" style={{ padding: '8px 12px', fontSize: 12 }}>
            Empty project. Create a file to begin.
          </p>
        )}
      </div>
      <div style={{ padding: 8, borderTop: '1px solid var(--gf-border)' }}>
        <span className="gf-label">Search</span>
        <input
          className="gf-input"
          placeholder="Search files…"
          onKeyDown={async (e) => {
            if (e.key !== 'Enter') return;
            const q = (e.target as HTMLInputElement).value;
            try {
              const hits = await invoke<string[]>('ws_search', {
                project_root: projectRoot,
                query: q,
              });
              onToast(hits.length ? `${hits.length} match(es): ${hits.slice(0, 5).join(', ')}` : 'No matches');
            } catch (err) {
              onToast(desktopError(err));
            }
          }}
        />
      </div>
    </div>
  );
}
