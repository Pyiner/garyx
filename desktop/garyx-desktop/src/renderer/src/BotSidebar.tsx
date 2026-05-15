import { useMemo, useState } from 'react';

import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { botRootBoundThreadId } from './bot-console-model';
import { ChevronDownIcon, ForwardIcon } from './app-shell/icons';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { ChannelLogo } from './channel-logo';
import { useI18n } from './i18n';

function botRootThreadIds(group: DesktopBotConsoleSummary): Set<string> {
  const ids = [
    botRootBoundThreadId(group),
    group.mainThreadId,
    group.mainEndpoint?.threadId,
  ].filter((value): value is string => Boolean(value));
  return new Set(ids);
}

function botRootCanOpen(group: DesktopBotConsoleSummary): boolean {
  return group.rootBehavior !== 'expand_only' || botRootThreadIds(group).size > 0;
}

type BotSidebarProps = {
  groups: DesktopBotConsoleSummary[];
  selectedThreadId: string | null;
  activeConversationGroupId: string | null;
  onOpenBot: (group: DesktopBotConsoleSummary) => void;
  onToggleConversationGroup: (group: DesktopBotConsoleSummary) => void;
  onAddBot: () => void;
};

export function BotSidebar({
  groups,
  selectedThreadId,
  activeConversationGroupId,
  onOpenBot,
  onToggleConversationGroup,
  onAddBot,
}: BotSidebarProps) {
  const { t } = useI18n();
  const [sectionCollapsed, setSectionCollapsed] = useState(false);
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

  return (
    <div className="sidebar-thread-block sidebar-bot-block">
      <div className="panel-header sidebar-section-header sidebar-section-header-interactive">
        <button
          aria-expanded={!sectionCollapsed}
          aria-label={sectionCollapsed ? t('Expand bots') : t('Collapse bots')}
          className="sidebar-section-toggle"
          onClick={() => setSectionCollapsed((current) => !current)}
          type="button"
        >
          <span className="sidebar-section-title">{t('Bots')}</span>
          <ChevronDownIcon
            size={16}
            className={`icon sidebar-section-chevron ${sectionCollapsed ? 'collapsed' : ''}`}
          />
        </button>
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

      <div
        aria-hidden={sectionCollapsed}
        className={`sidebar-collapsible sidebar-section-panel ${sectionCollapsed ? 'is-collapsed' : ''}`}
        inert={sectionCollapsed ? true : undefined}
      >
        <div className="sidebar-collapsible-inner workspace-list sidebar-bot-list">
          {groups.map((group) => {
            const childEntries = group.conversationNodes || [];
            const isConversationRailOpen = activeConversationGroupId === group.id;
            const rootThreadIds = botRootThreadIds(group);
            const rootCanOpen = botRootCanOpen(group);
            const rowCanOpen = rootCanOpen || childEntries.length > 0;
            const rootIsSelected = selectedThreadId ? rootThreadIds.has(selectedThreadId) : false;
            const rowIsSelected = rootIsSelected;
            const rowActionLabel = rootCanOpen
              ? t('Open {name} thread', { name: group.title })
              : t('Show conversations');

            return (
              <section className="workspace-group" key={group.id}>
                <div
                  className={`workspace-row workspace-row-shell bot-group-shell ${rowIsSelected ? 'active' : ''}`}
                >
                  <button
                    aria-current={rootIsSelected ? 'page' : undefined}
                    aria-label={rowActionLabel}
                    className="workspace-row-main bot-group-main"
                    disabled={!rowCanOpen}
                    onClick={() => {
                      if (rootCanOpen) {
                        onOpenBot(group);
                        return;
                      }
                      if (childEntries.length) {
                        onToggleConversationGroup(group);
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
                    </div>
                  </button>
                  {childEntries.length ? (
                    <div className="bot-group-conversation-tools">
                      <button
                        aria-expanded={isConversationRailOpen}
                        aria-label={isConversationRailOpen ? t('Hide conversations') : t('Show conversations')}
                        className={`bot-group-expand-button ${isConversationRailOpen ? 'active' : ''}`}
                        onClick={(event) => {
                          event.stopPropagation();
                          onToggleConversationGroup(group);
                        }}
                        tabIndex={-1}
                        title={isConversationRailOpen ? t('Hide conversations') : t('Show conversations')}
                        type="button"
                      >
                        <ForwardIcon />
                      </button>
                    </div>
                  ) : null}
                </div>
              </section>
            );
          })}

        </div>
      </div>
    </div>
  );
}
