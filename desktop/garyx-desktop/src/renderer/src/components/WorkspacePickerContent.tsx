import { useState } from 'react';

import type { DesktopWorkspace } from '@shared/contracts';

import {
  CodexNewProjectIcon,
  CodexNoWorkspaceIcon,
  CodexPickerCheckIcon,
  CodexPickerProjectIcon,
  CodexPickerSearchIcon,
} from './codex-icons';
import { useI18n } from '../i18n';

export type WorkspacePickerContentProps = {
  workspaces: DesktopWorkspace[];
  gatewayHome: string | null;
  /** Currently selected workspace path; empty/null means none. */
  selectedPath?: string | null;
  /** Offer the explicit "No workspace" footer action. */
  allowNone?: boolean;
  /** Whether the explicit No-workspace state is currently selected. */
  noneSelected?: boolean;
  addWorkspaceBusy?: boolean;
  onSelectPath: (path: string) => void;
  onSelectNone?: () => void;
  onAddWorkspace?: () => void;
};

function abbreviatePath(path: string, gatewayHome: string | null): string {
  const home = gatewayHome?.replace(/\/+$/, '');
  if (home && (path === home || path.startsWith(`${home}/`))) {
    return `~${path.slice(home.length)}`;
  }
  return path;
}

/**
 * The one workspace picker body (Codex project picker, captured
 * 2026-07-21): search, the server-ordered workspace list with the current
 * selection checked, and the Add workspace… / No workspace footer actions.
 * Every picker host — the composer chip popover and the in-form dialog —
 * renders this content so ordering, icons, and copy never fork.
 */
export function WorkspacePickerContent({
  workspaces,
  gatewayHome,
  selectedPath = null,
  allowNone = false,
  noneSelected = false,
  addWorkspaceBusy = false,
  onSelectPath,
  onSelectNone,
  onAddWorkspace,
}: WorkspacePickerContentProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');

  const normalizedQuery = query.trim().toLowerCase();
  const normalizedSelected = (selectedPath || '').trim();
  const listed = workspaces.some(
    (workspace) => (workspace.path || '').trim() === normalizedSelected,
  );
  // Keep a selected-but-unlisted path visible so the check mark always has
  // a row (an explicit path outside the root list stays presentable).
  const rows: DesktopWorkspace[] =
    normalizedSelected && !listed
      ? [
          {
            name: abbreviatePath(normalizedSelected, gatewayHome),
            path: normalizedSelected,
            kind: 'local',
            createdAt: '',
            updatedAt: '',
            available: true,
            pinned: false,
            threadCount: 0,
            lastActivityAt: null,
            gitRepo: false,
          },
          ...workspaces,
        ]
      : workspaces;
  const filtered = normalizedQuery
    ? rows.filter((workspace) => {
        const name = workspace.name.toLowerCase();
        const path = (workspace.path || '').toLowerCase();
        return name.includes(normalizedQuery) || path.includes(normalizedQuery);
      })
    : rows;

  return (
    <>
      <div className="workspace-picker-search">
        <CodexPickerSearchIcon size={16} />
        <input
          aria-label={t('Search workspaces')}
          autoFocus
          className="workspace-picker-search-input"
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t('Search workspaces')}
          value={query}
        />
      </div>
      <div className="workspace-picker-list" role="listbox">
        {filtered.length === 0 ? (
          <div className="workspace-picker-empty">{t('No matches')}</div>
        ) : (
          filtered.map((workspace) => {
            const path = workspace.path || '';
            const checked = Boolean(path && path === normalizedSelected);
            return (
              <button
                aria-selected={checked}
                className="workspace-picker-item"
                disabled={!workspace.available || !path}
                key={path || workspace.name}
                onClick={() => {
                  if (path) {
                    onSelectPath(path);
                  }
                }}
                role="option"
                type="button"
              >
                <CodexPickerProjectIcon size={16} />
                <span className="workspace-picker-item-name">
                  {workspace.available
                    ? workspace.name
                    : t('{name} (Unavailable)', { name: workspace.name })}
                </span>
                <span className="workspace-picker-item-path">
                  {path ? abbreviatePath(path, gatewayHome) : ''}
                </span>
                {checked ? <CodexPickerCheckIcon size={17} /> : null}
              </button>
            );
          })
        )}
      </div>
      {onAddWorkspace || allowNone ? (
        <div className="workspace-picker-footer">
          {onAddWorkspace ? (
            <button
              className="workspace-picker-item"
              disabled={addWorkspaceBusy}
              onClick={onAddWorkspace}
              type="button"
            >
              <CodexNewProjectIcon size={16} />
              <span className="workspace-picker-item-name">
                {addWorkspaceBusy ? t('Opening folder…') : t('Add workspace…')}
              </span>
            </button>
          ) : null}
          {allowNone && onSelectNone ? (
            <button
              aria-selected={noneSelected}
              className="workspace-picker-item"
              onClick={onSelectNone}
              type="button"
            >
              <CodexNoWorkspaceIcon size={16} />
              <span className="workspace-picker-item-name">{t('No workspace')}</span>
              {noneSelected ? <CodexPickerCheckIcon size={17} /> : null}
            </button>
          ) : null}
        </div>
      ) : null}
    </>
  );
}
