import type {
  DesktopProviderModelOption,
  DesktopProviderModels,
} from '@shared/contracts';

export type ComposerModelControlState = {
  models: DesktopProviderModelOption[];
  effectiveModelId: string;
  defaultModelId: string;
  defaultModelLabel: string;
  defaultModelOption?: DesktopProviderModelOption;
  triggerLabel: string;
  reasoningEfforts: DesktopProviderModelOption[];
  effectiveReasoningEffortId: string;
  defaultReasoningEffortId: string;
  defaultEffortLabel: string;
  serviceTiers: DesktopProviderModelOption[];
  effectiveServiceTierId: string;
  defaultServiceTierLabel: string;
};

export function resolveComposerModelControlState({
  providerModels,
  agentConfiguredModel,
  effectiveModel,
  effectiveReasoningEffort,
  effectiveServiceTier,
  selectedModel,
  selectedReasoningEffort,
  selectedServiceTier,
  modelFallbackLabel,
  thinkingLevelFallbackLabel,
  standardServiceTierLabel,
}: {
  providerModels: DesktopProviderModels;
  agentConfiguredModel?: string | null;
  effectiveModel?: string | null;
  effectiveReasoningEffort?: string | null;
  effectiveServiceTier?: string | null;
  selectedModel?: string | null;
  selectedReasoningEffort?: string | null;
  selectedServiceTier?: string | null;
  modelFallbackLabel: string;
  thinkingLevelFallbackLabel: string;
  standardServiceTierLabel: string;
}): ComposerModelControlState {
  const catalogModels = providerModels.models || [];
  const selectedModelId = selectedModel?.trim() || '';
  const configuredDefaultModelId =
    agentConfiguredModel?.trim() || providerModels.defaultModel?.trim() || '';
  const effectiveModelId =
    selectedModelId ||
    effectiveModel?.trim() ||
    agentConfiguredModel?.trim() ||
    '';
  const models =
    effectiveModelId && !catalogModels.some((option) => option.id === effectiveModelId)
      ? [
          ...catalogModels,
          {
            id: effectiveModelId,
            label: effectiveModelId,
            recommended: false,
            supportedReasoningEfforts: providerModels.reasoningEfforts || [],
            serviceTiers: providerModels.serviceTiers || [],
          },
        ]
      : catalogModels;
  const selectedModelOption = selectedModelId
    ? models.find((option) => option.id === selectedModelId)
    : undefined;
  const effectiveModelOption = effectiveModelId
    ? models.find((option) => option.id === effectiveModelId)
    : undefined;
  const defaultModelId =
    configuredDefaultModelId || (!selectedModelId ? effectiveModel?.trim() || '' : '');
  const defaultModelOption = defaultModelId
    ? models.find((option) => option.id === defaultModelId)
    : undefined;
  const defaultModelLabel = defaultModelOption?.label || defaultModelId || modelFallbackLabel;
  const effortFilterModelOption = selectedModelOption || effectiveModelOption || defaultModelOption;
  const catalogReasoningEfforts =
    effortFilterModelOption?.supportedReasoningEfforts?.length
      ? effortFilterModelOption.supportedReasoningEfforts
      : providerModels.reasoningEfforts || [];
  const effectiveReasoningEffortId =
    selectedReasoningEffort?.trim() || effectiveReasoningEffort?.trim() || '';
  const reasoningEfforts =
    effectiveReasoningEffortId &&
    !catalogReasoningEfforts.some((option) => option.id === effectiveReasoningEffortId)
      ? [
          ...catalogReasoningEfforts,
          { id: effectiveReasoningEffortId, label: effectiveReasoningEffortId, recommended: false },
        ]
      : catalogReasoningEfforts;
  const selectedEffortOption = effectiveReasoningEffortId
    ? reasoningEfforts.find((option) => option.id === effectiveReasoningEffortId)
    : undefined;
  const configuredDefaultReasoningEffortId =
    providerModels.defaultReasoningEffort?.trim() || '';
  const supportedConfiguredDefaultReasoningEffortId =
    configuredDefaultReasoningEffortId &&
    reasoningEfforts.some((option) => option.id === configuredDefaultReasoningEffortId)
      ? configuredDefaultReasoningEffortId
      : '';
  const defaultReasoningEffortId =
    supportedConfiguredDefaultReasoningEffortId ||
    effortFilterModelOption?.defaultReasoningEffort?.trim() ||
    reasoningEfforts.find((option) => option.recommended)?.id ||
    reasoningEfforts[0]?.id ||
    '';
  const defaultEffortOption = defaultReasoningEffortId
    ? reasoningEfforts.find((option) => option.id === defaultReasoningEffortId)
    : undefined;
  const defaultEffortLabel =
    defaultEffortOption?.label || defaultReasoningEffortId || thinkingLevelFallbackLabel;
  const catalogServiceTiers =
    effortFilterModelOption?.serviceTiers?.length
      ? effortFilterModelOption.serviceTiers
      : providerModels.serviceTiers || [];
  const effectiveServiceTierId =
    selectedServiceTier?.trim() || effectiveServiceTier?.trim() || '';
  const serviceTiers =
    effectiveServiceTierId &&
    !catalogServiceTiers.some((option) => option.id === effectiveServiceTierId)
      ? [
          ...catalogServiceTiers,
          { id: effectiveServiceTierId, label: effectiveServiceTierId, recommended: false },
        ]
      : catalogServiceTiers;
  const triggerModelLabel = effectiveModelOption?.label ?? defaultModelLabel;
  const defaultTriggerEffortOption =
    defaultModelOption && !effectiveModelOption ? defaultEffortOption : undefined;
  const triggerEffortOption = selectedEffortOption ?? defaultTriggerEffortOption;
  const triggerLabel = triggerEffortOption
    ? `${triggerModelLabel} · ${triggerEffortOption.label}`
    : triggerModelLabel;

  return {
    models,
    effectiveModelId,
    defaultModelId,
    defaultModelLabel,
    defaultModelOption,
    triggerLabel,
    reasoningEfforts,
    effectiveReasoningEffortId,
    defaultReasoningEffortId,
    defaultEffortLabel,
    serviceTiers,
    effectiveServiceTierId,
    defaultServiceTierLabel: standardServiceTierLabel,
  };
}
