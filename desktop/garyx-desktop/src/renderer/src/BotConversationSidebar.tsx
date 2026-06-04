import type { PointerEvent as ReactPointerEvent } from 'react';
import { useMemo } from 'react';

import type { DesktopBotConsoleSummary, DesktopChannelEndpoint } from '@shared/contracts';

import { ChannelLogo } from './channel-logo';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { ThreadConversationSidebar } from './ThreadConversationSidebar';
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
  onRailResizeStart?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  railResizing?: boolean;
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
  onRailResizeStart,
  railResizing,
}: BotConversationSidebarProps) {
  const { t } = useI18n();
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const iconDataUrl = useMemo(() => {
    const byChannel = new Map(
      (pluginCatalog || []).map((entry) => [
        entry.id.toLowerCase(),
        entry.icon_data_url || null,
      ]),
    );
    return byChannel.get(group.channel.toLowerCase()) || null;
  }, [group.channel, pluginCatalog]);

  const rows = (group.conversationNodes || [])
    .filter((entry) => entry.endpoint.threadId !== deletingThreadId)
    .map((entry) => {
      const threadId = entry.endpoint.threadId || '';
      const archiveDisabled = !threadId || isThreadRuntimeBusy(threadId);
      return {
        key: entry.id,
        title: entry.title,
        time: entry.latestActivity,
        badge: entry.badge,
        isActive: selectedThreadId === entry.endpoint.threadId,
        openable: entry.openable,
        onOpen: () => onOpenEndpoint(entry.endpoint),
        onArchive: archiveDisabled ? undefined : () => onArchiveEndpoint(entry.endpoint),
      };
    });

  return (
    <ThreadConversationSidebar
      ariaLabel={t('{name} conversations', { name: group.title })}
      collapseLabel={t('Collapse conversations')}
      formatThreadTimestamp={formatThreadTimestamp}
      logo={
        <ChannelLogo
          channel={group.channel}
          className="channel-logo bot-conversation-logo"
          iconDataUrl={iconDataUrl}
          fallbackLabel={group.title}
        />
      }
      onClose={onClose}
      onRailResizeStart={onRailResizeStart}
      railResizing={railResizing}
      rows={rows}
      title={group.title}
    />
  );
}
