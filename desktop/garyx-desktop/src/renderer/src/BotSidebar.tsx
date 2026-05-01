import { type Dispatch, type SetStateAction, useEffect, useMemo, useState } from 'react';

import type { DesktopBotConsoleSummary, DesktopChannelEndpoint } from '@shared/contracts';

import { ChevronDownIcon } from './app-shell/icons';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { ChannelLogo } from './channel-logo';
import { useI18n } from './i18n';

function setsEqual(left: Set<string>, right: Set<string>): boolean {
  if (left.size !== right.size) {
    return false;
  }
  for (const value of left) {
    if (!right.has(value)) {
      return false;
    }
  }
  return true;
}

function botRootThreadIds(
  group: DesktopBotConsoleSummary,
  includeDefaultOpenThread: boolean,
): Set<string> {
  const ids = [
    group.mainThreadId,
    group.mainEndpoint?.threadId,
  ].filter((value): value is string => Boolean(value));
  if (includeDefaultOpenThread) {
    if (group.defaultOpenThreadId) {
      ids.push(group.defaultOpenThreadId);
    }
    if (group.defaultOpenEndpoint?.threadId) {
      ids.push(group.defaultOpenEndpoint.threadId);
    }
  }
  return new Set(ids);
}

function botRootCanOpen(group: DesktopBotConsoleSummary): boolean {
  return group.rootBehavior !== 'expand_only' || botRootThreadIds(group, true).size > 0;
}

function toggleGroupExpanded(
  groupId: string,
  setExpandedGroupIds: Dispatch<SetStateAction<Set<string>>>,
): void {
  setExpandedGroupIds((current) => {
    const next = new Set(current);
    if (next.has(groupId)) {
      next.delete(groupId);
    } else {
      next.add(groupId);
    }
    return next;
  });
}

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
  const { t } = useI18n();
  const [expandedGroupIds, setExpandedGroupIds] = useState<Set<string>>(new Set());
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
      return setsEqual(current, next) ? current : next;
    });
  }, [groups, selectedThreadId]);

  return (
    <div className="sidebar-thread-block sidebar-bot-block">
      <div className="panel-header sidebar-section-header">
        <div className="sidebar-section-copy">
          <span className="sidebar-section-title">{t('Bots')}</span>
        </div>
        {!groups.length ? (
          <div className="sidebar-section-tools">
            <button
              aria-label={t('Add bot')}
              className="sidebar-section-action sidebar-section-action-always"
              onClick={onAddBot}
              title={t('Add bot')}
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
            const rootThreadIds = botRootThreadIds(group, childEntries.length === 0);
            const rootCanOpen = botRootCanOpen(group);
            const rootIsSelected = selectedThreadId ? rootThreadIds.has(selectedThreadId) : false;

            return (
              <section className="workspace-group" key={group.id}>
                <div
                  className={`workspace-row workspace-row-shell bot-group-shell ${rootIsSelected ? 'active' : ''}`}
                >
                  <button
                    aria-current={rootIsSelected ? 'page' : undefined}
                    aria-label={t('Open {name} thread', { name: group.title })}
                    className="workspace-row-main bot-group-main"
                    disabled={!rootCanOpen}
                    onClick={() => {
                      if (!rootCanOpen) {
                        return;
                      }
                      onOpenBot(group);
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
                    </div>
                  </button>
                  {childEntries.length ? (
                    <button
                      aria-expanded={isExpanded}
                      aria-label={isExpanded ? t('Collapse') : t('Expand')}
                      className="bot-group-expand-button"
                      onClick={() => {
                        toggleGroupExpanded(group.id, setExpandedGroupIds);
                      }}
                      tabIndex={-1}
                      title={isExpanded ? t('Collapse') : t('Expand')}
                      type="button"
                    >
                      <ChevronDownIcon
                        size={16}
                        className={`icon sidebar-section-chevron ${isExpanded ? '' : 'collapsed'}`}
                      />
                    </button>
                  ) : null}
                </div>
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
