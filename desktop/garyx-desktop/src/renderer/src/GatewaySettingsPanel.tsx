import { useEffect, useMemo, useState } from 'react';
import type { FormEvent, ReactNode } from 'react';

import {
  DEFAULT_DESKTOP_SETTINGS,
  type ChannelPluginCatalogEntry,
  type ConnectionStatus,
  type DesktopCustomAgent,
  type DesktopGatewayProfile,
  type DesktopTeam,
  type DesktopSettings,
  type DesktopMcpServer,
  type DesktopSkillInfo,
  type GatewaySettingsSource,
  type GatewayThreadHistoryBackend,
  type McpTransportType,
  type SlashCommand,
  type UpdateMcpServerInput,
  type UpdateSlashCommandInput,
  type UpsertMcpServerInput,
  type UpsertSlashCommandInput,
  type PollWeixinChannelAuthResult,
  type StartFeishuChannelAuthInput,
  type StartFeishuChannelAuthResult,
  type PollFeishuChannelAuthInput,
  type PollFeishuChannelAuthResult,
} from '@shared/contracts';

import {
  defaultChannelAgentId,
  GatewaySettingsMode,
} from '@renderer/gateway-settings';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Textarea } from '@/components/ui/textarea';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { GatewayProfileHistoryButton } from './GatewayProfileHistoryButton';
import { AddBotDialog } from './app-shell/components/AddBotDialog';
import { ChannelPluginCatalogPanel } from './channel-plugins/ChannelPluginCatalogPanel';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { EditBotDialog, type EditBotDialogContext, type EditBotPatch } from './app-shell/components/EditBotDialog';

type DraftMutator = (mutator: (nextConfig: any) => void) => void;
type GatewaySettingsPanelProps = {
  activeTab: SettingsTabId;
  commands?: SlashCommand[];
  commandsLoading?: boolean;
  commandsSaving?: boolean;
  connection?: ConnectionStatus | null;
  localSettings?: DesktopSettings;
  gatewayProfiles?: DesktopGatewayProfile[];
  localSettingsDirty?: boolean;
  mcpServers?: DesktopMcpServer[];
  mcpServersLoading?: boolean;
  mcpServersSaving?: boolean;
  gatewayDraft?: any;
  gatewayDirty?: boolean;
  gatewayLoading?: boolean;
  gatewaySaving?: boolean;
  gatewayMode?: GatewaySettingsMode;
  gatewaySettingsSource?: GatewaySettingsSource;
  gatewayStatusMessage?: string | null;
  gatewayJsonDraft?: string;
  gatewayJsonError?: string | null;
  savingLocalSettings?: boolean;
  agents?: DesktopCustomAgent[];
  teams?: DesktopTeam[];
  skills?: DesktopSkillInfo[];
  onCreateSlashCommand?: (input: UpsertSlashCommandInput) => Promise<void>;
  onUpdateSlashCommand?: (input: UpdateSlashCommandInput) => Promise<void>;
  onDeleteSlashCommand?: (name: string) => Promise<void>;
  onCreateMcpServer?: (input: UpsertMcpServerInput) => Promise<void>;
  onUpdateMcpServer?: (input: UpdateMcpServerInput) => Promise<void>;
  onDeleteMcpServer?: (name: string) => Promise<void>;
  onToggleMcpServer?: (name: string, enabled: boolean) => Promise<void>;
  onLocalSettingsChange?: (mutator: (current: DesktopSettings) => DesktopSettings) => void;
  onSaveLocalSettings?: (event: FormEvent<HTMLFormElement>) => void;
  onSaveLocalSettingsNow?: (options?: {
    requireGatewayConnection?: boolean;
    reloadGatewaySettings?: boolean;
  }) => Promise<boolean>;
  onGatewayJsonChange?: (value: string) => void;
  onSaveGatewaySettings?: () => Promise<boolean>;
  onMutateGatewayDraft?: DraftMutator;
  onOpenAdvancedJson?: () => void;
  onRefreshAgentTargets?: () => Promise<void>;
  onAddChannelAccount?: (input: {
    channel: string;
    accountId: string;
    name?: string | null;
    workspaceDir?: string | null;
    agentId?: string | null;
    token?: string | null;
    appId?: string | null;
    appSecret?: string | null;
    baseUrl?: string | null;
    domain?: 'feishu' | 'lark' | null;
    config?: Record<string, unknown> | null;
  }) => Promise<void>;
  onStartWeixinChannelAuth?: (input: {
    accountId?: string | null;
    name?: string | null;
    workspaceDir?: string | null;
    baseUrl?: string | null;
  }) => Promise<{ sessionId: string; qrCodeDataUrl: string }>;
  onPollWeixinChannelAuth?: (input: { sessionId: string }) => Promise<PollWeixinChannelAuthResult>;
  onStartFeishuChannelAuth?: (
    input: StartFeishuChannelAuthInput,
  ) => Promise<StartFeishuChannelAuthResult>;
  onPollFeishuChannelAuth?: (
    input: PollFeishuChannelAuthInput,
  ) => Promise<PollFeishuChannelAuthResult>;
};

type AgentProviderFieldsProps = {
  provider: any;
  onOpenAdvancedJson: () => void;
  onMutate: (mutator: (provider: any) => void) => void;
};

type AgentTargetOption = {
  value: string;
  label: string;
};

type CommandDraft = UpsertSlashCommandInput;
type McpServerDraft = {
  name: string;
  transport: McpTransportType;
  // STDIO
  command: string;
  args: string[];
  envEntries: Array<{ key: string; value: string }>;
  workingDir: string;
  // Streamable HTTP
  url: string;
  headerEntries: Array<{ key: string; value: string }>;
  // Common
  enabled: boolean;
};

const SLASH_COMMAND_NAME_PATTERN = /^[a-z0-9_]{1,32}$/;
const noop = () => {};
const noopAsync = async () => {};
const noopAsyncBoolean = async () => false;

export type SettingsTabId =
  | 'connection'
  | 'gateway'
  | 'provider'
  | 'channels'
  | 'labs'
  | 'commands'
  | 'mcp'
  | 'advanced';

export const SETTINGS_TABS: Array<{
  id: SettingsTabId;
  label: string;
  eyebrow: string;
  description: string;
}> = [
  {
    id: 'gateway',
    label: 'Gateway',
    eyebrow: 'Gateway',
    description: 'Gateway URL, runtime, storage, image generation, and heartbeat defaults.',
  },
  {
    id: 'provider',
    label: 'Provider',
    eyebrow: 'Providers',
    description: 'Desktop-side Claude env overrides and Codex auth.',
  },
  {
    id: 'channels',
    label: 'Channels',
    eyebrow: 'Bots',
    description: 'Telegram and Feishu/Lark bot accounts.',
  },
  {
    id: 'labs',
    label: 'Labs',
    eyebrow: 'Mac Labs',
    description: 'Mac-only experimental surfaces that can be hidden without affecting your other Garyx workflows.',
  },
  {
    id: 'commands',
    label: 'Commands',
    eyebrow: 'Slash Commands',
    description: 'Manage global prompt shortcuts.',
  },
  {
    id: 'mcp',
    label: 'MCP Servers',
    eyebrow: 'MCP',
    description: 'Manage external MCP server definitions and local tool config sync.',
  },
  {
    id: 'advanced',
    label: 'Advanced',
    eyebrow: 'JSON',
    description: '',
  },
];

type SummaryItem = {
  label: string;
  value: string;
};

type SummaryChipProps = SummaryItem;
type SettingsFactTone = 'default' | 'success' | 'danger';

type SettingsSectionProps = {
  eyebrow: string;
  title: string;
  description?: string;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
};

type SettingsFactProps = {
  label: string;
  value: string;
  tone?: SettingsFactTone;
};

type SettingsSummaryRowProps = {
  label: string;
  value: string;
  details: string[];
  tone?: SettingsFactTone;
};

type SettingsSurfaceProps = {
  title?: string;
  note?: ReactNode;
  children: ReactNode;
  className?: string;
};

type SettingsControlRowProps = {
  label: string;
  description?: string;
  control: ReactNode;
  stacked?: boolean;
  className?: string;
};

type SettingsSwitchProps = {
  checked: boolean;
  label: string;
  onChange: (nextValue: boolean) => void;
};

type ChannelAccountCardProps = {
  accountId: string;
  enabled: boolean;
  provider: any;
  summaries: SummaryItem[];
  onRemove: () => void;
  children: ReactNode;
};

function providerTypeValue(provider: any): string {
  return String(provider?.provider_type || 'claude_code');
}

function providerTypeLabel(provider: any): string {
  const value = providerTypeValue(provider);
  if (value === 'codex_app_server') {
    return 'codex';
  }
  if (value === 'gemini_cli') {
    return 'gemini';
  }
  return 'claude';
}

function agentProviderLabel(providerType: DesktopCustomAgent['providerType']): string {
  if (providerType === 'codex_app_server') {
    return 'Codex';
  }
  if (providerType === 'gemini_cli') {
    return 'Gemini';
  }
  return 'Claude';
}

function sortedStandaloneAgents(agents: DesktopCustomAgent[]): DesktopCustomAgent[] {
  return [...agents]
    .filter((agent) => agent.standalone)
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
    });
}

