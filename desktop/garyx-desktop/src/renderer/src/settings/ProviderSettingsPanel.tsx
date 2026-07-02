import { useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';

import type {
  DesktopApiProviderType,
  DesktopCodingUsage,
  DesktopCustomAgent,
  DesktopProviderModelOption,
  DesktopProviderModels,
  DesktopProviderUsage,
  DesktopSettings,
} from '@shared/contracts';
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
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Textarea } from '@/components/ui/textarea';
import { useI18n, type Translate } from '../i18n';
import { usageProviderIdForModelProviderKey } from '../provider-usage';
import { classNames, noopAsync } from './shared';
import { SettingsControlRow } from './shared-components';

function countNonEmptyLines(value: string): number {
  return value
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith('#')).length;
}

type DraftMutator = (mutator: (nextConfig: any) => void) => void;
type GatewaySettingsSaveOptions = {
  silent?: boolean;
  refreshDesktopState?: 'await' | 'background' | 'skip';
};

type FixedModelProviderKey =
  | 'claude_code'
  | 'codex_app_server'
  | 'antigravity'
  | 'traex'
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
  usageProviderId?: string;
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

const MODEL_PROVIDER_ROWS: FixedModelProviderRow[] = [
  {
    key: 'claude_code',
    agentId: 'claude',
    label: 'Claude Code',
    providerType: 'claude_code',
    group: 'default',
    defaultModel: '(provider default)',
    usageProviderId: usageProviderIdForModelProviderKey('claude_code'),
  },
  {
    key: 'codex_app_server',
    agentId: 'codex',
    label: 'Codex',
    providerType: 'codex_app_server',
    group: 'default',
    defaultModel: '(provider default)',
    usageProviderId: usageProviderIdForModelProviderKey('codex_app_server'),
  },
  {
    key: 'antigravity',
    agentId: 'antigravity',
    label: 'Antigravity',
    providerType: 'antigravity',
    group: 'default',
    defaultModel: 'Claude Opus 4.6 (Thinking)',
    usageProviderId: usageProviderIdForModelProviderKey('antigravity'),
  },
  {
    key: 'traex',
    agentId: 'traex',
    label: 'Traex',
    providerType: 'traex',
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

const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default_model__';

const PROVIDER_DEFAULT_REASONING_VALUE = '__provider_default_reasoning__';

const REASONING_EFFORT_RANK: Record<string, number> = {
  off: 0,
  minimal: 1,
  low: 2,
  medium: 3,
  high: 4,
  xhigh: 5,
  max: 6,
};

function providerTypeValue(provider: any): string {
  return String(provider?.provider_type || 'claude_code');
}

function fixedModelProviderRow(key: FixedModelProviderKey): FixedModelProviderRow {
  return MODEL_PROVIDER_ROWS.find((row) => row.key === key) || MODEL_PROVIDER_ROWS[0];
}

function usageLevelClass(remainingPercent: number): string {
  if (remainingPercent >= 50) {
    return 'healthy';
  }
  if (remainingPercent >= 20) {
    return 'warning';
  }
  return 'critical';
}

function formatUsagePercent(value: number): string {
  const safe = Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0;
  return `${Math.round(safe)}%`;
}

function formatUsageDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const days = Math.floor(total / 86_400);
  if (days >= 1) {
    return `${days}d`;
  }
  const hours = Math.floor(total / 3_600);
  if (hours >= 1) {
    return `${hours}h`;
  }
  const minutes = Math.floor(total / 60);
  if (minutes >= 1) {
    return `${minutes}m`;
  }
  return '<1m';
}

function resetSecondsFromIso(value?: string | null): number | null {
  if (!value) {
    return null;
  }
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return null;
  }
  return Math.max(0, Math.floor((timestamp - Date.now()) / 1000));
}

function usageResetText(
  resetsAt?: string | null,
  resetAfterSeconds?: number | null,
  fallback = 'weekly left',
): string {
  const seconds = Number.isFinite(resetAfterSeconds || NaN)
    ? resetAfterSeconds || 0
    : resetSecondsFromIso(resetsAt);
  if (seconds && seconds > 0) {
    return `resets in ${formatUsageDuration(seconds)}`;
  }
  return fallback;
}

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
  _row: FixedModelProviderRow,
  providerModels: DesktopProviderModels | null | undefined,
): ModelProviderConfigDraft {
  if (!providerModels) {
    return draft;
  }
  const model = draft.model.trim();
  if (!model) {
    return {
      ...draft,
      model: '',
      modelReasoningEffort: '',
      modelServiceTier: '',
    };
  }
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
  const candidates = Array.from(new Set([row.agentId, ...(row.legacyAgentIds || []), row.key]));
  for (const candidate of candidates) {
    const value = agentsConfig[candidate];
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      return value;
    }
  }
  return {};
}

