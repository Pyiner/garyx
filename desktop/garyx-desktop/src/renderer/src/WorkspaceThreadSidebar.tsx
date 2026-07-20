import { useCallback, useEffect, useRef, useState } from 'react';
import { HoverCard as HoverCardPrimitive } from 'radix-ui';

import type { DesktopWorkspace, DesktopWorkspaceGitStatus } from '@shared/contracts';

import {
  CodexMenuCopyPathIcon,
  CodexMenuNewThreadIcon,
  CodexMenuPinIcon,
  CodexMenuRemoveIcon,
  CodexMenuRenameIcon,
  CodexPinIcon,
  CodexProjectActionsIcon,
  CodexProjectIcon,
  CodexProjectNewThreadIcon,
} from './components/codex-icons';
import { ChevronDownIcon } from './app-shell/icons';
import {
  type WorkspaceThreadGroup,
} from './thread-model';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './components/ui/dropdown-menu';
import { IconTooltip, TooltipProvider } from './components/ui/tooltip';
import { getDesktopApi } from './platform/desktop-api';
import { useI18n } from './i18n';

type WorkspaceMutation = 'assign' | 'add' | 'relink' | 'remove' | null;

/** Codex reference: the sidebar Projects section shows a bounded number of
 *  rows with a show-more/less toggle below (captured 2026-07-21). */
const DEFAULT_VISIBLE_WORKSPACE_ROWS = 7;

type WorkspaceThreadSidebarProps = {
  workspaceThreadGroups: WorkspaceThreadGroup[];
  workspaceMutation: WorkspaceMutation;
  workspaceMenuOpenPath: string | null;
  setWorkspaceMenuOpenPath: (value: string | ((current: string | null) => string | null) | null) => void;
  activeThreadId: string | null;
  gatewayHome: string | null;
  onOpenThread: (threadId: string) => void;
  onCreateThreadForWorkspace: (workspacePath: string) => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  onRequestRenameWorkspace: (workspace: DesktopWorkspace) => void;
  onPinWorkspace: (workspace: DesktopWorkspace, pinned: boolean) => void;
  onAddWorkspace: () => void;
};

function abbreviatePath(path: string, gatewayHome: string | null): string {
  const home = gatewayHome?.replace(/\/+$/, '');
  if (home && (path === home || path.startsWith(`${home}/`))) {
    return `~${path.slice(home.length)}`;
  }
  return path;
}

/** Delay-appear context card on workspace row hover (Codex reference:
 *  mark + name + pin + conversation count + abbreviated path). The git
 *  branch loads lazily when the card opens. */
