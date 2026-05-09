import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  MouseSensor,
  TouchSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from '@dnd-kit/core';
import { restrictToVerticalAxis } from '@dnd-kit/modifiers';
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { Trash } from 'lucide-react';
import { useMemo, type CSSProperties } from 'react';

import { useI18n, type Translate } from './i18n';
import type { MessageIntent } from './message-machine';

type QueueDropTarget = {
  intentId: string;
  position: 'before' | 'after';
};

type ComposerQueueProps = {
  activeQueue: MessageIntent[];
  canSteerQueuedPrompt: boolean;
  draggedQueueIntentId: string | null;
  isActiveSendingThread: boolean;
  queueDropTarget: QueueDropTarget | null;
  onCancelIntent: (threadId: string, intentId: string) => void;
  onQueueDropTargetChange: (target: QueueDropTarget | null) => void;
  onReorderQueuedIntent: (
    threadId: string,
    draggedIntentId: string,
    targetIntentId: string,
    position: 'before' | 'after',
  ) => void;
  onSetDraggedQueueIntentId: (intentId: string | null) => void;
  onSteerQueuedPrompt: (item: MessageIntent) => void;
};

const queueDragModifiers = [restrictToVerticalAxis];

function QueueGripIcon() {
  return (
    <svg
      aria-hidden
      className="queue-grip-icon"
      fill="none"
      height="24"
      viewBox="0 0 24 24"
      width="24"
      xmlns="http://www.w3.org/2000/svg"
    >
      <circle cx="9.5" cy="5.5" fill="currentColor" r="1.5" />
      <circle cx="9.5" cy="12" fill="currentColor" r="1.5" />
      <circle cx="9.5" cy="18.5" fill="currentColor" r="1.5" />
      <circle cx="14.5" cy="5.5" fill="currentColor" r="1.5" />
      <circle cx="14.5" cy="12" fill="currentColor" r="1.5" />
      <circle cx="14.5" cy="18.5" fill="currentColor" r="1.5" />
    </svg>
  );
}

function QueueSteerIcon() {
  return (
    <svg
      aria-hidden
      className="queue-steer-icon"
      fill="none"
      height="21"
      viewBox="0 0 21 21"
      width="21"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M13.1293 7.34753C13.3565 7.12027 13.7081 7.09207 13.9662 7.26257L14.0707 7.34753L18.0707 11.3475C18.3304 11.6072 18.3304 12.0292 18.0707 12.2889L14.0707 16.2889C13.811 16.5486 13.389 16.5486 13.1293 16.2889C12.8696 16.0292 12.8696 15.6072 13.1293 15.3475L15.9935 12.4833H6.59998C4.57585 12.4833 2.93494 10.8424 2.93494 8.81824V5.31824C2.93494 4.95097 3.23271 4.6532 3.59998 4.6532C3.96724 4.6532 4.26501 4.95097 4.26501 5.31824V8.81824C4.26501 10.1078 5.31039 11.1532 6.59998 11.1532H15.9935L13.1293 8.28894L13.0443 8.18445C12.8738 7.92632 12.902 7.5748 13.1293 7.34753Z"
        fill="currentColor"
      />
    </svg>
  );
}

function formatAttachmentSummary(imageCount: number, fileCount: number, t: Translate): string {
  const parts: string[] = [];
  if (imageCount > 0) {
    parts.push(imageCount === 1 ? t('1 image') : t('{count} images', { count: imageCount }));
  }
  if (fileCount > 0) {
    parts.push(fileCount === 1 ? t('1 file') : t('{count} files', { count: fileCount }));
  }
  return parts.join(', ');
}

function buildIntentPreview(item: MessageIntent, t: Translate): string {
  const trimmed = item.text.trim();
  const attachmentSummary = formatAttachmentSummary(
    item.images.length,
    item.files.length,
    t,
  );
  if (!trimmed && attachmentSummary) {
    return attachmentSummary;
  }
  if (!trimmed) {
    return t('Queued follow-up');
  }
  if (!attachmentSummary) {
    return trimmed;
  }
  return `${trimmed} (${attachmentSummary})`;
}

function isQueueIntentSteering(item: MessageIntent): boolean {
  return (
    item.dispatchMode === 'async_steer' &&
    ['dispatch_requested', 'dispatching', 'remote_accepted', 'awaiting_history'].includes(item.state)
  );
}

type QueueItemProps = {
  activeQueueLength: number;
  canSteerQueuedPrompt: boolean;
  isActiveSendingThread: boolean;
  item: MessageIntent;
  onCancelIntent: (threadId: string, intentId: string) => void;
  onSteerQueuedPrompt: (item: MessageIntent) => void;
  t: Translate;
};

