import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  MouseSensor,
  TouchSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from '@dnd-kit/core';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { Archive } from 'lucide-react';

import type { DesktopThreadSummary } from '@shared/contracts';

import { AgentOptionAvatar } from './app-shell/components/AgentOptionAvatar';
import { PinIcon } from './app-shell/icons';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './components/ui/tooltip';
import { useI18n } from './i18n';
import {
  reorderPinnedThreadIds,
  shouldCancelDanglingDrag,
} from './pinned-thread-reorder';
import type { ThreadAvatarIdentity } from './thread-avatar';

export type PinnedThreadRow = {
  thread: DesktopThreadSummary;
  isActive: boolean;
  isBusy: boolean;
  avatar: ThreadAvatarIdentity;
};

type PinnedThreadsSidebarProps = {
  rows: PinnedThreadRow[];
  syncPending: boolean;
  formatThreadTimestamp: (value?: string | null) => string;
  onOpenThread: (threadId: string) => void;
  onUnpinThread: (threadId: string) => void;
  onArchiveThread: (threadId: string) => void;
  onDragStart: () => void;
  onDragCancel: () => void;
  onReorderThreads: (threadIds: string[]) => void;
};

const pinnedDragModifiers = [restrictToVerticalAxis];

type SortablePinnedThreadRowProps = PinnedThreadRow & {
  confirmThreadId: string | null;
  formatThreadTimestamp: (value?: string | null) => string;
  rowCount: number;
  setConfirmThreadId: (threadId: string | null) => void;
  onOpenThread: (threadId: string) => void;
  onUnpinThread: (threadId: string) => void;
  onArchiveThread: (threadId: string) => void;
};

function SortablePinnedThreadRow({
  thread,
  isActive,
  isBusy,
  avatar,
  confirmThreadId,
  formatThreadTimestamp,
  rowCount,
  setConfirmThreadId,
  onOpenThread,
  onUnpinThread,
  onArchiveThread,
}: SortablePinnedThreadRowProps) {
  const { t } = useI18n();
  const isSortable = rowCount > 1;
  const {
    attributes,
    isDragging,
    listeners,
    setActivatorNodeRef,
    setNodeRef,
    transform,
    transition,
  } = useSortable({
    disabled: !isSortable,
    id: thread.id,
  });
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };
  const timeLabel = formatThreadTimestamp(thread.updatedAt);
  const isConfirming = confirmThreadId === thread.id;

  return (
    <div
      className={`pinned-thread-row-shell ${isActive ? 'active' : ''} ${isDragging ? 'dragging' : ''}`.trim()}
      data-dragging={isDragging ? 'true' : undefined}
      onMouseLeave={() => {
        if (confirmThreadId === thread.id) {
          setConfirmThreadId(null);
        }
      }}
      ref={setNodeRef}
      style={style}
    >
      <button
        aria-current={isActive ? 'page' : undefined}
        className={`pinned-thread-row ${isSortable ? 'sortable' : ''}`.trim()}
        onClick={() => {
          onOpenThread(thread.id);
        }}
        ref={setActivatorNodeRef}
        title={thread.title}
        type="button"
        {...attributes}
        {...listeners}
      >
        <span className="thread-row-avatar-wrap pinned-thread-avatar-wrap">
          <AgentOptionAvatar
            agentId={avatar.agentId}
            avatarDataUrl={avatar.avatarDataUrl}
            className="thread-row-agent-avatar"
            kind={avatar.kind}
            label={avatar.label}
            providerIcon={avatar.providerIcon}
            providerType={avatar.providerType}
            size="default"
          />
          {isBusy ? (
            <span aria-label={t('Loading')} className="thread-row-typing-badge" role="status">
              <span />
              <span />
              <span />
            </span>
          ) : null}
        </span>
        <span className="pinned-thread-title">{thread.title}</span>
        <span className="pinned-thread-time">{timeLabel}</span>
      </button>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            aria-label={t('Unpin {title}', { title: thread.title })}
            className="pinned-thread-unpin"
            onClick={(event) => {
              event.stopPropagation();
              onUnpinThread(thread.id);
            }}
            type="button"
          >
            <PinIcon size={15} />
          </button>
        </TooltipTrigger>
        <TooltipContent>{t('Unpin thread')}</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            aria-label={
              isConfirming
                ? t('Confirm archive {name}', { name: thread.title })
                : t('Archive {title}', { title: thread.title })
            }
            className={`pinned-thread-archive ${isConfirming ? 'confirm thread-delete-button' : ''}`.trim()}
            disabled={isBusy}
            onClick={(event) => {
              event.stopPropagation();
              if (!isConfirming) {
                setConfirmThreadId(thread.id);
                return;
              }
              setConfirmThreadId(null);
              onArchiveThread(thread.id);
            }}
            style={isConfirming ? { opacity: 1, pointerEvents: 'auto' } : undefined}
            type="button"
          >
            {isConfirming ? t('Confirm') : <Archive aria-hidden size={13} strokeWidth={1.55} />}
          </button>
        </TooltipTrigger>
        <TooltipContent>{t('Archive thread')}</TooltipContent>
      </Tooltip>
    </div>
  );
}

