import { useEffect, useState } from 'react';

import type { DesktopBotConsoleSummary, DesktopChannelEndpoint } from '@shared/contracts';

import { ChevronDownIcon, MoreDotsIcon } from './app-shell/icons';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { ChannelLogo } from './channel-logo';

type BotSidebarProps = {
  groups: DesktopBotConsoleSummary[];
  selectedThreadId: string | null;
  formatThreadTimestamp: (value?: string | null) => string;
  onOpenBot: (group: DesktopBotConsoleSummary) => void;
  onOpenEndpoint: (endpoint: DesktopChannelEndpoint) => void;
  onAddBot: () => void;
};

export function BotSidebar({
  groups,
  selectedThreadId,
  formatThreadTimestamp,
  onOpenBot,
  onOpenEndpoint,
  onAddBot,
}: BotSidebarProps) {
  const [expandedGroupIds, setExpandedGroupIds] = useState<Set<string>>(new Set());
  const { entries: pluginCatalog } = useChannelPluginCatalog();

  const iconDataUrlByChannel = new Map(
    (pluginCatalog || []).map((entry) => [entry.id.toLowerCase(), entry.icon_data_url || null]),
  );

  useEffect(() => {
    setExpandedGroupIds((current) => {
      const next = new Set<string>();
      for (const group of groups) {
        const childEntries = group.conversationNodes || [];
        if (!childEntries.length) {
          continue;
        }
        if (current.has(group.id) || childEntries.some((entry) => entry.endpoint.threadId === selectedThreadId)) {
          next.add(group.id);
        }
      }
      return next;
    });
  }, [groups, selectedThreadId]);

  return (
    <div className="sidebar-thread-block sidebar-bot-block">
      <div className="panel-header sidebar-section-header">
        <div className="sidebar-section-copy">
          <span className="sidebar-section-title">Bots</span>
        </div>
        {!groups.length ? (
          <div className="sidebar-section-tools">
            <button
              aria-label="Add bot"
              className="sidebar-section-action sidebar-section-action-always"
              onClick={onAddBot}
              title="Add bot"
              type="button"
            >
              <svg aria-hidden width="14" height="14" viewBox="0 0 15 15" fill="none" style={{ strokeWidth: 1.21 }}>
                <path d="M0.5 7.5H14.5M7.5 0.5V14.5" stroke="currentColor" strokeLinecap="round"/>
              </svg>
            </button>
          </div>
        ) : null}
        </div>

      <div className="workspace-list sidebar-bot-list">
          {groups.map((group) => {
            const childEntries = group.conversationNodes || [];
            const isExpanded = expandedGroupIds.has(group.id);
            const rootOpensDefault = group.rootBehavior !== 'expand_only';

            return (
              <section className="workspace-group" key={group.id}>
                <button
                  className="workspace-row workspace-row-shell bot-group-shell"
                  onClick={() => {
                    if (!rootOpensDefault || childEntries.length) {
                      setExpandedGroupIds((current) => {
                        const next = new Set(current);
                        if (next.has(group.id)) {
                          next.delete(group.id);
                        } else {
                          next.add(group.id);
                        }
                        return next;
                      });
                    } else if (rootOpensDefault) {
                      onOpenBot(group);
                    }
                  }}
                  tabIndex={-1}
                  type="button"
                >
                  <div className="workspace-row-copy">
                    <ChannelLogo
                      channel={group.channel}
                      className="channel-logo bot-row-logo"
                      iconDataUrl={iconDataUrlByChannel.get(group.channel.toLowerCase()) || null}
                      fallbackLabel={group.title}
                    />
                    <span className="workspace-name" title={group.title}>
                      {group.title}
                    </span>
                    {childEntries.length ? (
                      <ChevronDownIcon
                        size={16}
                        className={`icon sidebar-section-chevron ${isExpanded ? '' : 'collapsed'}`}
                      />
                    ) : null}
                  </div>
                </button>
                {isExpanded && childEntries.length ? (
                  <div className="bot-thread-list">
                    {childEntries.map((entry) => {
                      const isSelected = selectedThreadId === entry.endpoint.threadId;
                      return (
                        <button
                          className={`workspace-row bot-thread-row ${isSelected ? 'active' : ''}`}
                          key={entry.id}
                          onClick={() => {
                            onOpenEndpoint(entry.endpoint);
                          }}
                          tabIndex={-1}
                          type="button"
                        >
                          <div className="workspace-row-copy bot-thread-copy">
                            <span className="bot-thread-title" title={entry.title}>
                              {entry.title}
                            </span>
                            {entry.badge ? (
                              <span className="bot-thread-badge">{entry.badge}</span>
                            ) : null}
                          </div>
                          <span className="workspace-status">
                            {formatThreadTimestamp(entry.latestActivity)}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ) : null}
              </section>
            );
          })}

        </div>
    </div>
  );
}
