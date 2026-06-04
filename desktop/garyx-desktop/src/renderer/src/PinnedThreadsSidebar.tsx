import { IconPin } from '@tabler/icons-react';
import { Archive } from 'lucide-react';

import type { DesktopThreadSummary } from '@shared/contracts';

import { useI18n } from './i18n';

export type PinnedThreadRow = {
  thread: DesktopThreadSummary;
  isActive: boolean;
  isBusy: boolean;
};

type PinnedThreadsSidebarProps = {
  rows: PinnedThreadRow[];
  formatThreadTimestamp: (value?: string | null) => string;
  onOpenThread: (threadId: string) => void;
  onUnpinThread: (threadId: string) => void;
  onArchiveThread: (threadId: string) => void;
};

export function PinnedThreadsSidebar({
  rows,
  formatThreadTimestamp,
  onOpenThread,
  onUnpinThread,
  onArchiveThread,
}: PinnedThreadsSidebarProps) {
  const { t } = useI18n();

  if (!rows.length) {
    return null;
  }

  return (
    <div className="sidebar-thread-block pinned-thread-block">
      <div className="panel-header sidebar-section-header pinned-thread-header">
        <span className="sidebar-section-title">{t('Pinned')}</span>
      </div>

      <div className="pinned-thread-list">
        {rows.map(({ thread, isActive, isBusy }) => {
          const timeLabel = formatThreadTimestamp(thread.updatedAt);
          return (
            <div
              className={`pinned-thread-row-shell ${isActive ? 'active' : ''}`}
              key={thread.id}
            >
              <button
                aria-label={t('Unpin {title}', { title: thread.title })}
                className="pinned-thread-unpin"
                onClick={(event) => {
                  event.stopPropagation();
                  onUnpinThread(thread.id);
                }}
                title={t('Unpin thread')}
                type="button"
              >
                <IconPin aria-hidden className="pinned-thread-icon" size={16} stroke={1.55} />
              </button>
              <button
                aria-current={isActive ? 'page' : undefined}
                className="pinned-thread-row"
                onClick={() => {
                  onOpenThread(thread.id);
                }}
                title={thread.title}
                type="button"
              >
                <span className="pinned-thread-title">{thread.title}</span>
                {isBusy ? (
                  <span aria-label={t('Loading')} className="pinned-thread-spinner" />
                ) : (
                  <span className="pinned-thread-time">{timeLabel}</span>
                )}
              </button>
              <button
                aria-label={t('Archive {title}', { title: thread.title })}
                className="pinned-thread-archive"
                onClick={(event) => {
                  event.stopPropagation();
                  onArchiveThread(thread.id);
                }}
                title={t('Archive thread')}
                type="button"
              >
                <Archive aria-hidden size={13} strokeWidth={1.55} />
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
