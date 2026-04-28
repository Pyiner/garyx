import { useState } from 'react';

import {
  UIButton,
  UIBadge,
  UICard,
  UICardContent,
  UICardDescription,
  UICardHeader,
  UICardTitle,
} from './ui';

import type {
  DesktopBotConsoleSummary,
  GatewaySettingsPayload,
  GatewayThreadHistoryBackend,
} from '@shared/contracts';

type WebSettingsPageProps = {
  focusedBotId?: string | null;
  focusedBotSummary?: DesktopBotConsoleSummary | null;
  payload: GatewaySettingsPayload | null;
  jsonDraft: string;
  loading: boolean;
  saving: boolean;
  error?: string | null;
  status?: string | null;
  onChangeJson: (value: string) => void;
  onPatchGateway: (patch: {
    host?: string;
    port?: number;
    publicUrl?: string;
  }) => void;
  onPatchHeartbeat: (patch: {
    enabled?: boolean;
    every?: string;
    target?: string;
    ackMaxChars?: number;
    activeHoursStart?: string;
    activeHoursEnd?: string;
    activeHoursTimezone?: string;
  }) => void;
  onPatchSessions: (patch: {
    dataDir?: string;
    backend?: GatewayThreadHistoryBackend;
  }) => void;
  onAddTelegramAccount: (accountId: string) => void;
  onRemoveTelegramAccount: (accountId: string) => void;
  onPatchTelegramAccount: (accountId: string, patch: {
    enabled?: boolean;
    token?: string;
    name?: string;
    agentId?: string;
    workspaceDir?: string;
  }) => void;
  onAddFeishuAccount: (accountId: string) => void;
  onRemoveFeishuAccount: (accountId: string) => void;
  onPatchFeishuAccount: (accountId: string, patch: {
    enabled?: boolean;
    appId?: string;
    appSecret?: string;
    domain?: string;
    requireMention?: boolean;
    topicSessionMode?: string;
    name?: string;
    agentId?: string;
    workspaceDir?: string;
  }) => void;
  onRefresh?: () => void;
  onSave?: () => void;
};

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function countAccounts(value: unknown): number {
  const accounts = asRecord(value);
  return Object.keys(accounts).length;
}

function flatChannelMap(
  channels: Record<string, unknown>,
) {
  const next = { ...channels };
  const legacyPlugins = asRecord(next.plugins);
  for (const [channelId, channelValue] of Object.entries(legacyPlugins)) {
    if (!(channelId in next)) {
      next[channelId] = channelValue;
    }
  }
  delete next.plugins;
  return next;
}

function channelAccountsMap(
  channels: Record<string, unknown>,
  channelId: string,
) {
  return asRecord(asRecord(flatChannelMap(channels)[channelId]).accounts);
}

