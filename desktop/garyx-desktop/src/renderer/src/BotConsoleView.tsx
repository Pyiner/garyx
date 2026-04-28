import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { channelDisplayName, primaryBotEndpoint } from './bot-console-model';
import { useI18n, type Translate } from './i18n';
import {
  UIButton,
  UIBadge,
  UICard,
  UICardContent,
  UICardDescription,
  UICardHeader,
  UICardTitle,
} from './ui';

type BotConsoleViewProps = {
  groups: DesktopBotConsoleSummary[];
  totalEndpoints: number;
  focusedBotId?: string | null;
  focusedEndpointKey?: string | null;
  onOpenThread?: (threadId: string) => void;
  onOpenBot?: (botId: string) => void;
  onCreateThread?: (group: DesktopBotConsoleSummary) => void;
  onOpenSettings?: () => void;
  onRefresh?: () => void;
  busyBotId?: string | null;
  emptyCopy?: string;
  toolbarNote?: string | null;
  status?: string | null;
};

function formatTimestamp(value: string | null, t: Translate): string {
  if (!value) {
    return t('No recent activity');
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString(undefined, { hour12: false });
}

function statusTone(status: DesktopBotConsoleSummary['status']) {
  return status === 'connected' ? 'is-connected' : 'is-idle';
}

export function BotConsoleView({
  groups,
  totalEndpoints,
  focusedBotId,
  focusedEndpointKey,
  onOpenThread,
  onOpenBot,
  onCreateThread,
  onOpenSettings,
  onRefresh,
  busyBotId,
  emptyCopy,
  toolbarNote,
  status,
}: BotConsoleViewProps) {
  const { t } = useI18n();

  return (
    <div className="shadcn-shell bot-console-view">
      <section className="shadcn-hero">
        <div className="shadcn-hero-copy">
          <p className="shadcn-kicker">{t('Bot Console')}</p>
          <h1>{t('Per-bot controls')}</h1>
          <p className="shadcn-subcopy">
            {t('Mobile-friendly bot status, endpoint binding state, and quick thread actions.')}
          </p>
          {toolbarNote ? (
            <div className="shadcn-inline-note">
              <UIBadge>{t('Deep Link')}</UIBadge>
              <code>{toolbarNote}</code>
            </div>
          ) : null}
        </div>
        <div className="shadcn-hero-actions">
          {onRefresh ? (
            <UIButton onClick={onRefresh} variant="outline">
              {t('Refresh')}
            </UIButton>
          ) : null}
          {onOpenSettings ? (
            <UIButton onClick={onOpenSettings} variant="secondary">
              {t('Settings')}
            </UIButton>
          ) : null}
        </div>
      </section>

      {status ? (
        <UICard className="bot-console-status-card">
          <UICardContent className="bot-console-status-copy">
            <UIBadge className="is-connected">{t('Success')}</UIBadge>
            <p>{status}</p>
          </UICardContent>
        </UICard>
      ) : null}

      {groups.length ? (
        <div className="bot-console-grid">
          {groups.map((group) => {
            const primaryEndpoint = primaryBotEndpoint(group);
            const openThreadId = group.defaultOpenThreadId || group.mainThreadId || primaryEndpoint?.threadId || null;
            const createBusy = busyBotId === group.id;
            const isFocusedBot = Boolean(focusedBotId && group.id === focusedBotId);
            return (
              <UICard
                className={`bot-console-card ${isFocusedBot ? 'is-focused' : ''}`}
                key={group.id}
              >
                <UICardHeader className="bot-console-card-header">
                  <div className="bot-console-card-heading">
                    <div className="bot-console-card-heading-top">
                      <UIBadge>{group.subtitle}</UIBadge>
                      <UIBadge className={statusTone(group.status)}>
                        {group.status === 'connected' ? t('Connected') : t('Idle')}
                      </UIBadge>
                    </div>
                    <UICardTitle>{group.title}</UICardTitle>
                    <UICardDescription>
                      {t('Workspace')} <code>{group.workspaceDir || t('Not configured')}</code>
                    </UICardDescription>
                    <UICardDescription>
                      {t('Main endpoint {status} · {bound}/{total} endpoints bound · latest activity {time}', {
                        status: group.mainEndpointStatus,
                        bound: group.boundEndpointCount,
                        total: group.endpointCount,
                        time: formatTimestamp(group.latestActivity, t),
                      })}
                    </UICardDescription>
                  </div>
                  <div className="bot-console-card-actions">
                    {!isFocusedBot && onOpenBot ? (
                      <UIButton onClick={() => onOpenBot(group.id)} size="sm" variant="outline">
                        {t('Open Bot')}
                      </UIButton>
                    ) : null}
                    {openThreadId && onOpenThread ? (
                      <UIButton onClick={() => onOpenThread(openThreadId)} size="sm">
                        {t('Open Main Chat')}
                      </UIButton>
                    ) : null}
                    {onCreateThread ? (
                      <UIButton
                        disabled={createBusy}
                        onClick={() => onCreateThread(group)}
                        size="sm"
                        variant="outline"
                      >
                        {createBusy ? t('Opening…') : t('Open Main Chat')}
                      </UIButton>
                    ) : null}
                  </div>
                </UICardHeader>

                <UICardContent className="bot-console-endpoints">
                  {group.endpoints.length ? (
                    group.endpoints.map((endpoint) => (
                      <UICard
                        className={`bot-console-endpoint ${focusedEndpointKey === endpoint.endpointKey ? 'is-focused' : ''}`}
                        key={endpoint.endpointKey}
                      >
                        <UICardContent className="bot-console-endpoint-body">
                          <div className="bot-console-endpoint-copy">
                            <div className="bot-console-endpoint-title-row">
                              <strong>{endpoint.displayLabel || endpoint.peerId}</strong>
                              <span className="bot-console-endpoint-time">
                                {formatTimestamp(
                                  endpoint.lastInboundAt
                                    || endpoint.lastDeliveryAt
                                    || endpoint.threadUpdatedAt
                                    || null,
                                  t,
                                )}
                              </span>
                            </div>
                            <div className="bot-console-endpoint-badges">
                              <UIBadge>
                                {channelDisplayName(endpoint.channel)} · {endpoint.accountId}
                              </UIBadge>
                              <UIBadge className={endpoint.threadId ? 'is-connected' : 'is-idle'}>
                                {endpoint.threadId ? t('Bound') : t('Unbound')}
                              </UIBadge>
                            </div>
                            <p className="bot-console-mono-line">
                              {t('Delivery')} <code>{endpoint.deliveryTargetType}:{endpoint.deliveryTargetId || endpoint.chatId}</code>
                            </p>
                            <p className="bot-console-mono-line">
                              {t('Thread')} <code>{endpoint.threadId || t('Not bound')}</code>
                            </p>
                            <p className="bot-console-mono-line">
                              {t('Endpoint')} <code>{endpoint.endpointKey}</code>
                            </p>
                          </div>
                          {endpoint.threadId && onOpenThread ? (
                            <UIButton
                              onClick={() => onOpenThread(endpoint.threadId!)}
                              size="sm"
                              variant="secondary"
                            >
                              {t('Open')}
                            </UIButton>
                          ) : null}
                        </UICardContent>
                      </UICard>
                    ))
                  ) : (
                    <UICard className="bot-console-endpoint empty">
                      <UICardContent className="bot-console-empty-endpoint">
                        <p>
                          {emptyCopy
                            || t('No conversations yet. Send a message to this bot on {channel} to start a thread.', {
                              channel: channelDisplayName(group.channel),
                            })}
                        </p>
                        {onCreateThread ? (
                          <UIButton
                            disabled={createBusy}
                            onClick={() => onCreateThread(group)}
                            size="sm"
                          >
                            {createBusy ? t('Opening…') : t('Open Main Chat')}
                          </UIButton>
                        ) : null}
                      </UICardContent>
                    </UICard>
                  )}
                </UICardContent>
              </UICard>
            );
          })}
        </div>
      ) : (
        <UICard className="bot-console-empty-card">
          <UICardHeader>
            <UIBadge>{t('Bots')}</UIBadge>
            <UICardTitle>{t('No bots configured')}</UICardTitle>
            <UICardDescription>
              {emptyCopy || t('Add Telegram or Feishu bot accounts first, then the bot console will appear here.')}
            </UICardDescription>
            <UICardDescription>{t('{count} known endpoints', { count: totalEndpoints })}</UICardDescription>
          </UICardHeader>
          {onOpenSettings ? (
            <UICardContent>
              <UIButton onClick={onOpenSettings} variant="outline">
                {t('Open Settings')}
              </UIButton>
            </UICardContent>
          ) : null}
        </UICard>
      )}
    </div>
  );
}
