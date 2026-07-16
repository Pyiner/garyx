import { useMemo, useState } from 'react';
import { Plus, Trash } from 'lucide-react';

import type {
  ChannelPluginCatalogEntry,
  DesktopCustomAgent,
  DesktopWorkspace,
  GatewaySettingsSource,
} from '@shared/contracts';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Switch } from '@/components/ui/switch';
import { buildAgentTargetOptions, type AgentTargetOption } from '../app-shell/agent-options';
import { AddBotDialog } from '../app-shell/components/AddBotDialog';
import { AgentOptionAvatar } from '../app-shell/components/AgentOptionAvatar';
import { EditBotDialog, type EditBotDialogContext } from '../app-shell/components/EditBotDialog';
import { MoreDotsIcon } from '../app-shell/icons';
import { ChannelPluginCatalogPanel } from '../channel-plugins/ChannelPluginCatalogPanel';
import { useChannelPluginCatalog } from '../channel-plugins/useChannelPluginCatalog';
import { useI18n } from '../i18n';
import { classNames, noopAsync } from './shared';

type DraftMutator = (mutator: (nextConfig: any) => void) => void;
type GatewaySettingsSaveOptions = {
  silent?: boolean;
  refreshDesktopState?: 'await' | 'background' | 'skip';
};

function configuredChannelAccountsFromDraft(
  channels: unknown,
): Array<{ kind: string; accountId: string; account: any }> {
  if (!channels || typeof channels !== 'object' || Array.isArray(channels)) {
    return [];
  }

  const channelMap = channels as Record<string, any>;
  return Object.entries(channelMap)
    .filter(([kind]) => kind !== 'api' && kind !== 'plugins')
    .flatMap(([kind, channelValue]) => {
      const accounts =
        channelValue && typeof channelValue === 'object' && !Array.isArray(channelValue)
          ? channelValue.accounts
          : null;
      if (!accounts || typeof accounts !== 'object' || Array.isArray(accounts)) {
        return [];
      }
      return Object.entries(accounts).map(([accountId, account]) => ({
        kind,
        accountId,
        account,
      }));
    });
}

function sortedAgentTargets(
  agents: DesktopCustomAgent[],
): AgentTargetOption[] {
  return buildAgentTargetOptions(agents);
}

function resolveChannelAgentId(
  provider: any,
): string | null {
  const explicitAgentId = typeof provider?.agent_id === 'string'
    ? provider.agent_id.trim()
    : '';
  return explicitAgentId || null;
}

