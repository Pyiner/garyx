import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Pencil, Plus, RefreshCw, Server, Trash } from 'lucide-react';

import {
  DEFAULT_DESKTOP_SETTINGS,
  type ChannelPluginCatalogEntry,
  type ConnectionStatus,
  type DesktopApiProviderType,
  type DesktopCodingUsage,
  type DesktopCustomAgent,
  type DesktopFollowUpBehavior,
  type DesktopProviderModelOption,
  type DesktopProviderModels,
  type DesktopProviderUsage,
  type DesktopWorkspace,
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
import { AddBotDialog } from './app-shell/components/AddBotDialog';
import { AgentOptionAvatar } from './app-shell/components/AgentOptionAvatar';
import { GatewayHeadersEditor } from './GatewayHeadersEditor';
import { WorkspacePathPicker } from './components/WorkspacePathPicker';
import { MoreDotsIcon } from './app-shell/icons';
import { ChannelPluginCatalogPanel } from './channel-plugins/ChannelPluginCatalogPanel';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { EditBotDialog, type EditBotDialogContext, type EditBotPatch } from './app-shell/components/EditBotDialog';
import { languagePreferenceLabel, type Translate, useI18n } from './i18n';
import { usageProviderIdForModelProviderKey } from './provider-usage';
import { SETTINGS_TABS, type SettingsTabId } from './settings-tabs';
import { CommandsSettingsPanel } from './settings/CommandsSettingsPanel';
import { McpSettingsPanel } from './settings/McpSettingsPanel';
import { ProviderSettingsPanel } from './settings/ProviderSettingsPanel';
import { ChannelsSettingsPanel } from './settings/ChannelsSettingsPanel';

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
  savingLocalSettings?: boolean;
  agents?: DesktopCustomAgent[];
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
  onSaveGatewaySettings?: (options?: GatewaySettingsSaveOptions) => Promise<boolean>;
  onSaveGatewaySettingsPatch?: (
    patch: GatewayConfigDocument,
    options?: GatewaySettingsSaveOptions,
  ) => Promise<boolean>;
  onAddGatewayProfile?: (input: {
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
    gatewayHeaders?: string;
  }) => Promise<void>;
  onUpdateGatewayProfile?: (input: {
    profileId: string;
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
    gatewayHeaders?: string;
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

function providerTypeValue(provider: any): string {
  return String(provider?.provider_type || 'claude_code');
}

function providerTypeLabel(provider: any): string {
  const value = providerTypeValue(provider);
  if (value === 'codex_app_server') {
    return 'codex';
  }
  if (value === 'antigravity') {
    return 'antigravity';
  }
  if (value === 'traex') {
    return 'traex';
  }
  return 'claude';
}

const noop = () => {};
const noopAsync = async () => {};
const noopAsyncBoolean = async () => false;
const IDLE_UPDATE_STATUS: DesktopUpdateStatus = { phase: 'idle' };
const FOLLOW_UP_BEHAVIOR_TOGGLE_ITEM_CLASS =
  'relative h-8 !rounded-[12px] border-0 px-3 text-[12px] text-[#666663] data-[state=on]:z-10 data-[state=on]:bg-white data-[state=on]:text-[#111111] data-[state=on]:shadow-sm';
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
    gatewayHeaders?: string;
  }) => Promise<void>;
}) {
  const { t } = useI18n();
  const [label, setLabel] = useState('');
  const [gatewayUrl, setGatewayUrl] = useState('');
  const [gatewayAuthToken, setGatewayAuthToken] = useState('');
  const [gatewayHeaders, setGatewayHeaders] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setLabel(profile?.label ?? '');
      setGatewayUrl(profile?.gatewayUrl ?? '');
      setGatewayAuthToken(profile?.gatewayAuthToken ?? '');
      setGatewayHeaders(profile?.gatewayHeaders ?? '');
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
    setGatewayHeaders('');
  }

  async function handleSave() {
    if (!canSave || saving) {
      return;
    }
    setSaving(true);
    try {
      await onSubmit({ label, gatewayUrl, gatewayAuthToken, gatewayHeaders });
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
          <div className="gateway-setup-field">
            <GatewayHeadersEditor
              value={gatewayHeaders}
              onChange={setGatewayHeaders}
            />
          </div>
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
  savingLocalSettings = false,
  agents = [],
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
  const [gatewayDialogOpen, setGatewayDialogOpen] = useState(false);
  const [gatewayDialogProfile, setGatewayDialogProfile] = useState<DesktopGatewayProfile | null>(null);
  // Schema-driven catalog: icon + display_name + runtime state per
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


  const currentGatewayKey = localSettings.gatewayUrl.trim().toLowerCase();
  // Saved order is kept as-is; the active gateway is marked, not moved.
  const savedGatewayProfiles = useMemo(() => {
    return gatewayProfiles.filter((profile) => profile.gatewayUrl.trim().length > 0);
  }, [gatewayProfiles]);

  // The settings tab manages the saved gateway list only; switching the
  // active gateway lives in the sidebar identity bar.
  const connectionPanel = (
    <div className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">{t('Saved Gateways')}</span>
        <button
          className="codex-section-action"
          onClick={() => {
            setGatewayDialogProfile(null);
            setGatewayDialogOpen(true);
          }}
          type="button"
        >
          <Plus aria-hidden size={14} />
          {t('Add Gateway')}
        </button>
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
                  {countNonEmptyLines(profile.gatewayHeaders) > 0 ? (
                    <span className="gateway-profile-row-url">
                      {t('{count} custom headers', {
                        count: countNonEmptyLines(profile.gatewayHeaders),
                      })}
                    </span>
                  ) : null}
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
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button
                        aria-label={t('More actions for {name}', { name: profile.label })}
                        className="bot-table-action-button"
                        type="button"
                      >
                        <MoreDotsIcon size={14} />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onSelect={() => {
                          if (
                            window.confirm(
                              t('Remove {label} from saved gateways?', { label: profile.label }),
                            )
                          ) {
                            void onDeleteGatewayProfile(profile.id);
                          }
                        }}
                        variant="destructive"
                      >
                        <Trash aria-hidden />
                        {t('Remove')}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
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
    </>
  );

  let tabContent: ReactNode;
  switch (normalizedActiveTab) {
    case 'gateway':
      tabContent = gatewayPanel;
      break;
    case 'provider':
      tabContent = (
        <ProviderSettingsPanel
          gatewayDraft={gatewayDraft}
          onMutateGatewayDraft={onMutateGatewayDraft}
          onSaveGatewaySettings={onSaveGatewaySettings}
        />
      );
      break;
    case 'channels':
      tabContent = (
        <ChannelsSettingsPanel
          agents={agents}
          workspaces={workspaces}
          gatewayDraft={gatewayDraft}
          gatewaySaving={gatewaySaving}
          gatewaySettingsSource={gatewaySettingsSource}
          onAddWorkspace={onAddWorkspace}
          onMutateGatewayDraft={onMutateGatewayDraft}
          onSaveGatewaySettings={onSaveGatewaySettings}
          onRefreshAgentTargets={onRefreshAgentTargets}
          onAddChannelAccount={onAddChannelAccount}
          onStartWeixinChannelAuth={onStartWeixinChannelAuth}
          onPollWeixinChannelAuth={onPollWeixinChannelAuth}
          onStartFeishuChannelAuth={onStartFeishuChannelAuth}
          onPollFeishuChannelAuth={onPollFeishuChannelAuth}
        />
      );
      break;
    case 'labs':
      tabContent = labsPanel;
      break;
    case 'commands':
      tabContent = (
        <CommandsSettingsPanel
          commands={commands}
          commandsLoading={commandsLoading}
          commandsSaving={commandsSaving}
          onCreateSlashCommand={onCreateSlashCommand}
          onUpdateSlashCommand={onUpdateSlashCommand}
          onDeleteSlashCommand={onDeleteSlashCommand}
        />
      );
      break;
    case 'mcp':
      tabContent = (
        <McpSettingsPanel
          mcpServers={mcpServers}
          mcpServersLoading={mcpServersLoading}
          mcpServersSaving={mcpServersSaving}
          workspaces={workspaces}
          onAddWorkspace={onAddWorkspace}
          onCreateMcpServer={onCreateMcpServer}
          onToggleMcpServer={onToggleMcpServer}
          onUpdateMcpServer={onUpdateMcpServer}
          onDeleteMcpServer={onDeleteMcpServer}
        />
      );
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
