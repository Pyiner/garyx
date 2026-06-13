import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Pencil, Plus, RefreshCw, Server, Trash } from 'lucide-react';

import {
  DEFAULT_DESKTOP_SETTINGS,
  type ChannelPluginCatalogEntry,
  type ConnectionStatus,
  type DesktopApiProviderType,
  type CreateCustomAgentInput,
  type DesktopCustomAgent,
  type DesktopFollowUpBehavior,
  type DesktopProviderModelOption,
  type DesktopProviderModels,
  type DesktopWorkspace,
  type DesktopTeam,
  type DesktopGatewayProfile,
  type DesktopSettings,
  type DesktopMcpServer,
  type DesktopSkillInfo,
  type DesktopUpdateStatus,
  type GatewayConfigDocument,
  type GatewaySettingsSource,
  type McpTransportType,
  type SlashCommand,
  type UpdateMcpServerInput,
  type UpdateSlashCommandInput,
  type UpdateCustomAgentInput,
  type UpsertMcpServerInput,
  type UpsertSlashCommandInput,
  type PollWeixinChannelAuthResult,
  type StartFeishuChannelAuthInput,
  type StartFeishuChannelAuthResult,
  type PollFeishuChannelAuthInput,
  type PollFeishuChannelAuthResult,
} from '@shared/contracts';

import { defaultChannelAgentId } from '@renderer/gateway-settings';
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectGroup,
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
import { buildAgentTargetOptions, type AgentTargetOption } from './app-shell/agent-options';
import { AddBotDialog } from './app-shell/components/AddBotDialog';
import { AgentOptionAvatar } from './app-shell/components/AgentOptionAvatar';
import { WorkspacePathPicker } from './components/WorkspacePathPicker';
import { MoreDotsIcon } from './app-shell/icons';
import { ChannelPluginCatalogPanel } from './channel-plugins/ChannelPluginCatalogPanel';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { EditBotDialog, type EditBotDialogContext, type EditBotPatch } from './app-shell/components/EditBotDialog';
import { RendererPerformancePanel } from './app-shell/components/RendererPerformancePanel';
import { languagePreferenceLabel, type Translate, useI18n } from './i18n';
import type { RendererPerformanceSnapshot } from './perf-metrics';
import { SETTINGS_TABS, type SettingsTabId } from './settings-tabs';

const UNKNOWN_DESKTOP_APP_VERSION = '0.0.0';

