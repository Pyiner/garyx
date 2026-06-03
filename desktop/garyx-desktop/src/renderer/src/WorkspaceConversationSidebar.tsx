import type { PointerEvent as ReactPointerEvent } from 'react';
import { useEffect, useRef, useState } from 'react';
import { PanelLeftClose, Trash, Workflow } from 'lucide-react';

import type { DesktopState } from '@shared/contracts';

import { FolderOpenIcon } from './app-shell/icons';
import {
  buildWorkspaceThreadRows,
  type WorkspaceThreadGroup,
} from './thread-model';
import { useI18n } from './i18n';

type WorkspaceConversationSidebarProps = {
  desktopState: DesktopState | null;
  group: WorkspaceThreadGroup;
  selectedThreadId: string | null;
  deletingThreadId: string | null;
  formatThreadTimestamp: (value?: string | null) => string;
  isThreadRuntimeBusy: (threadId: string) => boolean;
  onClose: () => void;
  onDeleteThread: (threadId: string) => void;
  onOpenThread: (threadId: string) => void;
  onRailResizeStart?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  railResizing?: boolean;
};

export function WorkspaceConversationSidebar({
  desktopState,
  group,
  selectedThreadId,
  deletingThreadId,
  formatThreadTimestamp,
  isThreadRuntimeBusy,
  onClose,
  onDeleteThread,
  onOpenThread,
  onRailResizeStart,
  railResizing,
}: WorkspaceConversationSidebarProps) {
  const { t } = useI18n();
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const workspace = group.workspace;
  const rows = buildWorkspaceThreadRows({
    state: desktopState,
    threads: group.threads,
    selectedThreadId,
    deletingThreadId,
    isThreadRuntimeBusy,
  }).filter((row) => !row.isDeleting);

  useEffect(() => {
    if (!confirmDeleteId) {
      return;
    }
    confirmTimerRef.current = setTimeout(() => {
      setConfirmDeleteId(null);
    }, 3000);
    return () => {
      if (confirmTimerRef.current) {
        clearTimeout(confirmTimerRef.current);
      }
    };
  }, [confirmDeleteId]);

  return (
    <aside
      aria-label={t('{name} threads', { name: workspace.name })}
      className="bot-conversation-rail workspace-conversation-rail"
    >
      <div className="bot-conversation-header">
        <div className="bot-conversation-heading">
          <span className="workspace-conversation-logo">
            <FolderOpenIcon />
          </span>
          <div className="bot-conversation-title-copy">
            <div className="bot-conversation-title" title={workspace.path || workspace.name}>
              {workspace.name}
            </div>
          </div>
        </div>
        <button
          aria-label={t('Collapse workspace threads')}
          className="bot-conversation-collapse"
          onClick={onClose}
          title={t('Collapse workspace threads')}
          type="button"
        >
          <PanelLeftClose aria-hidden size={15} strokeWidth={1.8} />
        </button>
      </div>

      <div className="bot-conversation-list">
        {rows.length ? (
          rows.map((row) => {
            const { thread } = row;
            const isWorkflowRun = thread.threadType === 'workflow_run';
            return (
              <div
                className={`bot-conversation-row-shell workspace-conversation-row-shell ${row.isActive ? 'active' : ''} ${row.deleteDisabled ? 'no-delete' : ''}`}
                key={thread.id}
                onMouseLeave={() => {
                  if (confirmDeleteId === thread.id) {
                    setConfirmDeleteId(null);
                  }
                }}
              >
                <button
                  aria-current={row.isActive ? 'page' : undefined}
                  className="bot-conversation-row"
                  onClick={() => {
                    onOpenThread(thread.id);
                  }}
                  type="button"
                >
                  <div className="bot-conversation-row-main">
                    <span className="bot-conversation-row-title" title={thread.title}>
                      {thread.title}
                    </span>
                    {isWorkflowRun ? (
                      <span
                        aria-label={t('Workflow run')}
                        className="workflow-thread-badge"
                        title={t('Workflow run')}
                      >
                        <Workflow aria-hidden size={12} strokeWidth={1.8} />
                      </span>
                    ) : null}
                  </div>
                  <span className="bot-conversation-row-time">
                    {formatThreadTimestamp(thread.updatedAt)}
                  </span>
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
      </div>
      {onRailResizeStart ? (
        <div
          className={`sidebar-resizer ${railResizing ? "is-resizing" : ""}`}
          onPointerDown={onRailResizeStart}
        >
          <div className="sidebar-resizer-line" />
        </div>
      ) : null}
    </aside>
  );
}
