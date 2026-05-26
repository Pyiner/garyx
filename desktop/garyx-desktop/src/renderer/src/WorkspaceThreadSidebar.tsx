import { type CSSProperties, useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Trash } from 'lucide-react';

import { NewTabIcon } from './app-shell/icons';

import { ChevronDownIcon, FolderIcon, FolderOpenIcon, MoreDotsIcon } from './app-shell/icons';

import type { DesktopWorkspace } from '@shared/contracts';

import {
  type WorkspaceThreadGroup,
} from './thread-model';
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
  const [workspaceMenuStyle, setWorkspaceMenuStyle] = useState<CSSProperties | null>(null);
  const menuButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const visibleWorkspaceThreadGroups = workspaceThreadGroups;

  const updateWorkspaceMenuPosition = useCallback((workspacePath: string | null) => {
    if (!workspacePath) {
      setWorkspaceMenuStyle(null);
      return;
    }

    const button = menuButtonRefs.current[workspacePath];
    if (!button) {
      setWorkspaceMenuStyle(null);
      return;
    }

    const rect = button.getBoundingClientRect();
    const viewportPadding = 12;
    const menuWidth = 146;
    const estimatedHeight = 40;
    const gap = 4;
    const nextLeft = Math.max(
      viewportPadding,
      Math.min(rect.right - menuWidth, window.innerWidth - menuWidth - viewportPadding),
    );
    let nextTop = rect.bottom + gap;
    if (nextTop + estimatedHeight > window.innerHeight - viewportPadding) {
      nextTop = Math.max(viewportPadding, rect.top - estimatedHeight - gap);
    }

    setWorkspaceMenuStyle({
      left: `${nextLeft}px`,
      top: `${nextTop}px`,
    });
  }, []);

  useEffect(() => {
    if (!workspaceMenuOpenPath) {
      setWorkspaceMenuStyle(null);
      return;
    }

    const update = () => {
      updateWorkspaceMenuPosition(workspaceMenuOpenPath);
    };

    update();
    window.addEventListener('resize', update);
    window.addEventListener('scroll', update, true);
    return () => {
      window.removeEventListener('resize', update);
      window.removeEventListener('scroll', update, true);
    };
  }, [updateWorkspaceMenuPosition, workspaceMenuOpenPath]);

  const handleWorkspaceClick = useCallback(
    (workspacePath: string) => {
      onToggleWorkspaceThreads(workspacePath);
    },
    [onToggleWorkspaceThreads],
  );

  return (
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
          <button
            aria-label={t('Add workspace…')}
            className="sidebar-section-action sidebar-section-action-always"
            disabled={workspaceMutation === 'add'}
            onClick={onAddWorkspace}
            title={t('Add workspace…')}
            type="button"
          >
            <svg aria-hidden width="14" height="14" viewBox="0 0 15 15" fill="none" style={{ strokeWidth: 1.21 }}>
              <path d="M0.5 7.5H14.5M7.5 0.5V14.5" stroke="currentColor" strokeLinecap="round"/>
            </svg>
          </button>
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
                      <button
                        aria-label={t('Create thread in {name}', { name: workspace.name })}
                        className="workspace-action-icon-button"
                        disabled={!workspace.available || workspaceMutation === 'assign'}
                        onClick={(event) => {
                          event.stopPropagation();
                          onCreateThreadForWorkspace(workspacePath);
                        }}
                        tabIndex={-1}
                        title={
                          workspaceMutation === 'assign'
                            ? t('Creating thread…')
                            : t('Create thread in {name}', { name: workspace.name })
                        }
                        type="button"
                      >
                        <NewTabIcon />
                      </button>
                      {group.canManageWorkspace ? (
                        <div className="workspace-more-menu-shell">
                          <button
                            aria-expanded={isMenuOpen}
                            aria-haspopup="menu"
                            aria-label={t('More actions for {name}', { name: workspace.name })}
                            className="workspace-action-icon-button"
                            ref={(node) => {
                              menuButtonRefs.current[workspacePath] = node;
                            }}
                            onClick={(event) => {
                              event.stopPropagation();
                              setWorkspaceMenuOpenPath((current) => {
                                return current === workspacePath ? null : workspacePath;
                              });
                            }}
                            tabIndex={-1}
                            title={t('More actions for {name}', { name: workspace.name })}
                            type="button"
                          >
                            <MoreDotsIcon size={16} />
                          </button>
                          {isMenuOpen && workspaceMenuStyle && typeof document !== 'undefined'
                            ? createPortal(
                              <div
                                className="workspace-actions workspace-menu-portal"
                                style={{
                                  position: 'fixed',
                                  left: workspaceMenuStyle.left,
                                  top: workspaceMenuStyle.top,
                                  zIndex: 2000,
                                  display: 'block',
                                }}
                              >
                                <div
                                  className="workspace-more-menu"
                                  role="menu"
                                  style={{
                                    position: 'static',
                                    zIndex: 'auto',
                                    minWidth: '146px',
                                    maxHeight: 'min(240px, calc(100vh - 24px))',
                                    overflowY: 'auto',
                                  }}
                                >
                                  <button
                                    className="workspace-menu-item"
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      setWorkspaceMenuOpenPath(null);
                                      onRequestRemoveWorkspace(workspace);
                                    }}
                                    role="menuitem"
                                    title={t('Remove')}
                                    type="button"
                                  >
                                    <Trash aria-hidden />
                                    {t('Remove')}
                                  </button>
                                </div>
                              </div>,
                              document.body,
                            )
                            : null}
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
  );
}
