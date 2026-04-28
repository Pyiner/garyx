import { type CSSProperties, type KeyboardEvent, useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { NewTabIcon } from './app-shell/icons';

import { ChevronDownIcon, DeleteIcon, FolderIcon, FolderOpenIcon, MoreDotsIcon, NewFolderIcon, RenameIcon } from './app-shell/icons';

import type { DesktopState, DesktopWorkspace } from '@shared/contracts';

import {
  buildWorkspaceThreadRows,
  type WorkspaceThreadGroup,
} from './thread-model';

type WorkspaceMutation = 'assign' | 'add' | 'relink' | 'remove' | null;

type WorkspaceThreadSidebarProps = {
  desktopState: DesktopState | null;
  workspaceThreadGroups: WorkspaceThreadGroup[];
  selectedThreadId: string | null;
  deletingThreadId: string | null;
  workspaceMutation: WorkspaceMutation;
  workspaceMenuOpenId: string | null;
  renamingWorkspaceId: string | null;
  workspaceNameDraft: string;
  setWorkspaceMenuOpenId: (value: string | ((current: string | null) => string | null) | null) => void;
  setWorkspaceNameDraft: (value: string) => void;
  setContentView: (view: 'thread') => void;
  isThreadRuntimeBusy: (threadId: string) => boolean;
  formatThreadTimestamp: (value?: string | null) => string;
  onOpenFolder: () => void;
  onOpenThread: (threadId: string) => void;
  onSelectWorkspace: (workspaceId: string, preferredThreadId: string | null) => void;
  onCreateThreadForWorkspace: (workspaceId: string) => void;
  onBeginRenameWorkspace: (workspace: DesktopWorkspace) => void;
  onSubmitRenameWorkspace: (workspaceId: string) => void;
  onCancelRenameWorkspace: () => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  onDeleteThread: (threadId: string) => void;
};

export function WorkspaceThreadSidebar({
  desktopState,
  workspaceThreadGroups,
  selectedThreadId,
  deletingThreadId,
  workspaceMutation,
  workspaceMenuOpenId,
  renamingWorkspaceId,
  workspaceNameDraft,
  setWorkspaceMenuOpenId,
  setWorkspaceNameDraft,
  setContentView,
  isThreadRuntimeBusy,
  formatThreadTimestamp,
  onOpenFolder,
  onOpenThread,
  onSelectWorkspace,
  onCreateThreadForWorkspace,
  onBeginRenameWorkspace,
  onSubmitRenameWorkspace,
  onCancelRenameWorkspace,
  onRequestRemoveWorkspace,
  onDeleteThread,
}: WorkspaceThreadSidebarProps) {
  const [sectionCollapsed, setSectionCollapsed] = useState(false);
  const [collapsedIds, setCollapsedIds] = useState<Set<string>>(new Set());
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [confirmRemoveWorkspaceId, setConfirmRemoveWorkspaceId] = useState<string | null>(null);
  const [expandedWorkspacePreviewIds, setExpandedWorkspacePreviewIds] = useState<Set<string>>(new Set());
  const [workspaceMenuStyle, setWorkspaceMenuStyle] = useState<CSSProperties | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const menuButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({});

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

  const updateWorkspaceMenuPosition = useCallback((workspaceId: string | null) => {
    if (!workspaceId) {
      setWorkspaceMenuStyle(null);
      return;
    }

    const button = menuButtonRefs.current[workspaceId];
    if (!button) {
      setWorkspaceMenuStyle(null);
      return;
    }

    const rect = button.getBoundingClientRect();
    const viewportPadding = 12;
    const menuWidth = confirmRemoveWorkspaceId === workspaceId ? 228 : 196;
    const estimatedHeight = confirmRemoveWorkspaceId === workspaceId ? 154 : 104;
    const gap = 6;
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
  }, [confirmRemoveWorkspaceId]);

  useEffect(() => {
    if (!workspaceMenuOpenId) {
      setWorkspaceMenuStyle(null);
      setConfirmRemoveWorkspaceId(null);
      return;
    }

    const update = () => {
      updateWorkspaceMenuPosition(workspaceMenuOpenId);
    };

    update();
    window.addEventListener('resize', update);
    window.addEventListener('scroll', update, true);
    return () => {
      window.removeEventListener('resize', update);
      window.removeEventListener('scroll', update, true);
    };
  }, [updateWorkspaceMenuPosition, workspaceMenuOpenId]);

  useEffect(() => {
    if (!workspaceMenuOpenId && confirmRemoveWorkspaceId) {
      setConfirmRemoveWorkspaceId(null);
      return;
    }
    if (
      confirmRemoveWorkspaceId
      && workspaceMenuOpenId
      && confirmRemoveWorkspaceId !== workspaceMenuOpenId
    ) {
      setConfirmRemoveWorkspaceId(null);
    }
  }, [confirmRemoveWorkspaceId, workspaceMenuOpenId]);

  const handleWorkspaceClick = useCallback(
    (workspaceId: string, preferredThreadId: string | null) => {
      // If clicking the same workspace that already has a selected thread, toggle collapse
      const hasSelectedThread = workspaceThreadGroups.some(
        (g) => g.workspace.id === workspaceId && g.threads.some((t) => t.id === selectedThreadId),
      );
      if (hasSelectedThread) {
        setCollapsedIds((prev) => {
          const next = new Set(prev);
          if (next.has(workspaceId)) {
            next.delete(workspaceId);
          } else {
            next.add(workspaceId);
          }
          return next;
        });
        return;
      }
      // Expanding a different workspace — uncollapse it and select
      setCollapsedIds((prev) => {
        if (!prev.has(workspaceId)) return prev;
        const next = new Set(prev);
        next.delete(workspaceId);
        return next;
      });
      onSelectWorkspace(workspaceId, preferredThreadId);
    },
    [onSelectWorkspace, selectedThreadId, workspaceThreadGroups],
  );

  return (
    <div className="sidebar-thread-block">
      <div className="panel-header sidebar-section-header sidebar-section-header-interactive">
        <button
          aria-expanded={!sectionCollapsed}
          aria-label={sectionCollapsed ? 'Expand threads' : 'Collapse threads'}
          className="sidebar-section-toggle"
          onClick={() => setSectionCollapsed((c) => !c)}
          type="button"
        >
          <span className="sidebar-section-title">Threads</span>
          <ChevronDownIcon
            size={16}
            className={`icon sidebar-section-chevron ${sectionCollapsed ? 'collapsed' : ''}`}
          />
        </button>
        <button
          aria-label={workspaceMutation === 'add' ? 'Opening folder' : 'New folder'}
          className="sidebar-section-action"
          disabled={workspaceMutation === 'add'}
          onClick={onOpenFolder}
          title={workspaceMutation === 'add' ? 'Opening folder…' : 'New folder'}
          type="button"
        >
          <NewFolderIcon />
        </button>
      </div>

      {!sectionCollapsed ? <div className="workspace-list">
        {workspaceThreadGroups.map((group) => {
          const { workspace } = group;
          const isMenuOpen = workspaceMenuOpenId === workspace.id;
          const isRemoveConfirming = confirmRemoveWorkspaceId === workspace.id;
          const isRenaming = renamingWorkspaceId === workspace.id;
          const isPreviewExpanded = expandedWorkspacePreviewIds.has(workspace.id);
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
          const hiddenThreadCount = Math.max(visibleRows.length - 3, 0);

          const handleRenameInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              onSubmitRenameWorkspace(workspace.id);
            } else if (event.key === 'Escape') {
              event.preventDefault();
              onCancelRenameWorkspace();
            }
          };

          return (
            <section
              className={`workspace-group ${!workspace.available ? 'missing' : ''}`}
              key={workspace.id}
            >
              <div className="workspace-row workspace-row-shell">
                <button
                  className="workspace-row-main"
                  onClick={() => handleWorkspaceClick(workspace.id, group.preferredThreadId)}
                  tabIndex={-1}
                  type="button"
                >
                  <div className="workspace-row-copy">
                    <span className="workspace-folder-icon">
                      <span className="workspace-folder-icon-default">
                        {collapsedIds.has(workspace.id) ? <FolderIcon /> : <FolderOpenIcon />}
                      </span>
                      <span className="workspace-folder-icon-hover">
                        {collapsedIds.has(workspace.id) ? <FolderOpenIcon /> : <FolderIcon />}
                      </span>
                    </span>
                    {isRenaming ? (
                      <input
                        aria-label={`Rename ${workspace.name}`}
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
                      {group.status}
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
                          onSubmitRenameWorkspace(workspace.id);
                        }}
                        tabIndex={-1}
                        type="button"
                      >
                        Save
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
                        Cancel
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        aria-label={`Create thread in ${workspace.name}`}
                        className="workspace-action-icon-button"
                        disabled={!workspace.available || workspaceMutation === 'assign'}
                        onClick={(event) => {
                          event.stopPropagation();
                          onCreateThreadForWorkspace(workspace.id);
                        }}
                        tabIndex={-1}
                        title={
                          workspaceMutation === 'assign'
                            ? 'Creating thread…'
                            : `Create thread in ${workspace.name}`
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
                            aria-label={`More actions for ${workspace.name}`}
                            className="workspace-action-icon-button"
                            ref={(node) => {
                              menuButtonRefs.current[workspace.id] = node;
                            }}
                            onClick={(event) => {
                              event.stopPropagation();
                              setWorkspaceMenuOpenId((current) => {
                                return current === workspace.id ? null : workspace.id;
                              });
                            }}
                            tabIndex={-1}
                            title={`More actions for ${workspace.name}`}
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
                                    minWidth: '196px',
                                    maxHeight: 'min(280px, calc(100vh - 24px))',
                                    overflowY: 'auto',
                                  }}
                                >
                                  <button
                                    className="workspace-menu-item"
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      setConfirmRemoveWorkspaceId(null);
                                      onBeginRenameWorkspace(workspace);
                                    }}
                                    role="menuitem"
                                    type="button"
                                  >
                                    <RenameIcon />
                                    Rename Workspace
                                  </button>
                                  {isRemoveConfirming ? (
                                    <div className="workspace-menu-confirm" role="group" aria-label={`Confirm removal of ${workspace.name}`}>
                                      <div className="workspace-menu-confirm-copy">
                                        <span className="workspace-menu-confirm-title">Remove from Desktop?</span>
                                        <p>
                                          This only hides the workspace from Garyx. Threads stay intact.
                                        </p>
                                      </div>
                                      <div className="workspace-menu-confirm-actions">
                                        <button
                                          className="workspace-menu-confirm-button"
                                          onClick={(event) => {
                                            event.stopPropagation();
                                            setConfirmRemoveWorkspaceId(null);
                                          }}
                                          type="button"
                                        >
                                          Cancel
                                        </button>
                                        <button
                                          className="workspace-menu-confirm-button danger"
                                          onClick={(event) => {
                                            event.stopPropagation();
                                            setConfirmRemoveWorkspaceId(null);
                                            onRequestRemoveWorkspace(workspace);
                                          }}
                                          type="button"
                                        >
                                          Remove
                                        </button>
                                      </div>
                                    </div>
                                  ) : (
                                    <button
                                      className="workspace-menu-item workspace-menu-item-danger"
                                      onClick={(event) => {
                                        event.stopPropagation();
                                        setConfirmRemoveWorkspaceId(workspace.id);
                                      }}
                                      role="menuitem"
                                      title="Remove this workspace from Garyx"
                                      type="button"
                                    >
                                      <DeleteIcon />
                                      Remove from Desktop…
                                    </button>
                                  )}
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

              <div className={`thread-list workspace-thread-list ${collapsedIds.has(workspace.id) ? 'collapsed' : ''}`}>
                {visibleRows.length && !collapsedIds.has(workspace.id) ? (
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
                            aria-label={`Confirm delete ${thread.title}`}
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
                            Confirm
                          </button>
                        ) : (
                          <button
                            aria-label={`Delete ${thread.title}`}
                            className="thread-delete-button"
                            onClick={(event) => {
                              event.stopPropagation();
                              setConfirmDeleteId(thread.id);
                            }}
                            tabIndex={-1}
                            type="button"
                          >
                          <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
                            <path d="M11.8008 10.1816C12.1035 10.2438 12.3309 10.5119 12.3311 10.833C12.3311 11.1542 12.1036 11.4222 11.8008 11.4844L11.666 11.498H8.33301C7.96589 11.4979 7.66797 11.2002 7.66797 10.833C7.66814 10.466 7.966 10.1682 8.33301 10.168H11.666L11.8008 10.1816Z" fill="currentColor"/>
                            <path fillRule="evenodd" clipRule="evenodd" d="M15.417 2.66797C16.7045 2.66815 17.7489 3.71251 17.749 5V5.83301C17.749 6.33171 17.59 6.79271 17.3232 7.17188C17.3263 7.19763 17.3311 7.22343 17.3311 7.25V12.667C17.3311 13.3559 17.3317 13.9131 17.2949 14.3633C17.2622 14.7639 17.197 15.1246 17.0527 15.4609L16.9863 15.6035C16.7209 16.1245 16.3169 16.5602 15.8213 16.8643L15.6035 16.9863C15.2268 17.1782 14.8202 17.2575 14.3623 17.2949C13.9121 17.3317 13.3549 17.332 12.666 17.332H7.33301C6.64407 17.332 6.08689 17.3317 5.63672 17.2949C5.23627 17.2622 4.87521 17.1979 4.53906 17.0537L4.39648 16.9863C3.8754 16.7208 3.43882 16.3171 3.13477 15.8213L3.0127 15.6035C2.82089 15.227 2.74153 14.821 2.7041 14.3633C2.66732 13.9131 2.66797 13.3559 2.66797 12.667V7.25C2.66797 7.22312 2.67268 7.19694 2.67578 7.1709C2.4096 6.79197 2.25195 6.33115 2.25195 5.83301V5C2.25212 3.7124 3.29634 2.66797 4.58398 2.66797H15.417ZM16.001 8.08789C15.8141 8.13621 15.619 8.16501 15.417 8.16504H4.58398C4.38146 8.16504 4.18541 8.13644 3.99805 8.08789V12.667C3.99805 13.3778 3.99895 13.8714 4.03027 14.2549C4.06097 14.6303 4.11779 14.8421 4.19824 15L4.26855 15.126C4.44482 15.4134 4.69792 15.6478 5 15.8018L5.12988 15.8574C5.27361 15.9089 5.4633 15.9467 5.74512 15.9697C6.12858 16.0011 6.62215 16.002 7.33301 16.002H12.666C13.3767 16.002 13.8705 16.001 14.2539 15.9697C14.6292 15.9391 14.8411 15.8821 14.999 15.8018L15.126 15.7305C15.4132 15.5542 15.6479 15.3019 15.8018 15L15.8574 14.8691C15.9088 14.7255 15.9467 14.5363 15.9697 14.2549C16.0011 13.8714 16.001 13.3779 16.001 12.667V8.08789ZM4.58398 3.99805C4.03088 3.99805 3.5822 4.44693 3.58203 5V5.83301C3.58203 6.38621 4.03078 6.83496 4.58398 6.83496H15.417C15.97 6.83478 16.4189 6.3861 16.4189 5.83301V5C16.4188 4.44705 15.9699 3.99823 15.417 3.99805H4.58398Z" fill="currentColor"/>
                          </svg>
                        </button>
                        )}
                      </div>
                    );
                  })
                ) : collapsedIds.has(workspace.id) ? null : (
                  <p className="workspace-empty-note">No threads yet</p>
                )}
                {!collapsedIds.has(workspace.id) && hasPreviewOverflow ? (
                  <div className="workspace-thread-preview-row">
                    <button
                      aria-expanded={isPreviewExpanded}
                      className="workspace-thread-preview-toggle"
                      onClick={() => {
                        setExpandedWorkspacePreviewIds((current) => {
                          const next = new Set(current);
                          if (next.has(workspace.id)) {
                            next.delete(workspace.id);
                          } else {
                            next.add(workspace.id);
                          }
                          return next;
                        });
                      }}
                      type="button"
                    >
                      {isPreviewExpanded ? 'Show less' : `See ${hiddenThreadCount} more`}
                    </button>
                  </div>
                ) : null}
              </div>
            </section>
          );
        })}

        {!workspaceThreadGroups.length ? (
          <div className="workspace-empty-block">
            <span className="eyebrow">No Workspaces</span>
            <p>Add a folder to start grouping Garyx threads by workspace.</p>
          </div>
        ) : null}
      </div> : null}
    </div>
  );
}
