import type {
  DesktopApiProviderType,
  DesktopProviderModelOption,
  DesktopProviderModels,
} from '@shared/contracts';
import { usageProviderIdForModelProviderKey } from '../provider-usage.ts';

export type FixedModelProviderKey =
  | 'claude_code'
  | 'codex_app_server'
  | 'antigravity'
  | 'traex';

export type FixedModelProviderRow = {
  key: FixedModelProviderKey;
  agentId: string;
  label: string;
  providerType: DesktopApiProviderType;
  defaultModel: string;
  usageProviderId?: string;
};

export type ModelProviderConfigDraft = {
  key: FixedModelProviderKey;
  claudeCliMode: 'cctty' | 'native';
  claudeCliPath: string;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
};

export const MODEL_PROVIDER_ROWS: FixedModelProviderRow[] = [
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

export const REASONING_EFFORT_RANK: Record<string, number> = {
  off: 0,
  minimal: 1,
  low: 2,
  medium: 3,
  high: 4,
  xhigh: 5,
  max: 6,
};

export function fixedModelProviderRow(key: FixedModelProviderKey): FixedModelProviderRow {
  return MODEL_PROVIDER_ROWS.find((row) => row.key === key) || MODEL_PROVIDER_ROWS[0];
}

export function providerModelOptionsWithCurrent(
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

export function reasoningEffortOptionsForModel(
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

function rawServiceTiersForModel(
  providerModels: DesktopProviderModels | null | undefined,
  modelId: string,
): DesktopProviderModelOption[] {
  const selectedModel = providerModels?.models.find((model) => model.id === modelId.trim());
  return selectedModel?.serviceTiers?.length
    ? selectedModel.serviceTiers
    : providerModels?.serviceTiers || [];
}

export function serviceTierOptionsForModel(
  providerModels: DesktopProviderModels | null | undefined,
  modelId: string,
  currentServiceTier: string,
): DesktopProviderModelOption[] {
  const options = rawServiceTiersForModel(providerModels, modelId);
  const trimmed = currentServiceTier.trim();
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

/// A configured service tier survives a model change only when the target
/// model supports it; otherwise the draft falls back to the provider default
/// tier (mirrors the composer and iOS `selectModel` sanitize).
export function sanitizedServiceTier(
  providerModels: DesktopProviderModels | null | undefined,
  modelId: string,
  currentServiceTier: string,
): string {
  const trimmed = currentServiceTier.trim();
  if (!trimmed) {
    return '';
  }
  return rawServiceTiersForModel(providerModels, modelId).some((tier) => tier.id === trimmed)
    ? trimmed
    : '';
}

export function highestReasoningEffort(options: DesktopProviderModelOption[]): string {
  return options.reduce((best, option) => {
    if (!best) {
      return option.id;
    }
    return (REASONING_EFFORT_RANK[option.id] ?? -1) > (REASONING_EFFORT_RANK[best] ?? -1)
      ? option.id
      : best;
  }, '');
}

export function applyProviderCatalogDefaults(
  draft: ModelProviderConfigDraft,
  providerModels: DesktopProviderModels | null | undefined,
): ModelProviderConfigDraft {
  if (!providerModels) {
    return draft;
  }
  const model = draft.model.trim();
  const modelServiceTier = sanitizedServiceTier(providerModels, model, draft.modelServiceTier);
  if (!model) {
    return {
      ...draft,
      model: '',
      modelReasoningEffort: '',
      modelServiceTier,
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
    modelServiceTier,
  };
}

export function providerAgentConfig(
  gatewayDraft: any,
  key: FixedModelProviderKey,
): Record<string, any> {
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

export function normalizeClaudeCliMode(value: unknown): 'cctty' | 'native' {
  return String(value || '').trim().toLowerCase() === 'cctty' ? 'cctty' : 'native';
}

export function emptyModelProviderConfigDraft(
  key: FixedModelProviderKey = 'claude_code',
): ModelProviderConfigDraft {
  const row = fixedModelProviderRow(key);
  return {
    key,
    claudeCliMode: 'native',
    claudeCliPath: '',
    model: row.defaultModel.startsWith('(') ? '' : row.defaultModel,
    modelReasoningEffort: '',
    modelServiceTier: '',
  };
}

export function modelProviderDraftFromState(
  key: FixedModelProviderKey,
  gatewayDraft: any,
): ModelProviderConfigDraft {
  const providerConfig = providerAgentConfig(gatewayDraft, key);
  return {
    key,
    claudeCliMode: normalizeClaudeCliMode(providerConfig.claude_cli_mode),
    claudeCliPath: String(providerConfig.claude_cli_path || ''),
    model: String(providerConfig.default_model || ''),
    modelReasoningEffort: String(providerConfig.model_reasoning_effort || ''),
    modelServiceTier: String(providerConfig.model_service_tier || ''),
  };
}

/// The single writer for the provider Configure dialog: persists the model
/// defaults (model, reasoning effort, service tier) for every provider and
/// the Claude CLI mode/path for `claude_code`. The legacy `model` key is
/// retired in favor of `default_model`.
export function applyProviderConfigDraftToGatewayConfig(
  gatewayConfig: any,
  row: FixedModelProviderRow,
  draft: ModelProviderConfigDraft,
): void {
  gatewayConfig.agents = gatewayConfig.agents || {};
  const current = gatewayConfig.agents[row.agentId] && typeof gatewayConfig.agents[row.agentId] === 'object'
    ? gatewayConfig.agents[row.agentId]
    : {};
  const nextConfig: Record<string, any> = {
    ...current,
    provider_type: row.providerType,
    default_model: draft.model.trim(),
    model_reasoning_effort: draft.modelReasoningEffort.trim(),
    model_service_tier: draft.modelServiceTier.trim(),
  };
  delete nextConfig.model;
  if (row.key === 'claude_code') {
    nextConfig.claude_cli_mode = draft.claudeCliMode;
    const cliPath = draft.claudeCliPath.trim();
    if (cliPath) {
      nextConfig.claude_cli_path = cliPath;
    } else {
      delete nextConfig.claude_cli_path;
    }
  }
  gatewayConfig.agents[row.agentId] = nextConfig;
}