function compactAgentTargetLabel(target: AgentTargetOption | null, fallback: string): string {
  if (!target) return fallback;
  const withoutProvider = target.label.split(' · ')[0]?.trim() || target.label;
  const valuePattern = escapeRegExp(target.value);
  return withoutProvider
    .replace(new RegExp(`\\s*\\(${valuePattern}\\)$`), '')
    .trim()
    || withoutProvider;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

type ChannelsSettingsPanelProps = {
  agents?: DesktopCustomAgent[];
  effectiveDefaultAgentId?: string | null;
  workspaces?: DesktopWorkspace[];
  gatewayDraft?: any;
  gatewaySaving?: boolean;
  gatewaySettingsSource?: GatewaySettingsSource;
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onMutateGatewayDraft?: DraftMutator;
  onSaveGatewaySettings?: (options?: GatewaySettingsSaveOptions) => Promise<boolean>;
  onRefreshAgentTargets?: () => Promise<void>;
  onAddChannelAccount?: (input: {
    channel: string;
    accountId: string;
    name?: string | null;
    workspaceDir?: string | null;
    workspaceMode?: 'local' | 'worktree';
    agentId?: string | null;
    token?: string | null;
    appId?: string | null;
    appSecret?: string | null;
    baseUrl?: string | null;
    domain?: 'feishu' | 'lark' | null;
    config?: Record<string, unknown> | null;
  }) => Promise<void>;
};

export function ChannelsSettingsPanel({
  agents = [],
  effectiveDefaultAgentId = null,
  workspaces = [],
  gatewayDraft,
  gatewaySaving = false,
  gatewaySettingsSource,
  onAddWorkspace = async () => null,
  onMutateGatewayDraft = () => {},
  onSaveGatewaySettings = async () => true,
  onRefreshAgentTargets = noopAsync,
  onAddChannelAccount = noopAsync,
}: ChannelsSettingsPanelProps) {
  const { t } = useI18n();
  const pluginAccounts = configuredChannelAccountsFromDraft(gatewayDraft?.channels);
  const [isAddingChannel, setIsAddingChannel] = useState(false);
  const [editingBot, setEditingBot] = useState<EditBotDialogContext | null>(null);
  const agentTargets = sortedAgentTargets(agents);
  // channel. Degrades gracefully to an empty list before the first
  // fetch returns (all hardcoded logic still works).
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const pluginCatalogById = useMemo(() => {
    const map: Record<string, ChannelPluginCatalogEntry> = {};
    (pluginCatalog || []).forEach((entry) => {
      map[entry.id] = entry;
    });
    return map;
  }, [pluginCatalog]);
  const channelsSourceMessage = gatewaySettingsSource === 'local_file'
    ? t('Editing the gateway runtime config file on the gateway host.')
    : t('Editing live gateway settings over the API.');
  const configuredChannels = pluginAccounts;

  async function handleDeleteBotAccount(
    kind: string,
    accountId: string,
    displayName: string,
  ) {
    if (!window.confirm(t('Delete {kind} account "{name}"?', { kind, name: displayName || accountId }))) {
      return;
    }
    onMutateGatewayDraft((next) => {
      next.channels = next.channels || {};
      if (!next.channels?.[kind]?.accounts) return;
      delete next.channels[kind].accounts[accountId];
    });
    if (editingBot?.kind === kind && editingBot.accountId === accountId) {
      setEditingBot(null);
    }
    await onSaveGatewaySettings();
  }

  return (
    <>
      <ChannelPluginCatalogPanel />
      <section className="codex-section bot-panel">
        <div className="codex-section-header">
          <div className="bot-panel-title-row">
            <span className="codex-section-title">{t('Bots')}</span>
            <span className="bot-panel-count">{configuredChannels.length}</span>
            <span className="bot-panel-source">{channelsSourceMessage}</span>
          </div>
          <button
            className="codex-section-action"
            onClick={() => {
              void (async () => {
                await onRefreshAgentTargets();
                setIsAddingChannel(true);
              })();
            }}
            type="button"
          >
            <Plus aria-hidden size={14} />
            {t('Add bot')}
          </button>
        </div>
        {!configuredChannels.length ? (
          <div className="bot-panel-empty">{t('No bots configured. Click Add bot above to create one.')}</div>
        ) : (
          <div className="bot-table">
            <div className="bot-table-head">
              <span className="bot-table-col bot-table-col-name">{t('Name')}</span>
              <span className="bot-table-col bot-table-col-channel">{t('Channel')}</span>
              <span className="bot-table-col bot-table-col-account">{t('Account')}</span>
              <span className="bot-table-col bot-table-col-agent">{t('Agent')}</span>
              <span className="bot-table-col bot-table-col-status">{t('Enabled')}</span>
              <span className="bot-table-col bot-table-col-actions">{t('Actions')}</span>
            </div>
            {configuredChannels.map(({ kind, accountId, account }) => {
              const accountAgentId = resolveChannelAgentId(account);
              const selectedTarget = agentTargets.find((target) => target.value === accountAgentId) || null;
              const effectiveTarget = agentTargets.find(
                (target) => target.value === effectiveDefaultAgentId,
              ) || null;
              const selectedAgentMissing = Boolean(accountAgentId && !selectedTarget);
              const agentLabel = accountAgentId
                ? (selectedTarget?.label || accountAgentId)
                : effectiveTarget
                  ? t('Follow global default (currently {agent})', { agent: effectiveTarget.label })
                  : t('Follow global default (currently no enabled agent)');
              const agentDisplayName = compactAgentTargetLabel(
                selectedTarget || effectiveTarget,
                accountAgentId || t('Follow global default'),
              );
              const displayName = String(account?.name || accountId);
              const enabled = Boolean(account?.enabled);
              // Catalog-driven channel presentation: show the
              // plugin-supplied icon (or a colour-consistent letter
              // fallback) and the display_name from the gateway's
              // schema catalog. Falls back to the raw `kind` string
              // while the catalog is still loading on first paint.
              const channelMeta = pluginCatalogById[kind];
              const channelLabel = channelMeta?.display_name || kind;
              const channelIcon = channelMeta?.icon_data_url;
              const openEditor = () => {
                setEditingBot({
                  kind,
                  accountId,
                  account,
                  agentId: accountAgentId,
                });
              };
              const toggleEnabled = (nextEnabled: boolean) => {
                onMutateGatewayDraft((next) => {
                  next.channels = next.channels || {};
                  next.channels[kind] = next.channels[kind] || {};
                  next.channels[kind].accounts = next.channels[kind].accounts || {};
                  const draftAccount = next.channels[kind].accounts[accountId];
                  if (!draftAccount) return;
                  draftAccount.enabled = nextEnabled;
                });
                void onSaveGatewaySettings();
              };
              return (
                <div
                  className="bot-table-row"
                  key={`${kind}:${accountId}`}
                  onClick={openEditor}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault();
                      openEditor();
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <span className="bot-table-cell bot-table-col-name">{displayName}</span>
                  <span className="bot-table-cell bot-table-col-channel">
                    <span className="bot-table-channel-chip" title={channelLabel}>
                      {channelIcon ? (
                        <img
                          src={channelIcon}
                          alt=""
                          className="bot-table-channel-icon"
                          width={16}
                          height={16}
                        />
                      ) : (
                        <span
                          className="bot-table-channel-icon-fallback"
                          aria-hidden
                        >
                          {channelLabel.charAt(0).toUpperCase()}
                        </span>
                      )}
                      <span className="bot-table-channel-label">{channelLabel}</span>
                    </span>
                  </span>
                  <span className="bot-table-cell bot-table-col-account" title={accountId}>{accountId}</span>
                  <span
                    className={classNames('bot-table-cell bot-table-col-agent', selectedAgentMissing && 'missing')}
                    title={agentLabel}
                  >
                    <span className="bot-table-agent">
                      <AgentOptionAvatar
                        agentId={selectedTarget?.value || effectiveTarget?.value || accountAgentId}
                        avatarDataUrl={selectedTarget?.avatarDataUrl || effectiveTarget?.avatarDataUrl}
                        className={selectedAgentMissing ? 'agent-option-avatar--missing' : undefined}
                        kind={selectedTarget?.kind || effectiveTarget?.kind || 'agent'}
                        label={agentDisplayName}
                        providerIcon={selectedTarget?.providerIcon || effectiveTarget?.providerIcon}
                        providerType={selectedTarget?.providerType || effectiveTarget?.providerType}
                      />
                      <span className="bot-table-agent-copy">
                        <span className="bot-table-agent-name">{agentDisplayName}</span>
                        <span className="bot-table-agent-id">
                          {accountAgentId || t('Follow global')}
                        </span>
                      </span>
                    </span>
                  </span>
                  <span className="bot-table-cell bot-table-col-status">
                    <Switch
                      aria-label={t('{name} enabled', { name: displayName })}
                      checked={enabled}
                      disabled={gatewaySaving}
                      onCheckedChange={toggleEnabled}
                      onClick={(event) => {
                        event.stopPropagation();
                      }}
                      onKeyDown={(event) => {
                        event.stopPropagation();
                      }}
                    />
                  </span>
                  <span
                    className="bot-table-cell bot-table-col-actions"
                    onClick={(event) => {
                      event.stopPropagation();
                    }}
                    onKeyDown={(event) => {
                      event.stopPropagation();
                    }}
                  >
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <button
                          aria-label={t('More actions for {name}', { name: displayName })}
                          className="bot-table-action-button"
                          disabled={gatewaySaving}
                          type="button"
                        >
                          <MoreDotsIcon size={14} />
                        </button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          disabled={gatewaySaving}
                          onSelect={() => {
                            void handleDeleteBotAccount(kind, accountId, displayName);
                          }}
                          variant="destructive"
                        >
                          <Trash aria-hidden />
                          {t('Delete')}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </section>
      <EditBotDialog
        agentTargets={agentTargets}
        effectiveDefaultAgentId={effectiveDefaultAgentId}
        context={editingBot}
        onAddWorkspace={onAddWorkspace}
        onClose={() => setEditingBot(null)}
        onSave={async ({ kind, accountId, patch }) => {
          onMutateGatewayDraft((next) => {
            next.channels = next.channels || {};
            next.channels[kind] = next.channels[kind] || {};
            next.channels[kind].accounts = next.channels[kind].accounts || {};
            const account = next.channels[kind].accounts[accountId];
            if (!account) return;
            const finalAccountId = patch.nextAccountId?.trim() || accountId;
            if (patch.name !== undefined) account.name = patch.name;
            if (patch.workspaceDir !== undefined) account.workspace_dir = patch.workspaceDir;
            if (patch.workspaceMode !== undefined) account.workspace_mode = patch.workspaceMode;
            if (patch.agentId !== undefined) account.agent_id = patch.agentId;
            if (patch.config !== undefined) account.config = patch.config;
            if (finalAccountId !== accountId) {
              next.channels[kind].accounts[finalAccountId] = account;
              delete next.channels[kind].accounts[accountId];
            }
          });
          await onSaveGatewaySettings({ refreshDesktopState: 'background' });
        }}
        open={Boolean(editingBot)}
        saving={gatewaySaving}
        workspaces={workspaces}
      />
      <AddBotDialog
        agentTargets={agentTargets}
        effectiveDefaultAgentId={effectiveDefaultAgentId}
        onAddWorkspace={onAddWorkspace}
        onClose={() => {
          setIsAddingChannel(false);
        }}
        onCreateChannel={onAddChannelAccount}
        open={isAddingChannel}
        workspaces={workspaces}
      />
    </>
  );
}
