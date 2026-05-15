import { useEffect, useMemo, useRef, useState } from 'react';
import { PanelLeftClose, Trash } from 'lucide-react';

import type { DesktopBotConsoleSummary, DesktopChannelEndpoint } from '@shared/contracts';

import { ChannelLogo } from './channel-logo';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { useI18n } from './i18n';

type BotConversationSidebarProps = {
  group: DesktopBotConsoleSummary;
  selectedThreadId: string | null;
  deletingThreadId: string | null;
  formatThreadTimestamp: (value?: string | null) => string;
  isThreadRuntimeBusy: (threadId: string) => boolean;
  onArchiveEndpoint: (endpoint: DesktopChannelEndpoint) => void;
  onClose: () => void;
  onOpenEndpoint: (endpoint: DesktopChannelEndpoint) => void;
};

export function BotConversationSidebar({
  group,
  selectedThreadId,
  deletingThreadId,
  formatThreadTimestamp,
  isThreadRuntimeBusy,
  onArchiveEndpoint,
  onClose,
  onOpenEndpoint,
}: BotConversationSidebarProps) {
  const { t } = useI18n();
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const iconDataUrlByChannel = useMemo(
    () =>
      new Map(
        (pluginCatalog || []).map((entry) => [
          entry.id.toLowerCase(),
          entry.icon_data_url || null,
        ]),
      ),
    [pluginCatalog],
  );
  const entries = (group.conversationNodes || []).filter(
    (entry) => entry.endpoint.threadId !== deletingThreadId,
  );

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
      aria-label={t('{name} conversations', { name: group.title })}
      className="bot-conversation-rail"
    >
      <div className="bot-conversation-header">
        <div className="bot-conversation-heading">
          <ChannelLogo
            channel={group.channel}
            className="channel-logo bot-conversation-logo"
            iconDataUrl={iconDataUrlByChannel.get(group.channel.toLowerCase()) || null}
            fallbackLabel={group.title}
          />
          <div className="bot-conversation-title-copy">
            <div className="bot-conversation-title" title={group.title}>
              {group.title}
            </div>
          </div>
        </div>
        <button
          aria-label={t('Collapse conversations')}
          className="bot-conversation-collapse"
          onClick={onClose}
          title={t('Collapse conversations')}
          type="button"
        >
          <PanelLeftClose aria-hidden size={15} strokeWidth={1.8} />
        </button>
      </div>

      <div className="bot-conversation-list">
        {entries.map((entry) => {
          const isSelected = selectedThreadId === entry.endpoint.threadId;
          const threadId = entry.endpoint.threadId || '';
          const isBusy = Boolean(threadId && isThreadRuntimeBusy(threadId));
          const archiveDisabled = !threadId || isBusy;
          return (
            <div
              className={`bot-conversation-row-shell ${isSelected ? 'active' : ''} ${archiveDisabled ? 'no-delete' : ''}`}
              key={entry.id}
              onMouseLeave={() => {
                if (confirmDeleteId === threadId) {
                  setConfirmDeleteId(null);
                }
              }}
            >
              <button
                aria-current={isSelected ? 'page' : undefined}
                className="bot-conversation-row"
                disabled={!entry.openable}
                onClick={() => {
                  if (entry.openable) {
                    onOpenEndpoint(entry.endpoint);
                  }
                }}
                type="button"
              >
                <div className="bot-conversation-row-main">
                  <span className="bot-conversation-row-title" title={entry.title}>
                    {entry.title}
                  </span>
                  {entry.badge ? (
                    <span className="bot-thread-badge">{t(entry.badge)}</span>
                  ) : null}
                </div>
                <span className="bot-conversation-row-time">
                  {formatThreadTimestamp(entry.latestActivity)}
                </span>
              </button>
              {archiveDisabled ? null : confirmDeleteId === threadId ? (
                <button
                  aria-label={t('Confirm delete {name}', { name: entry.title })}
                  className="thread-delete-button confirm"
                  style={{ opacity: 1, pointerEvents: 'auto' }}
                  onClick={(event) => {
                    event.stopPropagation();
                    setConfirmDeleteId(null);
                    onArchiveEndpoint(entry.endpoint);
                  }}
                  tabIndex={-1}
                  type="button"
                >
                  {t('Confirm')}
                </button>
              ) : (
                <button
                  aria-label={t('Delete {name}', { name: entry.title })}
                  className="thread-delete-button"
                  onClick={(event) => {
                    event.stopPropagation();
                    setConfirmDeleteId(threadId);
                  }}
                  tabIndex={-1}
                  type="button"
                >
                  <Trash aria-hidden />
                </button>
              )}
            </div>
          );
        })}
      </div>
    </aside>
  );
}