function providerConfigEnv(providerConfig: Record<string, any>): Record<string, string> {
  const env = providerConfig.env;
  if (!env || typeof env !== 'object' || Array.isArray(env)) {
    return {};
  }
  return Object.fromEntries(
    Object.entries(env)
      .filter(([, value]) => typeof value === 'string')
      .map(([key, value]) => [key, value as string]),
  );
}

function apiKeyFromProviderConfig(
  providerConfig: Record<string, any>,
  providerType: DesktopApiProviderType,
): string {
  const envName = apiKeyEnvName(providerType);
  return envName ? providerConfigEnv(providerConfig)[envName] || '' : '';
}

function providerConfigHasNativeSettings(providerConfig: Record<string, any>): boolean {
  const keys = [
    'provider_type',
    'default_model',
    'model_reasoning_effort',
    'model_service_tier',
    'auth_source',
    'base_url',
  ];
  return keys.some((key) => String(providerConfig[key] || '').trim().length > 0)
    || Object.keys(providerConfigEnv(providerConfig)).length > 0;
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
  const configModel = String(providerConfig.default_model || '');
  const configReasoning = String(providerConfig.model_reasoning_effort || '');
  const configServiceTier = String(providerConfig.model_service_tier || '');
  return {
    key,
    claudeCliMode: normalizeClaudeCliMode(providerConfig.claude_cli_mode),
    claudeCliPath: String(providerConfig.claude_cli_path || ''),
    claudeEnv: localSettings.providerClaudeEnv,
    codexAuthMode: localSettings.providerCodexAuthMode,
    codexApiKey: localSettings.providerCodexApiKey,
    geminiEnv: localSettings.providerGeminiEnv,
    model: row.group === 'native'
      ? configModel || agent?.model || ''
      : configModel,
    modelReasoningEffort: row.group === 'native'
      ? configReasoning || agent?.modelReasoningEffort || ''
      : configReasoning,
    modelServiceTier: row.group === 'native'
      ? configServiceTier || agent?.modelServiceTier || ''
      : configServiceTier,
    authSource: String(providerConfig.auth_source || '').trim()
      || agent?.authSource
      || defaultNativeAuthSource(row.providerType),
    apiKey: apiKeyFromProviderConfig(providerConfig, row.providerType)
      || apiKeyFromProviderAgent(agent),
    baseUrl: String(providerConfig.base_url || '') || agent?.baseUrl || '',
  };
}

type AgentProviderFieldsProps = {
  provider: any;
  onMutate: (mutator: (provider: any) => void) => void;
};

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
                  <SelectItem value="traex">traex</SelectItem>
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

type ProviderSettingsPanelProps = {
  agents?: DesktopCustomAgent[];
  localSettings: DesktopSettings;
  gatewayDraft?: any;
  onMutateGatewayDraft?: DraftMutator;
  onSaveGatewaySettings?: (options?: GatewaySettingsSaveOptions) => Promise<boolean>;
  onSaveLocalSettingsDraft?: (
    nextSettings: DesktopSettings,
    options?: {
      requireGatewayConnection?: boolean;
      reloadGatewaySettings?: boolean;
    },
  ) => Promise<boolean>;
  onRefreshAgentTargets?: () => Promise<void>;
};