function preferredStandaloneAgentId(
  agents: DesktopCustomAgent[],
  providerType?: DesktopCustomAgent['providerType'] | string | null,
): string {
  if (!agents.length) {
    return '';
  }

  const normalizedProviderType =
    providerType === 'codex_app_server'
      ? 'codex_app_server'
      : providerType === 'gemini_cli'
        ? 'gemini_cli'
        : 'claude_code';
  const builtInId =
    normalizedProviderType === 'codex_app_server'
      ? 'codex'
      : normalizedProviderType === 'gemini_cli'
        ? 'gemini'
        : 'claude';

  return agents.find((agent) => agent.agentId === builtInId)?.agentId
    || agents.find((agent) => agent.providerType === normalizedProviderType)?.agentId
    || agents[0]?.agentId
    || '';
}

function formatAgentOptionLabel(agent: DesktopCustomAgent): string {
  return agent.displayName.trim() === agent.agentId.trim()
    ? agent.displayName
    : `${agent.displayName} (${agent.agentId})`;
}

function formatTeamOptionLabel(team: DesktopTeam): string {
  return team.displayName.trim() === team.teamId.trim()
    ? `${team.displayName} (team)`
    : `${team.displayName} (${team.teamId}, team)`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function compactAgentTargetLabel(target: AgentTargetOption | null, fallback: string): string {
  if (!target) return fallback;
  const withoutProvider = target.label.split(' · ')[0]?.trim() || target.label;
  const valuePattern = escapeRegExp(target.value);
  return withoutProvider
    .replace(new RegExp(`\\s*\\(${valuePattern}(?:, team)?\\)$`), '')
    .replace(/\s+\(team\)$/, '')
    .trim()
    || withoutProvider;
}

function sortedAgentTargets(
  agents: DesktopCustomAgent[],
  teams: DesktopTeam[],
): AgentTargetOption[] {
  const teamOptions = [...teams]
    .sort((left, right) => {
      return left.displayName.localeCompare(right.displayName) || left.teamId.localeCompare(right.teamId);
    })
    .map((team) => ({
      value: team.teamId,
      label: formatTeamOptionLabel(team),
    }));
  const agentOptions = sortedStandaloneAgents(agents).map((agent) => ({
    value: agent.agentId,
    label: `${formatAgentOptionLabel(agent)} · ${agentProviderLabel(agent.providerType)}`,
  }));
  return [...teamOptions, ...agentOptions];
}

function bindingAgentId(binding: any): string | null {
  const raw = typeof binding?.agentId === 'string'
    ? binding.agentId
    : typeof binding?.agent_id === 'string'
      ? binding.agent_id
      : '';
  const normalized = raw.trim();
  return normalized || null;
}

function bindingMatch(binding: any): Record<string, any> {
  return binding && typeof binding === 'object' && binding.match && typeof binding.match === 'object'
    ? binding.match as Record<string, any>
    : {};
}

function isAccountAgentBinding(binding: any, channel: string, accountId: string): boolean {
  if (!binding || typeof binding !== 'object' || Array.isArray(binding)) {
    return false;
  }

  const match = bindingMatch(binding);
  const bindingChannel = typeof match.channel === 'string' ? match.channel.trim() : '';
  const bindingAccountId = typeof match.accountId === 'string'
    ? match.accountId.trim()
    : typeof match.account_id === 'string'
      ? match.account_id.trim()
      : '';

  if (bindingChannel !== channel || bindingAccountId !== accountId) {
    return false;
  }

  return !match.peer && !match.guildId && !match.guild_id && !match.teamId && !match.team_id;
}

function findAccountAgentBinding(config: any, channel: string, accountId: string): any | null {
  const bindings = Array.isArray(config?.agents?.bindings) ? config.agents.bindings : [];
  return bindings.find((binding: any) => isAccountAgentBinding(binding, channel, accountId)) || null;
}

function resolveChannelAgentId(
  config: any,
  channel: string,
  accountId: string,
  provider: any,
  agents: DesktopCustomAgent[],
): string {
  const explicitAgentId = typeof provider?.agent_id === 'string'
    ? provider.agent_id.trim()
    : '';
  return explicitAgentId || preferredStandaloneAgentId(agents, 'claude_code');
}

function upsertAccountAgentBinding(
  config: any,
  channel: string,
  accountId: string,
  agentId: string,
): void {
  if (!config.agents || typeof config.agents !== 'object' || Array.isArray(config.agents)) {
    config.agents = {};
  }

  const currentBindings = Array.isArray(config.agents.bindings) ? config.agents.bindings : [];
  const existing = currentBindings.find((binding: any) => isAccountAgentBinding(binding, channel, accountId));
  const priority =
    typeof existing?.priority === 'number' && Number.isFinite(existing.priority) ? existing.priority : 100;

  config.agents.bindings = currentBindings.filter(
    (binding: any) => !isAccountAgentBinding(binding, channel, accountId),
  );

  if (agentId.trim()) {
    config.agents.bindings.push({
      agentId: agentId.trim(),
      match: {
        channel,
        accountId,
      },
      priority,
    });
  }
}

function syncAccountProviderWithTargetId(account: any, targetId: string | null): void {
  account.agent_id = targetId?.trim() || defaultChannelAgentId();
}

function compactPathLabel(value: unknown): string | null {
  const path = typeof value === 'string' ? value.trim() : '';
  if (!path) {
    return null;
  }

  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

function countRecordEntries(value: unknown): number {
  return value && typeof value === 'object' ? Object.keys(value as Record<string, unknown>).length : 0;
}

function summarizeTelegramRules(account: any): SettingsSummaryRowProps {
  const groups = account?.groups && typeof account.groups === 'object'
    ? Object.values(account.groups as Record<string, any>)
    : [];
  const groupRuleCount = groups.length;
  const topicRuleCount = groups.reduce((count, groupConfig) => {
    return count + countRecordEntries(groupConfig?.topics);
  }, 0);

  const details: string[] = [];
  if (groupRuleCount > 0) {
    details.push(`${groupRuleCount} group override${groupRuleCount === 1 ? '' : 's'}`);
  }
  if (topicRuleCount > 0) {
    details.push(`${topicRuleCount} topic override${topicRuleCount === 1 ? '' : 's'}`);
  }

  return {
    label: 'Advanced rules',
    value: details.length ? details[0] : 'Defaults only',
    details: details.length
      ? details
      : ['No inline button allowlists or group/topic overrides configured.'],
  };
}

function summarizeFeishuRules(account: any): SettingsSummaryRowProps {
  const requireMention = account?.require_mention !== false;
  const topicMode = String(account?.topic_session_mode || 'disabled');
  const details = [
    requireMention ? 'Requires @mention in group chats' : 'Responds to group chats without @mention',
    topicMode === 'enabled' ? 'Group chats are split by topic' : 'Each group chat uses one session',
  ];

  return {
    label: 'Group behavior',
    value: details[0],
    details,
  };
}


function countNonEmptyLines(value: string): number {
  return value
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith('#')).length;
}

function emptyCommandDraft(): CommandDraft {
  return {
    name: '',
    description: '',
    prompt: '',
  };
}

function commandDraftFromValue(command: SlashCommand): CommandDraft {
  return {
    name: command.name,
    description: command.description,
    prompt: command.prompt || '',
  };
}

function deriveSlashCommandDescription(prompt: string, name: string): string {
  const normalized = prompt.replace(/\s+/g, ' ').trim();
  if (!normalized) {
    return `/${name}`;
  }
  if (normalized.length <= 80) {
    return normalized;
  }
  return `${normalized.slice(0, 79).trimEnd()}…`;
}

function emptyMcpServerDraft(): McpServerDraft {
  return {
    name: '',
    transport: 'stdio',
    command: '',
    args: [''],
    envEntries: [{ key: '', value: '' }],
    workingDir: '',
    url: '',
    headerEntries: [{ key: '', value: '' }],
    enabled: true,
  };
}

function mcpServerDraftFromValue(server: DesktopMcpServer): McpServerDraft {
  const envEntries = Object.entries(server.env || {});
  const headerEntries = Object.entries(server.headers || {});
  return {
    name: server.name,
    transport: server.transport || 'stdio',
    command: server.command,
    args: server.args.length ? [...server.args] : [''],
    envEntries: envEntries.length
      ? envEntries.map(([key, value]) => ({ key, value }))
      : [{ key: '', value: '' }],
    workingDir: server.workingDir || '',
    url: server.url || '',
    headerEntries: headerEntries.length
      ? headerEntries.map(([key, value]) => ({ key, value }))
      : [{ key: '', value: '' }],
    enabled: server.enabled,
  };
}

function SummaryChip({ label, value }: SummaryChipProps) {
  return (
    <Badge
      variant="outline"
      className="h-auto rounded-full border-[#e7e7e5] bg-[#f7f7f6] px-2 py-0.5 text-[11px] font-normal text-[#40403d] shadow-none"
    >
      <span className="uppercase tracking-[0.08em] text-[#7d7d79]">{label}</span>
      <span>{value}</span>
    </Badge>
  );
}

function classNames(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

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

function SettingsFact({
  label,
  value,
  tone = 'default',
}: SettingsFactProps) {
  return (
    <span className={classNames('settings-fact', tone !== 'default' && `tone-${tone}`)}>
      <span className="settings-fact-label">{label}</span>
      <strong>{value}</strong>
    </span>
  );
}

function SettingsSummaryRow({
  label,
  value,
  details,
  tone = 'default',
}: SettingsSummaryRowProps) {
  const detailText = details.filter(Boolean).join(' · ');
  const title = [label, value, detailText].filter(Boolean).join(' · ');

  return (
    <div
      className={classNames('settings-summary-row', tone !== 'default' && `tone-${tone}`)}
      title={title}
    >
      <span className="settings-summary-row-label">{label}</span>
      <div className="settings-summary-row-content">
        <strong className="settings-summary-row-value">{value}</strong>
        {detailText ? <span className="settings-summary-row-details">{detailText}</span> : null}
      </div>
    </div>
  );
}

function SettingsSwitch({
  checked,
  label,
  onChange,
}: SettingsSwitchProps) {
  return (
    <Switch
      aria-label={label}
      checked={checked}
      onCheckedChange={onChange}
    />
  );
}

function SettingsSurface({
  title,
  note,
  children,
  className,
}: SettingsSurfaceProps) {
  return (
    <div className={classNames('settings-surface-group', className)}>
      {title || note ? (
        <div className="settings-surface-heading">
          {title ? <h4 className="settings-surface-title">{title}</h4> : <span />}
          {note ? <div className="settings-surface-note">{note}</div> : null}
        </div>
      ) : null}
      <div className="settings-surface-list">{children}</div>
    </div>
  );
}

function SettingsControlRow({
  label,
  description,
  control,
  stacked = false,
  className,
}: SettingsControlRowProps) {
  return (
    <div className={classNames('settings-control-row', stacked && 'stacked', className)}>
      <div className="settings-control-row-copy">
        <div className="settings-control-row-label">{label}</div>
        {description ? <p className="settings-control-row-description">{description}</p> : null}
      </div>
      <div className="settings-control-row-control">{control}</div>
    </div>
  );
}

function SettingsSection({
  eyebrow,
  title,
  description,
  aside,
  children,
  className,
}: SettingsSectionProps) {
  return (
    <section className={classNames('panel settings-section', className)}>
      <div className="panel-header settings-section-header">
        <div className="settings-section-copy">
          <span className="eyebrow">{eyebrow}</span>
          <h3 className="settings-section-title">{title}</h3>
          {description ? <p className="small-note">{description}</p> : null}
        </div>
        {aside ? <div className="settings-section-aside">{aside}</div> : null}
      </div>
      <div className="settings-section-body">{children}</div>
    </section>
  );
}

function ChannelAccountCard({
  accountId,
  enabled,
  provider,
  summaries,
  onRemove,
  children,
}: ChannelAccountCardProps) {
  const summaryItems: SummaryItem[] = [
    { label: 'provider', value: providerTypeLabel(provider) },
    ...summaries.filter((item) => item.value.trim().length > 0),
  ];

  return (
    <details className="settings-card settings-account-card settings-collapsible-card">
      <summary className="settings-card-summary">
        <div className="settings-card-summary-main">
          <div className="settings-card-summary-title">
            <strong>{accountId}</strong>
            <span className={`status-pill ${enabled ? '' : 'offline'}`}>
              {enabled ? 'enabled' : 'disabled'}
            </span>
          </div>
          <div className="settings-card-summary-meta">
            {summaryItems.map((item) => (
              <SummaryChip key={`${item.label}:${item.value}`} label={item.label} value={item.value} />
            ))}
          </div>
        </div>
      </summary>
      <div className="settings-card-body">
        <div className="row-between wrap">
          <p className="small-note">Open only when you need detailed config for this bot.</p>
          <Button
            className="rounded-xl border-[#f0d9d9] bg-white text-[#9b3d3d] shadow-none hover:bg-[#fdf3f3]"
            onClick={onRemove}
            size="sm"
            type="button"
            variant="outline"
          >
            Remove
          </Button>
        </div>
        {children}
      </div>
    </details>
  );
}

function AgentProviderFields({
  provider,
  onOpenAdvancedJson,
  onMutate,
}: AgentProviderFieldsProps) {
  const providerType = providerTypeValue(provider);

  return (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">Agent Provider</span>
        <span className="codex-section-note">Provider runtime</span>
      </div>
      <div className="codex-list-card">
        <SettingsControlRow
          control={
            <Select
              value={providerType}
              onValueChange={(value) => {
                onMutate((next) => {
                  next.provider_type = value;
                });
              }}
            >
              <SelectTrigger className="w-full rounded-[14px] border-[#e7e7e5] bg-white text-[13px] shadow-none">
                <SelectValue />
              </SelectTrigger>
              <SelectContent className="rounded-[14px] border-[#e7e7e5] bg-white shadow-[0_12px_32px_rgba(0,0,0,0.08)]">
                <SelectItem value="claude_code">claude_code</SelectItem>
                <SelectItem value="codex_app_server">codex_app_server</SelectItem>
                <SelectItem value="gemini_cli">gemini_cli</SelectItem>
              </SelectContent>
            </Select>
          }
          description="Select the runtime backing this bot."
          label="provider_type"
        />
        <SettingsControlRow
          control={
            <Input
              className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
              value={String(provider?.workspace_dir || '')}
              onChange={(event) => {
                onMutate((next) => {
                  next.workspace_dir = event.target.value.trim() || null;
                });
              }}
            />
          }
          description="Workspace bound to this bot. When the bot creates its first thread, that thread starts in this workspace."
          label="workspace_dir"
          stacked
        />
      </div>
    </div>
  );
}

function providerWorkspaceSummary(provider: any): string {
  return compactPathLabel(provider?.workspace_dir) || 'unset';
}

export function GatewaySettingsPanel({
  activeTab,
  commands = [],
  commandsLoading = false,
  commandsSaving = false,
  connection = null,
  localSettings = DEFAULT_DESKTOP_SETTINGS,
  gatewayProfiles = [],
  localSettingsDirty = false,
  mcpServers = [],
  mcpServersLoading = false,
  mcpServersSaving = false,
  gatewayDraft = {},
  gatewayDirty = false,
  gatewayLoading = false,
  gatewaySaving = false,
  gatewayMode = 'form',
  gatewaySettingsSource = 'gateway_api',
  gatewayStatusMessage = null,
  gatewayJsonDraft = '{}',
  gatewayJsonError = null,
  savingLocalSettings = false,
  agents = [],
  teams = [],
  skills = [],
  onCreateSlashCommand = noopAsync,
  onUpdateSlashCommand = noopAsync,
  onDeleteSlashCommand = noopAsync,
  onCreateMcpServer = noopAsync,
  onUpdateMcpServer = noopAsync,
  onDeleteMcpServer = noopAsync,
  onToggleMcpServer = noopAsync,
  onLocalSettingsChange = noop,
  onSaveLocalSettings = noop,
  onSaveLocalSettingsNow = noopAsyncBoolean,
  onGatewayJsonChange = noop,
  onSaveGatewaySettings = noopAsyncBoolean,
  onMutateGatewayDraft = noop,
  onOpenAdvancedJson = noop,
  onRefreshAgentTargets = noopAsync,
  onAddChannelAccount = noopAsync,
  onStartWeixinChannelAuth = async () => ({ sessionId: '', qrCodeDataUrl: '' }),
  onPollWeixinChannelAuth = async () => ({ status: 'wait', accountId: null }),
  onStartFeishuChannelAuth = async () => ({
    sessionId: '',
    verificationUrl: '',
    qrCodeDataUrl: '',
    userCode: '',
    expiresIn: 0,
    interval: 5,
    domain: 'feishu' as const,
  }),
  onPollFeishuChannelAuth = async () => ({ status: 'pending' as const, accountId: null }),
}: GatewaySettingsPanelProps) {
  const normalizedActiveTab: SettingsTabId =
    activeTab === 'connection' ? 'gateway' : activeTab;
  const pluginAccounts = configuredChannelAccountsFromDraft(gatewayDraft?.channels);
  const [isAddingChannel, setIsAddingChannel] = useState(false);
  const [editingBot, setEditingBot] = useState<EditBotDialogContext | null>(null);
  const standaloneAgents = sortedStandaloneAgents(agents);
  const agentTargets = sortedAgentTargets(agents, teams);
  // Schema-driven catalog: icon + display_name + runtime state per
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
  const [isAdvancedJsonEditing, setIsAdvancedJsonEditing] = useState(false);
  const [editingCommandName, setEditingCommandName] = useState<string | null>(null);
  const [commandDraft, setCommandDraft] = useState<CommandDraft>(() => emptyCommandDraft());
  const [commandDialogOpen, setCommandDialogOpen] = useState(false);
  const [editingMcpServerName, setEditingMcpServerName] = useState<string | null>(null);
  const [mcpServerDraft, setMcpServerDraft] = useState<McpServerDraft>(() => emptyMcpServerDraft());
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);
  const statusClass =
    gatewayJsonError
      || (gatewayStatusMessage && /(failed|error|invalid)/i.test(gatewayStatusMessage))
      ? 'error'
      : 'info';
  const remoteSyncLabel = gatewayLoading
    ? 'Refreshing latest remote config…'
    : gatewaySaving
      ? 'Saving config…'
      : gatewayDirty
        ? 'Unsaved config changes. Click Save to persist them.'
        : 'Config changes save only when you click Save.';
  const activeTabMeta =
    SETTINGS_TABS.find((tab) => tab.id === normalizedActiveTab) || SETTINGS_TABS[0];
  const configuredChannelCount = pluginAccounts.length;
  const enabledMcpServerCount = mcpServers.filter((server) => server.enabled).length;
  const syncStateLabel = gatewaySaving
    ? 'Saving'
    : gatewayLoading
      ? 'Refreshing'
      : gatewayDirty
        ? 'Unsaved'
        : 'Saved';
  const syncFactTone: SettingsFactTone =
    statusClass === 'error'
      ? 'danger'
      : gatewayDirty || gatewaySaving || gatewayLoading
        ? 'default'
        : 'success';
  const desktopStateTone: SettingsFactTone = connection?.ok ? 'success' : 'danger';
  const claudeEnvLineCount = countNonEmptyLines(localSettings.providerClaudeEnv);
  const channelsSourceMessage = gatewaySettingsSource === 'local_file'
    ? 'Editing the gateway runtime config file on the gateway host.'
    : 'Editing live gateway settings over the API.';
  const showGatewayHeaderStatus = normalizedActiveTab === 'gateway';
  const headerFacts: Array<{
    label: string;
    value: string;
    tone?: SettingsFactTone;
  }> = [
    {
      label: 'desktop',
      value: connection?.ok ? 'online' : 'offline',
      tone: desktopStateTone,
    },
    {
      label: 'sync',
      value: syncStateLabel.toLowerCase(),
      tone: syncFactTone,
    },
    {
      label: 'saved',
      value: localSettings.gatewayUrl.replace(/^https?:\/\//, '') || '(empty)',
    },
    {
      label: 'auth',
      value: localSettings.gatewayAuthToken.trim() ? 'configured' : 'required',
      tone: localSettings.gatewayAuthToken.trim() ? 'success' : 'danger',
    },
    {
      label: 'host',
      value: `${String(gatewayDraft?.gateway?.host || '0.0.0.0')}:${String(gatewayDraft?.gateway?.port ?? '--')}`,
    },
  ];

  function renderGatewaySaveAction(buttonLabel = 'Save JSON') {
    if (gatewayMode === 'json') {
      return (
        <Button
          className="rounded-xl bg-[#111111] text-white shadow-none hover:bg-[#222222]"
          disabled={!gatewayDirty || gatewayLoading || gatewaySaving}
          onClick={() => { void onSaveGatewaySettings(); }}
          size="sm"
          type="button"
        >
          {gatewaySaving ? '保存中…' : buttonLabel}
        </Button>
      );
    }
    const statusLabel = gatewaySaving
      ? '保存中…'
      : gatewayDirty
        ? '未保存…'
        : '已自动保存';
    return <span className="codex-autosave-status">{statusLabel}</span>;
  }

  function renderLocalSaveAction(label = 'Save Desktop Settings') {
    return (
      <Button
        className="rounded-xl bg-[#111111] text-white shadow-none hover:bg-[#222222]"
        disabled={!localSettingsDirty || savingLocalSettings}
        onClick={() => {
          void onSaveLocalSettingsNow();
        }}
        size="sm"
        type="button"
      >
        {savingLocalSettings ? 'Saving…' : label}
      </Button>
    );
  }

  const connectionPanel = (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">Gateway URL</span>
      </div>
      <form className="settings-form" onSubmit={onSaveLocalSettings}>
        <div className="codex-list-card">
          <SettingsControlRow
            control={
              <div className="gateway-url-input-shell">
                <Input
                  className="gateway-url-input-with-history rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                  value={localSettings.gatewayUrl}
                  onChange={(event) => {
                    onLocalSettingsChange((current) => ({
                      ...current,
                      gatewayUrl: event.target.value,
                    }));
                  }}
                />
                <GatewayProfileHistoryButton
                  profiles={gatewayProfiles}
                  onSelect={(profile) => {
                    onLocalSettingsChange((current) => ({
                      ...current,
                      gatewayUrl: profile.gatewayUrl,
                      gatewayAuthToken: profile.gatewayAuthToken,
                    }));
                  }}
                />
              </div>
            }
            description="Desktop-side saved endpoint for the Garyx gateway."
            label="Gateway URL"
            stacked
          />
          <SettingsControlRow
            control={
              <Input
                autoCapitalize="off"
                autoComplete="off"
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                placeholder="garyx gateway token"
                spellCheck={false}
                type="password"
                value={localSettings.gatewayAuthToken}
                onChange={(event) => {
                  onLocalSettingsChange((current) => ({
                    ...current,
                    gatewayAuthToken: event.target.value,
                  }));
                }}
              />
            }
            description="Required for protected gateway APIs. Run `garyx gateway token` on the gateway host, then paste the token here."
            label="Gateway Token"
            stacked
          />
          <SettingsControlRow
            control={
              <div className="settings-control-actions">
                <Button
                  className="rounded-xl bg-[#111111] text-white shadow-none hover:bg-[#222222]"
                  disabled={savingLocalSettings}
                  type="submit"
                >
                  {savingLocalSettings ? 'Saving…' : 'Save Gateway'}
                </Button>
              </div>
            }
            description="Verifies the gateway connection before saving."
            label="Save"
          />
        </div>
      </form>
    </div>
  );

  const gatewayRuntimePanel = (
    <>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">Gateway</span>
          {renderGatewaySaveAction()}
        </div>
        <div className="codex-list-card">
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.gateway?.host || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.gateway.host = event.target.value;
                  });
                }}
              />
            }
            description="HTTP listen address for the gateway runtime."
            label="gateway.host"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                type="number"
                value={String(gatewayDraft?.gateway?.port ?? 31337)}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.gateway.port = Number(event.target.value) || 31337;
                  });
                }}
              />
            }
            description="Port used by the desktop client and other runtime callers."
            label="gateway.port"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.gateway?.image_gen?.api_key || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.gateway.image_gen.api_key = event.target.value;
                  });
                }}
              />
            }
            description="API key used by the image generation runtime."
            label="gateway.image_gen.api_key"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.gateway?.image_gen?.model || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.gateway.image_gen.model = event.target.value;
                  });
                }}
              />
            }
            description="Default image model for generated image requests."
            label="gateway.image_gen.model"
          />
        </div>
      </div>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">Defaults</span>
          {renderGatewaySaveAction()}
        </div>
        <div className="codex-list-card">
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.sessions?.data_dir || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.sessions ||= {};
                    next.sessions.data_dir = event.target.value.trim() || null;
                  });
                }}
              />
            }
            description="Directory used by the gateway to persist thread history."
            label="sessions.data_dir"
            stacked
          />
          <SettingsControlRow
            control={
              <Select
                onValueChange={(value) => {
                  onMutateGatewayDraft((next) => {
                    next.sessions ||= {};
                    next.sessions.thread_history_backend = value as GatewayThreadHistoryBackend;
                  });
                }}
                value={
                  gatewayDraft?.sessions?.thread_history_backend === 'inline_messages'
                    ? 'inline_messages'
                    : 'transcript_v1'
                }
              >
                <SelectTrigger className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none">
                  <SelectValue placeholder="Select backend" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="transcript_v1">transcript_v1</SelectItem>
                  <SelectItem value="inline_messages">inline_messages</SelectItem>
                </SelectContent>
              </Select>
            }
            description="Persist thread history as append-only transcripts or legacy inline message snapshots."
            label="sessions.thread_history_backend"
          />
        </div>
      </div>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">Heartbeat Defaults</span>
          {renderGatewaySaveAction()}
        </div>
        <div className="codex-list-card">
          <SettingsControlRow
            control={
              <SettingsSwitch
                checked={Boolean(gatewayDraft?.agent_defaults?.heartbeat?.enabled)}
                label="heartbeat.enabled"
                onChange={(nextValue) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.enabled = nextValue;
                  });
                }}
              />
            }
            description="Turn the shared heartbeat behavior on or off."
            label="enabled"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.agent_defaults?.heartbeat?.every || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.every = event.target.value;
                  });
                }}
              />
            }
            description="Interval expression used by the default heartbeat schedule."
            label="every"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.agent_defaults?.heartbeat?.target || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.target = event.target.value;
                  });
                }}
              />
            }
            description="Default target for heartbeat pings and summaries."
            label="target"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                min={1}
                type="number"
                value={String(gatewayDraft?.agent_defaults?.heartbeat?.ack_max_chars ?? 500)}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.ack_max_chars = Number(event.target.value) || 500;
                  });
                }}
              />
            }
            description="Maximum length of the acknowledgement text."
            label="ack_max_chars"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.agent_defaults?.heartbeat?.active_hours?.start || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.active_hours.start = event.target.value;
                  });
                }}
              />
            }
            description="Start time for the active window."
            label="active_hours.start"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.agent_defaults?.heartbeat?.active_hours?.end || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.active_hours.end = event.target.value;
                  });
                }}
              />
            }
            description="End time for the active window."
            label="active_hours.end"
          />
          <SettingsControlRow
            control={
              <Input
                className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none"
                value={String(gatewayDraft?.agent_defaults?.heartbeat?.active_hours?.timezone || '')}
                onChange={(event) => {
                  onMutateGatewayDraft((next) => {
                    next.agent_defaults.heartbeat.active_hours.timezone = event.target.value;
                  });
                }}
              />
            }
            description="Time zone used when evaluating the active heartbeat window."
            label="active_hours.timezone"
            stacked
          />
        </div>
      </div>
    </>
  );

  const gatewayPanel = (
    <>
      {connectionPanel}
      {gatewayRuntimePanel}
    </>
  );

  const providerPanel = (
    <div className="settings-form provider-panel">
      <section className="provider-section">
        <div className="provider-section-head">
          <h2 className="provider-section-title">Claude Code</h2>
          <a
            className="provider-view-docs"
            href="https://docs.claude.com/claude-code/settings"
            rel="noreferrer"
            target="_blank"
          >
            View docs
            <svg aria-hidden className="provider-view-docs-icon" fill="none" viewBox="0 0 10 10">
              <path d="M3 3h4v4M7 3L3 7" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.3" />
            </svg>
          </a>
        </div>
        <Textarea
          className="provider-env-editor"
          placeholder={[
            'ANTHROPIC_API_KEY=sk-ant-...',
            'CLAUDE_CODE_USE_BEDROCK=1',
            'AWS_REGION=us-east-1',
            'AWS_PROFILE=default',
          ].join('\n')}
          spellCheck={false}
          value={localSettings.providerClaudeEnv}
          onChange={(event) => {
            onLocalSettingsChange((current) => ({
              ...current,
              providerClaudeEnv: event.target.value,
            }));
          }}
        />
        <p className="provider-hint">
          {claudeEnvLineCount
            ? `${claudeEnvLineCount} ${claudeEnvLineCount === 1 ? 'variable' : 'variables'} · one per line, format: `
            : 'One per line, format: '}
          <code>VAR_NAME=value</code> or <code>export VAR_NAME=value</code>
        </p>
      </section>
      <section className="provider-section">
        <div className="provider-section-head">
          <h2 className="provider-section-title">Codex</h2>
        </div>
        <div aria-label="Codex auth mode" className="provider-tiles">
          <button
            aria-pressed={localSettings.providerCodexAuthMode === 'cli'}
            className={classNames(
              'provider-tile',
              localSettings.providerCodexAuthMode === 'cli' && 'selected',
            )}
            disabled={savingLocalSettings}
            onClick={() => {
              onLocalSettingsChange((current) => ({
                ...current,
                providerCodexAuthMode: 'cli',
              }));
            }}
            type="button"
          >
            <span aria-hidden className="provider-tile-radio" />
            <span className="provider-tile-body">
              <span className="provider-tile-label">CLI</span>
              <span className="provider-tile-desc">
                Reuse the local <code>codex</code> login on this Mac.
              </span>
            </span>
            <span className="provider-tile-badge">no key</span>
          </button>
          <button
            aria-pressed={localSettings.providerCodexAuthMode === 'api_key'}
            className={classNames(
              'provider-tile',
              localSettings.providerCodexAuthMode === 'api_key' && 'selected',
            )}
            disabled={savingLocalSettings}
            onClick={() => {
              onLocalSettingsChange((current) => ({
                ...current,
                providerCodexAuthMode: 'api_key',
              }));
            }}
            type="button"
          >
            <span aria-hidden className="provider-tile-radio" />
            <span className="provider-tile-body">
              <span className="provider-tile-label">API Key</span>
              <span className="provider-tile-desc">Desktop-only key, stored in macOS Keychain.</span>
            </span>
            <span className="provider-tile-badge">keychain</span>
          </button>
        </div>
        {localSettings.providerCodexAuthMode === 'api_key' ? (
          <div className="provider-api-field">
            <label className="provider-api-label" htmlFor="codex-api-key">API Key</label>
            <Input
              autoCapitalize="off"
              autoComplete="off"
              className="provider-api-input"
              id="codex-api-key"
              placeholder="sk-YOUR-API-KEY"
              spellCheck={false}
              type="password"
              value={localSettings.providerCodexApiKey}
              onChange={(event) => {
                onLocalSettingsChange((current) => ({
                  ...current,
                  providerCodexApiKey: event.target.value,
                }));
              }}
            />
            <p className="provider-hint">
              Sets <code>CODEX_API_KEY</code>
            </p>
          </div>
        ) : null}
      </section>
      {localSettingsDirty ? (
        <div className="provider-save-row">{renderLocalSaveAction()}</div>
      ) : null}
    </div>
  );

  const configuredChannels = pluginAccounts;

  const channelsPanel = (
    <>
      <ChannelPluginCatalogPanel />
      <section className="bot-panel">
        <div className="bot-panel-toolbar">
          <div className="bot-panel-title-row">
            <h3 className="bot-panel-title">Bots</h3>
            <span className="bot-panel-count">{configuredChannels.length}</span>
            <span className="bot-panel-source">{channelsSourceMessage}</span>
          </div>
          <Button
            className="bot-panel-action"
            onClick={() => {
              void (async () => {
                await onRefreshAgentTargets();
                setIsAddingChannel(true);
              })();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            添加 Bot
          </Button>
        </div>
        {!configuredChannels.length ? (
          <div className="bot-panel-empty">还没有配置 Bot，点上方"添加 Bot"创建。</div>
        ) : (
          <div className="bot-table">
            <div className="bot-table-head">
              <span className="bot-table-col bot-table-col-name">Name</span>
              <span className="bot-table-col bot-table-col-channel">Channel</span>
              <span className="bot-table-col bot-table-col-account">Account</span>
              <span className="bot-table-col bot-table-col-agent">Agent</span>
              <span className="bot-table-col bot-table-col-status">Enabled</span>
            </div>
            {configuredChannels.map(({ kind, accountId, account }) => {
              const accountAgentId = resolveChannelAgentId(
                gatewayDraft,
                kind,
                accountId,
                account,
                standaloneAgents,
              );
              const selectedTarget = agentTargets.find((target) => target.value === accountAgentId) || null;
              const selectedAgentMissing = Boolean(accountAgentId && !selectedTarget);
              const agentLabel = selectedTarget
                ? selectedTarget.label
                : (accountAgentId || 'Default route');
              const agentDisplayName = compactAgentTargetLabel(
                selectedTarget,
                accountAgentId || 'Default route',
              );
              const agentDotSeed = selectedTarget?.value || accountAgentId || 'default';
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
                  resolvedAgentId: accountAgentId,
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
                      <span
                        className={classNames(
                          'bot-table-agent-dot',
                          selectedAgentMissing && 'missing',
                        )}
                        data-agent-tone={agentDotSeed.length % 4}
                        aria-hidden
                      />
                      <span className="bot-table-agent-copy">
                        <span className="bot-table-agent-name">{agentDisplayName}</span>
                        {accountAgentId ? (
                          <span className="bot-table-agent-id">{accountAgentId}</span>
                        ) : null}
                      </span>
                    </span>
                  </span>
                  <span className="bot-table-cell bot-table-col-status">
                    <Switch
                      aria-label={`${displayName} enabled`}
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
                </div>
              );
            })}
          </div>
        )}
      </section>
      <EditBotDialog
        agentTargets={agentTargets}
        context={editingBot}
        onClose={() => setEditingBot(null)}
        onRemove={async ({ kind, accountId }) => {
          onMutateGatewayDraft((next) => {
            next.channels = next.channels || {};
            if (!next.channels?.[kind]?.accounts) return;
            delete next.channels[kind].accounts[accountId];
          });
          await onSaveGatewaySettings();
        }}
        onSave={async ({ kind, accountId, patch }) => {
          onMutateGatewayDraft((next) => {
            next.channels = next.channels || {};
            next.channels[kind] = next.channels[kind] || {};
            next.channels[kind].accounts = next.channels[kind].accounts || {};
            const account = next.channels[kind].accounts[accountId];
            if (!account) return;
            const targetAccountId = patch.nextAccountId?.trim() || accountId;
            if (patch.name !== undefined) account.name = patch.name;
            if (patch.workspaceDir !== undefined) account.workspace_dir = patch.workspaceDir;
            if (patch.agentId !== undefined) account.agent_id = patch.agentId;
            if (patch.config !== undefined) account.config = patch.config;
            if (targetAccountId !== accountId) {
              next.channels[kind].accounts[targetAccountId] = account;
              delete next.channels[kind].accounts[accountId];
            }
          });
          await onSaveGatewaySettings();
        }}
        open={Boolean(editingBot)}
        saving={gatewaySaving}
      />
      <AddBotDialog
        agentTargets={agentTargets}
        onClose={() => {
          setIsAddingChannel(false);
        }}
        onCreateChannel={onAddChannelAccount}
        onPollWeixinAuth={onPollWeixinChannelAuth}
        onStartWeixinAuth={onStartWeixinChannelAuth}
        onPollFeishuAuth={onPollFeishuChannelAuth}
        onStartFeishuAuth={onStartFeishuChannelAuth}
        open={isAddingChannel}
      />
    </>
  );

  const normalizedCommandDraftName = commandDraft.name.trim().toLowerCase();
  const commandDraftPrompt = commandDraft.prompt?.trim() || '';
  const commandNameTaken = commands.some((command) => {
    return command.name === normalizedCommandDraftName && command.name !== editingCommandName;
  });
  const commandDraftReady = Boolean(
    SLASH_COMMAND_NAME_PATTERN.test(normalizedCommandDraftName)
      && commandDraftPrompt,
  );
  const commandDraftValidationMessage = commandNameTaken
    ? '同名命令已存在。'
    : normalizedCommandDraftName && !SLASH_COMMAND_NAME_PATTERN.test(normalizedCommandDraftName)
      ? '命令名仅支持小写字母、数字和下划线，最长 32 个字符。'
      : !commandDraftPrompt
        ? '请填写命令内容。'
          : '保存后会加入命令列表。';
  const commandPromptPreview = (command: SlashCommand) => {
    const preview = (command.prompt || command.description || '').trim();
    return preview.length > 140 ? `${preview.slice(0, 137)}…` : preview;
  };

  function resetCommandEditor() {
    setEditingCommandName(null);
    setCommandDraft(emptyCommandDraft());
  }

  function closeCommandDialog() {
    setCommandDialogOpen(false);
    resetCommandEditor();
  }

  function openCreateCommandDialog() {
    resetCommandEditor();
    setCommandDialogOpen(true);
  }

  function openEditCommandDialog(command: SlashCommand) {
    setEditingCommandName(command.name);
    setCommandDraft(commandDraftFromValue(command));
    setCommandDialogOpen(true);
  }

  async function handleSaveCommandDraft() {
    if (!commandDraftReady || commandNameTaken) {
      return;
    }

    const payload: UpsertSlashCommandInput = {
      name: normalizedCommandDraftName,
      description: deriveSlashCommandDescription(commandDraftPrompt, normalizedCommandDraftName),
      prompt: commandDraftPrompt || null,
    };

    if (editingCommandName) {
      await onUpdateSlashCommand({
        ...payload,
        currentName: editingCommandName,
      });
    } else {
      await onCreateSlashCommand(payload);
    }
    closeCommandDialog();
  }

  async function handleDeleteCommandClick(name: string) {
    await onDeleteSlashCommand(name);
    if (editingCommandName === name) {
      closeCommandDialog();
    }
  }

  const commandsPanel = (
    <>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">命令列表</span>
          <div className="codex-list-row-actions">
            <button
              className="codex-section-action"
              onClick={openCreateCommandDialog}
              type="button"
            >
              <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
                <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
              </svg>
              添加命令
            </button>
          </div>
        </div>

        {commandsLoading ? (
          <div className="commands-empty-state">
            <strong>正在读取快捷指令…</strong>
            <span>会从当前 Gateway 配置里拉取全局 prompt shortcut。</span>
          </div>
        ) : commands.length ? (
          <div className="commands-table">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="commands-table-col-command">Command</TableHead>
                  <TableHead className="commands-table-col-description">Description</TableHead>
                  <TableHead className="commands-table-col-prompt">Prompt</TableHead>
                  <TableHead className="commands-table-col-actions">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {commands.map((command) => (
                  <TableRow
                    data-testid={`slash-command-card-${command.name}`}
                    key={command.name}
                  >
                    <TableCell className="commands-table-col-command">
                      <span className="command-table-slash">/{command.name}</span>
                    </TableCell>
                    <TableCell
                      className="commands-table-col-description"
                      title={command.description || 'Prompt shortcut'}
                    >
                      {command.description || 'Prompt shortcut'}
                    </TableCell>
                    <TableCell
                      className="commands-table-col-prompt"
                      title={commandPromptPreview(command) || 'No prompt configured.'}
                    >
                      {commandPromptPreview(command) || 'No prompt configured.'}
                    </TableCell>
                    <TableCell className="commands-table-col-actions">
                      <div className="command-list-actions">
                        <button
                          className="command-row-action"
                          onClick={() => { openEditCommandDialog(command); }}
                          type="button"
                        >
                          编辑
                        </button>
                        <button
                          className="command-row-action danger"
                          disabled={commandsSaving}
                          onClick={() => { void handleDeleteCommandClick(command.name); }}
                          type="button"
                        >
                          删除
                        </button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <div className="commands-empty-state">
            <strong>还没有快捷指令</strong>
            <span>点上方“添加命令”，创建一个 /summary 这样的 prompt shortcut。</span>
          </div>
        )}
      </div>
      <Dialog
        open={commandDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            closeCommandDialog();
          }
        }}
      >
        <DialogContent
          className="commands-dialog"
          showCloseButton={false}
        >
          <DialogHeader className="commands-dialog-header">
            <Badge
              variant="outline"
              className="commands-dialog-badge"
            >
              {editingCommandName ? 'Edit Command' : 'Add Command'}
            </Badge>
            <div className="commands-dialog-title-group">
              <DialogTitle className="commands-dialog-title">
                {editingCommandName ? `编辑 /${editingCommandName}` : '添加命令'}
              </DialogTitle>
              <DialogDescription className="commands-dialog-description">
                只保留命令名和内容。保存时会自动为 Telegram 同步生成描述。
              </DialogDescription>
            </div>
          </DialogHeader>

          <div className="commands-dialog-body">
            <div className="commands-field">
              <div className="commands-field-header">
                <Label className="commands-field-label" htmlFor="slash-command-name">命令名</Label>
                <span className="commands-field-hint">仅支持 a-z、0-9、_</span>
              </div>
              <div className="commands-name-input">
                <span aria-hidden>/</span>
                <Input
                  className="commands-name-control"
                  id="slash-command-name"
                  placeholder="summary"
                  value={commandDraft.name}
                  onChange={(event) => {
                    setCommandDraft((current) => ({
                      ...current,
                      name: event.target.value.toLowerCase(),
                    }));
                  }}
                />
              </div>
            </div>

            <div className="commands-field">
              <div className="commands-field-header">
                <Label className="commands-field-label" htmlFor="slash-command-prompt">内容</Label>
                <span className="commands-field-hint">执行 /command 时会替换成这段 prompt</span>
              </div>
              <Textarea
                className="commands-prompt-control"
                id="slash-command-prompt"
                placeholder="请总结我们的对话要点"
                value={String(commandDraft.prompt || '')}
                onChange={(event) => {
                  setCommandDraft((current) => ({
                    ...current,
                    prompt: event.target.value,
                  }));
                }}
              />
            </div>

            <p className={classNames('small-note commands-modal-note', (commandNameTaken || !commandDraftReady) && 'error')}>
              {commandDraftValidationMessage}
            </p>
          </div>

          <DialogFooter className="commands-dialog-footer">
            <Button
              className="commands-dialog-button secondary"
              onClick={closeCommandDialog}
              type="button"
              variant="outline"
            >
              取消
            </Button>
            <Button
              className="commands-dialog-button primary"
              disabled={!commandDraftReady || commandNameTaken || commandsSaving}
              onClick={() => {
                void handleSaveCommandDraft();
              }}
              type="button"
            >
              {commandsSaving ? '保存中…' : '保存命令'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );

  const normalizedMcpServerName = mcpServerDraft.name.trim();
  const normalizedMcpServerCommand = mcpServerDraft.command.trim();
  const normalizedMcpUrl = mcpServerDraft.url.trim();
  const normalizedMcpArgs = mcpServerDraft.args
    .map((value) => value.trim())
    .filter(Boolean);
  const mcpServerNameTaken = mcpServers.some((server) => {
    return server.name === normalizedMcpServerName && server.name !== editingMcpServerName;
  });
  const mcpTransportReady = mcpServerDraft.transport === 'stdio'
    ? Boolean(normalizedMcpServerCommand)
    : Boolean(normalizedMcpUrl);
  const mcpServerDraftReady = Boolean(
    normalizedMcpServerName
      && mcpTransportReady
      && !mcpServerNameTaken,
  );
  const mcpServerDraftValidationMessage = mcpServerNameTaken
    ? '同名 MCP server 已存在。'
    : !normalizedMcpServerName
      ? '请填写 server 名称。'
      : mcpServerDraft.transport === 'stdio' && !normalizedMcpServerCommand
        ? '请填写启动命令。'
        : mcpServerDraft.transport === 'streamable_http' && !normalizedMcpUrl
          ? '请填写 URL。'
          : '保存后会更新 gateway 端的 garyx.json，并同步 Claude / Codex 的本地 MCP 配置。';

  function resetMcpServerEditor() {
    setEditingMcpServerName(null);
    setMcpServerDraft(emptyMcpServerDraft());
  }

  function closeMcpDialog() {
    setMcpDialogOpen(false);
    resetMcpServerEditor();
  }

  function openCreateMcpDialog() {
    resetMcpServerEditor();
    setMcpDialogOpen(true);
  }

  function openEditMcpDialog(server: DesktopMcpServer) {
    setEditingMcpServerName(server.name);
    setMcpServerDraft(mcpServerDraftFromValue(server));
    setMcpDialogOpen(true);
  }

  async function handleSaveMcpServerDraft() {
    if (!mcpServerDraftReady) {
      return;
    }

    const payload: UpsertMcpServerInput = {
      name: normalizedMcpServerName,
      transport: mcpServerDraft.transport,
      enabled: mcpServerDraft.enabled,
      ...(mcpServerDraft.transport === 'stdio'
        ? {
            command: normalizedMcpServerCommand,
            args: normalizedMcpArgs,
            env: Object.fromEntries(
              mcpServerDraft.envEntries.flatMap(({ key, value }) => {
                const normalizedKey = key.trim();
                return normalizedKey ? [[normalizedKey, value]] : [];
              }),
            ),
            workingDir: mcpServerDraft.workingDir.trim() || null,
          }
        : {
            url: normalizedMcpUrl,
            headers: Object.fromEntries(
              mcpServerDraft.headerEntries.flatMap(({ key, value }) => {
                const normalizedKey = key.trim();
                return normalizedKey ? [[normalizedKey, value]] : [];
              }),
            ),
          }),
    };

    if (editingMcpServerName) {
      await onUpdateMcpServer({
        ...payload,
        currentName: editingMcpServerName,
      });
    } else {
      await onCreateMcpServer(payload);
    }
    closeMcpDialog();
  }

  async function handleDeleteMcpServer(name: string) {
    if (!window.confirm(`删除 MCP 服务器 "${name}"？`)) return;
    await onDeleteMcpServer(name);
    if (editingMcpServerName === name) {
      closeMcpDialog();
    }
  }

  const mcpPanel = (
    <>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">自定义服务器</span>
          <button
            className="codex-section-action"
            disabled={mcpServersSaving}
            onClick={openCreateMcpDialog}
            type="button"
          >
            <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
              <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
            </svg>
            添加服务器
          </button>
        </div>
        {mcpServersLoading ? (
          <div className="codex-empty-state">正在读取当前配置…</div>
        ) : mcpServers.length ? (
          <div className="codex-list-card">
            {mcpServers.map((server) => (
              <div
                className="codex-list-row"
                data-testid={`mcp-server-card-${server.name}`}
                key={server.name}
              >
                <span className="codex-list-row-name">{server.name}</span>
                <div className="codex-list-row-actions">
                  <button
                    className="codex-icon-button"
                    onClick={() => { openEditMcpDialog(server); }}
                    title="配置"
                    type="button"
                  >
                    <svg aria-hidden width="18" height="18" viewBox="0 0 21 21" fill="none">
                      <path d="M10.7228 2.53564C11.5515 2.53564 12.3183 2.97502 12.7374 3.68994L13.5587 5.09033L13.6124 5.15967C13.6736 5.22007 13.7566 5.2556 13.8448 5.25635L15.4601 5.26904L15.6144 5.27588C16.3826 5.33292 17.0775 5.76649 17.465 6.43994L17.7931 7.01123L17.8663 7.14697C18.1815 7.78943 18.1843 8.54208 17.8741 9.18701L17.8028 9.32275L17.0001 10.7446C16.9427 10.8467 16.9426 10.9717 17.0001 11.0737L17.8028 12.4946L17.8741 12.6313C18.1842 13.2763 18.1816 14.029 17.8663 14.6714L17.7931 14.8071L17.465 15.3784C17.0774 16.0517 16.3825 16.4855 15.6144 16.5425L15.4601 16.5483L13.8448 16.562C13.7565 16.5628 13.6736 16.5982 13.6124 16.6587L13.5587 16.7271L12.7374 18.1284C12.3183 18.8432 11.5514 19.2827 10.7228 19.2827H10.0763C9.29958 19.2826 8.57714 18.8964 8.14465 18.2593L8.06261 18.1284L7.24133 16.7271C7.1966 16.6509 7.12417 16.5966 7.04113 16.5737L6.95519 16.562L5.33996 16.5483C4.56297 16.542 3.84347 16.1503 3.41613 15.5093L3.33508 15.3784L3.00695 14.8071C2.59564 14.0921 2.59168 13.2129 2.99719 12.4946L3.79894 11.0737L3.83215 10.9937C3.84657 10.9383 3.84652 10.88 3.83215 10.8247L3.79894 10.7446L2.99719 9.32275C2.59184 8.60451 2.59571 7.72612 3.00695 7.01123L3.33508 6.43994L3.41613 6.30908C3.84345 5.66796 4.56288 5.27538 5.33996 5.26904L6.95519 5.25635L7.04113 5.24463C7.12427 5.22177 7.1966 5.16664 7.24133 5.09033L8.06261 3.68994L8.14465 3.55908C8.57712 2.92179 9.29949 2.5358 10.0763 2.53564H10.7228ZM10.0763 3.86572C9.76448 3.86587 9.47308 4.01039 9.28429 4.25244L9.21008 4.36279L8.38879 5.76318C8.12941 6.20571 7.68297 6.49995 7.18273 6.56982L6.96594 6.58643L5.3507 6.59912C5.03877 6.60167 4.74854 6.74903 4.56164 6.99268L4.48742 7.10303L4.15929 7.67432C3.98236 7.98202 3.98089 8.36033 4.15539 8.66943L4.95715 10.0903L5.05187 10.2856C5.21318 10.6851 5.21302 11.1323 5.05187 11.5317L4.95715 11.728L4.15539 13.1489C3.98092 13.4581 3.98228 13.8363 4.15929 14.144L4.48742 14.7144L4.56164 14.8247C4.74853 15.0686 5.03859 15.2157 5.3507 15.2183L6.96594 15.2319L7.18273 15.2476C7.68301 15.3174 8.12939 15.6126 8.38879 16.0552L9.21008 17.4556L9.28429 17.5649C9.47307 17.8072 9.76431 17.9525 10.0763 17.9526H10.7228C11.0794 17.9526 11.4096 17.7632 11.59 17.4556L12.4112 16.0552L12.5333 15.8745C12.8433 15.4758 13.3212 15.2361 13.8341 15.2319L15.4493 15.2183L15.5812 15.2085C15.8855 15.1657 16.1569 14.985 16.3126 14.7144L16.6407 14.144L16.6984 14.0259C16.7984 13.7835 16.8 13.5113 16.7023 13.2681L16.6446 13.1489L15.8419 11.728C15.5551 11.2201 15.5552 10.5983 15.8419 10.0903L16.6446 8.66943L16.7023 8.55029C16.8001 8.30708 16.7983 8.03486 16.6984 7.79248L16.6407 7.67432L16.3126 7.10303C16.1569 6.8324 15.8856 6.65166 15.5812 6.60889L15.4493 6.59912L13.8341 6.58643C13.3213 6.58224 12.8433 6.34243 12.5333 5.94385L12.4112 5.76318L11.59 4.36279C11.4096 4.05506 11.0795 3.86572 10.7228 3.86572H10.0763ZM11.9855 10.9087C11.9853 10.0336 11.2755 9.32399 10.4005 9.32373C9.52524 9.32373 8.81474 10.0335 8.81457 10.9087C8.81457 11.7841 9.52513 12.4937 10.4005 12.4937C11.2757 12.4934 11.9855 11.7839 11.9855 10.9087ZM13.3146 10.9087C13.3146 12.5184 12.0102 13.8235 10.4005 13.8237C8.7906 13.8237 7.48547 12.5186 7.48547 10.9087C7.48564 9.29893 8.7907 7.99365 10.4005 7.99365C12.0101 7.99391 13.3144 9.29909 13.3146 10.9087Z" fill="currentColor"/>
                    </svg>
                  </button>
                  <Switch
                    aria-label={`${server.name} enabled`}
                    checked={server.enabled}
                    onCheckedChange={(nextValue) => {
                      void onToggleMcpServer(server.name, nextValue);
                    }}
                  />
                  <button
                    aria-label={`删除 ${server.name}`}
                    className="codex-icon-button codex-icon-button-danger"
                    disabled={mcpServersSaving}
                    onClick={() => { void handleDeleteMcpServer(server.name); }}
                    title="删除"
                    type="button"
                  >
                    <svg aria-hidden width="16" height="16" viewBox="0 0 20 20" fill="none">
                      <path d="M8 3.5h4a.5.5 0 0 1 .5.5V5h3a.5.5 0 0 1 0 1h-.6l-.8 9.6A2 2 0 0 1 12.1 17.5H7.9a2 2 0 0 1-1.99-1.9L5.1 6H4.5a.5.5 0 0 1 0-1h3V4a.5.5 0 0 1 .5-.5zm.5 1.5V5h3v-.5zm-2.4 2l.8 9.52a1 1 0 0 0 1 .98h4.2a1 1 0 0 0 1-1l.8-9.5H6.1zM8.5 8a.5.5 0 0 1 .5.5v5a.5.5 0 0 1-1 0v-5a.5.5 0 0 1 .5-.5zm3 0a.5.5 0 0 1 .5.5v5a.5.5 0 0 1-1 0v-5a.5.5 0 0 1 .5-.5z" fill="currentColor"/>
                    </svg>
                  </button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="codex-empty-state">还没有 MCP 服务器，点上方"添加服务器"创建。</div>
        )}
      </div>
      <Dialog
        open={mcpDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            closeMcpDialog();
          }
        }}
      >
        <DialogContent
          className="max-w-[520px] rounded-[12px] border-[#e8e8e5] bg-white p-0 shadow-[0_8px_24px_rgba(0,0,0,0.08)] gap-0"
          showCloseButton={false}
        >
          <DialogHeader className="border-b border-[#efefec] px-4 py-3">
            <DialogTitle className="text-[14px] font-semibold tracking-[-0.01em] text-[#111111]">
              {editingMcpServerName ? `编辑 ${editingMcpServerName}` : '添加服务器'}
            </DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-4 py-4">
            <div className="grid gap-3 md:grid-cols-[1fr_auto]">
              <div className="space-y-1.5">
                <Label className="text-[11px] font-medium text-[#666663]">名称</Label>
                <Input
                  className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                  placeholder="MCP server name"
                  value={mcpServerDraft.name}
                  onChange={(event) => {
                    setMcpServerDraft((current) => ({
                      ...current,
                      name: event.target.value,
                    }));
                  }}
                />
              </div>

              <div className="space-y-1.5">
                <Label className="text-[11px] font-medium text-[#666663]">Transport</Label>
                <ToggleGroup
                  className="h-8 rounded-[8px] bg-[#f3f3f1] p-0.5"
                  type="single"
                  value={mcpServerDraft.transport}
                  onValueChange={(nextValue) => {
                    if (!nextValue) {
                      return;
                    }
                    setMcpServerDraft((current) => ({
                      ...current,
                      transport: nextValue as McpTransportType,
                    }));
                  }}
                >
                  <ToggleGroupItem
                    className="h-7 rounded-[6px] border-0 px-3 text-[11px] text-[#666663] data-[state=on]:text-[#111111]"
                    value="stdio"
                  >
                    STDIO
                  </ToggleGroupItem>
                  <ToggleGroupItem
                    className="h-7 rounded-[6px] border-0 px-3 text-[11px] text-[#666663] data-[state=on]:text-[#111111]"
                    value="streamable_http"
                  >
                    HTTP
                  </ToggleGroupItem>
                </ToggleGroup>
              </div>
            </div>

            {mcpServerDraft.transport === 'stdio' ? (
              <>
                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">启动命令</Label>
                  <Input
                    className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                    placeholder="openai-dev-mcp serve-sqlite"
                    value={mcpServerDraft.command}
                    onChange={(event) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        command: event.target.value,
                      }));
                    }}
                  />
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">参数</Label>
                    <button
                      className="text-[11px] text-[#666663] hover:text-[#111111]"
                      onClick={() => {
                        setMcpServerDraft((current) => ({
                          ...current,
                          args: [...current.args, ''],
                        }));
                      }}
                      type="button"
                    >
                      + 添加
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.args.map((value, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[1fr_auto]" key={`arg-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          value={value}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              args: current.args.map((entry, entryIndex) => {
                                return entryIndex === index ? event.target.value : entry;
                              }),
                            }));
                          }}
                        />
                        <button
                          className="px-2 text-[11px] text-[#9b3d3d] hover:text-[#7a2f2f] disabled:cursor-not-allowed disabled:text-[#c7c7c4]"
                          disabled={mcpServerDraft.args.length <= 1}
                          onClick={() => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              args: current.args.filter((_, entryIndex) => entryIndex !== index),
                            }));
                          }}
                          type="button"
                        >
                          删除
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">环境变量</Label>
                    <button
                      className="text-[11px] text-[#666663] hover:text-[#111111]"
                      onClick={() => {
                        setMcpServerDraft((current) => ({
                          ...current,
                          envEntries: [...current.envEntries, { key: '', value: '' }],
                        }));
                      }}
                      type="button"
                    >
                      + 添加
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.envEntries.map((entry, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[0.9fr_1.1fr_auto]" key={`env-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder="键"
                          value={entry.key}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              envEntries: current.envEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, key: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder="值"
                          value={entry.value}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              envEntries: current.envEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, value: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <button
                          className="px-2 text-[11px] text-[#9b3d3d] hover:text-[#7a2f2f] disabled:cursor-not-allowed disabled:text-[#c7c7c4]"
                          disabled={mcpServerDraft.envEntries.length <= 1}
                          onClick={() => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              envEntries: current.envEntries.filter((_, entryIndex) => entryIndex !== index),
                            }));
                          }}
                          type="button"
                        >
                          删除
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">Working directory</Label>
                  <Input
                    className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                    placeholder="/path/to/workspace"
                    value={mcpServerDraft.workingDir}
                    onChange={(event) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        workingDir: event.target.value,
                      }));
                    }}
                  />
                </div>
              </>
            ) : (
              <>
                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">URL</Label>
                  <Input
                    className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                    placeholder="https://mcp.example.com/mcp"
                    value={mcpServerDraft.url}
                    onChange={(event) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        url: event.target.value,
                      }));
                    }}
                  />
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">标头</Label>
                    <button
                      className="text-[11px] text-[#666663] hover:text-[#111111]"
                      onClick={() => {
                        setMcpServerDraft((current) => ({
                          ...current,
                          headerEntries: [...current.headerEntries, { key: '', value: '' }],
                        }));
                      }}
                      type="button"
                    >
                      + 添加
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.headerEntries.map((entry, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[0.9fr_1.1fr_auto]" key={`header-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder="键"
                          value={entry.key}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              headerEntries: current.headerEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, key: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder="值"
                          value={entry.value}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              headerEntries: current.headerEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, value: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <button
                          className="px-2 text-[11px] text-[#9b3d3d] hover:text-[#7a2f2f] disabled:cursor-not-allowed disabled:text-[#c7c7c4]"
                          disabled={mcpServerDraft.headerEntries.length <= 1}
                          onClick={() => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              headerEntries: current.headerEntries.filter((_, entryIndex) => entryIndex !== index),
                            }));
                          }}
                          type="button"
                        >
                          删除
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              </>
            )}

            <p className={classNames('text-[11px] leading-4 text-[#8a8a87]', (mcpServerNameTaken || !mcpServerDraftReady) && '!text-[#9b3d3d]')}>
              {mcpServerDraftValidationMessage}
            </p>
          </div>

          <DialogFooter className="flex !justify-between border-t border-[#efefec] px-4 py-3 sm:!justify-between">
            <div>
              {editingMcpServerName ? (
                <Button
                  className="h-8 rounded-[8px] border-[#f0d9d9] bg-white px-3 text-[12px] text-[#9b3d3d] shadow-none hover:bg-[#fdf3f3]"
                  disabled={mcpServersSaving}
                  onClick={() => { void handleDeleteMcpServer(editingMcpServerName); }}
                  type="button"
                  variant="outline"
                >
                  删除
                </Button>
              ) : null}
            </div>
            <div className="flex gap-2">
              <Button
                className="h-8 rounded-[8px] border-[#e7e7e5] bg-white px-3 text-[12px] text-[#111111] shadow-none hover:bg-[#f6f6f5]"
                onClick={closeMcpDialog}
                type="button"
                variant="outline"
              >
                取消
              </Button>
              <Button
                className="h-8 rounded-[8px] bg-[#111111] px-3 text-[12px] text-white shadow-none hover:bg-[#1c1c1c]"
                disabled={!mcpServerDraftReady || mcpServersSaving}
                onClick={() => {
                  void handleSaveMcpServerDraft();
                }}
                type="button"
              >
                {mcpServersSaving ? '保存中…' : '保存'}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );

  const labsPanel = (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">Mac Labs</span>
        {renderGatewaySaveAction('Save Labs')}
      </div>
      <div className="codex-list-card">
        <SettingsControlRow
          control={
            <SettingsSwitch
              checked={Boolean(gatewayDraft?.desktop?.labs?.auto_research)}
              label="desktop.labs.auto_research"
              onChange={(nextValue) => {
                onMutateGatewayDraft((next) => {
                  next.desktop ||= {};
                  next.desktop.labs ||= {};
                  next.desktop.labs.auto_research = nextValue;
                });
              }}
            />
          }
          description="Show or hide the Auto Research entry in the Mac app. Disabling it only hides the Mac surface."
          label="Auto Research"
        />
      </div>
    </div>
  );

  const advancedPanel = (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">Config JSON</span>
        <div className="codex-list-row-actions">
          {renderGatewaySaveAction()}
          <button
            className="codex-section-action"
            onClick={() => { setIsAdvancedJsonEditing((current) => !current); }}
            type="button"
          >
            {isAdvancedJsonEditing ? '完成' : '编辑'}
          </button>
        </div>
      </div>
      <div className="codex-list-card">
        <div className="settings-editor-block">
          {isAdvancedJsonEditing ? (
            <Textarea
              className="settings-json-editor min-h-[360px] rounded-[16px] border-[#e7e7e5] bg-[#fafaf9] font-mono text-[12px] shadow-none"
              value={gatewayJsonDraft}
              onChange={(event) => {
                onGatewayJsonChange(event.target.value);
              }}
            />
          ) : (
            <pre className="settings-json-preview">{gatewayJsonDraft}</pre>
          )}
          {gatewayJsonError ? <span className="settings-json-error">{gatewayJsonError}</span> : null}
        </div>
      </div>
    </div>
  );

  let tabContent: ReactNode;
  switch (normalizedActiveTab) {
    case 'gateway':
      tabContent = gatewayPanel;
      break;
    case 'provider':
      tabContent = providerPanel;
      break;
    case 'channels':
      tabContent = channelsPanel;
      break;
    case 'labs':
      tabContent = labsPanel;
      break;
    case 'commands':
      tabContent = commandsPanel;
      break;
    case 'mcp':
      tabContent = mcpPanel;
      break;
    case 'advanced':
      tabContent = advancedPanel;
      break;
    default:
      tabContent = gatewayPanel;
      break;
  }

  return (
    <div className={classNames('settings-content', `settings-content-${normalizedActiveTab}`)}>
      <div className="settings-content-column">
        <section className="settings-page-header">
          <div className="settings-page-header-main">
            <span className="eyebrow">{activeTabMeta.eyebrow}</span>
            <h3 className="settings-tab-title">{activeTabMeta.label}</h3>
            <p className="small-note">{activeTabMeta.description}</p>
            {showGatewayHeaderStatus ? (
              <p
                className={`small-note settings-tab-hint ${
                  statusClass === 'error' ? 'error' : ''
                }`}
              >
                {gatewayStatusMessage || remoteSyncLabel}
              </p>
            ) : null}
          </div>
          {showGatewayHeaderStatus ? (
            <div className="settings-page-header-aside">
              <span className={`status-pill ${connection?.ok ? '' : 'offline'}`}>
                {connection?.ok ? 'online' : 'offline'}
              </span>
            </div>
          ) : null}
        </section>

        {showGatewayHeaderStatus ? (
          <div className="settings-hero-facts">
            {headerFacts.map((fact) => (
              <SettingsFact
                key={`${fact.label}:${fact.value}`}
                label={fact.label}
                value={fact.value}
                tone={fact.tone}
              />
            ))}
          </div>
        ) : null}

        <div className="settings-page-sections">{tabContent}</div>
      </div>
    </div>
  );
}
