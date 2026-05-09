import { type CSSProperties, type KeyboardEvent, useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Trash } from 'lucide-react';

import { NewTabIcon } from './app-shell/icons';

import { ChevronDownIcon, FolderIcon, FolderOpenIcon, MoreDotsIcon } from './app-shell/icons';

import type { DesktopState, DesktopWorkspace } from '@shared/contracts';

import {
  buildWorkspaceThreadRows,
  type WorkspaceThreadGroup,
} from './thread-model';
import { useI18n } from './i18n';

type WorkspaceMutation = 'assign' | 'add' | 'relink' | 'remove' | null;

type WorkspaceThreadSidebarProps = {
  desktopState: DesktopState | null;
  workspaceThreadGroups: WorkspaceThreadGroup[];
  selectedThreadId: string | null;
  deletingThreadId: string | null;
  workspaceMutation: WorkspaceMutation;
  workspaceMenuOpenPath: string | null;
  renamingWorkspacePath: string | null;
  workspaceNameDraft: string;
  setWorkspaceMenuOpenPath: (value: string | ((current: string | null) => string | null) | null) => void;
  setWorkspaceNameDraft: (value: string) => void;
  setContentView: (view: 'thread') => void;
  isThreadRuntimeBusy: (threadId: string) => boolean;
  formatThreadTimestamp: (value?: string | null) => string;
  onOpenThread: (threadId: string) => void;
  onSelectWorkspace: (workspacePath: string, preferredThreadId: string | null) => void;
  onCreateThreadForWorkspace: (workspacePath: string) => void;
  onBeginRenameWorkspace: (workspace: DesktopWorkspace) => void;
  onSubmitRenameWorkspace: (workspacePath: string) => void;
  onCancelRenameWorkspace: () => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  onDeleteThread: (threadId: string) => void;
  onAddWorkspace: () => void;
};