function WorkspaceHoverCard({
  workspace,
  gatewayHome,
  children,
  onPinWorkspace,
  onCopyPath,
}: {
  workspace: DesktopWorkspace;
  gatewayHome: string | null;
  children: React.ReactNode;
  onPinWorkspace: (workspace: DesktopWorkspace, pinned: boolean) => void;
  onCopyPath: (path: string) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [gitStatus, setGitStatus] = useState<DesktopWorkspaceGitStatus | null>(null);
  const requestedRef = useRef(false);

  useEffect(() => {
    if (!open || requestedRef.current || !workspace.gitRepo || !workspace.path) {
      return;
    }
    requestedRef.current = true;
    let cancelled = false;
    void getDesktopApi()
      .getWorkspaceGitStatus({ workspacePath: workspace.path })
      .then((status) => {
        if (!cancelled) {
          setGitStatus(status);
        }
      })
      .catch(() => {
        // The branch line is optional context; the card renders without it.
      });
    return () => {
      cancelled = true;
    };
  }, [open, workspace.gitRepo, workspace.path]);

  const path = workspace.path || '';
  return (
    <HoverCardPrimitive.Root openDelay={550} closeDelay={120} open={open} onOpenChange={setOpen}>
      <HoverCardPrimitive.Trigger asChild>{children}</HoverCardPrimitive.Trigger>
      <HoverCardPrimitive.Portal>
        <HoverCardPrimitive.Content
          align="start"
          side="right"
          sideOffset={8}
          className="workspace-hover-card"
        >
          <div className="workspace-hover-card-head">
            <CodexProjectIcon size={16} />
            <span className="workspace-hover-card-name">{workspace.name}</span>
            <button
              aria-label={workspace.pinned ? t('Unpin workspace') : t('Pin workspace')}
              className={`workspace-hover-card-pin ${workspace.pinned ? 'is-pinned' : ''}`}
              onClick={() => onPinWorkspace(workspace, !workspace.pinned)}
              type="button"
            >
              <CodexPinIcon size={16} />
            </button>
          </div>
          <div className="workspace-hover-card-meta">
            <span>{t('{count} threads', { count: String(workspace.threadCount) })}</span>
            {gitStatus?.currentBranch ? (
              <span className="workspace-hover-card-branch">{gitStatus.currentBranch}</span>
            ) : null}
          </div>
          {path ? (
            <button
              className="workspace-hover-card-path"
              onClick={() => onCopyPath(path)}
              title={t('Copy path')}
              type="button"
            >
              {abbreviatePath(path, gatewayHome)}
            </button>
          ) : null}
        </HoverCardPrimitive.Content>
      </HoverCardPrimitive.Portal>
    </HoverCardPrimitive.Root>
  );
}

export function WorkspaceThreadSidebar({
  workspaceThreadGroups,
  workspaceMutation,
  workspaceMenuOpenPath,
  setWorkspaceMenuOpenPath,
  activeThreadId,
  gatewayHome,
  onOpenThread,
  onCreateThreadForWorkspace,
  onRequestRemoveWorkspace,
  onRequestRenameWorkspace,
  onPinWorkspace,
  onAddWorkspace,
}: WorkspaceThreadSidebarProps) {
  const { t } = useI18n();
  const [sectionCollapsed, setSectionCollapsed] = useState(false);
  const [showAllRows, setShowAllRows] = useState(false);
  const [expandedPaths, setExpandedPaths] = useState<ReadonlySet<string>>(new Set());

  const toggleExpanded = useCallback((workspacePath: string) => {
    setExpandedPaths((current) => {
      const next = new Set(current);
      if (next.has(workspacePath)) {
        next.delete(workspacePath);
      } else {
        next.add(workspacePath);
      }
      return next;
    });
  }, []);

  const handleCopyPath = useCallback((path: string) => {
    void navigator.clipboard.writeText(path);
  }, []);

  const overflowing = workspaceThreadGroups.length > DEFAULT_VISIBLE_WORKSPACE_ROWS;
  const visibleGroups = overflowing && !showAllRows
    ? workspaceThreadGroups.slice(0, DEFAULT_VISIBLE_WORKSPACE_ROWS)
    : workspaceThreadGroups;

  return (
    <TooltipProvider>
    <div className="sidebar-thread-block workspace-thread-block">
      <div className="panel-header sidebar-section-header sidebar-section-header-interactive">
        <button
          aria-expanded={!sectionCollapsed}
          aria-label={sectionCollapsed ? t('Expand workspaces') : t('Collapse workspaces')}
          className="sidebar-section-toggle"
          onClick={() => setSectionCollapsed((c) => !c)}
          type="button"
        >
          <span className="sidebar-section-title">{t('Workspaces')}</span>
          <ChevronDownIcon
            size={16}
            className={`icon sidebar-section-chevron ${sectionCollapsed ? 'collapsed' : ''}`}
          />
        </button>
        <div className="sidebar-section-tools">
          <IconTooltip label={t('Add workspace…')} side="bottom">
            <button
              aria-label={t('Add workspace…')}
              className="sidebar-section-action sidebar-section-action-always"
              disabled={workspaceMutation === 'add'}
              onClick={onAddWorkspace}
              type="button"
            >
              <svg aria-hidden width="12" height="12" viewBox="0 0 15 15" fill="none" style={{ strokeWidth: 1.21 }}>
                <path d="M0.5 7.5H14.5M7.5 0.5V14.5" stroke="currentColor" strokeLinecap="round"/>
              </svg>
            </button>
          </IconTooltip>
        </div>
      </div>

      <div
        aria-hidden={sectionCollapsed}
        className={`sidebar-collapsible sidebar-section-panel ${sectionCollapsed ? 'is-collapsed' : ''}`}
        inert={sectionCollapsed ? true : undefined}
      >
        <div className="sidebar-collapsible-inner workspace-list">
          {visibleGroups.map((group) => {
            const { workspace } = group;
            const workspacePath = workspace.path || workspace.name;
            const isExpanded = expandedPaths.has(workspacePath);
            const isMenuOpen = workspaceMenuOpenPath === workspacePath;

            return (
              <section
                className={`workspace-group ${!workspace.available ? 'missing' : ''}`}
                key={workspacePath}
              >
              <div className="workspace-row workspace-row-shell">
                <WorkspaceHoverCard
                  gatewayHome={gatewayHome}
                  onCopyPath={handleCopyPath}
                  onPinWorkspace={onPinWorkspace}
                  workspace={workspace}
                >
                  <button
                    aria-expanded={isExpanded}
                    className="workspace-row-main"
                    onClick={() => toggleExpanded(workspacePath)}
                    tabIndex={-1}
                    type="button"
                  >
                    <div className="workspace-row-copy">
                      <span className="workspace-folder-icon">
                        <CodexProjectIcon size={16} />
                      </span>
                      <span
                        className="workspace-name"
                        title={workspace.path || workspace.name}
                      >
                        {workspace.name}
                      </span>
                      {workspace.pinned ? (
                        <span className="workspace-pin-flag">
                          <CodexPinIcon size={12} />
                        </span>
                      ) : null}
                    </div>
                    {group.status ? (
                      <span
                        className={`workspace-status ${group.status === 'Unavailable' ? 'warning' : ''}`}
                      >
                        {t(group.status)}
                      </span>
                    ) : null}
                  </button>
                </WorkspaceHoverCard>

                <div className="workspace-actions">
                  <div className="workspace-more-menu-shell">
                    <DropdownMenu
                      open={isMenuOpen}
                      onOpenChange={(nextOpen) => {
                        setWorkspaceMenuOpenPath(nextOpen ? workspacePath : null);
                      }}
                    >
                      <IconTooltip
                        label={t('More actions for {name}', { name: workspace.name })}
                        side="bottom"
                      >
                        <DropdownMenuTrigger asChild>
                          <button
                            aria-label={t('More actions for {name}', { name: workspace.name })}
                            className="workspace-action-icon-button"
                            onClick={(event) => {
                              event.stopPropagation();
                            }}
                            tabIndex={-1}
                            type="button"
                          >
                            <CodexProjectActionsIcon size={16} />
                          </button>
                        </DropdownMenuTrigger>
                      </IconTooltip>
                      <DropdownMenuContent
                        align="end"
                        className="min-w-[186px]"
                        onClick={(event) => {
                          event.stopPropagation();
                        }}
                      >
                        <DropdownMenuItem
                          onSelect={() => {
                            onPinWorkspace(workspace, !workspace.pinned);
                          }}
                        >
                          <CodexMenuPinIcon aria-hidden />
                          <span>{workspace.pinned ? t('Unpin workspace') : t('Pin workspace')}</span>
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          onSelect={() => {
                            onRequestRenameWorkspace(workspace);
                          }}
                        >
                          <CodexMenuRenameIcon aria-hidden />
                          <span>{t('Rename…')}</span>
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          disabled={!workspace.available || workspaceMutation === 'assign'}
                          onSelect={() => {
                            onCreateThreadForWorkspace(workspacePath);
                          }}
                        >
                          <CodexMenuNewThreadIcon aria-hidden />
                          <span>{t('New thread')}</span>
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          onSelect={() => {
                            if (workspace.path) {
                              handleCopyPath(workspace.path);
                            }
                          }}
                        >
                          <CodexMenuCopyPathIcon aria-hidden />
                          <span>{t('Copy path')}</span>
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          onSelect={() => {
                            onRequestRemoveWorkspace(workspace);
                          }}
                        >
                          <CodexMenuRemoveIcon aria-hidden />
                          <span>{t('Remove…')}</span>
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  <IconTooltip
                    label={t('Create thread in {name}', { name: workspace.name })}
                    side="bottom"
                  >
                    <button
                      aria-label={t('Create thread in {name}', { name: workspace.name })}
                      className="workspace-action-icon-button"
                      disabled={!workspace.available || workspaceMutation === 'assign'}
                      onClick={(event) => {
                        event.stopPropagation();
                        onCreateThreadForWorkspace(workspacePath);
                      }}
                      tabIndex={-1}
                      type="button"
                    >
                      <CodexProjectNewThreadIcon size={16} />
                    </button>
                  </IconTooltip>
                  <button
                    aria-expanded={isExpanded}
                    aria-label={isExpanded
                      ? t('Collapse threads for {name}', { name: workspace.name })
                      : t('Expand threads for {name}', { name: workspace.name })}
                    className="workspace-action-icon-button workspace-row-chevron"
                    onClick={(event) => {
                      event.stopPropagation();
                      toggleExpanded(workspacePath);
                    }}
                    tabIndex={-1}
                    type="button"
                  >
                    <ChevronDownIcon
                      className={`icon workspace-chevron ${isExpanded ? 'expanded' : ''}`}
                      size={14}
                    />
                  </button>
                </div>
              </div>

              {isExpanded ? (
                <div className="workspace-thread-subtree" role="list">
                  {group.threads.length === 0 ? (
                    <div className="workspace-thread-subtree-empty">{t('No threads yet')}</div>
                  ) : (
                    group.threads.map((thread) => (
                      <button
                        className={`workspace-thread-subtree-row ${thread.id === activeThreadId ? 'is-active' : ''}`}
                        key={thread.id}
                        onClick={() => onOpenThread(thread.id)}
                        role="listitem"
                        title={thread.title}
                        type="button"
                      >
                        <span className="workspace-thread-subtree-title">{thread.title}</span>
                      </button>
                    ))
                  )}
                </div>
              ) : null}
            </section>
            );
          })}

          {overflowing ? (
            <button
              className="workspace-list-overflow-toggle"
              onClick={() => setShowAllRows((value) => !value)}
              type="button"
            >
              {showAllRows
                ? t('Show less')
                : t('Show more ({count})', {
                    count: String(workspaceThreadGroups.length - DEFAULT_VISIBLE_WORKSPACE_ROWS),
                  })}
            </button>
          ) : null}
        </div>
      </div>
    </div>
    </TooltipProvider>
  );
}
