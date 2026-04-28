import { IconGripVertical, IconX } from '@tabler/icons-react';

import type { MessageIntent } from './message-machine';

type QueueDropTarget = {
  intentId: string;
  position: 'before' | 'after';
};

type ComposerQueueProps = {
  activeQueue: MessageIntent[];
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

function formatAttachmentSummary(imageCount: number, fileCount: number): string {
  const parts: string[] = [];
  if (imageCount > 0) {
    parts.push(`${imageCount} image${imageCount === 1 ? '' : 's'}`);
  }
  if (fileCount > 0) {
    parts.push(`${fileCount} file${fileCount === 1 ? '' : 's'}`);
  }
  return parts.join(', ');
}

function buildIntentPreview(item: MessageIntent): string {
  const trimmed = item.text.trim();
  const attachmentSummary = formatAttachmentSummary(
    item.images.length,
    item.files.length,
  );
  if (!trimmed && attachmentSummary) {
    return attachmentSummary;
  }
  if (!trimmed) {
    return 'Queued follow-up';
  }
  if (!attachmentSummary) {
    return trimmed;
  }
  return `${trimmed} (${attachmentSummary})`;
}

export function ComposerQueue({
  activeQueue,
  draggedQueueIntentId,
  isActiveSendingThread,
  queueDropTarget,
  onCancelIntent,
  onQueueDropTargetChange,
  onReorderQueuedIntent,
  onSetDraggedQueueIntentId,
  onSteerQueuedPrompt,
}: ComposerQueueProps) {
  if (!activeQueue.length) {
    return null;
  }

  return (
    <div className="composer-queue">
      <div className="composer-queue-header">
        <div className="composer-queue-copy">
          <IconGripVertical aria-hidden className="composer-queue-summary-icon" size={16} stroke={1.7} />
          <span className="composer-queue-note">
            {activeQueue.length} follow-up{activeQueue.length === 1 ? '' : 's'} ready
          </span>
        </div>
      </div>
      <div className="composer-queue-list">
        {activeQueue.map((item) => {
          const isSteering =
            item.dispatchMode === 'async_steer' &&
            ['dispatch_requested', 'dispatching', 'remote_accepted', 'awaiting_history'].includes(item.state);
          const canSteerNow = true;
          const isDragging = draggedQueueIntentId === item.intentId;
          const dropPosition = queueDropTarget?.intentId === item.intentId
            ? queueDropTarget.position
            : null;
          return (
            <div
              className={`composer-queue-item ${isDragging ? 'dragging' : ''} ${dropPosition ? `drop-${dropPosition}` : ''}`}
              key={item.intentId}
              onDragOver={(event) => {
                const draggedIntentId = draggedQueueIntentId || event.dataTransfer.getData('text/plain');
                if (!draggedIntentId || draggedIntentId === item.intentId) {
                  return;
                }
                event.preventDefault();
                const bounds = event.currentTarget.getBoundingClientRect();
                const position = event.clientY < bounds.top + bounds.height / 2 ? 'before' : 'after';
                if (
                  queueDropTarget?.intentId !== item.intentId ||
                  queueDropTarget.position !== position
                ) {
                  onQueueDropTargetChange({
                    intentId: item.intentId,
                    position,
                  });
                }
                event.dataTransfer.dropEffect = 'move';
              }}
              onDrop={(event) => {
                event.preventDefault();
                const draggedIntentId = draggedQueueIntentId || event.dataTransfer.getData('text/plain');
                const bounds = event.currentTarget.getBoundingClientRect();
                const position = event.clientY < bounds.top + bounds.height / 2 ? 'before' : 'after';
                if (draggedIntentId && draggedIntentId !== item.intentId) {
                  onReorderQueuedIntent(item.threadId, draggedIntentId, item.intentId, position);
                }
                onSetDraggedQueueIntentId(null);
                onQueueDropTargetChange(null);
              }}
            >
              <button
                className="queue-drag-handle"
                disabled={isSteering || activeQueue.length < 2}
                draggable={!isSteering && activeQueue.length > 1}
                onDragEnd={() => {
                  onSetDraggedQueueIntentId(null);
                  onQueueDropTargetChange(null);
                }}
                onDragStart={(event) => {
                  onSetDraggedQueueIntentId(item.intentId);
                  onQueueDropTargetChange(null);
                  event.dataTransfer.effectAllowed = 'move';
                  event.dataTransfer.setData('text/plain', item.intentId);
                  const row = event.currentTarget.closest('.composer-queue-item');
                  if (row instanceof HTMLElement) {
                    event.dataTransfer.setDragImage(row, 28, 18);
                  }
                }}
                title={activeQueue.length > 1 ? 'Drag to reorder' : 'Queue order locked'}
                tabIndex={-1}
                type="button"
              >
                <IconGripVertical aria-hidden size={16} stroke={1.7} />
                <span className="sr-only">Drag to reorder queued follow-up</span>
              </button>
              <span
                className="composer-queue-text"
                title={buildIntentPreview(item)}
              >
                {buildIntentPreview(item)}
              </span>
              {isActiveSendingThread && canSteerNow ? (
                <button
                  className="ghost-button queue-steer-button"
                  disabled={isSteering}
                  onClick={() => {
                    onSteerQueuedPrompt(item);
                  }}
                  type="button"
                >
                  <span>{isSteering ? 'Steering…' : 'Steer'}</span>
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
                <IconX aria-hidden size={16} stroke={1.7} />
                <span className="sr-only">Remove queued follow-up</span>
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