export function WorkspaceThreadSidebar({
  desktopState,
  workspaceThreadGroups,
  selectedThreadId,
  deletingThreadId,
  workspaceMutation,
  workspaceMenuOpenPath,
  renamingWorkspacePath,
  workspaceNameDraft,
  setWorkspaceMenuOpenPath,
  setWorkspaceNameDraft,
  setContentView,
  isThreadRuntimeBusy,
  formatThreadTimestamp,
  onOpenThread,
  onSelectWorkspace,
  onCreateThreadForWorkspace,
  onBeginRenameWorkspace,
  onSubmitRenameWorkspace,
  onCancelRenameWorkspace,
  onRequestRemoveWorkspace,
  onDeleteThread,
  onAddWorkspace,
}: WorkspaceThreadSidebarProps) {
  const { t } = useI18n();
  const [sectionCollapsed, setSectionCollapsed] = useState(false);
  const [collapsedPaths, setCollapsedPaths] = useState<Set<string>>(new Set());
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [expandedWorkspacePreviewPaths, setExpandedWorkspacePreviewPaths] = useState<Set<string>>(new Set());
  const [workspaceMenuStyle, setWorkspaceMenuStyle] = useState<CSSProperties | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const menuButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const visibleWorkspaceThreadGroups = workspaceThreadGroups;

  // Auto-dismiss the confirm state after 3 seconds
  useEffect(() => {
    if (!confirmDeleteId) return;
    confirmTimerRef.current = setTimeout(() => {
      setConfirmDeleteId(null);
    }, 3000);
    return () => {
      if (confirmTimerRef.current) {
        clearTimeout(confirmTimerRef.current);
      }
    };
  }, [confirmDeleteId]);

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
      setCollapsedPaths((prev) => {
        const next = new Set(prev);
        if (next.has(workspacePath)) {
          next.delete(workspacePath);
        } else {
          next.add(workspacePath);
        }
        return next;
      });
    },
    [],
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
            const isWorkspaceCollapsed = collapsedPaths.has(workspacePath);
            const isMenuOpen = workspaceMenuOpenPath === workspacePath;
            const isRenaming = renamingWorkspacePath === workspacePath;
            const isPreviewExpanded = expandedWorkspacePreviewPaths.has(workspacePath);
            const rows = buildWorkspaceThreadRows({
              state: desktopState,
              threads: group.threads,
              selectedThreadId,
              deletingThreadId,
              isThreadRuntimeBusy,
            });
            // Hide pending deletes immediately so the sidebar does not wait for
            // the IPC + gateway round-trip before the row disappears.
            const visibleRows = rows.filter((row) => !row.isDeleting);
            const hasPreviewOverflow = visibleRows.length > 3;
            const previewRows = isPreviewExpanded ? visibleRows : visibleRows.slice(0, 3);

            const handleRenameInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                onSubmitRenameWorkspace(workspacePath);
              } else if (event.key === 'Escape') {
                event.preventDefault();
                onCancelRenameWorkspace();
              }
            };

            return (
              <section
                className={`workspace-group ${!workspace.available ? 'missing' : ''}`}
                key={workspacePath}
              >
              <div className="workspace-row workspace-row-shell">
                <button
                  aria-expanded={!isWorkspaceCollapsed}
                  className="workspace-row-main"
                  onClick={() => handleWorkspaceClick(workspacePath)}
                  tabIndex={-1}
                  type="button"
                >
                  <div className="workspace-row-copy">
                    <span className="workspace-folder-icon">
                      <span className="workspace-folder-icon-default">
                        {isWorkspaceCollapsed ? <FolderIcon /> : <FolderOpenIcon />}
                      </span>
                      <span className="workspace-folder-icon-hover">
                        {isWorkspaceCollapsed ? <FolderOpenIcon /> : <FolderIcon />}
                      </span>
                    </span>
                    {isRenaming ? (
                      <input
                        aria-label={t('Rename {name}', { name: workspace.name })}
                        className="workspace-rename-input"
                        onChange={(event) => {
                          setWorkspaceNameDraft(event.target.value);
                        }}
                        onClick={(event) => {
                          event.stopPropagation();
                        }}
                        onKeyDown={handleRenameInputKeyDown}
                        value={workspaceNameDraft}
                      />
                    ) : (
                      <span
                        className="workspace-name"
                        title={workspace.path || workspace.name}
                      >
                        {workspace.name}
                      </span>
                    )}
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
                  {isRenaming ? (
                    <>
                      <button
                        className="workspace-action-button workspace-action-confirm"
                        onClick={(event) => {
                          event.stopPropagation();
                          onSubmitRenameWorkspace(workspacePath);
                        }}
                        tabIndex={-1}
                        type="button"
                      >
                        {t('Save')}
                      </button>
                      <button
                        className="workspace-action-button"
                        onClick={(event) => {
                          event.stopPropagation();
                          onCancelRenameWorkspace();
                        }}
                        tabIndex={-1}
                        type="button"
                      >
                        {t('Cancel')}
                      </button>
                    </>
                  ) : (
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
                  )}
                </div>
              </div>

              <div
                aria-hidden={isWorkspaceCollapsed}
                className={`thread-list workspace-thread-list sidebar-collapsible ${isWorkspaceCollapsed ? 'is-collapsed' : ''}`}
                inert={isWorkspaceCollapsed ? true : undefined}
              >
                <div className="sidebar-collapsible-inner workspace-thread-list-inner">
                  {visibleRows.length ? (
                    previewRows.map((row) => {
                      const { thread } = row;
                      return (
                        <div
                          key={thread.id}
                          className={`thread-item ${row.isActive ? 'active' : ''} ${row.deleteDisabled ? 'no-delete' : ''}`}
                          onMouseLeave={() => {
                            if (confirmDeleteId === thread.id) {
                              setConfirmDeleteId(null);
                            }
                          }}
                        >
                          <button
                            className="thread-item-main"
                            onClick={() => {
                              setContentView('thread');
                              onOpenThread(thread.id);
                            }}
                            tabIndex={-1}
                            type="button"
                          >
                            <div className="thread-row">
                              <span
                                className="thread-title"
                                title={thread.title}
                              >
                                {thread.title}
                              </span>
                              <span className="thread-time">{formatThreadTimestamp(thread.updatedAt)}</span>
                            </div>
                          </button>
                          {row.deleteDisabled ? null : confirmDeleteId === thread.id ? (
                            <button
                              aria-label={t('Confirm delete {name}', { name: thread.title })}
                              className="thread-delete-button confirm"
                              style={{ opacity: 1, pointerEvents: 'auto' }}
                              onClick={(event) => {
                                event.stopPropagation();
                                setConfirmDeleteId(null);
                                onDeleteThread(thread.id);
                              }}
                              tabIndex={-1}
                              type="button"
                            >
                              {t('Confirm')}
                            </button>
                          ) : (
                            <button
                              aria-label={t('Delete {name}', { name: thread.title })}
                              className="thread-delete-button"
                              onClick={(event) => {
                                event.stopPropagation();
                                setConfirmDeleteId(thread.id);
                              }}
                              tabIndex={-1}
                              type="button"
                            >
                              <Trash aria-hidden />
                            </button>
                          )}
                        </div>
                      );
                    })
                  ) : (
                    <p className="workspace-empty-note">{t('No threads yet')}</p>
                  )}
                  {hasPreviewOverflow ? (
                    <div className="workspace-thread-preview-row">
                      <button
                        aria-expanded={isPreviewExpanded}
                        className="workspace-thread-preview-toggle"
                        onClick={() => {
                          setExpandedWorkspacePreviewPaths((current) => {
                            const next = new Set(current);
                            if (next.has(workspacePath)) {
                              next.delete(workspacePath);
                            } else {
                              next.add(workspacePath);
                            }
                            return next;
                          });
                        }}
                        type="button"
                      >
                        {isPreviewExpanded ? t('Show less') : t('Expand')}
                      </button>
                    </div>
                  ) : null}
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