type DraftMutator = (mutator: (nextConfig: any) => void) => void;
type GatewaySettingsSaveOptions = {
  silent?: boolean;
  refreshDesktopState?: 'await' | 'background' | 'skip';
};
type GatewaySettingsPanelProps = {
  activeTab: SettingsTabId;
  commands?: SlashCommand[];
  commandsLoading?: boolean;
  commandsSaving?: boolean;
  connection?: ConnectionStatus | null;
  localSettings?: DesktopSettings;
  localSettingsDirty?: boolean;
  mcpServers?: DesktopMcpServer[];
  mcpServersLoading?: boolean;
  mcpServersSaving?: boolean;
  gatewayDraft?: any;
  gatewayDirty?: boolean;
  gatewayLoading?: boolean;
  gatewayProfiles?: DesktopGatewayProfile[];
  gatewaySaving?: boolean;
  gatewaySettingsSource?: GatewaySettingsSource;
  gatewayStatusMessage?: string | null;
  performanceSnapshot: RendererPerformanceSnapshot;
  savingLocalSettings?: boolean;
  agents?: DesktopCustomAgent[];
  teams?: DesktopTeam[];
  skills?: DesktopSkillInfo[];
  workspaces?: DesktopWorkspace[];
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onCreateSlashCommand?: (input: UpsertSlashCommandInput) => Promise<void>;
  onUpdateSlashCommand?: (input: UpdateSlashCommandInput) => Promise<void>;
  onDeleteSlashCommand?: (name: string) => Promise<void>;
  onCreateMcpServer?: (input: UpsertMcpServerInput) => Promise<void>;
  onUpdateMcpServer?: (input: UpdateMcpServerInput) => Promise<void>;
  onDeleteMcpServer?: (name: string) => Promise<void>;
  onToggleMcpServer?: (name: string, enabled: boolean) => Promise<void>;
  onLocalSettingsChange?: (mutator: (current: DesktopSettings) => DesktopSettings) => void;
  onSaveLocalSettingsNow?: (options?: {
    requireGatewayConnection?: boolean;
    reloadGatewaySettings?: boolean;
  }) => Promise<boolean>;
  onSaveLocalSettingsDraft?: (
    nextSettings: DesktopSettings,
    options?: {
      requireGatewayConnection?: boolean;
      reloadGatewaySettings?: boolean;
    },
  ) => Promise<boolean>;
  onSaveGatewaySettings?: (options?: GatewaySettingsSaveOptions) => Promise<boolean>;
  onSaveGatewaySettingsPatch?: (
    patch: GatewayConfigDocument,
    options?: GatewaySettingsSaveOptions,
  ) => Promise<boolean>;
  onAddGatewayProfile?: (input: {
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
  }) => Promise<void>;
  onUpdateGatewayProfile?: (input: {
    profileId: string;
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
  }) => Promise<void>;
  onDeleteGatewayProfile?: (profileId: string) => Promise<void>;
  onMutateGatewayDraft?: DraftMutator;
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
  onMutate: (mutator: (provider: any) => void) => void;
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

type FixedModelProviderKey =
  | 'claude_code'
  | 'codex_app_server'
  | 'gemini_cli'
  | 'gpt'
  | 'anthropic'
  | 'google';

type FixedModelProviderRow = {
  key: FixedModelProviderKey;
  agentId: string;
  legacyAgentIds?: string[];
  label: string;
  providerType: DesktopApiProviderType;
  group: 'default' | 'native';
  defaultModel: string;
};

type ModelProviderConfigDraft = {
  key: FixedModelProviderKey;
  claudeCliMode: 'cctty' | 'native';
  claudeCliPath: string;
  claudeEnv: string;
  codexAuthMode: DesktopSettings['providerCodexAuthMode'];
  codexApiKey: string;
  geminiEnv: string;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
  authSource: string;
  apiKey: string;
  baseUrl: string;
};

const SLASH_COMMAND_NAME_PATTERN = /^[a-z0-9_]{1,32}$/;
const noop = () => {};
const noopAsync = async () => {};
const noopAsyncBoolean = async () => false;
const IDLE_UPDATE_STATUS: DesktopUpdateStatus = { phase: 'idle' };
const FOLLOW_UP_BEHAVIOR_TOGGLE_ITEM_CLASS =
  'relative h-8 !rounded-[12px] border-0 px-3 text-[12px] text-[#666663] data-[state=on]:z-10 data-[state=on]:bg-white data-[state=on]:text-[#111111] data-[state=on]:shadow-sm';
const MODEL_PROVIDER_ROWS: FixedModelProviderRow[] = [
  {
    key: 'claude_code',
    agentId: 'claude',
    label: 'Claude Code',
    providerType: 'claude_code',
    group: 'default',
    defaultModel: '(provider default)',
  },
  {
    key: 'codex_app_server',
    agentId: 'codex',
    label: 'Codex',
    providerType: 'codex_app_server',
    group: 'default',
    defaultModel: '(provider default)',
  },
  {
    key: 'gemini_cli',
    agentId: 'gemini',
    label: 'Gemini CLI',
    providerType: 'gemini_cli',
    group: 'default',
    defaultModel: 'gemini-3-flash-preview',
  },
  {
    key: 'gpt',
    agentId: 'gpt',
    label: 'GPT',
    providerType: 'gpt',
    group: 'native',
    defaultModel: 'gpt-5.5',
  },
  {
    key: 'anthropic',
    agentId: 'anthropic',
    legacyAgentIds: ['claude_llm'],
    label: 'Claude',
    providerType: 'anthropic',
    group: 'native',
    defaultModel: 'claude-sonnet-4-6',
  },
  {
    key: 'google',
    agentId: 'google',
    legacyAgentIds: ['gemini_llm'],
    label: 'Gemini',
    providerType: 'google',
    group: 'native',
    defaultModel: 'gemini-3-flash-preview',
  },
];
const MODEL_PROVIDER_SYSTEM_PROMPT =
  'You are Garyx. Follow the user request directly and use available tools when needed.';
const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default_model__';
const PROVIDER_DEFAULT_REASONING_VALUE = '__provider_default_reasoning__';

type UpdateFeedback = {
  message: string;
  tone: 'info' | 'success' | 'danger';
};

function updateCheckFailureMessage(reason: string, t: Translate): string {
  if (reason === 'dev-build') {
    return t('Update checks are available in packaged builds.');
  }
  if (reason === 'update-not-downloaded') {
    return t('The update is not ready to install yet.');
  }
  return reason || t('Failed to check for updates.');
}

function updateStatusDisplay(
  status: DesktopUpdateStatus,
  feedback: UpdateFeedback | null,
  t: Translate,
): UpdateFeedback {
  switch (status.phase) {
    case 'checking':
      return { message: t('Checking for updates...'), tone: 'info' };
    case 'available':
      return {
        message: t('Update v{version} found. Downloading will start automatically.', {
          version: status.info.version,
        }),
        tone: 'info',
      };
    case 'downloading':
      return {
        message: t('Downloading update ({percent}%).', {
          percent: Math.round(status.percent),
        }),
        tone: 'info',
      };
    case 'downloaded':
      return {
        message: t('Update v{version} is ready to install.', {
          version: status.info.version,
        }),
        tone: 'success',
      };
    case 'installing':
      return {
        message: t('Installing update v{version}...', {
          version: status.info.version,
        }),
        tone: 'info',
      };
    case 'error':
      return { message: status.message ? t(status.message) : t('Update check failed.'), tone: 'danger' };
    case 'idle':
    default:
      return feedback || {
        message: t('Garyx checks for updates automatically in the background.'),
        tone: 'info',
      };
  }
}

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
  disabled?: boolean;
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
  if (value === 'gpt') {
    return 'gpt';
  }
  if (value === 'anthropic' || value === 'claude_llm') {
    return 'anthropic';
  }
  if (value === 'google' || value === 'gemini_llm') {
    return 'google';
  }
  return 'claude';
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

  let normalizedProviderType: DesktopCustomAgent['providerType'] = 'claude_code';
  if (providerType === 'codex_app_server') {
    normalizedProviderType = 'codex_app_server';
  } else if (providerType === 'gemini_cli') {
    normalizedProviderType = 'gemini_cli';
  } else if (providerType === 'gpt') {
    normalizedProviderType = 'gpt';
  } else if (providerType === 'anthropic' || providerType === 'claude_llm') {
    normalizedProviderType = 'anthropic';
  } else if (providerType === 'google' || providerType === 'gemini_llm') {
    normalizedProviderType = 'google';
  }

  let builtInId = 'claude';
  if (normalizedProviderType === 'codex_app_server') {
    builtInId = 'codex';
  } else if (normalizedProviderType === 'gemini_cli') {
    builtInId = 'gemini';
  } else if (normalizedProviderType === 'gpt') {
    builtInId = 'gpt';
  } else if (normalizedProviderType === 'anthropic') {
    builtInId = 'anthropic';
  } else if (normalizedProviderType === 'google') {
    builtInId = 'google';
  }

  return agents.find((agent) => agent.agentId === builtInId)?.agentId
    || agents.find((agent) => agent.providerType === normalizedProviderType)?.agentId
    || agents[0]?.agentId
    || '';
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
  return buildAgentTargetOptions(agents, teams, { teamsFirst: true });
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

function fixedModelProviderRow(key: FixedModelProviderKey): FixedModelProviderRow {
  return MODEL_PROVIDER_ROWS.find((row) => row.key === key) || MODEL_PROVIDER_ROWS[0];
}

const REASONING_EFFORT_RANK: Record<string, number> = {
  off: 0,
  minimal: 1,
  low: 2,
  medium: 3,
  high: 4,
  xhigh: 5,
};

function providerModelOptionsWithCurrent(
  providerModels: DesktopProviderModels | null | undefined,
  currentModel: string,
): DesktopProviderModelOption[] {
  const options = providerModels?.models || [];
  const trimmed = currentModel.trim();
  if (!trimmed || options.some((model) => model.id === trimmed)) {
    return options;
  }
  return [
    ...options,
    {
      id: trimmed,
      label: trimmed,
      recommended: false,
      supportedReasoningEfforts: providerModels?.reasoningEfforts || [],
      serviceTiers: providerModels?.serviceTiers || [],
    },
  ];
}

function reasoningEffortOptionsForModel(
  providerModels: DesktopProviderModels | null | undefined,
  modelId: string,
  currentEffort: string,
): DesktopProviderModelOption[] {
  const selectedModel = providerModels?.models.find((model) => model.id === modelId.trim());
  const options = selectedModel?.supportedReasoningEfforts?.length
    ? selectedModel.supportedReasoningEfforts
    : providerModels?.reasoningEfforts || [];
  const trimmed = currentEffort.trim();
  if (!trimmed || options.some((option) => option.id === trimmed)) {
    return options;
  }
  return [
    ...options,
    {
      id: trimmed,
      label: trimmed,
      recommended: false,
    },
  ];
}

function serviceTierOptionsForModel(
  providerModels: DesktopProviderModels | null | undefined,
  modelId: string,
): DesktopProviderModelOption[] {
  const selectedModel = providerModels?.models.find((model) => model.id === modelId.trim());
  return selectedModel?.serviceTiers?.length
    ? selectedModel.serviceTiers
    : providerModels?.serviceTiers || [];
}

function highestReasoningEffort(options: DesktopProviderModelOption[]): string {
  return options.reduce((best, option) => {
    if (!best) {
      return option.id;
    }
    return (REASONING_EFFORT_RANK[option.id] ?? -1) > (REASONING_EFFORT_RANK[best] ?? -1)
      ? option.id
      : best;
  }, '');
}

function applyProviderCatalogDefaults(
  draft: ModelProviderConfigDraft,
  row: FixedModelProviderRow,
  providerModels: DesktopProviderModels | null | undefined,
): ModelProviderConfigDraft {
  if (row.group !== 'native' || !providerModels) {
    return draft;
  }
  const model = draft.model.trim()
    || providerModels.defaultModel?.trim()
    || (row.defaultModel.startsWith('(') ? '' : row.defaultModel);
  const reasoningOptions = reasoningEffortOptionsForModel(
    providerModels,
    model,
    draft.modelReasoningEffort,
  );
  const currentReasoning = draft.modelReasoningEffort.trim();
  const modelReasoningEffort = currentReasoning
    && reasoningOptions.some((option) => option.id === currentReasoning)
    ? currentReasoning
    : highestReasoningEffort(reasoningOptions);
  return {
    ...draft,
    model,
    modelReasoningEffort,
  };
}

function apiKeyEnvName(value: DesktopApiProviderType): string | null {
  if (value === 'gpt') {
    return 'OPENAI_API_KEY';
  }
  if (value === 'anthropic' || value === 'claude_llm') {
    return 'ANTHROPIC_API_KEY';
  }
  if (value === 'google' || value === 'gemini_llm') {
    return 'GEMINI_API_KEY';
  }
  return null;
}

function defaultNativeAuthSource(value: DesktopApiProviderType): string {
  return value === 'gpt' ? 'codex' : 'api_key';
}

function apiKeyFromProviderAgent(agent: DesktopCustomAgent | null | undefined): string {
  if (!agent) {
    return '';
  }
  const envName = apiKeyEnvName(agent.providerType);
  return envName ? agent.providerEnv?.[envName] || '' : '';
}

function configuredProviderAgent(
  agents: DesktopCustomAgent[],
  key: FixedModelProviderKey,
): DesktopCustomAgent | null {
  const row = fixedModelProviderRow(key);
  if (row.group !== 'native') {
    return null;
  }
  return agents.find((agent) => agent.agentId === row.agentId && !agent.builtIn)
    || agents.find((agent) => (row.legacyAgentIds || []).includes(agent.agentId) && !agent.builtIn)
    || null;
}

function providerAgentConfig(gatewayDraft: any, key: FixedModelProviderKey): Record<string, any> {
  const row = fixedModelProviderRow(key);
  const agentsConfig = gatewayDraft && typeof gatewayDraft === 'object' && gatewayDraft.agents && typeof gatewayDraft.agents === 'object'
    ? gatewayDraft.agents
    : {};
  const candidates = [row.agentId, ...(row.legacyAgentIds || []), row.key];
  for (const candidate of candidates) {
    const value = agentsConfig[candidate];
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      return value;
    }
  }
  return {};
}

function claudeAgentConfig(gatewayDraft: any): Record<string, any> {
  return providerAgentConfig(gatewayDraft, 'claude_code');
}

function normalizeClaudeCliMode(value: unknown): 'cctty' | 'native' {
  return String(value || '').trim().toLowerCase() === 'cctty' ? 'cctty' : 'native';
}

function claudeCliModeLabel(value: 'cctty' | 'native', t: Translate): string {
  return value === 'native' ? t('Native Claude CLI') : t('cctty TTY wrapper');
}

function emptyModelProviderConfigDraft(key: FixedModelProviderKey = 'claude_code'): ModelProviderConfigDraft {
  const row = fixedModelProviderRow(key);
  return {
    key,
    claudeCliMode: 'native',
    claudeCliPath: '',
    claudeEnv: '',
    codexAuthMode: 'cli',
    codexApiKey: '',
    geminiEnv: '',
    model: row.defaultModel.startsWith('(') ? '' : row.defaultModel,
    modelReasoningEffort: '',
    modelServiceTier: '',
    authSource: defaultNativeAuthSource(row.providerType),
    apiKey: '',
    baseUrl: '',
  };
}