export function PinnedThreadsSidebar({
  rows,
  syncPending,
  formatThreadTimestamp,
  onOpenThread,
  onUnpinThread,
  onArchiveThread,
  onDragStart,
  onDragCancel,
  onReorderThreads,
}: PinnedThreadsSidebarProps) {
  const { t } = useI18n();
  const [confirmThreadId, setConfirmThreadId] = useState<string | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const rowIds = useMemo(() => rows.map(({ thread }) => thread.id), [rows]);
  // DndContext fires no onDragCancel when its subtree disappears mid-drag.
  // The reachable path is the rows projection emptying (remote unpin during
  // a drag): this component then renders null while STAYING MOUNTED — the
  // parent renders it unconditionally and the 720px collapse is CSS-only —
  // so the cancel must key off rows change; an unmount-only cleanup never
  // fires there (review #TASK-2312 P2). The unmount cleanup below remains as
  // defense for true unmounts.
  const dragActiveRef = useRef(false);
  const onDragCancelRef = useRef(onDragCancel);
  onDragCancelRef.current = onDragCancel;
  useEffect(() => {
    if (shouldCancelDanglingDrag(rows.length, dragActiveRef.current)) {
      dragActiveRef.current = false;
      onDragCancelRef.current();
    }
  }, [rows.length]);
  useEffect(() => {
    return () => {
      if (dragActiveRef.current) {
        dragActiveRef.current = false;
        onDragCancelRef.current();
      }
    };
  }, []);
  const sensors = useSensors(
    useSensor(MouseSensor, {
      activationConstraint: { distance: 3 },
    }),
    useSensor(TouchSensor, {
      activationConstraint: { delay: 120, tolerance: 5 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  useEffect(() => {
    if (!confirmThreadId) {
      return;
    }
    confirmTimerRef.current = setTimeout(() => {
      setConfirmThreadId(null);
    }, 3000);
    return () => {
      if (confirmTimerRef.current) {
        clearTimeout(confirmTimerRef.current);
      }
    };
  }, [confirmThreadId]);

  if (!rows.length) {
    return null;
  }

  function handleDragStart() {
    dragActiveRef.current = true;
    onDragStart();
  }

  function handleDragCancel() {
    dragActiveRef.current = false;
    onDragCancel();
  }

  function handleDragEnd(event: DragEndEvent) {
    dragActiveRef.current = false;
    const nextOrder = reorderPinnedThreadIds(
      rowIds,
      String(event.active.id),
      event.over ? String(event.over.id) : null,
    );
    if (!nextOrder) {
      onDragCancel();
      return;
    }
    onReorderThreads(nextOrder);
  }

  return (
    <TooltipProvider>
    <div className="sidebar-thread-block pinned-thread-block">
      <div className="panel-header sidebar-section-header pinned-thread-header">
        <span className="sidebar-section-title">{t('Pinned')}</span>
        {syncPending ? (
          <span
            aria-label={t('Pinned order pending sync')}
            className="pinned-thread-sync-pending"
            role="status"
            title={t('Pinned order pending sync')}
          />
        ) : null}
      </div>

      <DndContext
        collisionDetection={closestCenter}
        modifiers={pinnedDragModifiers}
        onDragCancel={handleDragCancel}
        onDragEnd={handleDragEnd}
        onDragStart={handleDragStart}
        sensors={sensors}
      >
        <SortableContext items={rowIds} strategy={verticalListSortingStrategy}>
          <div className="pinned-thread-list">
            {rows.map((row) => (
              <SortablePinnedThreadRow
                {...row}
                confirmThreadId={confirmThreadId}
                formatThreadTimestamp={formatThreadTimestamp}
                key={row.thread.id}
                onArchiveThread={onArchiveThread}
                onOpenThread={onOpenThread}
                onUnpinThread={onUnpinThread}
                rowCount={rows.length}
                setConfirmThreadId={setConfirmThreadId}
              />
            ))}
          </div>
        </SortableContext>
      </DndContext>
    </div>
    </TooltipProvider>
  );
}
