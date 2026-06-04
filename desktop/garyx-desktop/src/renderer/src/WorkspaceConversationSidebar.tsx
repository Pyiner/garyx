import type { PointerEvent as ReactPointerEvent } from 'react';

import type { DesktopState } from '@shared/contracts';

import { FolderOpenIcon } from './app-shell/icons';
import {
  buildWorkspaceThreadRows,
  type WorkspaceThreadGroup,
} from './thread-model';
import { ThreadConversationSidebar } from './ThreadConversationSidebar';
import { useI18n } from './i18n';

type WorkspaceConversationSidebarProps = {
  desktopState: DesktopState | null;
  group: WorkspaceThreadGroup;
  selectedThreadId: string | null;
  deletingThreadId: string | null;
  formatThreadTimestamp: (value?: string | null) => string;
  isThreadRuntimeBusy: (threadId: string) => boolean;
  onClose: () => void;
  onArchiveThread: (threadId: string) => void;
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
  onArchiveThread,
  onOpenThread,
  onRailResizeStart,
  railResizing,
}: WorkspaceConversationSidebarProps) {
  const { t } = useI18n();
  const workspace = group.workspace;
  const rows = buildWorkspaceThreadRows({
    state: desktopState,
    threads: group.threads,
    selectedThreadId,
    deletingThreadId,
    isThreadRuntimeBusy,
  })
    .filter((row) => !row.isDeleting)
    .map((row) => ({
      key: row.thread.id,
      title: row.thread.title,
      time: row.thread.updatedAt,
      isActive: row.isActive,
      onOpen: () => onOpenThread(row.thread.id),
      onArchive: row.isBusy ? undefined : () => onArchiveThread(row.thread.id),
    }));

  return (
    <ThreadConversationSidebar
      ariaLabel={t('{name} threads', { name: workspace.name })}
      className="workspace-conversation-rail"
      collapseLabel={t('Collapse workspace threads')}
      emptyLabel={t('No threads yet')}
      formatThreadTimestamp={formatThreadTimestamp}
      logo={
        <span className="workspace-conversation-logo">
          <FolderOpenIcon />
        </span>
      }
      onClose={onClose}
      onRailResizeStart={onRailResizeStart}
      railResizing={railResizing}
      rowClassName="workspace-conversation-row-shell"
      rows={rows}
      title={workspace.name}
      titleTooltip={workspace.path || workspace.name}
    />
  );
}