function SortableQueueItem({
  activeQueueLength,
  canSteerQueuedPrompt,
  isActiveSendingThread,
  item,
  onCancelIntent,
  onSteerQueuedPrompt,
  t,
}: QueueItemProps) {
  const isSteering = isQueueIntentSteering(item);
  const isSortable = !isSteering && activeQueueLength > 1;
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
    id: item.intentId,
  });
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div
      className={`composer-queue-item ${isDragging ? 'dragging' : ''}`}
      data-dragging={isDragging ? 'true' : undefined}
      data-queue-intent-id={item.intentId}
      ref={setNodeRef}
      style={style}
    >
      <div className="composer-queue-primary">
        <button
          className="queue-drag-handle"
          disabled={!isSortable}
          ref={setActivatorNodeRef}
          title={isSortable ? t('Drag to reorder') : t('Queue order locked')}
          type="button"
          {...attributes}
          {...listeners}
          tabIndex={isSortable ? 0 : -1}
        >
          <QueueGripIcon />
          <span className="sr-only">{t('Drag to reorder queued follow-up')}</span>
        </button>
        <span
          className="composer-queue-text"
          title={buildIntentPreview(item, t)}
        >
          {buildIntentPreview(item, t)}
        </span>
      </div>
      <div className="composer-queue-actions">
        {isActiveSendingThread && canSteerQueuedPrompt ? (
          <button
            className="ghost-button queue-steer-button"
            disabled={isSteering}
            onClick={() => {
              onSteerQueuedPrompt(item);
            }}
            type="button"
          >
            <QueueSteerIcon />
            <span>{isSteering ? t('Steering…') : t('Steer')}</span>
          </button>
        ) : null}
        <button
          className="queue-remove-button"
          disabled={isSteering}
          onClick={() => {
            onCancelIntent(item.threadId, item.intentId);
          }}
          tabIndex={-1}
          type="button"
        >
          <Trash aria-hidden />
          <span className="sr-only">{t('Remove queued follow-up')}</span>
        </button>
      </div>
    </div>
  );
}

export function ComposerQueue({
  activeQueue,
  canSteerQueuedPrompt,
  isActiveSendingThread,
  onCancelIntent,
  onQueueDropTargetChange,
  onReorderQueuedIntent,
  onSetDraggedQueueIntentId,
  onSteerQueuedPrompt,
}: ComposerQueueProps) {
  const { t } = useI18n();
  const queueIntentIds = useMemo(
    () => activeQueue.map((item) => item.intentId),
    [activeQueue],
  );
  const sensors = useSensors(
    useSensor(MouseSensor, {
      activationConstraint: {
        distance: 3,
      },
    }),
    useSensor(TouchSensor, {
      activationConstraint: {
        delay: 120,
        tolerance: 5,
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  function clearDragState() {
    onSetDraggedQueueIntentId(null);
    onQueueDropTargetChange(null);
  }

  function handleDragStart(event: DragStartEvent) {
    onSetDraggedQueueIntentId(String(event.active.id));
    onQueueDropTargetChange(null);
  }

  function handleDragCancel() {
    clearDragState();
  }

  function handleDragEnd(event: DragEndEvent) {
    const activeIntentId = String(event.active.id);
    const overIntentId = event.over ? String(event.over.id) : null;
    if (overIntentId && activeIntentId !== overIntentId) {
      const activeIndex = activeQueue.findIndex((item) => item.intentId === activeIntentId);
      const overIndex = activeQueue.findIndex((item) => item.intentId === overIntentId);
      const activeItem = activeQueue[activeIndex];
      if (activeItem && activeIndex >= 0 && overIndex >= 0) {
        onReorderQueuedIntent(
          activeItem.threadId,
          activeIntentId,
          overIntentId,
          activeIndex < overIndex ? 'after' : 'before',
        );
      }
    }
    clearDragState();
  }

  if (!activeQueue.length) {
    return null;
  }

  return (
    <div className="composer-queue">
      <DndContext
        collisionDetection={closestCenter}
        modifiers={queueDragModifiers}
        onDragCancel={handleDragCancel}
        onDragEnd={handleDragEnd}
        onDragStart={handleDragStart}
        sensors={sensors}
      >
        <SortableContext items={queueIntentIds} strategy={verticalListSortingStrategy}>
          <div className="composer-queue-list">
            {activeQueue.map((item) => (
              <SortableQueueItem
                activeQueueLength={activeQueue.length}
                canSteerQueuedPrompt={canSteerQueuedPrompt}
                isActiveSendingThread={isActiveSendingThread}
                item={item}
                key={item.intentId}
                onCancelIntent={onCancelIntent}
                onSteerQueuedPrompt={onSteerQueuedPrompt}
                t={t}
              />
            ))}
          </div>
        </SortableContext>
      </DndContext>
    </div>
  );
}
