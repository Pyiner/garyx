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
} from './codex-icons';
import { WorkspacePickerContent } from './WorkspacePickerContent';
import { useWorkspaceEpoch } from './workspace-data-adapter';
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

/** Codex parity: the chip echoes only the leaf directory name; the
 *  gateway home itself keeps its `~` spelling (a bare leaf would be
 *  meaningless there). */
function workspaceChipLabel(path: string, gatewayHome: string | null): string {
  const home = gatewayHome?.replace(/\/+$/, '');
  if (home && path === home) {
    return '~';
  }
  return path.split('/').filter(Boolean).pop() || path;
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
  const [gitStatusRepoPath, setGitStatusRepoPath] = useState<string | null>(null);
  const workspaceEpoch = useWorkspaceEpoch();

  // Gateway switch: the picker must not stay open over a new universe.
  useEffect(() => {
    setOpen(false);
  }, [workspaceEpoch]);

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
  }, [onWorkspaceModeChange, selectedPath, workspaceEpoch]);

  const worktreeCapable = Boolean(
    selectedPath &&
      (selectedWorkspace?.gitRepo || gitStatusRepoPath === selectedPath),
  );

  // Label precedence is intentional: a catalog workspace shows its
  // user-chosen display name (Codex project rows do the same); the
  // leaf/`~` path spelling applies only to paths outside the catalog.
  const chipLabel =
    selection?.kind === 'none'
      ? t('No workspace')
      : selectedWorkspace?.name ||
        (selectedPath ? workspaceChipLabel(selectedPath, gatewayHome) : t('No workspace'));

  const pick = (next: DraftWorkspaceSelection) => {
    setOpen(false);
    onSelectionChange(next);
  };

  return (
    <div className="workspace-chip-cluster">
      <Popover
        onOpenChange={setOpen}
        open={open}
      >
        <PopoverTrigger asChild>
          <button
            aria-label={t('Change workspace')}
            className={`workspace-composer-chip ${selection?.kind === 'none' ? 'is-none' : ''}`}
            title={selectedPath ?? undefined}
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
          <WorkspacePickerContent
            addWorkspaceBusy={addWorkspaceBusy}
            allowNone
            gatewayHome={gatewayHome}
            noneSelected={selection?.kind === 'none'}
            onAddWorkspace={() => {
              setOpen(false);
              onAddWorkspace();
            }}
            onSelectNone={() => pick({ kind: 'none' })}
            onSelectPath={(path) => pick({ kind: 'path', path })}
            selectedPath={selectedPath}
            workspaces={workspaces}
          />
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
                <span className="new-thread-menu-text">{t('Local')}</span>
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