export function ProviderSettingsPanel({
  agents = [],
  localSettings,
  gatewayDraft,
  onMutateGatewayDraft = () => {},
  onSaveGatewaySettings = async () => true,
  onSaveLocalSettingsDraft = async () => true,
  onRefreshAgentTargets = noopAsync,
}: ProviderSettingsPanelProps) {
  const { t } = useI18n();
  const claudeEnvLineCount = countNonEmptyLines(localSettings.providerClaudeEnv);
  const geminiEnvLineCount = countNonEmptyLines(localSettings.providerGeminiEnv);
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
  const [codingUsage, setCodingUsage] = useState<DesktopCodingUsage | null>(null);
  const [codingUsageLoading, setCodingUsageLoading] = useState(false);
  const [codingUsageError, setCodingUsageError] = useState<string | null>(null);
  const providerConfigRow = providerConfigKey ? fixedModelProviderRow(providerConfigKey) : null;
  const providerConfigAgent = providerConfigKey
    ? configuredProviderAgent(agents, providerConfigKey)
    : null;
  const providerConfigRuntime = providerConfigKey
    ? providerAgentConfig(gatewayDraft, providerConfigKey)
    : {};
  const providerConfigHasSettings = providerConfigRow?.group === 'native'
    && (providerConfigHasNativeSettings(providerConfigRuntime) || Boolean(providerConfigAgent));
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
  const codingUsageByProviderId = useMemo(() => {
    const map: Record<string, DesktopProviderUsage> = {};
    for (const provider of codingUsage?.providers || []) {
      map[provider.id] = provider;
    }
    return map;
  }, [codingUsage]);
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
    let cancelled = false;
    setCodingUsageLoading(true);
    setCodingUsageError(null);
    void window.garyxDesktop.getCodingUsage().then((usage) => {
      if (cancelled) return;
      setCodingUsage(usage);
    }).catch((error) => {
      if (cancelled) return;
      setCodingUsageError(error instanceof Error ? error.message : t('Failed to load usage.'));
    }).finally(() => {
      if (cancelled) return;
      setCodingUsageLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [t]);

  useEffect(() => {
    if (!providerConfigRow || !activeProviderModels) {
      return;
    }
    setProviderConfigDraft((current) => {
      if (current.key !== providerConfigRow.key) {
        return current;
      }
      return applyProviderCatalogDefaults(current, providerConfigRow, activeProviderModels);
    });
  }, [providerConfigRow?.key, providerConfigRow?.group, activeProviderModels]);
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
    if (row.key === 'traex') {
      // TRAE CLI authenticates via `traex login`; no desktop-managed auth.
      return {
        status: t('Default'),
        auth: t('CLI'),
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
    const authSource = String(runtimeConfig.auth_source || '').trim()
      || agent?.authSource
      || defaultNativeAuthSource(row.providerType);
    const configuredApiKey = apiKeyFromProviderConfig(runtimeConfig, row.providerType)
      || apiKeyFromProviderAgent(agent);
    return {
      status: providerConfigHasNativeSettings(runtimeConfig) || agent ? t('Configured') : t('Not configured'),
      auth: row.providerType === 'gpt' && authSource === 'codex'
        ? t('GPT token')
        : configuredApiKey
          ? t('API Key')
          : t('Env / API key'),
      model: configuredDefaultModel || agent?.model || row.defaultModel,
    };
  }

  function renderProviderUsageCell(row: FixedModelProviderRow): ReactNode {
    if (!row.usageProviderId) {
      return <span className="provider-usage-muted">{t('Not tracked')}</span>;
    }
    const usage = codingUsageByProviderId[row.usageProviderId];
    if (!usage) {
      if (codingUsageLoading) {
        return <span className="provider-usage-muted">{t('Loading')}</span>;
      }
      return (
        <span className="provider-usage-muted" title={codingUsageError || undefined}>
          {codingUsageError ? t('Unavailable') : t('No data')}
        </span>
      );
    }
    if (!usage.available) {
      return (
        <span className="provider-usage-muted" title={usage.error || undefined}>
          {t('Unavailable')}
        </span>
      );
    }
    if (usage.models.length > 0) {
      // Keep the row height fixed: surface the tightest model quota and fold
      // the full per-model breakdown into the hover title.
      const tightest = usage.models.reduce((worst, model) => {
        if (typeof model.remainingPercent !== 'number') return worst;
        if (typeof worst.remainingPercent !== 'number') return model;
        return model.remainingPercent < worst.remainingPercent ? model : worst;
      }, usage.models[0]);
      const breakdown = usage.models
        .map((model) => `${model.name}: ${formatUsagePercent(model.remainingPercent)}`)
        .join('\n');
      return (
        <div className="provider-usage-summary" title={breakdown}>
          <span className={`provider-usage-value ${usageLevelClass(tightest.remainingPercent)}`}>
            {formatUsagePercent(tightest.remainingPercent)}
          </span>
          <span className="provider-usage-detail">
            {usage.models.length > 1
              ? t('{name} · {count} models', { name: tightest.name, count: usage.models.length })
              : tightest.name}
          </span>
        </div>
      );
    }
    if (usage.weekly) {
      const level = usageLevelClass(usage.weekly.remainingPercent);
      return (
        <div className="provider-usage-summary">
          <span className={`provider-usage-value ${level}`}>
            {formatUsagePercent(usage.weekly.remainingPercent)}
          </span>
          <span className="provider-usage-detail">
            {usage.stale
              ? t('stale data')
              : usageResetText(usage.weekly.resetsAt, usage.weekly.resetAfterSeconds)}
          </span>
        </div>
      );
    }
    return <span className="provider-usage-muted">{t('No data')}</span>;
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
    // Optimistically updates the settings draft; handleSaveProviderConfig keeps
    // the dialog open if the subsequent gateway save fails so the user can retry.
    onMutateGatewayDraft((next) => {
      next.agents = next.agents || {};
      const current = next.agents[row.agentId] && typeof next.agents[row.agentId] === 'object'
        ? next.agents[row.agentId]
        : {};
      const nextConfig: Record<string, any> = {
        ...current,
        provider_type: row.providerType,
        default_model: draft.model.trim(),
        model_reasoning_effort: draft.modelReasoningEffort.trim(),
      };
      delete nextConfig.model;
      if (row.providerType === 'gpt') {
        nextConfig.model_service_tier = draft.modelServiceTier.trim();
      } else {
        delete nextConfig.model_service_tier;
      }
      if (row.group === 'native') {
        const envName = apiKeyEnvName(row.providerType);
        const env = providerConfigEnv(current);
        if (envName) {
          const apiKey = draft.apiKey.trim();
          if (apiKey) {
            env[envName] = apiKey;
          } else {
            delete env[envName];
          }
        }
        if (Object.keys(env).length > 0) {
          nextConfig.env = env;
        } else {
          delete nextConfig.env;
        }
        nextConfig.auth_source = draft.authSource.trim()
          || defaultNativeAuthSource(row.providerType);
        nextConfig.base_url = draft.baseUrl.trim();
      }
      next.agents[row.agentId] = nextConfig;
    });
  }

  function mutateClearNativeProviderConfig(row: FixedModelProviderRow) {
    onMutateGatewayDraft((next) => {
      if (!next.agents || typeof next.agents !== 'object') {
        return;
      }
      delete next.agents[row.agentId];
      for (const legacyAgentId of row.legacyAgentIds || []) {
        delete next.agents[legacyAgentId];
      }
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
      if (providerConfigRow.key === 'traex') {
        // TRAE CLI has no desktop-managed auth/env; persist model defaults only.
        mutateGatewayProviderModelDefaults(providerConfigRow, providerConfigDraft);
        if (await onSaveGatewaySettings({ refreshDesktopState: 'background' })) {
          closeProviderConfigDialog();
        }
        return;
      }
      if (providerConfigRow.group === 'native') {
        mutateGatewayProviderModelDefaults(providerConfigRow, providerConfigDraft);
        if (!(await onSaveGatewaySettings({ refreshDesktopState: 'background' }))) {
          return;
        }
        if (providerConfigAgent) {
          await window.garyxDesktop.deleteCustomAgent({ agentId: providerConfigAgent.agentId });
          await onRefreshAgentTargets();
        }
        closeProviderConfigDialog();
        return;
      }

    } finally {
      setProviderConfigSaving(false);
    }
  }

  async function handleClearProviderConfig() {
    if (!providerConfigRow || providerConfigRow.group !== 'native' || !providerConfigHasSettings) {
      return;
    }
    if (!window.confirm(t('Clear configuration for {name}?', { name: providerConfigRow.label }))) {
      return;
    }
    setProviderConfigSaving(true);
    try {
      mutateClearNativeProviderConfig(providerConfigRow);
      if (!(await onSaveGatewaySettings({ refreshDesktopState: 'background' }))) {
        return;
      }
      if (providerConfigAgent) {
        await window.garyxDesktop.deleteCustomAgent({ agentId: providerConfigAgent.agentId });
        await onRefreshAgentTargets();
      }
      closeProviderConfigDialog();
    } finally {
      setProviderConfigSaving(false);
    }
  }
  const providerConfigTablePanel = (
    <section className="codex-section">
      <div className="codex-section-header">
        <span className="codex-section-title">{t('Configured Providers')}</span>
      </div>
      <div className="provider-config-table">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="provider-config-col-provider">{t('Provider')}</TableHead>
              <TableHead className="provider-config-col-kind">{t('Type')}</TableHead>
              <TableHead className="provider-config-col-auth">{t('Auth')}</TableHead>
              <TableHead className="provider-config-col-model">{t('Model')}</TableHead>
              <TableHead className="provider-config-col-usage">{t('Usage')}</TableHead>
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
                  <TableCell className="provider-config-col-usage">
                    {renderProviderUsageCell(row)}
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

  return (
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
              {providerConfigRow?.group === 'native' && providerConfigHasSettings ? (
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
                disabled={providerConfigSaving}
                onClick={() => { void handleSaveProviderConfig(); }}
                type="button"
              >
                {providerConfigSaving ? t('Saving…') : t('Save')}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
