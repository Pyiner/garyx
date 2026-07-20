import { useEffect, useMemo, useState } from 'react';
import { GitBranch, Laptop } from 'lucide-react';

import type {
  DesktopWorkspace,
  DesktopWorkspaceMode,
  DraftWorkspaceSelection,
} from '@shared/contracts';

import {
  CodexChipNoProjectIcon,
  CodexChipProjectIcon,
  CodexNewProjectIcon,
  CodexNoWorkspaceIcon,
  CodexPickerCheckIcon,
  CodexPickerProjectIcon,
  CodexPickerSearchIcon,
} from './codex-icons';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from './ui/popover';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from './ui/select';
import {
  loadWorkspaceGitStatusCached,
  workspaceGitStatusCache,
} from '../workspace-git-status-cache';
import { useI18n } from '../i18n';

const GIT_STATUS_CHECK_DELAY_MS = 120;

type WorkspaceComposerChipProps = {
  selection: DraftWorkspaceSelection | null;
  workspaces: DesktopWorkspace[];
  gatewayHome: string | null;
  workspaceMode: DesktopWorkspaceMode;
  addWorkspaceBusy: boolean;
  onSelectionChange: (selection: DraftWorkspaceSelection) => void;
  onWorkspaceModeChange: (workspaceMode: DesktopWorkspaceMode) => void;
  onAddWorkspace: () => void;
};

function abbreviatePath(path: string, gatewayHome: string | null): string {
  const home = gatewayHome?.replace(/\/+$/, '');
  if (home && (path === home || path.startsWith(`${home}/`))) {
    return `~${path.slice(home.length)}`;
  }
  return path;
}

/**
 * The draft workspace chip in the composer footer (Codex composer project
 * chip, captured 2026-07-21): a pill showing the draft's tri-state
 * selection; clicking opens the anchored picker popover — search, the
 * server-ordered workspace list with the current selection checked, and the
 * `Add workspace…` / `No workspace` footer actions. The worktree mode
 * select renders alongside when the selected workspace is a git repo.
 */
