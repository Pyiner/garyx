import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';

import type {
  DesktopApiProviderType,
  DesktopCodingUsage,
  DesktopProviderModelOption,
  DesktopProviderModels,
  DesktopProviderUsage,
  DesktopModelUsage,
  DesktopUsageWindow,
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
import { ProviderAgentIcon } from '../app-shell/components/ProviderAgentIcon';
import { useI18n, type Translate } from '../i18n';
import { shouldRequestProviderModelCatalog } from '../provider-model-catalog';
import {
  clampUsagePercent,
  formatUsageDuration,
  formatUsagePercent,
  usageLevelForRemainingPercent,
  usageProviderIdForModelProviderKey,
  usageResetText,
} from '../provider-usage';
import { classNames } from './shared';

type DraftMutator = (mutator: (nextConfig: any) => void) => void;
type GatewaySettingsSaveOptions = {
  silent?: boolean;
  refreshDesktopState?: 'await' | 'background' | 'skip';
};

type FixedModelProviderKey =
  | 'claude_code'
  | 'codex_app_server'
  | 'antigravity'
  | 'traex';

type FixedModelProviderRow = {
  key: FixedModelProviderKey;
  agentId: string;
  label: string;
  providerType: DesktopApiProviderType;
  defaultModel: string;
  usageProviderId?: string;
};

type ModelProviderConfigDraft = {
  key: FixedModelProviderKey;
  claudeCliMode: 'cctty' | 'native';
  claudeCliPath: string;
  model: string;
  modelReasoningEffort: string;
};

type ProviderAuthState = 'ready' | 'error';

type ProviderRowDetails = {
  status: string;
  auth: string;
  authState: ProviderAuthState;
  authTooltip?: string;
  model: string;
  reasoning: string;
  serviceTier: string;
};

const MODEL_PROVIDER_ROWS: FixedModelProviderRow[] = [
  {
    key: 'claude_code',
    agentId: 'claude',
    label: 'Claude Code',
    providerType: 'claude_code',
    defaultModel: '(provider default)',
    usageProviderId: usageProviderIdForModelProviderKey('claude_code'),
  },
  {
    key: 'codex_app_server',
    agentId: 'codex',
    label: 'Codex',
    providerType: 'codex_app_server',
    defaultModel: '(provider default)',
    usageProviderId: usageProviderIdForModelProviderKey('codex_app_server'),
  },
  {
    key: 'antigravity',
    agentId: 'antigravity',
    label: 'Antigravity',
    providerType: 'antigravity',
    defaultModel: 'Claude Opus 4.6 (Thinking)',
    usageProviderId: usageProviderIdForModelProviderKey('antigravity'),
  },
  {
    key: 'traex',
    agentId: 'traex',
    label: 'Traex',
    providerType: 'traex',
    defaultModel: '(provider default)',
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

const PROVIDER_MODEL_TYPES = Array.from(
  new Set(MODEL_PROVIDER_ROWS.map((row) => row.providerType)),
);

const METERED_MODEL_PROVIDER_ROWS = MODEL_PROVIDER_ROWS.filter((row) => row.usageProviderId);

function fixedModelProviderRow(key: FixedModelProviderKey): FixedModelProviderRow {
  return MODEL_PROVIDER_ROWS.find((row) => row.key === key) || MODEL_PROVIDER_ROWS[0];
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

function providerAgentConfig(gatewayDraft: any, key: FixedModelProviderKey): Record<string, any> {
  const row = fixedModelProviderRow(key);
  const agentsConfig = gatewayDraft && typeof gatewayDraft === 'object' && gatewayDraft.agents && typeof gatewayDraft.agents === 'object'
    ? gatewayDraft.agents
    : {};
  const candidates = Array.from(new Set([row.agentId, row.key]));
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
    model: row.defaultModel.startsWith('(') ? '' : row.defaultModel,
    modelReasoningEffort: '',
  };
}

function modelProviderDraftFromState(
  key: FixedModelProviderKey,
  gatewayDraft: any,
): ModelProviderConfigDraft {
  const providerConfig = providerAgentConfig(gatewayDraft, key);
  const configModel = String(providerConfig.default_model || '');
  const configReasoning = String(providerConfig.model_reasoning_effort || '');
  return {
    key,
    claudeCliMode: normalizeClaudeCliMode(providerConfig.claude_cli_mode),
    claudeCliPath: String(providerConfig.claude_cli_path || ''),
    model: configModel,
    modelReasoningEffort: configReasoning,
  };
}

type ProviderSettingsPanelProps = {
  gatewayDraft?: any;
  onMutateGatewayDraft?: DraftMutator;
  onSaveGatewaySettings?: (options?: GatewaySettingsSaveOptions) => Promise<boolean>;
};

export function ProviderSettingsPanel({
  gatewayDraft,
  onMutateGatewayDraft = () => {},
  onSaveGatewaySettings = async () => true,
}: ProviderSettingsPanelProps) {
  const { t } = useI18n();
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
  const providerModelRequestsRef = useRef<
    Partial<Record<DesktopApiProviderType, Promise<void>>>
  >({});
  const providerModelAttemptedRef = useRef<
    Partial<Record<DesktopApiProviderType, boolean>>
  >({});
  const [codingUsage, setCodingUsage] = useState<DesktopCodingUsage | null>(null);
  const [codingUsageLoading, setCodingUsageLoading] = useState(false);
  const [codingUsageError, setCodingUsageError] = useState<string | null>(null);
  const providerConfigRow = providerConfigKey ? fixedModelProviderRow(providerConfigKey) : null;
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
  const codingUsageByProviderId = useMemo(() => {
    const map: Record<string, DesktopProviderUsage> = {};
    for (const provider of codingUsage?.providers || []) {
      map[provider.id] = provider;
    }
    return map;
  }, [codingUsage]);

  function ensureProviderModels(
    providerType: DesktopApiProviderType,
    options: { retry?: boolean } = {},
  ) {
    if (!shouldRequestProviderModelCatalog({
      catalogs: providerModelsByType,
      requests: providerModelRequestsRef.current,
      attempted: providerModelAttemptedRef.current,
    }, providerType, options)) {
      return;
    }
    providerModelAttemptedRef.current[providerType] = true;
    setProviderModelsLoading((current) => ({
      ...current,
      [providerType]: true,
    }));
    const request = window.garyxDesktop.listProviderModels(providerType).then((models) => {
      setProviderModelsByType((current) => ({
        ...current,
        [providerType]: models,
      }));
    }).catch(() => {
      // The dialog keeps the raw model input fallback if catalog loading fails.
    }).finally(() => {
      delete providerModelRequestsRef.current[providerType];
      setProviderModelsLoading((current) => ({
        ...current,
        [providerType]: false,
      }));
    });
    providerModelRequestsRef.current[providerType] = request;
  }

  async function refreshCodingUsage() {
    setCodingUsageLoading(true);
    setCodingUsageError(null);
    try {
      const usage = await window.garyxDesktop.getCodingUsage();
      setCodingUsage(usage);
    } catch (error) {
      setCodingUsageError(error instanceof Error ? error.message : t('Failed to load usage.'));
    } finally {
      setCodingUsageLoading(false);
    }
  }

  useEffect(() => {
    for (const providerType of PROVIDER_MODEL_TYPES) {
      ensureProviderModels(providerType);
    }
    // Prefetch once when the provider panel mounts so Configure dropdowns are
    // backed by the gateway catalog instead of the current-value fallback.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!providerConfigRow) {
      return;
    }
    ensureProviderModels(providerConfigRow.providerType, { retry: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerConfigRow?.providerType]);

  useEffect(() => {
    void refreshCodingUsage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
  }, [providerConfigRow?.key, activeProviderModels]);
  function providerRowDetails(row: FixedModelProviderRow): ProviderRowDetails {
    const runtimeConfig = providerAgentConfig(gatewayDraft, row.key);
    const configuredDefaultModel = String(runtimeConfig.default_model || '').trim();
    const configuredReasoning = String(runtimeConfig.model_reasoning_effort || '').trim();
    const configuredServiceTier = String(runtimeConfig.model_service_tier || '').trim();
    const usage = row.usageProviderId ? codingUsageByProviderId[row.usageProviderId] : null;
    const usageAuthError = usage && !usage.available && usage.error ? usage.error : '';
    const finalize = (details: ProviderRowDetails): ProviderRowDetails => {
      if (!usageAuthError) {
        return details;
      }
      return {
        ...details,
        auth: t('Error'),
        authState: 'error',
        authTooltip: usageAuthError,
      };
    };
    if (row.key === 'claude_code') {
      const mode = normalizeClaudeCliMode(claudeAgentConfig(gatewayDraft).claude_cli_mode);
      return finalize({
        status: t('Default'),
        auth: claudeCliModeLabel(mode, t),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    if (row.key === 'codex_app_server') {
      return finalize({
        status: t('Default'),
        auth: t('CLI'),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    if (row.key === 'antigravity') {
      return finalize({
        status: t('Default'),
        auth: t('CLI'),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    if (row.key === 'traex') {
      // TRAE CLI authenticates via `traex login`; no desktop-managed auth.
      return finalize({
        status: t('Default'),
        auth: t('CLI'),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    return finalize({
      status: t('Default'),
      auth: t('CLI'),
      authState: 'ready',
      model: configuredDefaultModel || row.defaultModel,
      reasoning: configuredReasoning,
      serviceTier: configuredServiceTier,
    });
  }

  function providerModelLabel(value: string): string {
    return value === '(provider default)' ? t('Provider default') : value;
  }

  function renderProviderDefaultChips(details: ProviderRowDetails): ReactNode {
    const chips = [
      {
        key: 'model',
        label: providerModelLabel(details.model),
        title: t('Model: {value}', { value: providerModelLabel(details.model) }),
      },
      {
        key: 'reasoning',
        label: details.reasoning || t('Reasoning default'),
        title: t('Reasoning: {value}', { value: details.reasoning || t('Provider default') }),
      },
    ];
    const tier = details.serviceTier;
    if (tier) {
      chips.push({
        key: 'tier',
        label: tier,
        title: t('Tier: {value}', { value: tier }),
      });
    }
    const tooltip = chips.map((chip) => chip.title).join('\n');
    return (
      <div className="provider-config-default-cell" title={tooltip}>
        {chips.map((chip) => (
          <span className="provider-config-default-chip" key={chip.key}>
            {chip.label}
          </span>
        ))}
      </div>
    );
  }

  function usageUpdatedText(): string | null {
    const timestamp = Date.parse(codingUsage?.refreshedAt || '');
    if (!Number.isFinite(timestamp)) {
      return null;
    }
    const elapsedSeconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
    return t('updated {age} ago', { age: formatUsageDuration(elapsedSeconds) });
  }

  function usageWindowCaption(
    window: DesktopUsageWindow,
    fallback: string,
  ): string {
    return usageResetText(window.resetsAt, window.resetAfterSeconds, fallback);
  }

  function modelUsageCaption(model: DesktopModelUsage): string {
    return model.description?.trim()
      || usageResetText(model.resetsAt, model.resetAfterSeconds, t('reset time unknown'));
  }

  function sortedModelsByRemaining(usage: DesktopProviderUsage): DesktopModelUsage[] {
    return [...usage.models].sort((left, right) =>
      clampUsagePercent(left.remainingPercent) - clampUsagePercent(right.remainingPercent),
    );
  }

  function renderUsagePills(usage: DesktopProviderUsage, compact = false): ReactNode {
    const updated = usageUpdatedText();
    if (!usage.plan && !usage.stale && (!updated || compact)) {
      return null;
    }
    return (
      <div className={classNames('provider-usage-pills', compact && 'compact')}>
        {usage.plan ? <span className="provider-usage-pill">{usage.plan}</span> : null}
        {usage.stale ? <span className="provider-usage-pill stale">{t('stale')}</span> : null}
        {!compact && updated ? <span className="provider-usage-updated">{updated}</span> : null}
      </div>
    );
  }

  function renderUsageMeter(
    label: string,
    remainingPercent: number,
    caption: string,
    options?: {
      compact?: boolean;
      stale?: boolean;
      title?: string;
    },
  ): ReactNode {
    const percent = clampUsagePercent(remainingPercent);
    const level = usageLevelForRemainingPercent(percent);
    return (
      <div
        className={classNames('provider-usage-meter', options?.compact && 'compact')}
        data-level={level}
        data-stale={options?.stale ? 'true' : undefined}
        title={options?.title}
      >
        <div className="provider-usage-meter-header">
          <span className="provider-usage-meter-label">{label}</span>
          <span className="provider-usage-meter-percent">{formatUsagePercent(percent)}</span>
        </div>
        <div className="provider-usage-meter-track" aria-hidden>
          <span className="provider-usage-meter-fill" style={{ width: `${percent}%` }} />
        </div>
        {caption ? <div className="provider-usage-meter-caption">{caption}</div> : null}
      </div>
    );
  }

  function quotaGaugeStyle(remainingPercent: number): CSSProperties {
    return {
      '--provider-quota-percent': `${clampUsagePercent(remainingPercent)}%`,
    } as CSSProperties;
  }

  function renderQuotaGauge(
    label: string,
    remainingPercent: number,
    detail: string,
    options?: {
      available?: boolean;
      stale?: boolean;
      title?: string;
    },
  ): ReactNode {
    const percent = clampUsagePercent(remainingPercent);
    const level = usageLevelForRemainingPercent(percent, options?.available !== false);
    return (
      <div
        className="provider-quota-gauge"
        data-level={level}
        data-stale={options?.stale ? 'true' : undefined}
        title={options?.title}
      >
        <div
          className="provider-quota-gauge-ring"
          style={quotaGaugeStyle(percent)}
          aria-hidden
        >
          <span className="provider-quota-gauge-value">{formatUsagePercent(percent)}</span>
          <span className="provider-quota-gauge-label">{label}</span>
        </div>
        <span className="provider-quota-gauge-detail">{detail}</span>
      </div>
    );
  }

  function renderProviderQuotaCard(row: FixedModelProviderRow): ReactNode {
    const usage = row.usageProviderId ? codingUsageByProviderId[row.usageProviderId] : null;
    let body: ReactNode;
    let footer: ReactNode = null;
    const updated = usageUpdatedText();
    if (!usage) {
      const label = codingUsageLoading ? t('Loading') : codingUsageError ? t('Unavailable') : t('No data');
      body = renderQuotaGauge(label, 0, codingUsageError || t('Quota data pending'), {
        available: false,
      });
    } else if (!usage.available) {
      body = renderQuotaGauge(t('Unavailable'), 0, usage.error || t('No usage data'), {
        available: false,
        stale: usage.stale,
      });
    } else if (usage.models.length > 0) {
      const models = sortedModelsByRemaining(usage);
      const tightest = models[0];
      body = (
        <>
          {renderQuotaGauge(tightest.name, tightest.remainingPercent, modelUsageCaption(tightest), {
            stale: usage.stale,
            title: models
              .map((model) => `${model.name}: ${formatUsagePercent(model.remainingPercent)} · ${modelUsageCaption(model)}`)
              .join('\n'),
          })}
          <div className="provider-quota-secondary">
            <span>{t('{count} models', { count: models.length })}</span>
            <span>{t('tightest bucket')}</span>
          </div>
        </>
      );
    } else {
      const primary = usage.weekly || usage.session || null;
      const primaryLabel = usage.weekly ? t('Weekly') : t('Session');
      const primaryFallback = usage.weekly ? t('weekly window') : t('session window');
      body = primary ? (
        <>
          {renderQuotaGauge(
            primaryLabel,
            primary.remainingPercent,
            usageWindowCaption(primary, primaryFallback),
            { stale: usage.stale },
          )}
          {usage.session && usage.weekly ? (
            <div className="provider-quota-secondary">
              {renderUsageMeter(
                t('Session'),
                usage.session.remainingPercent,
                usageWindowCaption(usage.session, t('session window')),
                { compact: true, stale: usage.stale },
              )}
            </div>
          ) : null}
        </>
      ) : renderQuotaGauge(t('No data'), 0, t('Usage not reported'), {
        available: false,
        stale: usage.stale,
      });
    }

    if (usage) {
      footer = (
        <div className="provider-quota-card-meta">
          {usage.plan ? <span className="provider-usage-pill">{usage.plan}</span> : null}
          {usage.stale ? <span className="provider-usage-pill stale">{t('stale')}</span> : null}
          {usage.stale && updated ? <span className="provider-usage-updated">{updated}</span> : null}
        </div>
      );
    }

    return (
      <article
        className={classNames('provider-quota-card', usage?.stale && 'is-stale')}
        key={row.key}
      >
        <div className="provider-quota-card-header">
          <span className="provider-config-icon" aria-hidden>
            <ProviderAgentIcon
              agentId={row.agentId}
              providerType={row.providerType}
              size={22}
            />
          </span>
          <div className="provider-config-name-cell">
            <span className="provider-config-name">{row.label}</span>
            <span className="provider-config-subtitle">
              <code>{row.providerType}</code>
            </span>
          </div>
        </div>
        {body}
        {footer}
      </article>
    );
  }

  function renderProviderQuotaHero(): ReactNode {
    return (
      <section className="provider-quota-hero">
        <div className="provider-quota-card-grid">
          {METERED_MODEL_PROVIDER_ROWS.map((row) => renderProviderQuotaCard(row))}
        </div>
      </section>
    );
  }

  function renderProviderConfigUsageSection(row: FixedModelProviderRow): ReactNode {
    if (!row.usageProviderId) {
      return null;
    }
    const usage = codingUsageByProviderId[row.usageProviderId];
    let content: ReactNode;
    if (!usage) {
      content = codingUsageLoading
        ? <span className="provider-usage-muted">{t('Loading')}</span>
        : (
          <span className="provider-usage-muted" title={codingUsageError || undefined}>
            {codingUsageError ? t('Unavailable') : t('No quota data')}
          </span>
        );
    } else if (!usage.available) {
      content = (
        <span className="provider-usage-muted" title={usage.error || undefined}>
          {t('Unavailable')}
        </span>
      );
    } else if (usage.models.length > 0) {
      content = (
        <div className="provider-usage-models">
          {sortedModelsByRemaining(usage).map((model) => (
            <div className="provider-usage-model-row" key={model.id || model.name}>
              {renderUsageMeter(model.name, model.remainingPercent, modelUsageCaption(model), {
                stale: usage.stale,
                title: model.description || undefined,
              })}
            </div>
          ))}
        </div>
      );
    } else {
      const windows: Array<{ key: string; label: string; value: DesktopUsageWindow; fallback: string }> = [];
      if (usage.session) {
        windows.push({
          key: 'session',
          label: t('Session'),
          value: usage.session,
          fallback: t('session window'),
        });
      }
      if (usage.weekly) {
        windows.push({
          key: 'weekly',
          label: t('Weekly'),
          value: usage.weekly,
          fallback: t('weekly window'),
        });
      }
      content = windows.length > 0 ? (
        <div className="provider-usage-window-grid">
          {windows.map((entry) => (
            <div className="provider-usage-window-card" key={entry.key}>
              {renderUsageMeter(
                entry.label,
                entry.value.remainingPercent,
                usageWindowCaption(entry.value, entry.fallback),
                { stale: usage.stale },
              )}
            </div>
          ))}
        </div>
      ) : <span className="provider-usage-muted">{t('No quota data')}</span>;
    }
    return (
      <section className={classNames('provider-config-usage-section', usage?.stale && 'is-stale')}>
        <div className="provider-config-usage-header">
          <div>
            <span className="provider-config-usage-title">{t('Usage')}</span>
            <span className="provider-config-usage-note">
              {usage ? t('{name} quota windows', { name: usage.name || row.label }) : t('Quota windows')}
            </span>
          </div>
          {usage ? renderUsagePills(usage) : null}
        </div>
        {content}
      </section>
    );
  }

  function openProviderConfigDialog(key: FixedModelProviderKey) {
    const row = fixedModelProviderRow(key);
    const draft = modelProviderDraftFromState(key, gatewayDraft);
    ensureProviderModels(row.providerType, { retry: true });
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
      delete nextConfig.model_service_tier;
      next.agents[row.agentId] = nextConfig;
    });
  }

  async function handleSaveProviderConfig() {
    if (!providerConfigRow || providerConfigSaving) {
      return;
    }
    setProviderConfigSaving(true);
    try {
      if (providerConfigRow.key === 'claude_code') {
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
      // CLI providers authenticate through their own login or provider env;
      // this surface persists model defaults only.
      mutateGatewayProviderModelDefaults(providerConfigRow, providerConfigDraft);
      if (await onSaveGatewaySettings({ refreshDesktopState: 'background' })) {
        closeProviderConfigDialog();
      }
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
              <TableHead className="provider-config-col-auth">{t('Auth')}</TableHead>
              <TableHead className="provider-config-col-default">{t('Default')}</TableHead>
              <TableHead className="provider-config-col-status">{t('Status')}</TableHead>
              <TableHead className="provider-config-col-actions">{t('Actions')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {MODEL_PROVIDER_ROWS.map((row) => {
              const details = providerRowDetails(row);
              return (
                <TableRow key={row.key}>
                  <TableCell className="provider-config-col-provider">
                    <div className="provider-config-provider-cell">
                      <span className="provider-config-icon" aria-hidden>
                        <ProviderAgentIcon
                          agentId={row.agentId}
                          providerType={row.providerType}
                          size={22}
                        />
                      </span>
                      <div className="provider-config-name-cell">
                        <span className="provider-config-name">{row.label}</span>
                        <span className="provider-config-subtitle">
                          <code>{row.providerType}</code>
                          <span>{t('Default CLI')}</span>
                        </span>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="provider-config-col-auth">
                    <Badge
                      className="provider-config-auth"
                      data-state={details.authState}
                      title={details.authTooltip || details.auth}
                      variant="outline"
                    >
                      {details.auth}
                    </Badge>
                  </TableCell>
                  <TableCell className="provider-config-col-default">
                    {renderProviderDefaultChips(details)}
                  </TableCell>
                  <TableCell className="provider-config-col-status">
                    <Badge
                      className="provider-config-status"
                      data-state={details.authState === 'error' ? 'empty' : 'ready'}
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
      {renderProviderQuotaHero()}
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
              {t('Default Provider')}
            </Badge>
            <div className="provider-config-dialog-heading">
              {providerConfigRow ? (
                <span className="provider-config-icon large" aria-hidden>
                  <ProviderAgentIcon
                    agentId={providerConfigRow.agentId}
                    providerType={providerConfigRow.providerType}
                    size={28}
                  />
                </span>
              ) : null}
              <div className="commands-dialog-title-group">
                <DialogTitle className="commands-dialog-title">
                  {providerConfigRow ? t('Configure {name}', { name: providerConfigRow.label }) : t('Configure Provider')}
                </DialogTitle>
                <DialogDescription className="commands-dialog-description provider-config-dialog-description">
                  {providerConfigRow ? (
                    <span className="provider-config-header-meta">
                      <code>{providerConfigRow.providerType}</code>
                      <span>{t('Default CLI')}</span>
                    </span>
                  ) : null}
                  <span>{t('Provider rows are fixed. Configuration controls whether each provider is ready to use.')}</span>
                </DialogDescription>
              </div>
            </div>
          </DialogHeader>

          <div className="commands-dialog-body provider-config-dialog-body">
            {providerConfigRow ? renderProviderConfigUsageSection(providerConfigRow) : null}

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
              </>
            ) : null}

            {providerConfigRow ? (
              <>
                <div className="provider-config-grid">
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-model">{t('Model')}</Label>
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
                            };
                          });
                        }}
                      >
                        <SelectTrigger id="provider-model">
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
                        id="provider-model"
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
              </>
            ) : null}
          </div>

          <DialogFooter className="commands-dialog-footer">
            <div className="provider-config-footer-left" />
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
