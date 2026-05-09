import { useI18n, type Translate } from './i18n';
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

function QueueTrashIcon() {
  return (
    <svg
      aria-hidden
      className="queue-remove-icon"
      fill="currentColor"
      height="20"
      viewBox="0 0 20 20"
      width="20"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M10.6299 1.33496C12.0335 1.33496 13.2695 2.25996 13.666 3.60645L13.8809 4.33496H17L17.1338 4.34863C17.4369 4.41057 17.665 4.67858 17.665 5C17.665 5.32142 17.4369 5.58943 17.1338 5.65137L17 5.66504H16.6543L15.8574 14.9912C15.7177 16.629 14.3478 17.8877 12.7041 17.8877H7.2959C5.75502 17.8877 4.45439 16.7815 4.18262 15.2939L4.14258 14.9912L3.34668 5.66504H3C2.63273 5.66504 2.33496 5.36727 2.33496 5C2.33496 4.63273 2.63273 4.33496 3 4.33496H6.11914L6.33398 3.60645L6.41797 3.3584C6.88565 2.14747 8.05427 1.33496 9.37012 1.33496H10.6299ZM5.46777 14.8779L5.49121 15.0537C5.64881 15.9161 6.40256 16.5576 7.2959 16.5576H12.7041C13.6571 16.5576 14.4512 15.8275 14.5322 14.8779L15.3193 5.66504H4.68164L5.46777 14.8779ZM7.66797 12.8271V8.66016C7.66797 8.29299 7.96588 7.99528 8.33301 7.99512C8.70028 7.99512 8.99805 8.29289 8.99805 8.66016V12.8271C8.99779 13.1942 8.70012 13.4912 8.33301 13.4912C7.96604 13.491 7.66823 13.1941 7.66797 12.8271ZM11.002 12.8271V8.66016C11.002 8.29289 11.2997 7.99512 11.667 7.99512C12.0341 7.9953 12.332 8.293 12.332 8.66016V12.8271C12.3318 13.1941 12.0339 13.491 11.667 13.4912C11.2999 13.4912 11.0022 13.1942 11.002 12.8271ZM9.37012 2.66504C8.60726 2.66504 7.92938 3.13589 7.6582 3.83789L7.60938 3.98145L7.50586 4.33496H12.4941L12.3906 3.98145C12.1607 3.20084 11.4437 2.66504 10.6299 2.66504H9.37012Z" />
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
  const { t } = useI18n();

  if (!activeQueue.length) {
    return null;
  }

  return (
    <div className="composer-queue">
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
              <div className="composer-queue-primary">
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
                      event.dataTransfer.setDragImage(row, 20, 14);
                    }
                  }}
                  title={activeQueue.length > 1 ? t('Drag to reorder') : t('Queue order locked')}
                  tabIndex={-1}
                  type="button"
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
                {isActiveSendingThread && canSteerNow ? (
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
                <QueueTrashIcon />
                <span className="sr-only">{t('Remove queued follow-up')}</span>
              </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