export function WorkspaceComposerChip({
  selection,
  workspaces,
  gatewayHome,
  workspaceMode,
  addWorkspaceBusy,
  onSelectionChange,
  onWorkspaceModeChange,
  onAddWorkspace,
}: WorkspaceComposerChipProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [gitStatusRepoPath, setGitStatusRepoPath] = useState<string | null>(null);

  const selectedPath = selection?.kind === 'path' ? selection.path : null;
  const selectedWorkspace = useMemo(
    () =>
      selectedPath
        ? workspaces.find((workspace) => workspace.path === selectedPath) ?? null
        : null,
    [selectedPath, workspaces],
  );

  // Worktree gating: the mode select renders only for git-repo workspaces.
  // The list's git_repo flag answers immediately; the cached git-status
  // probe confirms (and keeps the cache warm for the branch display).
  useEffect(() => {
    let cancelled = false;
    setGitStatusRepoPath(null);
    if (!selectedPath) {
      onWorkspaceModeChange('local');
      return;
    }
    const cached = workspaceGitStatusCache.get(selectedPath);
    if (cached) {
      if (cached.isGitRepo) {
        setGitStatusRepoPath(selectedPath);
      } else {
        onWorkspaceModeChange('local');
      }
      return;
    }
    const timeout = window.setTimeout(() => {
      void loadWorkspaceGitStatusCached({
        cache: workspaceGitStatusCache,
        workspacePath: selectedPath,
        load: () =>
          window.garyxDesktop.getWorkspaceGitStatus({
            workspacePath: selectedPath,
          }),
      })
        .then((status) => {
          if (cancelled) return;
          if (status.isGitRepo) {
            setGitStatusRepoPath(selectedPath);
          } else {
            onWorkspaceModeChange('local');
          }
        })
        .catch(() => {
          if (cancelled) return;
          onWorkspaceModeChange('local');
        });
    }, GIT_STATUS_CHECK_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
    };
  }, [onWorkspaceModeChange, selectedPath]);

  const worktreeCapable = Boolean(
    selectedPath &&
      (selectedWorkspace?.gitRepo || gitStatusRepoPath === selectedPath),
  );

  const normalizedQuery = query.trim().toLowerCase();
  const filteredWorkspaces = normalizedQuery
    ? workspaces.filter((workspace) => {
        const name = workspace.name.toLowerCase();
        const path = (workspace.path || '').toLowerCase();
        return name.includes(normalizedQuery) || path.includes(normalizedQuery);
      })
    : workspaces;

  const chipLabel =
    selection?.kind === 'none'
      ? t('No workspace')
      : selectedWorkspace?.name ||
        (selectedPath ? abbreviatePath(selectedPath, gatewayHome) : t('No workspace'));

  const pick = (next: DraftWorkspaceSelection) => {
    setOpen(false);
    setQuery('');
    onSelectionChange(next);
  };

  return (
    <div className="workspace-chip-cluster">
      <Popover
        onOpenChange={(nextOpen) => {
          setOpen(nextOpen);
          if (!nextOpen) {
            setQuery('');
          }
        }}
        open={open}
      >
        <PopoverTrigger asChild>
          <button
            aria-label={t('Change workspace')}
            className={`workspace-composer-chip ${selection?.kind === 'none' ? 'is-none' : ''}`}
            type="button"
          >
            {selection?.kind === 'none' ? (
              <CodexChipNoProjectIcon size={16} />
            ) : (
              <CodexChipProjectIcon size={16} />
            )}
            <span className="workspace-composer-chip-label">{chipLabel}</span>
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="menu-popover-surface workspace-picker-popover"
          side="top"
          sideOffset={8}
        >
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
            {filteredWorkspaces.length === 0 ? (
              <div className="workspace-picker-empty">{t('No matches')}</div>
            ) : (
              filteredWorkspaces.map((workspace) => {
                const path = workspace.path || '';
                const checked = selection?.kind === 'path' && selection.path === path;
                return (
                  <button
                    aria-selected={checked}
                    className="workspace-picker-item"
                    key={path || workspace.name}
                    onClick={() => {
                      if (path) {
                        pick({ kind: 'path', path });
                      }
                    }}
                    role="option"
                    type="button"
                  >
                    <CodexPickerProjectIcon size={16} />
                    <span className="workspace-picker-item-name">{workspace.name}</span>
                    <span className="workspace-picker-item-path">
                      {path ? abbreviatePath(path, gatewayHome) : ''}
                    </span>
                    {checked ? <CodexPickerCheckIcon size={17} /> : null}
                  </button>
                );
              })
            )}
          </div>
          <div className="workspace-picker-footer">
            <button
              className="workspace-picker-item"
              disabled={addWorkspaceBusy}
              onClick={() => {
                setOpen(false);
                setQuery('');
                onAddWorkspace();
              }}
              type="button"
            >
              <CodexNewProjectIcon size={16} />
              <span className="workspace-picker-item-name">{t('Add workspace…')}</span>
            </button>
            <button
              aria-selected={selection?.kind === 'none'}
              className="workspace-picker-item"
              onClick={() => pick({ kind: 'none' })}
              type="button"
            >
              <CodexNoWorkspaceIcon size={16} />
              <span className="workspace-picker-item-name">{t('No workspace')}</span>
              {selection?.kind === 'none' ? <CodexPickerCheckIcon size={17} /> : null}
            </button>
          </div>
        </PopoverContent>
      </Popover>

      {worktreeCapable ? (
        <Select
          onValueChange={(value) =>
            onWorkspaceModeChange(value as DesktopWorkspaceMode)
          }
          value={workspaceMode}
        >
          <SelectTrigger
            aria-label={t('Workspace mode')}
            className="workspace-chip-mode-trigger"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent
            align="start"
            className="new-thread-mode-menu"
            position="popper"
            side="top"
            sideOffset={4}
          >
            <SelectGroup>
              <SelectLabel>{t('Workspace mode')}</SelectLabel>
              <SelectItem value="local">
                <Laptop aria-hidden size={16} strokeWidth={1.7} />
                <span className="new-thread-menu-text">{t('Direct')}</span>
              </SelectItem>
              <SelectItem value="worktree">
                <GitBranch aria-hidden size={16} strokeWidth={1.7} />
                <span className="new-thread-menu-text">{t('Worktree')}</span>
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
      ) : null}
    </div>
  );
}
