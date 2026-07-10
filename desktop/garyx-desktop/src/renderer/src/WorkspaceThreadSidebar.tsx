import { useCallback, useState } from 'react';
import { Trash } from 'lucide-react';

import { NewTabIcon } from './app-shell/icons';

import { ChevronDownIcon, FolderIcon, FolderOpenIcon, MoreDotsIcon } from './app-shell/icons';

import type { DesktopWorkspace } from '@shared/contracts';

import {
  type WorkspaceThreadGroup,
} from './thread-model';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './components/ui/dropdown-menu';
import { IconTooltip, TooltipProvider } from './components/ui/tooltip';
import { useI18n } from './i18n';

type WorkspaceMutation = 'assign' | 'add' | 'relink' | 'remove' | null;

type WorkspaceThreadSidebarProps = {
  activeWorkspacePath: string | null;
  workspaceThreadGroups: WorkspaceThreadGroup[];
  workspaceMutation: WorkspaceMutation;
  workspaceMenuOpenPath: string | null;
  setWorkspaceMenuOpenPath: (value: string | ((current: string | null) => string | null) | null) => void;
  onToggleWorkspaceThreads: (workspacePath: string) => void;
  onCreateThreadForWorkspace: (workspacePath: string) => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  onAddWorkspace: () => void;
};

export function WorkspaceThreadSidebar({
  activeWorkspacePath,
  workspaceThreadGroups,
  workspaceMutation,
  workspaceMenuOpenPath,
  setWorkspaceMenuOpenPath,
  onToggleWorkspaceThreads,
  onCreateThreadForWorkspace,
  onRequestRemoveWorkspace,
  onAddWorkspace,
}: WorkspaceThreadSidebarProps) {
  const { t } = useI18n();
  const [sectionCollapsed, setSectionCollapsed] = useState(false);
  const visibleWorkspaceThreadGroups = workspaceThreadGroups;

  const handleWorkspaceClick = useCallback(
    (workspacePath: string) => {
      onToggleWorkspaceThreads(workspacePath);
    },
    [onToggleWorkspaceThreads],
  );

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
          {visibleWorkspaceThreadGroups.map((group) => {
            const { workspace } = group;
            const workspacePath = workspace.path || workspace.name;
            const isWorkspacePanelOpen =
              Boolean(activeWorkspacePath) &&
              activeWorkspacePath?.trim().toLowerCase() ===
                workspacePath.trim().toLowerCase();
            const isMenuOpen = workspaceMenuOpenPath === workspacePath;

            return (
              <section
                className={`workspace-group ${!workspace.available ? 'missing' : ''}`}
                key={workspacePath}
              >
              <div className="workspace-row workspace-row-shell">
                <button
                  aria-expanded={isWorkspacePanelOpen}
                  className="workspace-row-main"
                  onClick={() => handleWorkspaceClick(workspacePath)}
                  tabIndex={-1}
                  type="button"
                >
                  <div className="workspace-row-copy">
                    <span className="workspace-folder-icon">
                      <span className="workspace-folder-icon-default">
                        {isWorkspacePanelOpen ? <FolderOpenIcon /> : <FolderIcon />}
                      </span>
                      <span className="workspace-folder-icon-hover">
                        {isWorkspacePanelOpen ? <FolderIcon /> : <FolderOpenIcon />}
                      </span>
                    </span>
                    <span
                      className="workspace-name"
                      title={workspace.path || workspace.name}
                    >
                      {workspace.name}
                    </span>
                  </div>
                  {group.status ? (
                    <span
                      className={`workspace-status ${group.status === 'Unavailable' ? 'warning' : ''}`}
                    >
                      {t(group.status)}
                    </span>
                  ) : null}
                </button>

                <div className="workspace-actions">
                  <>
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
                          <NewTabIcon />
                        </button>
                      </IconTooltip>
                      {group.canManageWorkspace ? (
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
                                  <MoreDotsIcon size={16} />
                                </button>
                              </DropdownMenuTrigger>
                            </IconTooltip>
                            <DropdownMenuContent
                              align="end"
                              className="min-w-[146px]"
                              onClick={(event) => {
                                event.stopPropagation();
                              }}
                            >
                              <DropdownMenuItem
                                onSelect={() => {
                                  onRequestRemoveWorkspace(workspace);
                                }}
                              >
                                <Trash aria-hidden />
                                <span>{t('Remove')}</span>
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </div>
                      ) : null}
                  </>
                </div>
              </div>
            </section>
            );
          })}

        </div>
      </div>
    </div>
    </TooltipProvider>
  );
}