function modelProviderDraftFromState(
  key: FixedModelProviderKey,
  localSettings: DesktopSettings,
  agents: DesktopCustomAgent[],
  gatewayDraft: any,
): ModelProviderConfigDraft {
  const row = fixedModelProviderRow(key);
  const agent = configuredProviderAgent(agents, key);
  const providerConfig = providerAgentConfig(gatewayDraft, key);
  return {
    key,
    claudeCliMode: normalizeClaudeCliMode(providerConfig.claude_cli_mode),
    claudeCliPath: String(providerConfig.claude_cli_path || ''),
    claudeEnv: localSettings.providerClaudeEnv,
    codexAuthMode: localSettings.providerCodexAuthMode,
    codexApiKey: localSettings.providerCodexApiKey,
    geminiEnv: localSettings.providerGeminiEnv,
    model: row.group === 'native'
      ? agent?.model || (row.defaultModel.startsWith('(') ? '' : row.defaultModel)
      : String(providerConfig.default_model || ''),
    modelReasoningEffort: row.group === 'native'
      ? agent?.modelReasoningEffort || ''
      : String(providerConfig.model_reasoning_effort || ''),
    modelServiceTier: row.group === 'native'
      ? agent?.modelServiceTier || ''
      : String(providerConfig.model_service_tier || ''),
    authSource: agent?.authSource || defaultNativeAuthSource(row.providerType),
    apiKey: apiKeyFromProviderAgent(agent),
    baseUrl: agent?.baseUrl || '',
  };
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
  disabled = false,
  label,
  onChange,
}: SettingsSwitchProps) {
  return (
    <Switch
      aria-label={label}
      checked={checked}
      disabled={disabled}
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

function GatewayProfileDialog({
  open,
  profile,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  profile: DesktopGatewayProfile | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (input: {
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
  }) => Promise<void>;
}) {
  const { t } = useI18n();
  const [label, setLabel] = useState('');
  const [gatewayUrl, setGatewayUrl] = useState('');
  const [gatewayAuthToken, setGatewayAuthToken] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setLabel(profile?.label ?? '');
      setGatewayUrl(profile?.gatewayUrl ?? '');
      setGatewayAuthToken(profile?.gatewayAuthToken ?? '');
    }
  }, [open, profile]);

  const canSave = useMemo(() => {
    try {
      const parsed = new URL(gatewayUrl.trim());
      return (parsed.protocol === 'http:' || parsed.protocol === 'https:') && Boolean(parsed.host);
    } catch {
      return false;
    }
  }, [gatewayUrl]);

  function resetFields() {
    setLabel('');
    setGatewayUrl('');
    setGatewayAuthToken('');
  }

  async function handleSave() {
    if (!canSave || saving) {
      return;
    }
    setSaving(true);
    try {
      await onSubmit({ label, gatewayUrl, gatewayAuthToken });
      resetFields();
      onOpenChange(false);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          resetFields();
        }
        onOpenChange(next);
      }}
    >
      <DialogContent className="gateway-add-dialog" size="compact">
        <DialogHeader>
          <DialogTitle>{profile ? t('Edit Gateway') : t('Add Gateway')}</DialogTitle>
          <DialogDescription>
            {t('Saved gateways appear in the sidebar gateway switcher.')}
          </DialogDescription>
        </DialogHeader>
        <div className="gateway-add-fields">
          <label className="gateway-setup-field">
            <span>{t('Name')}</span>
            <Input
              autoCapitalize="off"
              autoComplete="off"
              spellCheck={false}
              type="text"
              value={label}
              onChange={(event) => setLabel(event.target.value)}
            />
          </label>
          <label className="gateway-setup-field">
            <span>{t('Gateway URL')}</span>
            <Input
              autoCapitalize="off"
              autoComplete="off"
              placeholder="http://127.0.0.1:31337"
              spellCheck={false}
              type="text"
              value={gatewayUrl}
              onChange={(event) => setGatewayUrl(event.target.value)}
            />
          </label>
          <label className="gateway-setup-field">
            <span>{t('Gateway Token')}</span>
            <Input
              autoCapitalize="off"
              autoComplete="off"
              spellCheck={false}
              type="password"
              value={gatewayAuthToken}
              onChange={(event) => setGatewayAuthToken(event.target.value)}
            />
          </label>
        </div>
        <DialogFooter>
          <Button
            className="rounded-xl border-[#e7e7e5] bg-white shadow-none hover:bg-[#f7f7f6]"
            onClick={() => onOpenChange(false)}
            type="button"
            variant="outline"
          >
            {t('Cancel')}
          </Button>
          <Button
            className="rounded-xl bg-[#111111] text-white shadow-none hover:bg-[#222222]"
            disabled={!canSave || saving}
            onClick={() => void handleSave()}
            type="button"
          >
            {t('Save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
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
  const { t } = useI18n();
  const summaryItems: SummaryItem[] = [
    { label: t('provider'), value: providerTypeLabel(provider) },
    ...summaries.filter((item) => item.value.trim().length > 0),
  ];

  return (
    <details className="settings-card settings-account-card settings-collapsible-card">
      <summary className="settings-card-summary">
        <div className="settings-card-summary-main">
          <div className="settings-card-summary-title">
            <strong>{accountId}</strong>
            <span className={`status-pill ${enabled ? '' : 'offline'}`}>
              {enabled ? t('enabled') : t('disabled')}
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
          <p className="small-note">{t('Open only when you need detailed config for this bot.')}</p>
          <Button
            className="rounded-xl border-[#f0d9d9] bg-white text-[#9b3d3d] shadow-none hover:bg-[#fdf3f3]"
            onClick={onRemove}
            size="sm"
            type="button"
            variant="outline"
          >
            {t('Remove')}
          </Button>
        </div>
        {children}
      </div>
    </details>
  );
}

function AgentProviderFields({
  provider,
  onMutate,
}: AgentProviderFieldsProps) {
  const { t } = useI18n();
  const providerType = providerTypeValue(provider);

  return (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">{t('Agent Provider')}</span>
        <span className="codex-section-note">{t('Provider runtime')}</span>
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
                <SelectGroup>
                  <SelectItem value="claude_code">claude_code</SelectItem>
                  <SelectItem value="codex_app_server">codex_app_server</SelectItem>
                  <SelectItem value="gemini_cli">gemini_cli</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          }
          description={t('Select the runtime backing this bot.')}
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
          description={t('Workspace bound to this bot. When the bot creates its first thread, that thread starts in this workspace.')}
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
  localSettingsDirty = false,
  mcpServers = [],
  mcpServersLoading = false,
  mcpServersSaving = false,
  gatewayDraft = {},
  gatewayDirty = false,
  gatewayLoading = false,
  gatewaySaving = false,
  gatewaySettingsSource = 'gateway_api',
  gatewayStatusMessage = null,
  performanceSnapshot,
  savingLocalSettings = false,
  agents = [],
  teams = [],
  skills = [],
  workspaces = [],
  onAddWorkspace,
  onCreateSlashCommand = noopAsync,
  onUpdateSlashCommand = noopAsync,
  onDeleteSlashCommand = noopAsync,
  onCreateMcpServer = noopAsync,
  onUpdateMcpServer = noopAsync,
  onDeleteMcpServer = noopAsync,
  onToggleMcpServer = noopAsync,
  onLocalSettingsChange = noop,
  onSaveLocalSettingsNow = noopAsyncBoolean,
  onSaveLocalSettingsDraft = noopAsyncBoolean,
  onSaveGatewaySettings = noopAsyncBoolean,
  onSaveGatewaySettingsPatch = noopAsyncBoolean,
  gatewayProfiles = [],
  onAddGatewayProfile = noopAsync,
  onUpdateGatewayProfile = noopAsync,
  onDeleteGatewayProfile = noopAsync,
  onMutateGatewayDraft = noop,
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
  const { t } = useI18n();
  const normalizedActiveTab: SettingsTabId =
    activeTab === 'connection' ? 'gateway' : activeTab;
  const pluginAccounts = configuredChannelAccountsFromDraft(gatewayDraft?.channels);
  const [isAddingChannel, setIsAddingChannel] = useState(false);
  const [gatewayDialogOpen, setGatewayDialogOpen] = useState(false);
  const [gatewayDialogProfile, setGatewayDialogProfile] = useState<DesktopGatewayProfile | null>(null);
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
  const [editingCommandName, setEditingCommandName] = useState<string | null>(null);
  const [commandDraft, setCommandDraft] = useState<CommandDraft>(() => emptyCommandDraft());
  const [commandDialogOpen, setCommandDialogOpen] = useState(false);
  const [editingMcpServerName, setEditingMcpServerName] = useState<string | null>(null);
  const [mcpServerDraft, setMcpServerDraft] = useState<McpServerDraft>(() => emptyMcpServerDraft());
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);
  const [providerConfigKey, setProviderConfigKey] = useState<FixedModelProviderKey | null>(null);
  const [providerConfigDraft, setProviderConfigDraft] = useState<ModelProviderConfigDraft>(() =>
    emptyModelProviderConfigDraft(),
  );
  const [providerConfigSaving, setProviderConfigSaving] = useState(false);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<DesktopApiProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<DesktopApiProviderType, boolean>>
  >({});
  const [updateStatus, setUpdateStatus] = useState<DesktopUpdateStatus>(IDLE_UPDATE_STATUS);
  const [updateFeedback, setUpdateFeedback] = useState<UpdateFeedback | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [desktopAppVersion, setDesktopAppVersion] = useState(UNKNOWN_DESKTOP_APP_VERSION);
  const updateStatusRef = useRef<DesktopUpdateStatus>(IDLE_UPDATE_STATUS);
  const statusClass =
    gatewayStatusMessage && /(failed|error|invalid)/i.test(gatewayStatusMessage)
      ? 'error'
      : 'info';
  const remoteSyncLabel = gatewayLoading
    ? t('Refreshing latest remote config…')
    : gatewaySaving
      ? t('Saving config…')
      : gatewayDirty
        ? t('Unsaved config changes. Click Save to persist them.')
        : t('Config changes save only when you click Save.');
  const activeTabMeta =
    SETTINGS_TABS.find((tab) => tab.id === normalizedActiveTab) || SETTINGS_TABS[0];
  const configuredChannelCount = pluginAccounts.length;
  const enabledMcpServerCount = mcpServers.filter((server) => server.enabled).length;
  const syncStateLabel = gatewaySaving
    ? t('Saving')
    : gatewayLoading
      ? t('Refreshing')
      : gatewayDirty
        ? t('Unsaved')
        : t('Saved');
  const syncFactTone: SettingsFactTone =
    statusClass === 'error'
      ? 'danger'
      : gatewayDirty || gatewaySaving || gatewayLoading
        ? 'default'
        : 'success';
  const desktopStateTone: SettingsFactTone = connection?.ok ? 'success' : 'danger';
  const claudeEnvLineCount = countNonEmptyLines(localSettings.providerClaudeEnv);
  const geminiEnvLineCount = countNonEmptyLines(localSettings.providerGeminiEnv);
  const providerConfigRow = providerConfigKey ? fixedModelProviderRow(providerConfigKey) : null;
  const providerConfigAgent = providerConfigKey
    ? configuredProviderAgent(agents, providerConfigKey)
    : null;
  const activeProviderModels = providerConfigRow
    ? providerModelsByType[providerConfigRow.providerType] || null
    : null;
  const activeProviderModelsLoading = providerConfigRow
    ? providerModelsLoading[providerConfigRow.providerType] === true
    : false;
  const activeProviderModelOptions = providerModelOptionsWithCurrent(
    activeProviderModels,
    providerConfigDraft.model,
  );
  const activeReasoningOptions = reasoningEffortOptionsForModel(
    activeProviderModels,
    providerConfigDraft.model,
    providerConfigDraft.modelReasoningEffort,
  );
  const activeServiceTierOptions = serviceTierOptionsForModel(
    activeProviderModels,
    providerConfigDraft.model,
  );
  const channelsSourceMessage = gatewaySettingsSource === 'local_file'
    ? t('Editing the gateway runtime config file on the gateway host.')
    : t('Editing live gateway settings over the API.');
  const showGatewayHeaderStatus = normalizedActiveTab === 'gateway';
  const headerFacts: Array<{
    label: string;
    value: string;
    tone?: SettingsFactTone;
  }> = [
    {
      label: t('desktop'),
      value: connection?.ok ? t('online') : t('offline'),
      tone: desktopStateTone,
    },
    {
      label: t('sync'),
      value: syncStateLabel.toLowerCase(),
      tone: syncFactTone,
    },
    {
      label: t('saved'),
      value: localSettings.gatewayUrl.replace(/^https?:\/\//, '') || '(empty)',
    },
    {
      label: t('auth'),
      value: localSettings.gatewayAuthToken.trim() ? t('configured') : t('required'),
      tone: localSettings.gatewayAuthToken.trim() ? 'success' : 'danger',
    },
  ];
  const updateDisplay = updateStatusDisplay(updateStatus, updateFeedback, t);
  const updateCheckBusy =
    checkingUpdate
    || updateStatus.phase === 'checking'
    || updateStatus.phase === 'available'
    || updateStatus.phase === 'downloading';
  const updateCheckDisabled = updateCheckBusy || installingUpdate;

  useEffect(() => {
    const api = window.garyxDesktop;
    let cancelled = false;
    const listener = (next: DesktopUpdateStatus) => {
      if (cancelled) return;
      updateStatusRef.current = next;
      setUpdateStatus(next);
      if (next.phase !== 'idle') {
        setUpdateFeedback(null);
      }
    };

    void api.getUpdateStatus().then((initial) => {
      if (cancelled) return;
      updateStatusRef.current = initial;
      setUpdateStatus(initial);
    }).catch(() => {
      if (cancelled) return;
      setUpdateFeedback({
        message: t('Failed to read update status.'),
        tone: 'danger',
      });
    });
    api.subscribeUpdateStatus(listener);

    return () => {
      cancelled = true;
      api.unsubscribeUpdateStatus(listener);
    };
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    void window.garyxDesktop.getAppVersion().then((version) => {
      if (cancelled) return;
      setDesktopAppVersion(version.trim() || UNKNOWN_DESKTOP_APP_VERSION);
    }).catch(() => {
      if (cancelled) return;
      setDesktopAppVersion(UNKNOWN_DESKTOP_APP_VERSION);
    });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!providerConfigRow) {
      return;
    }
    const providerType = providerConfigRow.providerType;
    if (providerModelsByType[providerType] || providerModelsLoading[providerType]) {
      return;
    }
    let cancelled = false;
    setProviderModelsLoading((current) => ({
      ...current,
      [providerType]: true,
    }));
    void window.garyxDesktop.listProviderModels(providerType).then((models) => {
      if (cancelled) return;
      setProviderModelsByType((current) => ({
        ...current,
        [providerType]: models,
      }));
    }).catch(() => {
      // The dialog keeps the raw model input fallback if catalog loading fails.
    }).finally(() => {
      setProviderModelsLoading((current) => ({
        ...current,
        [providerType]: false,
      }));
    });
    return () => {
      cancelled = true;
    };
  }, [providerConfigRow?.providerType, providerModelsByType, providerModelsLoading]);

  useEffect(() => {
    if (!providerConfigRow || providerConfigRow.group !== 'native' || !activeProviderModels) {
      return;
    }
    setProviderConfigDraft((current) => {
      if (current.key !== providerConfigRow.key) {
        return current;
      }
      return applyProviderCatalogDefaults(current, providerConfigRow, activeProviderModels);
    });
  }, [providerConfigRow?.key, providerConfigRow?.group, activeProviderModels]);

  function renderGatewaySaveAction(_buttonLabel?: string) {
    const statusLabel = gatewaySaving
      ? t('Saving…')
      : gatewayDirty
        ? t('Unsaved')
        : t('Saved');
    return <span className="codex-autosave-status">{statusLabel}</span>;
  }

  function renderLocalSaveAction(label = t('Save Desktop Settings')) {
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
        {savingLocalSettings ? t('Saving…') : label}
      </Button>
    );
  }

  async function handleCheckForUpdatesNow() {
    if (checkingUpdate || installingUpdate) {
      return;
    }
    setCheckingUpdate(true);
    setUpdateFeedback(null);
    try {
      const result = await window.garyxDesktop.checkForUpdatesNow();
      if (!result.ok) {
        setUpdateFeedback({
          message: updateCheckFailureMessage(result.reason, t),
          tone: 'danger',
        });
        return;
      }
      if (updateStatusRef.current.phase === 'idle') {
        setUpdateFeedback({
          message: t('No update found.'),
          tone: 'success',
        });
      }
    } catch {
      setUpdateFeedback({
        message: t('Failed to check for updates.'),
        tone: 'danger',
      });
    } finally {
      setCheckingUpdate(false);
    }
  }

  async function handleInstallUpdate() {
    if (installingUpdate) {
      return;
    }
    setInstallingUpdate(true);
    setUpdateFeedback(null);
    try {
      const result = await window.garyxDesktop.installUpdate();
      if (!result.ok) {
        setUpdateFeedback({
          message: updateCheckFailureMessage(result.reason, t),
          tone: 'danger',
        });
        setInstallingUpdate(false);
      }
    } catch {
      setUpdateFeedback({
        message: t('Failed to install update.'),
        tone: 'danger',
      });
      setInstallingUpdate(false);
    }
  }

  function providerRowDetails(row: FixedModelProviderRow): {
    status: string;
    auth: string;
    model: string;
  } {
    const runtimeConfig = providerAgentConfig(gatewayDraft, row.key);
    const configuredDefaultModel = String(runtimeConfig.default_model || '').trim();
    if (row.key === 'claude_code') {
      const mode = normalizeClaudeCliMode(claudeAgentConfig(gatewayDraft).claude_cli_mode);
      return {
        status: t('Default'),
        auth: claudeEnvLineCount
          ? `${claudeCliModeLabel(mode, t)} · ${t('{count} env vars', { count: claudeEnvLineCount })}`
          : claudeCliModeLabel(mode, t),
        model: configuredDefaultModel || row.defaultModel,
      };
    }
    if (row.key === 'codex_app_server') {
      return {
        status: t('Default'),
        auth: localSettings.providerCodexAuthMode === 'api_key' ? t('API Key') : t('CLI'),
        model: configuredDefaultModel || row.defaultModel,
      };
    }
    if (row.key === 'gemini_cli') {
      return {
        status: t('Default'),
        auth: geminiEnvLineCount
          ? t('{count} env vars', { count: geminiEnvLineCount })
          : t('CLI / env'),
        model: configuredDefaultModel || row.defaultModel,
      };
    }

    const agent = configuredProviderAgent(agents, row.key);
    const authSource = agent?.authSource || defaultNativeAuthSource(row.providerType);
    return {
      status: agent ? t('Configured') : t('Not configured'),
      auth: row.providerType === 'gpt' && authSource === 'codex'
        ? t('GPT token')
        : apiKeyFromProviderAgent(agent)
          ? t('API Key')
          : t('Env / API key'),
      model: agent?.model || row.defaultModel,
    };
  }

  function openProviderConfigDialog(key: FixedModelProviderKey) {
    const row = fixedModelProviderRow(key);
    const draft = modelProviderDraftFromState(key, localSettings, agents, gatewayDraft);
    setProviderConfigDraft(applyProviderCatalogDefaults(draft, row, providerModelsByType[row.providerType]));
    setProviderConfigKey(key);
  }

  function closeProviderConfigDialog() {
    setProviderConfigKey(null);
    setProviderConfigDraft(emptyModelProviderConfigDraft());
  }

  function mutateGatewayProviderModelDefaults(
    row: FixedModelProviderRow,
    draft: ModelProviderConfigDraft,
  ) {
    onMutateGatewayDraft((next) => {
      next.agents = next.agents || {};
      const current = next.agents[row.agentId] && typeof next.agents[row.agentId] === 'object'
        ? next.agents[row.agentId]
        : {};
      next.agents[row.agentId] = {
        ...current,
        provider_type: row.providerType,
        default_model: draft.model.trim(),
        model_reasoning_effort: draft.modelReasoningEffort.trim(),
      };
    });
  }

  async function handleSaveProviderConfig() {
    if (!providerConfigRow || providerConfigSaving) {
      return;
    }
    setProviderConfigSaving(true);
    try {
      if (providerConfigRow.key === 'claude_code') {
        const nextSettings = {
          ...localSettings,
          providerClaudeEnv: providerConfigDraft.claudeEnv,
        };
        const savedLocal = await onSaveLocalSettingsDraft(nextSettings, { reloadGatewaySettings: false });
        if (!savedLocal) {
          return;
        }
        onMutateGatewayDraft((next) => {
          next.agents = next.agents || {};
          const current = next.agents.claude && typeof next.agents.claude === 'object'
            ? next.agents.claude
            : {};
          next.agents.claude = {
            ...current,
            provider_type: 'claude_code',
            claude_cli_mode: providerConfigDraft.claudeCliMode,
            default_model: providerConfigDraft.model.trim(),
            model_reasoning_effort: providerConfigDraft.modelReasoningEffort.trim(),
          };
          const cliPath = providerConfigDraft.claudeCliPath.trim();
          if (cliPath) {
            next.agents.claude.claude_cli_path = cliPath;
          } else {
            delete next.agents.claude.claude_cli_path;
          }
        });
        if (await onSaveGatewaySettings({ refreshDesktopState: 'background' })) {
          closeProviderConfigDialog();
        }
        return;
      }
      if (providerConfigRow.key === 'codex_app_server') {
        const nextSettings = {
          ...localSettings,
          providerCodexAuthMode: providerConfigDraft.codexAuthMode,
          providerCodexApiKey: providerConfigDraft.codexApiKey,
        };
        const savedLocal = await onSaveLocalSettingsDraft(nextSettings, { reloadGatewaySettings: false });
        if (!savedLocal) {
          return;
        }
        mutateGatewayProviderModelDefaults(providerConfigRow, providerConfigDraft);
        if (await onSaveGatewaySettings({ refreshDesktopState: 'background' })) {
          closeProviderConfigDialog();
        }
        return;
      }
      if (providerConfigRow.key === 'gemini_cli') {
        const nextSettings = {
          ...localSettings,
          providerGeminiEnv: providerConfigDraft.geminiEnv,
        };
        const savedLocal = await onSaveLocalSettingsDraft(nextSettings, { reloadGatewaySettings: false });
        if (!savedLocal) {
          return;
        }
        mutateGatewayProviderModelDefaults(providerConfigRow, providerConfigDraft);
        if (await onSaveGatewaySettings({ refreshDesktopState: 'background' })) {
          closeProviderConfigDialog();
        }
        return;
      }

      const envName = apiKeyEnvName(providerConfigRow.providerType);
      const providerEnv = envName && providerConfigDraft.apiKey.trim()
        ? { [envName]: providerConfigDraft.apiKey.trim() }
        : {};
      const payload: CreateCustomAgentInput = {
        agentId: providerConfigRow.agentId,
        displayName: providerConfigRow.label,
        providerType: providerConfigRow.providerType,
        model: providerConfigDraft.model.trim(),
        modelReasoningEffort: providerConfigDraft.modelReasoningEffort.trim(),
        modelServiceTier: providerConfigRow.providerType === 'gpt'
          ? providerConfigDraft.modelServiceTier.trim()
          : '',
        providerEnv,
        authSource: providerConfigDraft.authSource.trim()
          || defaultNativeAuthSource(providerConfigRow.providerType),
        baseUrl: providerConfigDraft.baseUrl.trim(),
        defaultWorkspaceDir: '',
        systemPrompt: providerConfigAgent?.systemPrompt || MODEL_PROVIDER_SYSTEM_PROMPT,
      };
      if (providerConfigAgent) {
        if (providerConfigAgent.agentId !== providerConfigRow.agentId) {
          await window.garyxDesktop.createCustomAgent(payload);
          await window.garyxDesktop.deleteCustomAgent({ agentId: providerConfigAgent.agentId });
        } else {
          const updatePayload: UpdateCustomAgentInput = {
            ...payload,
            currentAgentId: providerConfigAgent.agentId,
          };
          await window.garyxDesktop.updateCustomAgent(updatePayload);
        }
      } else {
        await window.garyxDesktop.createCustomAgent(payload);
      }
      await onRefreshAgentTargets();
      closeProviderConfigDialog();
    } finally {
      setProviderConfigSaving(false);
    }
  }

  async function handleClearProviderConfig() {
    if (!providerConfigRow || providerConfigRow.group !== 'native' || !providerConfigAgent) {
      return;
    }
    if (!window.confirm(t('Clear configuration for {name}?', { name: providerConfigRow.label }))) {
      return;
    }
    setProviderConfigSaving(true);
    try {
      await window.garyxDesktop.deleteCustomAgent({ agentId: providerConfigAgent.agentId });
      await onRefreshAgentTargets();
      closeProviderConfigDialog();
    } finally {
      setProviderConfigSaving(false);
    }
  }

  const currentGatewayKey = localSettings.gatewayUrl.trim().toLowerCase();
  // Saved order is kept as-is; the active gateway is marked, not moved.
  const savedGatewayProfiles = useMemo(() => {
    return gatewayProfiles.filter((profile) => profile.gatewayUrl.trim().length > 0);
  }, [gatewayProfiles]);

  // The settings tab manages the saved gateway list only; switching the
  // active gateway lives in the sidebar identity bar.
  const connectionPanel = (
    <div className="codex-section">
      <div className="codex-section-header gateway-profiles-header">
        <Button
          className="rounded-xl border-[#e7e7e5] bg-white shadow-none hover:bg-[#f7f7f6]"
          onClick={() => {
            setGatewayDialogProfile(null);
            setGatewayDialogOpen(true);
          }}
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus aria-hidden size={14} strokeWidth={1.8} />
          {t('Add Gateway')}
        </Button>
      </div>
      <div className="codex-list-card gateway-profiles-card">
        {savedGatewayProfiles.length === 0 ? (
          <p className="gateway-profiles-empty">{t('No saved gateways yet.')}</p>
        ) : (
          savedGatewayProfiles.map((profile) => {
            const isCurrent = profile.gatewayUrl.trim().toLowerCase() === currentGatewayKey;
            return (
              <div className="gateway-profile-row" key={profile.id}>
                <span aria-hidden className="gateway-row-glyph">
                  <Server size={13} strokeWidth={1.8} />
                  {isCurrent ? (
                    <span
                      className={`gateway-glyph-badge ${connection?.ok ? 'is-connected' : 'is-syncing'}`}
                    />
                  ) : null}
                </span>
                <span className="gateway-profile-row-copy">
                  <span className="gateway-profile-row-name">{profile.label}</span>
                  <span className="gateway-profile-row-url">{profile.gatewayUrl}</span>
                </span>
                {isCurrent ? (
                  <span className="gateway-profile-current">{t('Current')}</span>
                ) : null}
                <button
                  aria-label={t('Edit Gateway')}
                  className="gateway-profile-edit"
                  onClick={() => {
                    setGatewayDialogProfile(profile);
                    setGatewayDialogOpen(true);
                  }}
                  title={t('Edit Gateway')}
                  type="button"
                >
                  <Pencil aria-hidden size={13} strokeWidth={1.8} />
                </button>
                {!isCurrent ? (
                  <button
                    aria-label={t('Remove')}
                    className="gateway-profile-delete"
                    onClick={() => {
                      if (
                        window.confirm(
                          t('Remove {label} from saved gateways?', { label: profile.label }),
                        )
                      ) {
                        void onDeleteGatewayProfile(profile.id);
                      }
                    }}
                    title={t('Remove')}
                    type="button"
                  >
                    <Trash aria-hidden size={13} strokeWidth={1.8} />
                  </button>
                ) : null}
              </div>
            );
          })
        )}
      </div>
      <GatewayProfileDialog
        open={gatewayDialogOpen}
        profile={gatewayDialogProfile}
        onOpenChange={setGatewayDialogOpen}
        onSubmit={async (input) => {
          if (gatewayDialogProfile) {
            await onUpdateGatewayProfile({
              profileId: gatewayDialogProfile.id,
              ...input,
            });
            return;
          }
          await onAddGatewayProfile(input);
        }}
      />
    </div>
  );

  // Client-side preferences live on the General tab; the gateway tab
  // only manages the gateway connection.
  const desktopSettingsSection = (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">{t('Desktop Settings')}</span>
      </div>
      <div className="codex-list-card">
        <SettingsControlRow
          control={
            <Select
              value={localSettings.languagePreference}
              onValueChange={(value) => {
                onLocalSettingsChange((current) => ({
                  ...current,
                  languagePreference: value === 'en' || value === 'zh-CN' ? value : 'system',
                }));
              }}
            >
              <SelectTrigger className="rounded-[14px] border-[#e7e7e5] bg-white shadow-none">
                <SelectValue
                  placeholder={languagePreferenceLabel(localSettings.languagePreference, t)}
                />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="system">{t('Follow System')}</SelectItem>
                  <SelectItem value="en">{t('English')}</SelectItem>
                  <SelectItem value="zh-CN">{t('Chinese')}</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          }
          description={t('Select the language used by this Mac app. System follows macOS language and falls back to English.')}
          label={t('Language')}
        />
        <SettingsControlRow
          control={
            <ToggleGroup
              className="h-9 rounded-[14px] bg-[#f3f3f1] p-0.5"
              type="single"
              value={localSettings.followUpBehavior}
              onValueChange={(nextValue) => {
                if (nextValue !== 'queue' && nextValue !== 'steer') {
                  return;
                }
                onLocalSettingsChange((current) => ({
                  ...current,
                  followUpBehavior: nextValue as DesktopFollowUpBehavior,
                }));
              }}
            >
              <ToggleGroupItem
                className={FOLLOW_UP_BEHAVIOR_TOGGLE_ITEM_CLASS}
                value="queue"
              >
                {t('Queue')}
              </ToggleGroupItem>
              <ToggleGroupItem
                className={FOLLOW_UP_BEHAVIOR_TOGGLE_ITEM_CLASS}
                value="steer"
              >
                {t('Steer')}
              </ToggleGroupItem>
            </ToggleGroup>
          }
          description={t('Choose whether follow-ups sent while Garyx is running are queued or sent into the active run. Press Command+Enter to use the opposite behavior for one message.')}
          label={t('Follow-up behavior')}
        />
        {localSettingsDirty ? (
          <SettingsControlRow
            control={<div className="settings-control-actions">{renderLocalSaveAction()}</div>}
            label={t('Desktop Settings')}
          />
        ) : null}
      </div>
    </div>
  );

  const gatewayPanel = <>{connectionPanel}</>;

  const providerConfigTablePanel = (
    <section className="provider-section">
      <div className="provider-section-head">
        <h2 className="provider-section-title">{t('Configured Providers')}</h2>
      </div>
      <div className="provider-config-table">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="provider-config-col-provider">{t('Provider')}</TableHead>
              <TableHead className="provider-config-col-kind">{t('Type')}</TableHead>
              <TableHead className="provider-config-col-auth">{t('Auth')}</TableHead>
              <TableHead className="provider-config-col-model">{t('Model')}</TableHead>
              <TableHead className="provider-config-col-status">{t('Status')}</TableHead>
              <TableHead className="provider-config-col-actions">{t('Actions')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {MODEL_PROVIDER_ROWS.map((row) => {
              const details = providerRowDetails(row);
              const modelLabel = details.model === '(provider default)'
                ? t('(provider default)')
                : details.model;
              const rowReady = row.group === 'default' || Boolean(configuredProviderAgent(agents, row.key));
              return (
                <TableRow key={row.key}>
                  <TableCell className="provider-config-col-provider">
                    <div className="provider-config-name-cell">
                      <span className="provider-config-name">{row.label}</span>
                      {row.group === 'default' ? (
                        <span className="provider-config-subtitle">{t('Built-in')}</span>
                      ) : (
                        <span className="provider-config-subtitle">{t('Native agent loop')}</span>
                      )}
                    </div>
                  </TableCell>
                  <TableCell className="provider-config-col-kind">
                    <code>{row.providerType}</code>
                  </TableCell>
                  <TableCell className="provider-config-col-auth">{details.auth}</TableCell>
                  <TableCell className="provider-config-col-model" title={modelLabel}>
                    {modelLabel}
                  </TableCell>
                  <TableCell className="provider-config-col-status">
                    <Badge
                      className="provider-config-status"
                      data-state={rowReady ? 'ready' : 'empty'}
                      variant="outline"
                    >
                      {details.status}
                    </Badge>
                  </TableCell>
                  <TableCell className="provider-config-col-actions">
                    <button
                      className="command-row-action"
                      onClick={() => { openProviderConfigDialog(row.key); }}
                      type="button"
                    >
                      {t('Configure')}
                    </button>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      </div>
    </section>
  );

  const providerPanel = (
    <div className="settings-form provider-panel">
      {providerConfigTablePanel}
      <Dialog
        open={Boolean(providerConfigKey)}
        onOpenChange={(open) => {
          if (!open) {
            closeProviderConfigDialog();
          }
        }}
      >
        <DialogContent
          className="provider-config-dialog"
          showCloseButton={false}
          size="form"
        >
          <DialogHeader className="commands-dialog-header">
            <Badge variant="outline" className="commands-dialog-badge">
              {providerConfigRow?.group === 'native' ? t('Native Provider') : t('Default Provider')}
            </Badge>
            <div className="commands-dialog-title-group">
              <DialogTitle className="commands-dialog-title">
                {providerConfigRow ? t('Configure {name}', { name: providerConfigRow.label }) : t('Configure Provider')}
              </DialogTitle>
              <DialogDescription className="commands-dialog-description">
                {t('Provider rows are fixed. Configuration controls whether each provider is ready to use.')}
              </DialogDescription>
            </div>
          </DialogHeader>

          <div className="commands-dialog-body provider-config-dialog-body">
            {providerConfigRow?.key === 'claude_code' ? (
              <>
                <div className="commands-field">
                  <Label className="commands-field-label">{t('Agent SDK CLI')}</Label>
                  <Select
                    value={providerConfigDraft.claudeCliMode}
                    onValueChange={(value) => {
                      setProviderConfigDraft((current) => ({
                        ...current,
                        claudeCliMode: value === 'native' ? 'native' : 'cctty',
                      }));
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="native">{t('Native Claude CLI')}</SelectItem>
                        <SelectItem value="cctty">{t('cctty TTY wrapper')}</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
                <div className="commands-field">
                  <div className="commands-field-header">
                    <Label className="commands-field-label" htmlFor="provider-claude-cli-path">{t('CLI path')}</Label>
                    <span className="commands-field-hint">{t('Leave empty to use native Claude from PATH or embedded cctty.')}</span>
                  </div>
                  <Input
                    id="provider-claude-cli-path"
                    placeholder={providerConfigDraft.claudeCliMode === 'native' ? 'claude' : 'cctty'}
                    value={providerConfigDraft.claudeCliPath}
                    onChange={(event) => {
                      setProviderConfigDraft((current) => ({
                        ...current,
                        claudeCliPath: event.target.value,
                      }));
                    }}
                  />
                </div>
                <div className="commands-field">
                  <div className="commands-field-header">
                    <Label className="commands-field-label" htmlFor="provider-claude-env">{t('Environment')}</Label>
                    <span className="commands-field-hint">{t('One variable per line.')}</span>
                  </div>
                  <Textarea
                    className="provider-env-editor"
                    id="provider-claude-env"
                    placeholder={[
                      'ANTHROPIC_API_KEY=sk-ant-...',
                      'CLAUDE_CODE_USE_BEDROCK=1',
                      'AWS_REGION=us-east-1',
                      'AWS_PROFILE=default',
                    ].join('\n')}
                    spellCheck={false}
                    value={providerConfigDraft.claudeEnv}
                    onChange={(event) => {
                      setProviderConfigDraft((current) => ({
                        ...current,
                        claudeEnv: event.target.value,
                      }));
                    }}
                  />
                </div>
              </>
            ) : null}

            {providerConfigRow?.key === 'codex_app_server' ? (
              <>
                <div className="commands-field">
                  <Label className="commands-field-label">{t('Auth')}</Label>
                  <Select
                    value={providerConfigDraft.codexAuthMode}
                    onValueChange={(value) => {
                      setProviderConfigDraft((current) => ({
                        ...current,
                        codexAuthMode: value === 'api_key' ? 'api_key' : 'cli',
                      }));
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="cli">{t('CLI')}</SelectItem>
                        <SelectItem value="api_key">{t('API Key')}</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
                {providerConfigDraft.codexAuthMode === 'api_key' ? (
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-codex-api-key">{t('API Key')}</Label>
                    <Input
                      autoCapitalize="off"
                      autoComplete="off"
                      id="provider-codex-api-key"
                      placeholder="OPENAI_API_KEY"
                      spellCheck={false}
                      type="password"
                      value={providerConfigDraft.codexApiKey}
                      onChange={(event) => {
                        setProviderConfigDraft((current) => ({
                          ...current,
                          codexApiKey: event.target.value,
                        }));
                      }}
                    />
                  </div>
                ) : null}
              </>
            ) : null}

            {providerConfigRow?.key === 'gemini_cli' ? (
              <div className="commands-field">
                <div className="commands-field-header">
                  <Label className="commands-field-label" htmlFor="provider-gemini-env">{t('Environment')}</Label>
                  <span className="commands-field-hint">{t('One variable per line.')}</span>
                </div>
                <Textarea
                  className="provider-env-editor"
                  id="provider-gemini-env"
                  placeholder={[
                    'GEMINI_API_KEY=...',
                    'GOOGLE_API_KEY=...',
                    'GEMINI_CLI_HOME=~/.gemini',
                  ].join('\n')}
                  spellCheck={false}
                  value={providerConfigDraft.geminiEnv}
                  onChange={(event) => {
                    setProviderConfigDraft((current) => ({
                      ...current,
                      geminiEnv: event.target.value,
                    }));
                  }}
                />
              </div>
            ) : null}

            {providerConfigRow ? (
              <>
                {providerConfigRow.group === 'native' && providerConfigRow.providerType === 'gpt' ? (
                  <div className="commands-field">
                    <Label className="commands-field-label">{t('Auth')}</Label>
                    <Select
                      value={providerConfigDraft.authSource || 'codex'}
                      onValueChange={(value) => {
                        setProviderConfigDraft((current) => ({
                          ...current,
                          authSource: value,
                          apiKey: value === 'codex' ? '' : current.apiKey,
                        }));
                      }}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectGroup>
                          <SelectItem value="codex">{t('Use GPT token')}</SelectItem>
                          <SelectItem value="api_key">{t('Use API key')}</SelectItem>
                        </SelectGroup>
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}
                {providerConfigRow.group === 'native' && (providerConfigRow.providerType !== 'gpt' || providerConfigDraft.authSource === 'api_key') ? (
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-native-api-key">{t('API Key')}</Label>
                    <Input
                      autoCapitalize="off"
                      autoComplete="off"
                      id="provider-native-api-key"
                      placeholder={apiKeyEnvName(providerConfigRow.providerType) || 'API_KEY'}
                      spellCheck={false}
                      type="password"
                      value={providerConfigDraft.apiKey}
                      onChange={(event) => {
                        setProviderConfigDraft((current) => ({
                          ...current,
                          apiKey: event.target.value,
                        }));
                      }}
                    />
                  </div>
                ) : null}
                <div className="provider-config-grid">
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-native-model">{t('Model')}</Label>
                    {activeProviderModelOptions.length > 0 ? (
                      <Select
                        value={providerConfigDraft.model.trim() || PROVIDER_DEFAULT_MODEL_VALUE}
                        onValueChange={(value) => {
                          setProviderConfigDraft((current) => {
                            const nextModel = value === PROVIDER_DEFAULT_MODEL_VALUE ? '' : value;
                            const reasoningOptions = reasoningEffortOptionsForModel(
                              activeProviderModels,
                              nextModel,
                              '',
                            );
                            return {
                              ...current,
                              model: nextModel,
                              modelReasoningEffort: nextModel ? highestReasoningEffort(reasoningOptions) : '',
                              modelServiceTier: '',
                            };
                          });
                        }}
                      >
                        <SelectTrigger id="provider-native-model">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={PROVIDER_DEFAULT_MODEL_VALUE}>{t('Provider default')}</SelectItem>
                            {activeProviderModelOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    ) : (
                      <Input
                        id="provider-native-model"
                        placeholder={activeProviderModelsLoading ? t('Loading models...') : providerConfigRow.defaultModel}
                        value={providerConfigDraft.model}
                        onChange={(event) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            model: event.target.value,
                          }));
                        }}
                      />
                    )}
                  </div>
                  <div className="commands-field">
                    <Label className="commands-field-label">{t('Reasoning')}</Label>
                    {activeReasoningOptions.length > 0 ? (
                      <Select
                        value={providerConfigDraft.modelReasoningEffort.trim() || PROVIDER_DEFAULT_REASONING_VALUE}
                        onValueChange={(value) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            modelReasoningEffort: value === PROVIDER_DEFAULT_REASONING_VALUE ? '' : value,
                          }));
                        }}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={PROVIDER_DEFAULT_REASONING_VALUE}>{t('Provider default')}</SelectItem>
                            {activeReasoningOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    ) : (
                      <Input
                        disabled
                        value=""
                        placeholder={activeProviderModelsLoading ? t('Loading models...') : t('Unavailable')}
                        readOnly
                      />
                    )}
                  </div>
                </div>
                {providerConfigRow.group === 'native' && providerConfigRow.providerType === 'gpt' ? (
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-native-service-tier">{t('Speed')}</Label>
                    {activeServiceTierOptions.length > 0 ? (
                      <Select
                        value={providerConfigDraft.modelServiceTier || '__standard__'}
                        onValueChange={(value) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            modelServiceTier: value === '__standard__' ? '' : value,
                          }));
                        }}
                      >
                        <SelectTrigger id="provider-native-service-tier">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value="__standard__">{t('Standard')}</SelectItem>
                            {activeServiceTierOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    ) : (
                      <Input
                        id="provider-native-service-tier"
                        placeholder={t('Standard')}
                        value={providerConfigDraft.modelServiceTier}
                        onChange={(event) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            modelServiceTier: event.target.value,
                          }));
                        }}
                      />
                    )}
                  </div>
                ) : null}
                {providerConfigRow.group === 'native' ? (
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-native-base-url">{t('Base URL')}</Label>
                    <Input
                      id="provider-native-base-url"
                      placeholder={t('Provider default')}
                      value={providerConfigDraft.baseUrl}
                      onChange={(event) => {
                        setProviderConfigDraft((current) => ({
                          ...current,
                          baseUrl: event.target.value,
                        }));
                      }}
                    />
                  </div>
                ) : null}
              </>
            ) : null}
          </div>

          <DialogFooter className="commands-dialog-footer">
            <div className="provider-config-footer-left">
              {providerConfigRow?.group === 'native' && providerConfigAgent ? (
                <Button
                  className="commands-dialog-button danger"
                  disabled={providerConfigSaving}
                  onClick={() => { void handleClearProviderConfig(); }}
                  type="button"
                  variant="outline"
                >
                  {t('Clear')}
                </Button>
              ) : null}
            </div>
            <div className="provider-config-footer-actions">
              <Button
                className="commands-dialog-button secondary"
                onClick={closeProviderConfigDialog}
                type="button"
                variant="outline"
              >
                {t('Cancel')}
              </Button>
              <Button
                className="commands-dialog-button primary"
                disabled={providerConfigSaving || savingLocalSettings}
                onClick={() => { void handleSaveProviderConfig(); }}
                type="button"
              >
                {providerConfigSaving || savingLocalSettings ? t('Saving…') : t('Save')}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );

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

  const channelsPanel = (
    <>
      <ChannelPluginCatalogPanel />
      <section className="bot-panel">
        <div className="bot-panel-toolbar">
          <div className="bot-panel-title-row">
            <h3 className="bot-panel-title">{t('Bots')}</h3>
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
            {t('Add bot')}
          </Button>
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
                : (accountAgentId || t('Default route'));
              const agentDisplayName = compactAgentTargetLabel(
                selectedTarget,
                accountAgentId || t('Default route'),
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
                      <AgentOptionAvatar
                        agentId={selectedTarget?.value || accountAgentId}
                        avatarDataUrl={selectedTarget?.avatarDataUrl}
                        className={selectedAgentMissing ? 'agent-option-avatar--missing' : undefined}
                        kind={selectedTarget?.kind || 'agent'}
                        label={agentDisplayName}
                        providerIcon={selectedTarget?.providerIcon}
                        providerType={selectedTarget?.providerType}
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
                      <DropdownMenuContent align="end" sideOffset={4}>
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
        onAddWorkspace={onAddWorkspace}
        onClose={() => {
          setIsAddingChannel(false);
        }}
        onCreateChannel={onAddChannelAccount}
        onPollWeixinAuth={onPollWeixinChannelAuth}
        onStartWeixinAuth={onStartWeixinChannelAuth}
        onPollFeishuAuth={onPollFeishuChannelAuth}
        onStartFeishuAuth={onStartFeishuChannelAuth}
        open={isAddingChannel}
        workspaces={workspaces}
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
    ? t('A command with this name already exists.')
    : normalizedCommandDraftName && !SLASH_COMMAND_NAME_PATTERN.test(normalizedCommandDraftName)
      ? t('Command names only support lowercase letters, numbers, and underscores, up to 32 characters.')
      : !commandDraftPrompt
        ? t('Enter command content.')
          : t('The command will be added to the list after saving.');
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
          <span className="codex-section-title">{t('Command List')}</span>
          <div className="codex-list-row-actions">
            <button
              className="codex-section-action"
              onClick={openCreateCommandDialog}
              type="button"
            >
              <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
                <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
              </svg>
              {t('Add Command')}
            </button>
          </div>
        </div>

        {commandsLoading ? (
          <div className="commands-empty-state">
            <strong>{t('Loading shortcuts...')}</strong>
            <span>{t('Global prompt shortcuts are loaded from the current Gateway config.')}</span>
          </div>
        ) : commands.length ? (
          <div className="commands-table">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="commands-table-col-command">{t('Command')}</TableHead>
                  <TableHead className="commands-table-col-description">{t('Description')}</TableHead>
                  <TableHead className="commands-table-col-prompt">{t('Prompt')}</TableHead>
                  <TableHead className="commands-table-col-actions">{t('Actions')}</TableHead>
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
                      title={command.description || t('Prompt shortcut')}
                    >
                      {command.description || t('Prompt shortcut')}
                    </TableCell>
                    <TableCell
                      className="commands-table-col-prompt"
                      title={commandPromptPreview(command) || t('No prompt configured.')}
                    >
                      {commandPromptPreview(command) || t('No prompt configured.')}
                    </TableCell>
                    <TableCell className="commands-table-col-actions">
                      <div className="command-list-actions">
                        <button
                          className="command-row-action"
                          onClick={() => { openEditCommandDialog(command); }}
                          type="button"
                        >
                          {t('Edit')}
                        </button>
                        <button
                          className="command-row-action danger"
                          disabled={commandsSaving}
                          onClick={() => { void handleDeleteCommandClick(command.name); }}
                          type="button"
                        >
                          {t('Delete')}
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
            <strong>{t('No shortcuts yet')}</strong>
            <span>{t('Click Add Command above to create a prompt shortcut like /summary.')}</span>
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
          size="form"
        >
          <DialogHeader className="commands-dialog-header">
            <Badge
              variant="outline"
              className="commands-dialog-badge"
            >
              {editingCommandName ? t('Edit Command') : t('Add Command')}
            </Badge>
            <div className="commands-dialog-title-group">
              <DialogTitle className="commands-dialog-title">
                {editingCommandName ? t('Edit /{name}', { name: editingCommandName }) : t('Add Command')}
              </DialogTitle>
              <DialogDescription className="commands-dialog-description">
                {t('Only the command name and content are needed. Telegram descriptions are generated on save.')}
              </DialogDescription>
            </div>
          </DialogHeader>

          <div className="commands-dialog-body">
            <div className="commands-field">
              <div className="commands-field-header">
                <Label className="commands-field-label" htmlFor="slash-command-name">{t('Command name')}</Label>
                <span className="commands-field-hint">{t('Only a-z, 0-9, and _')}</span>
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
                <Label className="commands-field-label" htmlFor="slash-command-prompt">{t('Content')}</Label>
                <span className="commands-field-hint">{t('This prompt runs when /command is invoked.')}</span>
              </div>
              <Textarea
                className="commands-prompt-control"
                id="slash-command-prompt"
                placeholder={t('Summarize the key points of our conversation.')}
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
              {t('Cancel')}
            </Button>
            <Button
              className="commands-dialog-button primary"
              disabled={!commandDraftReady || commandNameTaken || commandsSaving}
              onClick={() => {
                void handleSaveCommandDraft();
              }}
              type="button"
            >
              {commandsSaving ? t('Saving…') : t('Save Command')}
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
    ? t('An MCP server with this name already exists.')
    : !normalizedMcpServerName
      ? t('Enter a server name.')
      : mcpServerDraft.transport === 'stdio' && !normalizedMcpServerCommand
        ? t('Enter a start command.')
        : mcpServerDraft.transport === 'streamable_http' && !normalizedMcpUrl
          ? t('Enter a URL.')
          : t('Saving updates garyx.json on the gateway and syncs local Claude / Codex MCP config.');

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
    if (!window.confirm(t('Delete MCP server "{name}"?', { name }))) return;
    await onDeleteMcpServer(name);
    if (editingMcpServerName === name) {
      closeMcpDialog();
    }
  }

  const mcpPanel = (
    <>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">{t('Custom Servers')}</span>
          <button
            className="codex-section-action"
            disabled={mcpServersSaving}
            onClick={openCreateMcpDialog}
            type="button"
          >
            <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
              <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
            </svg>
            {t('Add Server')}
          </button>
        </div>
        {mcpServersLoading ? (
          <div className="codex-empty-state">{t('Loading current config...')}</div>
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
                    title={t('Configure')}
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
                    aria-label={t('Delete {name}', { name: server.name })}
                    className="codex-icon-button codex-icon-button-danger"
                    disabled={mcpServersSaving}
                    onClick={() => { void handleDeleteMcpServer(server.name); }}
                    title={t('Delete')}
                    type="button"
                  >
                    <Trash aria-hidden />
                  </button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="codex-empty-state">{t('No MCP servers yet. Click Add Server above to create one.')}</div>
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
          size="narrow"
        >
          <DialogHeader className="border-b border-[#efefec] px-4 py-3">
            <DialogTitle className="text-[14px] font-semibold tracking-[-0.01em] text-[#111111]">
              {editingMcpServerName ? t('Edit {name}', { name: editingMcpServerName }) : t('Add Server')}
            </DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-4 py-4">
            <div className="grid gap-3 md:grid-cols-[1fr_auto]">
              <div className="space-y-1.5">
                <Label className="text-[11px] font-medium text-[#666663]">{t('Name')}</Label>
                <Input
                  className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                  placeholder={t('MCP server name')}
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
                <Label className="text-[11px] font-medium text-[#666663]">{t('Transport')}</Label>
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
                  <Label className="text-[11px] font-medium text-[#666663]">{t('Start command')}</Label>
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
                    <Label className="text-[11px] font-medium text-[#666663]">{t('Arguments')}</Label>
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
                      + {t('Add')}
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
                          {t('Delete')}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">{t('Environment variables')}</Label>
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
                      + {t('Add')}
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.envEntries.map((entry, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[0.9fr_1.1fr_auto]" key={`env-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder={t('Key')}
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
                          placeholder={t('Value')}
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
                          {t('Delete')}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">{t('Working directory')}</Label>
                  <WorkspacePathPicker
                    allowEmpty
                    onAddWorkspace={onAddWorkspace}
                    onChange={(value) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        workingDir: value,
                      }));
                    }}
                    placeholder={t('Choose workspace')}
                    triggerClassName="min-h-8 rounded-[8px] border-[#e7e7e5] bg-white px-3 py-1.5 text-[13px] shadow-none"
                    value={mcpServerDraft.workingDir}
                    workspaces={workspaces}
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
                    <Label className="text-[11px] font-medium text-[#666663]">{t('Headers')}</Label>
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
                      + {t('Add')}
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.headerEntries.map((entry, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[0.9fr_1.1fr_auto]" key={`header-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder={t('Key')}
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
                          placeholder={t('Value')}
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
                          {t('Delete')}
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
                  {t('Delete')}
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
                {t('Cancel')}
              </Button>
              <Button
                className="h-8 rounded-[8px] bg-[#111111] px-3 text-[12px] text-white shadow-none hover:bg-[#1c1c1c]"
                disabled={!mcpServerDraftReady || mcpServersSaving}
                onClick={() => {
                  void handleSaveMcpServerDraft();
                }}
                type="button"
              >
                {mcpServersSaving ? t('Saving…') : t('Save')}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );

  const labsPanel = (
    <>
      {desktopSettingsSection}
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">{t('Updates')}</span>
        </div>
        <div className="codex-list-card">
          <SettingsControlRow
            className="settings-update-row"
            control={
              <div className="settings-update-control">
                <div className="settings-update-summary">
                  <span className="settings-update-version">
                    {t('Current version {version}', { version: `v${desktopAppVersion}` })}
                  </span>
                  <span className={`settings-update-status tone-${updateDisplay.tone}`}>
                    {updateDisplay.message}
                  </span>
                </div>
                <div className="settings-update-actions">
                  {updateStatus.phase === 'downloaded' ? (
                    <Button
                      className="rounded-xl bg-[#111111] text-white shadow-none hover:bg-[#222222]"
                      disabled={installingUpdate}
                      onClick={() => { void handleInstallUpdate(); }}
                      size="sm"
                      type="button"
                    >
                      {installingUpdate ? t('Restarting...') : t('Restart to Update')}
                    </Button>
                  ) : null}
                  {updateStatus.phase !== 'downloaded' ? (
                    <Button
                      className="rounded-xl border-[#e7e7e5] bg-white text-[#111111] shadow-none hover:bg-[#f6f6f5]"
                      disabled={updateCheckDisabled}
                      onClick={() => { void handleCheckForUpdatesNow(); }}
                      size="sm"
                      title={t('Check for updates')}
                      type="button"
                      variant="outline"
                    >
                      <RefreshCw
                        aria-hidden
                        className={updateCheckBusy ? 'settings-update-spin' : undefined}
                        size={13}
                        strokeWidth={2}
                      />
                      {updateCheckBusy ? t('Checking...') : t('Check Now')}
                    </Button>
                  ) : null}
                </div>
              </div>
            }
            description={t('Packaged builds check for updates automatically. Use this to refresh the update state immediately.')}
            label={t('Garyx updates')}
            stacked
          />
        </div>
      </div>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">{t('Labs')}</span>
          {renderGatewaySaveAction()}
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
            description={t('Show or hide the Auto Research entry in the desktop client. Disabling it only hides the desktop surface.')}
            label={t('Auto Research')}
          />
          <SettingsControlRow
            control={
              <SettingsSwitch
                checked={Boolean(gatewayDraft?.dreams?.enabled)}
                disabled={gatewaySaving}
                label="dreams.enabled"
                onChange={(nextValue) => {
                  void onSaveGatewaySettingsPatch(
                    { dreams: { enabled: nextValue } },
                    { refreshDesktopState: 'background' },
                  );
                }}
              />
            }
            description={t('Show Dreams in the apps and run automatic scans on the configured interval when recent user messages exist.')}
            label={t('Dreams')}
          />
        </div>
      </div>
    </>
  );

  const performancePanel = (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">{t('Renderer Diagnostics')}</span>
      </div>
      <SettingsSurface className="settings-performance-surface">
        <RendererPerformancePanel snapshot={performanceSnapshot} />
      </SettingsSurface>
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
    case 'performance':
      tabContent = performancePanel;
      break;
    case 'commands':
      tabContent = commandsPanel;
      break;
    case 'mcp':
      tabContent = mcpPanel;
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
            <span className="eyebrow">{t(activeTabMeta.eyebrow)}</span>
            <h3 className="settings-tab-title">{t(activeTabMeta.label)}</h3>
            <p className="small-note">{t(activeTabMeta.description)}</p>
            {showGatewayHeaderStatus ? (
              <p
                className={`small-note settings-tab-hint ${
                  statusClass === 'error' ? 'error' : ''
                }`}
              >
                {gatewayStatusMessage ? t(gatewayStatusMessage) : remoteSyncLabel}
              </p>
            ) : null}
          </div>
          {showGatewayHeaderStatus ? (
            <div className="settings-page-header-aside">
              <span className={`status-pill ${connection?.ok ? '' : 'offline'}`}>
                {connection?.ok ? t('online') : t('offline')}
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
