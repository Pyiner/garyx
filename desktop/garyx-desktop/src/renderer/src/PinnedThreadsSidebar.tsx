import { useEffect, useRef, useState } from 'react';
import { IconPin } from '@tabler/icons-react';
import { Archive } from 'lucide-react';

import type { DesktopThreadSummary } from '@shared/contracts';

import { AgentOptionAvatar } from './app-shell/components/AgentOptionAvatar';
import { useI18n } from './i18n';
import type { ThreadAvatarIdentity } from './thread-avatar';

export type PinnedThreadRow = {
  thread: DesktopThreadSummary;
  isActive: boolean;
  isBusy: boolean;
  avatar: ThreadAvatarIdentity;
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
  const [confirmThreadId, setConfirmThreadId] = useState<string | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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

  return (
    <div className="sidebar-thread-block pinned-thread-block">
      <div className="panel-header sidebar-section-header pinned-thread-header">
        <span className="sidebar-section-title">{t('Pinned')}</span>
      </div>

      <div className="pinned-thread-list">
        {rows.map(({ thread, isActive, isBusy, avatar }) => {
          const timeLabel = formatThreadTimestamp(thread.updatedAt);
          const isConfirming = confirmThreadId === thread.id;
          return (
            <div
              className={`pinned-thread-row-shell ${isActive ? 'active' : ''}`}
              key={thread.id}
              onMouseLeave={() => {
                if (confirmThreadId === thread.id) {
                  setConfirmThreadId(null);
                }
              }}
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
                style={
                  isConfirming
                    ? { opacity: 1, pointerEvents: 'auto' }
                    : undefined
                }
                title={
                  isConfirming
                    ? t('Confirm archive {name}', { name: thread.title })
                    : t('Archive thread')
                }
                type="button"
              >
                {isConfirming ? t('Confirm') : <Archive aria-hidden size={13} strokeWidth={1.55} />}
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
