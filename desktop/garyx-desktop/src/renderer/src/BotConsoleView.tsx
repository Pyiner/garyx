import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { channelDisplayName, primaryBotEndpoint } from './bot-console-model';
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

function formatTimestamp(value: string | null): string {
  if (!value) {
    return 'No recent activity';
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
  return (
    <div className="shadcn-shell bot-console-view">
      <section className="shadcn-hero">
        <div className="shadcn-hero-copy">
          <p className="shadcn-kicker">Bot Console</p>
          <h1>Per-bot controls</h1>
          <p className="shadcn-subcopy">
            Mobile-friendly bot status, endpoint binding state, and quick thread actions.
          </p>
          {toolbarNote ? (
            <div className="shadcn-inline-note">
              <UIBadge>Deep Link</UIBadge>
              <code>{toolbarNote}</code>
            </div>
          ) : null}
        </div>
        <div className="shadcn-hero-actions">
          {onRefresh ? (
            <UIButton onClick={onRefresh} variant="outline">
              Refresh
            </UIButton>
          ) : null}
          {onOpenSettings ? (
            <UIButton onClick={onOpenSettings} variant="secondary">
              Settings
            </UIButton>
          ) : null}
        </div>
      </section>

      {status ? (
        <UICard className="bot-console-status-card">
          <UICardContent className="bot-console-status-copy">
            <UIBadge className="is-connected">Success</UIBadge>
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
                        {group.status === 'connected' ? 'Connected' : 'Idle'}
                      </UIBadge>
                    </div>
                    <UICardTitle>{group.title}</UICardTitle>
                    <UICardDescription>
                      Workspace <code>{group.workspaceDir || 'Not configured'}</code>
                    </UICardDescription>
                    <UICardDescription>
                      Main endpoint {group.mainEndpointStatus} · {group.boundEndpointCount}/{group.endpointCount} endpoints bound · latest activity{' '}
                      {formatTimestamp(group.latestActivity)}
                    </UICardDescription>
                  </div>
                  <div className="bot-console-card-actions">
                    {!isFocusedBot && onOpenBot ? (
                      <UIButton onClick={() => onOpenBot(group.id)} size="sm" variant="outline">
                        Open Bot
                      </UIButton>
                    ) : null}
                    {openThreadId && onOpenThread ? (
                      <UIButton onClick={() => onOpenThread(openThreadId)} size="sm">
                        Open Main Chat
                      </UIButton>
                    ) : null}
                    {onCreateThread ? (
                      <UIButton
                        disabled={createBusy}
                        onClick={() => onCreateThread(group)}
                        size="sm"
                        variant="outline"
                      >
                        {createBusy ? 'Opening…' : 'Open Main Chat'}
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
                                )}
                              </span>
                            </div>
                            <div className="bot-console-endpoint-badges">
                              <UIBadge>
                                {channelDisplayName(endpoint.channel)} · {endpoint.accountId}
                              </UIBadge>
                              <UIBadge className={endpoint.threadId ? 'is-connected' : 'is-idle'}>
                                {endpoint.threadId ? 'Bound' : 'Unbound'}
                              </UIBadge>
                            </div>
                            <p className="bot-console-mono-line">
                              Delivery <code>{endpoint.deliveryTargetType}:{endpoint.deliveryTargetId || endpoint.chatId}</code>
                            </p>
                            <p className="bot-console-mono-line">
                              Thread <code>{endpoint.threadId || 'Not bound'}</code>
                            </p>
                            <p className="bot-console-mono-line">
                              Endpoint <code>{endpoint.endpointKey}</code>
                            </p>
                          </div>
                          {endpoint.threadId && onOpenThread ? (
                            <UIButton
                              onClick={() => onOpenThread(endpoint.threadId!)}
                              size="sm"
                              variant="secondary"
                            >
                              Open
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
                            || `No conversations yet. Send a message to this bot on ${channelDisplayName(group.channel)} to start a thread.`}
                        </p>
                        {onCreateThread ? (
                          <UIButton
                            disabled={createBusy}
                            onClick={() => onCreateThread(group)}
                            size="sm"
                          >
                            {createBusy ? 'Opening…' : 'Open Main Chat'}
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
            <UIBadge>Bots</UIBadge>
            <UICardTitle>No bots configured</UICardTitle>
            <UICardDescription>
              {emptyCopy || 'Add Telegram or Feishu bot accounts first, then the bot console will appear here.'}
            </UICardDescription>
            <UICardDescription>{totalEndpoints} known endpoints</UICardDescription>
          </UICardHeader>
          {onOpenSettings ? (
            <UICardContent>
              <UIButton onClick={onOpenSettings} variant="outline">
                Open Settings
              </UIButton>
            </UICardContent>
          ) : null}
        </UICard>
      )}
    </div>
  );
}