export function WebSettingsPage({
  focusedBotId,
  focusedBotSummary,
  payload,
  jsonDraft,
  loading,
  saving,
  error,
  status,
  onChangeJson,
  onPatchGateway,
  onPatchHeartbeat,
  onPatchSessions,
  onAddTelegramAccount,
  onRemoveTelegramAccount,
  onPatchTelegramAccount,
  onAddFeishuAccount,
  onRemoveFeishuAccount,
  onPatchFeishuAccount,
  onRefresh,
  onSave,
}: WebSettingsPageProps) {
  const [copiedSecretKey, setCopiedSecretKey] = useState<string | null>(null);
  const config = asRecord(payload?.config);
  const gateway = asRecord(config.gateway);
  const heartbeat = asRecord(asRecord(config.agent_defaults).heartbeat);
  const sessions = asRecord(config.sessions);
  const threadHistoryBackend =
    sessions.thread_history_backend === 'inline_messages' ? 'inline_messages' : 'transcript_v1';
  const channels = flatChannelMap(asRecord(config.channels));
  const publicUrl = String(gateway.public_url || '').trim();
  const channelConfigs = Object.fromEntries(Object.entries(channels).filter(([channelId]) => channelId !== 'api'));
  const telegramAccountsMap = channelAccountsMap(channels, 'telegram');
  const feishuAccountsMap = channelAccountsMap(channels, 'feishu');
  const totalPluginAccounts = Object.values(channelConfigs).reduce<number>((sum, pluginValue) => {
    return sum + countAccounts(asRecord(asRecord(pluginValue).accounts));
  }, 0);
  const telegramAccounts = countAccounts(telegramAccountsMap);
  const feishuAccounts = countAccounts(feishuAccountsMap);
  const telegramEntries = Object.entries(telegramAccountsMap);
  const feishuEntries = Object.entries(feishuAccountsMap);
  const botParts = (focusedBotId || '').split('::');
  const focusedChannel = botParts.length >= 2 ? botParts[0] : null;
  const focusedAccountId = botParts.length >= 2 ? botParts.slice(1).join('::') : null;
  const isScopedBotSettings = Boolean(focusedChannel && focusedAccountId);
  const visibleTelegramEntries = isScopedBotSettings && focusedChannel === 'telegram'
    ? telegramEntries.filter(([accountId]) => accountId === focusedAccountId)
    : telegramEntries;
  const visibleFeishuEntries = isScopedBotSettings && focusedChannel === 'feishu'
    ? feishuEntries.filter(([accountId]) => accountId === focusedAccountId)
    : feishuEntries;
  const hasRuntimeBotSummary = Boolean(
    focusedBotSummary
      && focusedChannel === focusedBotSummary.channel
      && focusedAccountId === focusedBotSummary.accountId,
  );
  const runtimeBotSummary = hasRuntimeBotSummary ? focusedBotSummary : null;

  async function copySecret(secretKey: string, value: string) {
    if (!value) {
      return;
    }
    try {
      await navigator.clipboard.writeText(value);
      setCopiedSecretKey(secretKey);
      window.setTimeout(() => {
        setCopiedSecretKey((current) => (current === secretKey ? null : current));
      }, 1200);
    } catch {
      // Ignore clipboard failures in constrained environments.
    }
  }

  return (
    <div className="thread-history-shell">
      <div className="thread-history-page shadcn-shell">
        <section className="shadcn-hero">
          <div className="shadcn-hero-copy">
            <p className="shadcn-kicker">Settings</p>
            <h1>{isScopedBotSettings ? 'Bot settings' : 'Gateway settings'}</h1>
            <p className="shadcn-subcopy">
              {isScopedBotSettings
                ? `${focusedChannel}/${focusedAccountId}`
                : `Source ${payload?.source || 'unknown'}${payload ? ` · secrets ${payload.secretsMasked ? 'masked' : 'visible'}` : ''}`}
            </p>
          </div>
          <div className="shadcn-hero-actions">
            {onRefresh ? (
              <UIButton onClick={onRefresh} variant="outline">
                Refresh
              </UIButton>
            ) : null}
            {onSave ? (
              <UIButton disabled={loading || saving} onClick={onSave}>
                {saving ? 'Saving…' : 'Save'}
              </UIButton>
            ) : null}
          </div>
        </section>

        {error ? (
          <div className="bot-console-error" role="alert">
            {error}
          </div>
        ) : null}

        {status ? (
          <UICard className="bot-console-status-card">
            <UICardContent className="bot-console-status-copy">
              <UIBadge className="is-connected">Saved</UIBadge>
              <p>{status}</p>
            </UICardContent>
          </UICard>
        ) : null}

        {loading ? (
          <div className="empty-state">
            <span className="eyebrow">Settings</span>
            <h3>Loading settings</h3>
          </div>
        ) : (
          <section className="thread-history-panel">
            {!isScopedBotSettings ? (
              <>
                <div className="web-settings-summary-grid">
                  <UICard className="web-settings-summary-card">
                    <UICardHeader>
                      <UIBadge>Gateway</UIBadge>
                      <UICardTitle>{String(gateway.host || '0.0.0.0')}:{String(gateway.port || 31337)}</UICardTitle>
                      <UICardDescription>{String(gateway.public_url || 'No public_url configured')}</UICardDescription>
                    </UICardHeader>
                  </UICard>
                  <UICard className="web-settings-summary-card">
                    <UICardHeader>
                      <UIBadge>Heartbeat</UIBadge>
                      <UICardTitle>{heartbeat.enabled === false ? 'Disabled' : 'Enabled'}</UICardTitle>
                      <UICardDescription>
                        every {String(heartbeat.every || '--')} · target {String(heartbeat.target || '--')}
                      </UICardDescription>
                    </UICardHeader>
                  </UICard>
                  <UICard className="web-settings-summary-card">
                    <UICardHeader>
                      <UIBadge>Threads</UIBadge>
                      <UICardTitle>{threadHistoryBackend}</UICardTitle>
                      <UICardDescription>
                        {String(sessions.data_dir || 'No data_dir configured')}
                      </UICardDescription>
                    </UICardHeader>
                  </UICard>
                  <UICard className="web-settings-summary-card">
                    <UICardHeader>
                      <UIBadge>Channels</UIBadge>
                      <UICardTitle>{totalPluginAccounts} configured accounts</UICardTitle>
                      <UICardDescription>Configured account count</UICardDescription>
                    </UICardHeader>
                  </UICard>
                </div>
                <div className="web-settings-form-grid">
              <label className="web-settings-field">
                <span className="eyebrow">gateway.host</span>
                <input
                  onChange={(event) => {
                    onPatchGateway({ host: event.target.value });
                  }}
                  type="text"
                  value={String(gateway.host || '')}
                />
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">gateway.port</span>
                <input
                  onChange={(event) => {
                    const parsed = Number.parseInt(event.target.value, 10);
                    onPatchGateway({ port: Number.isFinite(parsed) ? parsed : 31337 });
                  }}
                  type="number"
                  value={String(gateway.port || 31337)}
                />
              </label>
              <label className="web-settings-field web-settings-field-span">
                <span className="eyebrow">gateway.public_url</span>
                <input
                  onChange={(event) => {
                    onPatchGateway({ publicUrl: event.target.value });
                  }}
                  placeholder="https://example.com"
                  type="text"
                  value={String(gateway.public_url || '')}
                />
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">heartbeat.enabled</span>
                <select
                  onChange={(event) => {
                    onPatchHeartbeat({ enabled: event.target.value === 'true' });
                  }}
                  value={heartbeat.enabled === false ? 'false' : 'true'}
                >
                  <option value="true">Enabled</option>
                  <option value="false">Disabled</option>
                </select>
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">heartbeat.every</span>
                <input
                  onChange={(event) => {
                    onPatchHeartbeat({ every: event.target.value });
                  }}
                  placeholder="3h"
                  type="text"
                  value={String(heartbeat.every || '')}
                />
              </label>
              <label className="web-settings-field web-settings-field-span">
                <span className="eyebrow">heartbeat.target</span>
                <input
                  onChange={(event) => {
                    onPatchHeartbeat({ target: event.target.value });
                  }}
                  placeholder="last"
                  type="text"
                  value={String(heartbeat.target || '')}
                />
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">heartbeat.ack_max_chars</span>
                <input
                  onChange={(event) => {
                    const parsed = Number.parseInt(event.target.value, 10);
                    onPatchHeartbeat({ ackMaxChars: Number.isFinite(parsed) ? parsed : 500 });
                  }}
                  type="number"
                  value={String(heartbeat.ack_max_chars ?? 500)}
                />
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">heartbeat.active_hours.start</span>
                <input
                  onChange={(event) => {
                    onPatchHeartbeat({ activeHoursStart: event.target.value });
                  }}
                  placeholder="09:00"
                  type="text"
                  value={String(asRecord(heartbeat.active_hours).start || '')}
                />
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">heartbeat.active_hours.end</span>
                <input
                  onChange={(event) => {
                    onPatchHeartbeat({ activeHoursEnd: event.target.value });
                  }}
                  placeholder="23:00"
                  type="text"
                  value={String(asRecord(heartbeat.active_hours).end || '')}
                />
              </label>
              <label className="web-settings-field web-settings-field-span">
                <span className="eyebrow">heartbeat.active_hours.timezone</span>
                <input
                  onChange={(event) => {
                    onPatchHeartbeat({ activeHoursTimezone: event.target.value });
                  }}
                  placeholder="user"
                  type="text"
                  value={String(asRecord(heartbeat.active_hours).timezone || '')}
                />
              </label>
              <label className="web-settings-field web-settings-field-span">
                <span className="eyebrow">sessions.data_dir</span>
                <input
                  onChange={(event) => {
                    onPatchSessions({ dataDir: event.target.value });
                  }}
                  placeholder="/path/to/thread-history"
                  type="text"
                  value={String(sessions.data_dir || '')}
                />
              </label>
              <label className="web-settings-field">
                <span className="eyebrow">sessions.thread_history_backend</span>
                <select
                  onChange={(event) => {
                    onPatchSessions({
                      backend: event.target.value as GatewayThreadHistoryBackend,
                    });
                  }}
                  value={threadHistoryBackend}
                >
                  <option value="transcript_v1">transcript_v1</option>
                  <option value="inline_messages">inline_messages</option>
                </select>
              </label>
                </div>
              </>
            ) : null}
            <section className="thread-history-panel">
              <div className="thread-history-toolbar">
                <div className="thread-history-toolbar-copy">
                  <span className="eyebrow">Telegram</span>
                  <p>{visibleTelegramEntries.length || (hasRuntimeBotSummary && focusedChannel === 'telegram' ? 1 : 0)} configured accounts</p>
                </div>
                {!isScopedBotSettings ? (
                  <div className="thread-history-toolbar-actions">
                  <UIButton
                    onClick={() => {
                      const accountId = window.prompt('Telegram account id');
                      if (accountId) {
                        onAddTelegramAccount(accountId);
                      }
                    }}
                    size="sm"
                    type="button"
                  >
                    Add Telegram
                  </UIButton>
                  </div>
                ) : null}
              </div>
              {!visibleTelegramEntries.length ? (
                <div className="empty-state">
                  <span className="eyebrow">Telegram</span>
                  <h3>{isScopedBotSettings ? 'Bot config is runtime-only' : 'No Telegram accounts'}</h3>
                  {hasRuntimeBotSummary && focusedChannel === 'telegram' ? (
                    <p className="small-note">
                      This bot is active at runtime but not present in <code>/api/settings</code>.
                    </p>
                  ) : null}
                </div>
              ) : (
                <div className="web-settings-account-grid">
                  {visibleTelegramEntries.map(([accountId, rawAccount]) => {
                    const envelope = asRecord(rawAccount);
                    const account = {
                      ...asRecord(envelope.config),
                      ...envelope,
                    };
                    return (
                      <article className="web-settings-account-card" key={accountId}>
                        <div className="thread-history-message-meta">
                          <strong>{accountId}</strong>
                          <div className="thread-history-toolbar-actions">
                            {!isScopedBotSettings ? (
                            <UIButton
                              onClick={() => {
                                onRemoveTelegramAccount(accountId);
                              }}
                              size="sm"
                              type="button"
                              variant="outline"
                            >
                              Remove
                            </UIButton>
                            ) : null}
                          </div>
                        </div>
                        <label className="web-settings-check">
                          <input
                            checked={Boolean(account.enabled)}
                            onChange={(event) => {
                              onPatchTelegramAccount(accountId, { enabled: event.target.checked });
                            }}
                            type="checkbox"
                          />
                          <span className="small-note">enabled</span>
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">name</span>
                          <input
                            onChange={(event) => {
                              onPatchTelegramAccount(accountId, { name: event.target.value });
                            }}
                            type="text"
                            value={String(account.name || '')}
                          />
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">token</span>
                          {isScopedBotSettings ? (
                            <div className="web-settings-secret-row">
                              <input readOnly type="text" value={String(account.token || '')} />
                              <UIButton
                                onClick={() => {
                                  void copySecret(`telegram:${accountId}:token`, String(account.token || ''));
                                }}
                                size="sm"
                                type="button"
                                variant="outline"
                              >
                                {copiedSecretKey === `telegram:${accountId}:token` ? 'Copied' : 'Copy'}
                              </UIButton>
                            </div>
                          ) : (
                          <input
                            onChange={(event) => {
                              onPatchTelegramAccount(accountId, { token: event.target.value });
                            }}
                            type="text"
                            value={String(account.token || '')}
                          />
                          )}
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">agent_id</span>
                          <input
                            onChange={(event) => {
                              onPatchTelegramAccount(accountId, { agentId: event.target.value });
                            }}
                            type="text"
                            value={String(account.agent_id || 'claude')}
                          />
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">workspace_dir</span>
                          <input
                            onChange={(event) => {
                              onPatchTelegramAccount(accountId, { workspaceDir: event.target.value });
                            }}
                            type="text"
                            value={String(account.workspace_dir || '')}
                          />
                        </label>
                      </article>
                    );
                  })}
                </div>
              )}
              {isScopedBotSettings && runtimeBotSummary && focusedChannel === 'telegram' && !visibleTelegramEntries.length ? (
                <article className="web-settings-account-card">
                  <div className="thread-history-message-meta">
                    <strong>{runtimeBotSummary.accountId}</strong>
                    <UIBadge className={runtimeBotSummary.status === 'connected' ? 'is-connected' : 'is-idle'}>
                      {runtimeBotSummary.status}
                    </UIBadge>
                  </div>
                  <p className="small-note">
                    Runtime profile from bot console endpoint.
                  </p>
                  <label className="web-settings-field">
                    <span className="eyebrow">workspace_dir</span>
                    <input readOnly type="text" value={runtimeBotSummary.workspaceDir || ''} />
                  </label>
                  <label className="web-settings-field">
                    <span className="eyebrow">endpoints</span>
                    <input
                      readOnly
                      type="text"
                      value={`${runtimeBotSummary.boundEndpointCount}/${runtimeBotSummary.endpointCount} bound`}
                    />
                  </label>
                </article>
              ) : null}
            </section>
            <section className="thread-history-panel">
              <div className="thread-history-toolbar">
                <div className="thread-history-toolbar-copy">
                  <span className="eyebrow">Feishu</span>
                  <p>{visibleFeishuEntries.length || (hasRuntimeBotSummary && focusedChannel === 'feishu' ? 1 : 0)} configured accounts</p>
                </div>
                {!isScopedBotSettings ? (
                  <div className="thread-history-toolbar-actions">
                  <UIButton
                    onClick={() => {
                      const accountId = window.prompt('Feishu account id');
                      if (accountId) {
                        onAddFeishuAccount(accountId);
                      }
                    }}
                    size="sm"
                    type="button"
                  >
                    Add Feishu
                  </UIButton>
                  </div>
                ) : null}
              </div>
              {!visibleFeishuEntries.length ? (
                <div className="empty-state">
                  <span className="eyebrow">Feishu</span>
                  <h3>{isScopedBotSettings ? 'Bot config is runtime-only' : 'No Feishu accounts'}</h3>
                </div>
              ) : (
                <div className="web-settings-account-grid">
                  {visibleFeishuEntries.map(([accountId, rawAccount]) => {
                    const envelope = asRecord(rawAccount);
                    const account = {
                      ...asRecord(envelope.config),
                      ...envelope,
                    };
                    return (
                      <article className="web-settings-account-card" key={accountId}>
                        <div className="thread-history-message-meta">
                          <strong>{accountId}</strong>
                          {!isScopedBotSettings ? (
                          <UIButton
                            onClick={() => {
                              onRemoveFeishuAccount(accountId);
                            }}
                            size="sm"
                            type="button"
                            variant="outline"
                          >
                            Remove
                          </UIButton>
                          ) : null}
                        </div>
                        <label className="web-settings-check">
                          <input
                            checked={Boolean(account.enabled)}
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { enabled: event.target.checked });
                            }}
                            type="checkbox"
                          />
                          <span className="small-note">enabled</span>
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">name</span>
                          <input
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { name: event.target.value });
                            }}
                            type="text"
                            value={String(account.name || '')}
                          />
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">app_id</span>
                          {isScopedBotSettings ? (
                            <div className="web-settings-secret-row">
                              <input readOnly type="text" value={String(account.app_id || '')} />
                              <UIButton
                                onClick={() => {
                                  void copySecret(`feishu:${accountId}:app_id`, String(account.app_id || ''));
                                }}
                                size="sm"
                                type="button"
                                variant="outline"
                              >
                                {copiedSecretKey === `feishu:${accountId}:app_id` ? 'Copied' : 'Copy'}
                              </UIButton>
                            </div>
                          ) : (
                          <input
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { appId: event.target.value });
                            }}
                            type="text"
                            value={String(account.app_id || '')}
                          />
                          )}
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">app_secret</span>
                          {isScopedBotSettings ? (
                            <div className="web-settings-secret-row">
                              <input readOnly type="text" value={String(account.app_secret || '')} />
                              <UIButton
                                onClick={() => {
                                  void copySecret(`feishu:${accountId}:app_secret`, String(account.app_secret || ''));
                                }}
                                size="sm"
                                type="button"
                                variant="outline"
                              >
                                {copiedSecretKey === `feishu:${accountId}:app_secret` ? 'Copied' : 'Copy'}
                              </UIButton>
                            </div>
                          ) : (
                          <input
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { appSecret: event.target.value });
                            }}
                            type="text"
                            value={String(account.app_secret || '')}
                          />
                          )}
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">domain</span>
                          <select
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { domain: event.target.value });
                            }}
                            value={String(account.domain || 'feishu')}
                          >
                            <option value="feishu">feishu</option>
                            <option value="lark">lark</option>
                          </select>
                        </label>
                        <label className="web-settings-check">
                          <input
                            checked={Boolean(account.require_mention)}
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { requireMention: event.target.checked });
                            }}
                            type="checkbox"
                          />
                          <span className="small-note">require_mention</span>
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">topic_session_mode</span>
                          <select
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { topicSessionMode: event.target.value });
                            }}
                            value={String(account.topic_session_mode || 'disabled')}
                          >
                            <option value="disabled">group</option>
                            <option value="enabled">topic</option>
                          </select>
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">agent_id</span>
                          <input
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { agentId: event.target.value });
                            }}
                            type="text"
                            value={String(account.agent_id || 'claude')}
                          />
                        </label>
                        <label className="web-settings-field">
                          <span className="eyebrow">workspace_dir</span>
                          <input
                            onChange={(event) => {
                              onPatchFeishuAccount(accountId, { workspaceDir: event.target.value });
                            }}
                            type="text"
                            value={String(account.workspace_dir || '')}
                          />
                        </label>
                      </article>
                    );
                  })}
                </div>
              )}
              {isScopedBotSettings && runtimeBotSummary && focusedChannel === 'feishu' && !visibleFeishuEntries.length ? (
                <article className="web-settings-account-card">
                  <div className="thread-history-message-meta">
                    <strong>{runtimeBotSummary.accountId}</strong>
                    <UIBadge className={runtimeBotSummary.status === 'connected' ? 'is-connected' : 'is-idle'}>
                      {runtimeBotSummary.status}
                    </UIBadge>
                  </div>
                  <p className="small-note">
                    Runtime profile from bot console endpoint.
                  </p>
                  <label className="web-settings-field">
                    <span className="eyebrow">workspace_dir</span>
                    <input readOnly type="text" value={runtimeBotSummary.workspaceDir || ''} />
                  </label>
                  <label className="web-settings-field">
                    <span className="eyebrow">endpoints</span>
                    <input
                      readOnly
                      type="text"
                      value={`${runtimeBotSummary.boundEndpointCount}/${runtimeBotSummary.endpointCount} bound`}
                    />
                  </label>
                </article>
              ) : null}
            </section>
            {!isScopedBotSettings ? (
            <textarea
              className="web-settings-editor"
              onChange={(event) => {
                onChangeJson(event.target.value);
              }}
              spellCheck={false}
              value={jsonDraft}
            />
            ) : null}
          </section>
        )}
      </div>
    </div>
  );
}
