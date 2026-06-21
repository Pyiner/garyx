import { useEffect, useMemo, useState } from 'react';

import type {
  CreateCustomAgentInput,
  DesktopCustomAgent,
  DesktopProviderModels,
  UpdateCustomAgentInput,
} from '@shared/contracts';

import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Textarea } from '../../components/ui/textarea';
import { useI18n } from '../../i18n';

type ProviderType = 'claude_code' | 'codex_app_server' | 'antigravity' | 'traex' | 'gemini_cli' | 'gpt' | 'anthropic' | 'google' | 'claude_llm' | 'gemini_llm';
type EditorMode = 'inspect' | 'create' | 'edit';

type AgentsPanelProps = {
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

type AgentDraft = {
  agentId: string;
  displayName: string;
  providerType: ProviderType;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
  authSource: string;
  apiKey: string;
  baseUrl: string;
  defaultWorkspaceDir: string;
  systemPrompt: string;
};

const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default__';
const PROVIDER_DEFAULT_REASONING_VALUE = '__provider_default_reasoning__';
const PROVIDER_DEFAULT_SERVICE_TIER_VALUE = '__provider_default_service_tier__';

const FALLBACK_REASONING_EFFORTS = [
  { id: 'none', label: 'None', description: 'No reasoning', recommended: false },
  { id: 'minimal', label: 'Minimal', description: 'Minimal reasoning', recommended: false },
  { id: 'low', label: 'Low', description: 'Faster responses', recommended: false },
  { id: 'medium', label: 'Medium', description: 'Balanced speed and reasoning', recommended: true },
  { id: 'high', label: 'High', description: 'Deeper reasoning', recommended: false },
  { id: 'xhigh', label: 'Extra High', description: 'Maximum reasoning', recommended: false },
];

function emptyDraft(): AgentDraft {
  return {
    agentId: '',
    displayName: '',
    providerType: 'claude_code',
    model: '',
    modelReasoningEffort: '',
    modelServiceTier: '',
    authSource: 'codex',
    apiKey: '',
    baseUrl: '',
    defaultWorkspaceDir: '',
    systemPrompt: '',
  };
}

function deriveAgentId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}

function providerLabel(value: ProviderType): string {
  if (value === 'codex_app_server') {
    return 'Codex';
  }
  if (value === 'antigravity') {
    return 'Antigravity';
  }
  if (value === 'traex') {
    return 'Traex';
  }
  if (value === 'gemini_cli') {
    return 'Gemini';
  }
  if (value === 'gpt') {
    return 'GPT';
  }
  if (value === 'anthropic' || value === 'claude_llm') {
    return 'Claude';
  }
  if (value === 'google' || value === 'gemini_llm') {
    return 'Gemini';
  }
  return 'Claude';
}

function isNativeModelProvider(value: ProviderType): boolean {
  return value === 'gpt' || value === 'anthropic' || value === 'google' || value === 'claude_llm' || value === 'gemini_llm';
}

function defaultAuthSource(value: ProviderType): string {
  return value === 'gpt' ? 'codex' : 'api_key';
}

function apiKeyEnvName(value: ProviderType): string | null {
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

function apiKeyFromAgent(agent: DesktopCustomAgent): string {
  const envName = apiKeyEnvName(agent.providerType as ProviderType);
  return envName ? agent.providerEnv?.[envName] || '' : '';
}

function providerModelsWithCurrent(
  providerModels: DesktopProviderModels | undefined,
  currentModel: string,
): DesktopProviderModels['models'] {
  const models = providerModels?.models || [];
  const normalized = currentModel.trim();
  if (!normalized || models.some((model) => model.id === normalized)) {
    return models;
  }
  return [{ id: normalized, label: normalized, description: null, recommended: false }, ...models];
}

function reasoningEffortsWithCurrent(
  providerModels: DesktopProviderModels | undefined,
  currentModel: string,
  currentEffort: string,
): DesktopProviderModels['models'] {
  const selectedModel = providerModels?.models.find((model) => model.id === currentModel.trim());
  const efforts = selectedModel?.supportedReasoningEfforts?.length
    ? selectedModel.supportedReasoningEfforts
    : providerModels?.reasoningEfforts?.length
    ? providerModels.reasoningEfforts
    : FALLBACK_REASONING_EFFORTS;
  const normalized = currentEffort.trim();
  if (!normalized || efforts.some((effort) => effort.id === normalized)) {
    return efforts;
  }
  return [{ id: normalized, label: normalized, description: null, recommended: false }, ...efforts];
}

function serviceTiersWithCurrent(
  providerModels: DesktopProviderModels | undefined,
  currentModel: string,
  currentServiceTier: string,
): DesktopProviderModels['models'] {
  const selectedModel = providerModels?.models.find((model) => model.id === currentModel.trim());
  const tiers = selectedModel?.serviceTiers?.length
    ? selectedModel.serviceTiers
    : providerModels?.serviceTiers?.length
    ? providerModels.serviceTiers
    : [];
  const normalized = currentServiceTier.trim();
  if (!normalized || tiers.some((tier) => tier.id === normalized)) {
    return tiers;
  }
  return [{ id: normalized, label: normalized, description: null, recommended: false }, ...tiers];
}

const plusIcon = (
  <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
    <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
  </svg>
);

export function AgentsPanel({ onToast }: AgentsPanelProps) {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [agents, setAgents] = useState<DesktopCustomAgent[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [editorMode, setEditorMode] = useState<EditorMode>('inspect');
  const [draft, setDraft] = useState<AgentDraft>(() => emptyDraft());
  const [draftIdTouched, setDraftIdTouched] = useState(false);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<ProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<ProviderType, boolean>>
  >({});

  async function loadAgents(preferredAgentId?: string | null) {
    setLoading(true);
    try {
      const nextAgents = await window.garyxDesktop.listCustomAgents();
      const visibleAgents = nextAgents
        .filter((agent) => agent.standalone)
        .sort((left, right) => {
          if (left.builtIn !== right.builtIn) {
            return left.builtIn ? -1 : 1;
          }
          return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
        });
      setAgents(visibleAgents);
      setSelectedAgentId(preferredAgentId || selectedAgentId || visibleAgents[0]?.agentId || null);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to load agents'), 'error');
    } finally {
      setLoading(false);
    }
  }

  async function ensureProviderModels(providerType: ProviderType) {
    if (providerModelsByType[providerType] || providerModelsLoading[providerType]) {
      return;
    }
    setProviderModelsLoading((current) => ({ ...current, [providerType]: true }));
    try {
      const result = await window.garyxDesktop.listProviderModels(providerType);
      setProviderModelsByType((current) => ({ ...current, [providerType]: result }));
    } catch (error) {
      setProviderModelsByType((current) => ({
        ...current,
        [providerType]: {
          providerType,
          supportsModelSelection: false,
          models: [],
          defaultModel: null,
          source: 'desktop',
          error: error instanceof Error ? error.message : t('Failed to load models'),
        },
      }));
    } finally {
      setProviderModelsLoading((current) => ({ ...current, [providerType]: false }));
    }
  }

  useEffect(() => {
    void loadAgents();
  }, []);

  useEffect(() => {
    if (editorMode === 'create' || editorMode === 'edit') {
      void ensureProviderModels(draft.providerType);
    }
  }, [draft.providerType, editorMode]);

  useEffect(() => {
    if (editorMode !== 'create' || draftIdTouched) {
      return;
    }
    const nextId = deriveAgentId(draft.displayName);
    setDraft((current) => (current.agentId === nextId ? current : { ...current, agentId: nextId }));
  }, [draft.displayName, draftIdTouched, editorMode]);

  const selectedAgent = useMemo(
    () => agents.find((agent) => agent.agentId === selectedAgentId) || null,
    [agents, selectedAgentId],
  );
  const activeProviderModels = providerModelsByType[draft.providerType];
  const providerModelsBusy = providerModelsLoading[draft.providerType] === true;
  const modelOptions = providerModelsWithCurrent(activeProviderModels, draft.model);
  const supportsModelSelection =
    activeProviderModels?.supportsModelSelection === true && modelOptions.length > 0;
  const reasoningEffortOptions = reasoningEffortsWithCurrent(
    activeProviderModels,
    draft.model || activeProviderModels?.defaultModel || '',
    draft.modelReasoningEffort,
  );
  const supportsReasoningEffortSelection =
    activeProviderModels?.supportsReasoningEffortSelection === true
    && reasoningEffortOptions.length > 0;
  const serviceTierOptions = serviceTiersWithCurrent(
    activeProviderModels,
    draft.model || activeProviderModels?.defaultModel || '',
    draft.modelServiceTier,
  );
  const supportsServiceTierSelection =
    draft.providerType === 'gpt'
    && (activeProviderModels?.supportsServiceTierSelection === true || draft.modelServiceTier.trim().length > 0)
    && serviceTierOptions.length > 0;
  const modelStatus =
    draft.providerType === 'gemini_cli' && !supportsModelSelection
      ? providerModelsBusy
        ? t('Loading models from local Gemini ACP...')
        : activeProviderModels?.error
          ? t('Local Gemini ACP does not expose a model list yet.')
          : null
      : null;

  function openCreateEditor() {
    setEditorMode('create');
    setDraft(emptyDraft());
    setDraftIdTouched(false);
  }

  function openEditEditor(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      return;
    }
    setEditorMode('edit');
    setDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      modelReasoningEffort: agent.modelReasoningEffort,
      modelServiceTier: agent.modelServiceTier,
      authSource: agent.authSource || defaultAuthSource(agent.providerType as ProviderType),
      apiKey: apiKeyFromAgent(agent),
      baseUrl: agent.baseUrl || '',
      defaultWorkspaceDir: agent.defaultWorkspaceDir,
      systemPrompt: agent.systemPrompt,
    });
    setDraftIdTouched(true);
  }

  async function handleDelete(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      return;
    }
    setSaving(true);
    try {
      await window.garyxDesktop.deleteCustomAgent({ agentId: agent.agentId });
      onToast?.(t('Custom agent deleted'), 'success');
      setEditorMode('inspect');
      await loadAgents(agents.find((item) => item.agentId !== agent.agentId)?.agentId || null);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      const nativeProvider = isNativeModelProvider(draft.providerType);
      const apiKeyEnv = apiKeyEnvName(draft.providerType);
      const providerEnv = nativeProvider && apiKeyEnv && draft.apiKey.trim()
        ? { [apiKeyEnv]: draft.apiKey.trim() }
        : null;
      const payload: CreateCustomAgentInput = {
        agentId: draft.agentId.trim(),
        displayName: draft.displayName.trim(),
        providerType: draft.providerType,
        model: supportsModelSelection ? draft.model.trim() : '',
        modelReasoningEffort: supportsReasoningEffortSelection ? draft.modelReasoningEffort.trim() : '',
        modelServiceTier: supportsServiceTierSelection ? draft.modelServiceTier.trim() : '',
        providerEnv,
        authSource: nativeProvider
          ? (draft.authSource.trim() || defaultAuthSource(draft.providerType))
          : null,
        baseUrl: nativeProvider ? draft.baseUrl.trim() : null,
        defaultWorkspaceDir: draft.defaultWorkspaceDir.trim(),
        systemPrompt: draft.systemPrompt.trim(),
      };
      let saved: DesktopCustomAgent;
      if (editorMode === 'create') {
        saved = await window.garyxDesktop.createCustomAgent(payload);
        onToast?.(t('Custom agent created'), 'success');
      } else {
        const updatePayload: UpdateCustomAgentInput = {
          ...payload,
          currentAgentId: selectedAgent?.agentId || payload.agentId,
        };
        saved = await window.garyxDesktop.updateCustomAgent(updatePayload);
        onToast?.(t('Custom agent updated'), 'success');
      }
      setEditorMode('inspect');
      setDraft(emptyDraft());
      setDraftIdTouched(false);
      await loadAgents(saved.agentId);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  const validationError =
    !draft.displayName.trim()
      ? t('Name is required.')
      : !draft.agentId.trim()
        ? t('Agent ID is required.')
        : !draft.systemPrompt.trim()
          ? t('System prompt is required.')
          : null;

  const showingEditor = editorMode === 'create' || (editorMode === 'edit' && selectedAgent && !selectedAgent.builtIn);

  return (
    <div className="grid h-full min-h-0 w-full gap-6" style={{ gridTemplateColumns: '340px minmax(0,1fr)' }}>
      {/* ── Left column: agent list ── */}
      <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
        <div className="codex-section">
          <div className="codex-section-header">
            <span className="codex-section-title">{t('Agents')}</span>
            <button className="codex-section-action" onClick={openCreateEditor} type="button">
              {plusIcon} {t('New')}
            </button>
          </div>
        </div>
        {loading ? (
          <div className="codex-empty-state">{t('Loading agents...')}</div>
        ) : agents.length ? (
          <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
            {agents.map((agent) => {
              const active = agent.agentId === selectedAgentId && editorMode === 'inspect';
              return (
                <button
                  key={agent.agentId}
                  className={`codex-list-row w-full text-left ${active ? 'codex-list-row-active' : ''}`}
                  onClick={() => {
                    setSelectedAgentId(agent.agentId);
                    setEditorMode('inspect');
                  }}
                  type="button"
                >
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                    <span className="codex-list-row-name">{agent.displayName}</span>
                    <span className="codex-command-row-desc">{agent.agentId}</span>
                  </div>
                  <div className="codex-list-row-actions">
                    <span className="codex-sync-pill ok">
                      {agent.builtIn ? t('built-in') : providerLabel(agent.providerType)}
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        ) : (
          <div className="codex-empty-state">{t('No agents found.')}</div>
        )}
      </div>

      {/* ── Right column: inspect or edit ── */}
      {showingEditor ? (
        <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
          <div className="codex-section">
            <div className="codex-section-header">
              <span className="codex-section-title">
                {editorMode === 'create' ? t('New Agent') : t('Edit Agent')}
              </span>
            </div>
          </div>
          <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
            <form onSubmit={handleSubmit}>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-display-name">{t('Name')}</Label>
                <Input
                  id="agent-display-name"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, displayName: event.target.value }));
                  }}
                  value={draft.displayName}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-id">{t('Agent ID')}</Label>
                <Input
                  disabled={editorMode === 'edit'}
                  id="agent-id"
                  onChange={(event) => {
                    setDraftIdTouched(true);
                    setDraft((current) => ({ ...current, agentId: event.target.value }));
                  }}
                  value={draft.agentId}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label">{t('Provider')}</Label>
                <Select
                  onValueChange={(value: ProviderType) => {
                    setDraft((current) => ({
                      ...current,
                      providerType: value,
                      model: '',
                      modelReasoningEffort: value === 'codex_app_server' || value === 'traex' || isNativeModelProvider(value)
                        ? current.modelReasoningEffort
                        : '',
                      modelServiceTier: value === 'gpt' ? current.modelServiceTier : '',
                      authSource: isNativeModelProvider(value) ? defaultAuthSource(value) : '',
                      apiKey: '',
                      baseUrl: '',
                    }));
                    void ensureProviderModels(value);
                  }}
                  value={draft.providerType}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t('Select provider')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectItem value="claude_code">Claude</SelectItem>
                      <SelectItem value="codex_app_server">Codex</SelectItem>
                      <SelectItem value="antigravity">Antigravity</SelectItem>
                      <SelectItem value="traex">Trae</SelectItem>
                      <SelectItem value="gemini_cli">Gemini</SelectItem>
                      <SelectItem value="gpt">GPT</SelectItem>
                      <SelectItem value="anthropic">Claude</SelectItem>
                      <SelectItem value="google">Gemini</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
                {modelStatus ? <span className="codex-form-hint">{modelStatus}</span> : null}
              </div>
              {isNativeModelProvider(draft.providerType) ? (
                <>
                  {draft.providerType === 'gpt' ? (
                    <div className="codex-form-field">
                      <Label className="codex-form-label">{t('GPT auth')}</Label>
                      <Select
                        onValueChange={(value) => {
                          setDraft((current) => ({
                            ...current,
                            authSource: value,
                            apiKey: value === 'codex' ? '' : current.apiKey,
                          }));
                        }}
                        value={draft.authSource || 'codex'}
                      >
                        <SelectTrigger>
                          <SelectValue placeholder={t('Select auth')} />
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
                  {draft.providerType !== 'gpt' || draft.authSource === 'api_key' ? (
                    <div className="codex-form-field">
                      <Label className="codex-form-label" htmlFor="agent-api-key">{t('API Key')}</Label>
                      <Input
                        autoCapitalize="off"
                        autoComplete="off"
                        id="agent-api-key"
                        onChange={(event) => {
                          setDraft((current) => ({ ...current, apiKey: event.target.value }));
                        }}
                        placeholder={
                          draft.providerType === 'anthropic' || draft.providerType === 'claude_llm'
                            ? 'ANTHROPIC_API_KEY'
                            : draft.providerType === 'google' || draft.providerType === 'gemini_llm'
                              ? 'GEMINI_API_KEY'
                              : 'OPENAI_API_KEY'
                        }
                        spellCheck={false}
                        type="password"
                        value={draft.apiKey}
                      />
                      <span className="codex-form-hint">{t('Stored on this custom provider config.')}</span>
                    </div>
                  ) : null}
                  <div className="codex-form-field">
                    <Label className="codex-form-label" htmlFor="agent-base-url">{t('Base URL')}</Label>
                    <Input
                      autoCapitalize="off"
                      autoComplete="off"
                      id="agent-base-url"
                      onChange={(event) => {
                        setDraft((current) => ({ ...current, baseUrl: event.target.value }));
                      }}
                      placeholder={t('(provider default)')}
                      spellCheck={false}
                      value={draft.baseUrl}
                    />
                  </div>
                </>
              ) : null}
              {supportsModelSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-model">{t('Model')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setDraft((current) => ({
                        ...current,
                        model: value === PROVIDER_DEFAULT_MODEL_VALUE ? '' : value,
                        modelServiceTier: '',
                      }));
                    }}
                    value={draft.model.trim() || PROVIDER_DEFAULT_MODEL_VALUE}
                  >
                    <SelectTrigger id="agent-model">
                      <SelectValue placeholder={t('Select model')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_MODEL_VALUE}>{t('Provider default')}</SelectItem>
                        {modelOptions.map((model) => (
                          <SelectItem key={model.id} value={model.id}>
                            {model.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {activeProviderModels?.defaultModel
                      ? t('Gateway default: {model}', { model: activeProviderModels.defaultModel })
                      : t('Optional. Leave empty to use the provider default.')}
                  </span>
                </div>
              ) : null}
              {supportsReasoningEffortSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-reasoning-effort">{t('Reasoning effort')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setDraft((current) => ({
                        ...current,
                        modelReasoningEffort: value === PROVIDER_DEFAULT_REASONING_VALUE ? '' : value,
                      }));
                    }}
                    value={draft.modelReasoningEffort.trim() || PROVIDER_DEFAULT_REASONING_VALUE}
                  >
                    <SelectTrigger id="agent-reasoning-effort">
                      <SelectValue placeholder={t('Select reasoning effort')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_REASONING_VALUE}>{t('Provider default')}</SelectItem>
                        {reasoningEffortOptions.map((effort) => (
                          <SelectItem key={effort.id} value={effort.id}>
                            {effort.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {t('Lower values are faster; higher values spend more reasoning.')}
                  </span>
                </div>
              ) : null}
              {supportsServiceTierSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-service-tier">{t('Service tier')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setDraft((current) => ({
                        ...current,
                        modelServiceTier: value === PROVIDER_DEFAULT_SERVICE_TIER_VALUE ? '' : value,
                      }));
                    }}
                    value={draft.modelServiceTier.trim() || PROVIDER_DEFAULT_SERVICE_TIER_VALUE}
                  >
                    <SelectTrigger id="agent-service-tier">
                      <SelectValue placeholder={t('Select service tier')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_SERVICE_TIER_VALUE}>{t('Provider default')}</SelectItem>
                        {serviceTierOptions.map((tier) => (
                          <SelectItem key={tier.id} value={tier.id}>
                            {tier.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {t('Fast service tiers trade higher usage for lower latency when the model supports them.')}
                  </span>
                </div>
              ) : null}
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-default-workspace">
                  {t('Default workspace directory')}
                </Label>
                <Input
                  id="agent-default-workspace"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, defaultWorkspaceDir: event.target.value }));
                  }}
                  placeholder={t('/path/to/project')}
                  value={draft.defaultWorkspaceDir}
                />
                <span className="codex-form-hint">
                  {t('Used when a new bot or task thread has no explicit workspace.')}
                </span>
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-system-prompt">{t('System Prompt')}</Label>
                <Textarea
                  className="min-h-[260px]"
                  id="agent-system-prompt"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, systemPrompt: event.target.value }));
                  }}
                  value={draft.systemPrompt}
                />
              </div>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, padding: '12px 16px' }}>
                <span className="codex-form-hint" style={{ color: '#ef4444' }}>{validationError}</span>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <button
                    className="codex-section-action"
                    onClick={() => {
                      setEditorMode('inspect');
                      setDraft(emptyDraft());
                      setDraftIdTouched(false);
                    }}
                    type="button"
                  >
                    {t('Cancel')}
                  </button>
                  <button
                    className="codex-section-action"
                    disabled={Boolean(validationError) || saving}
                    style={{ color: 'var(--color-token-text-primary)', fontWeight: 500 }}
                    type="submit"
                  >
                    {saving ? t('Saving...') : editorMode === 'create' ? t('Create Agent') : t('Save Agent')}
                  </button>
                </div>
              </div>
            </form>
          </div>
        </div>
      ) : (
        <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
          <div className="codex-section">
            <div className="codex-section-header">
              <span className="codex-section-title">{selectedAgent?.displayName || t('Agent')}</span>
              {selectedAgent && !selectedAgent.builtIn ? (
                <div className="codex-list-row-actions">
                  <button className="codex-section-action" onClick={() => openEditEditor(selectedAgent)} type="button">
                    {t('Edit')}
                  </button>
                  <button
                    className="codex-section-action"
                    onClick={() => { void handleDelete(selectedAgent); }}
                    style={{ color: '#ef4444' }}
                    type="button"
                  >
                    {t('Delete')}
                  </button>
                </div>
              ) : null}
            </div>
          </div>
          {selectedAgent ? (
            <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Agent ID')}</span>
                <span className="codex-command-row-desc">{selectedAgent.agentId}</span>
              </div>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Provider')}</span>
                <span className="codex-command-row-desc">{providerLabel(selectedAgent.providerType)}</span>
              </div>
              {selectedAgent.providerType === 'gemini_cli' || isNativeModelProvider(selectedAgent.providerType as ProviderType) || selectedAgent.model.trim() ? (
                <div className="codex-list-row">
                  <span className="codex-list-row-name">{t('Model')}</span>
                  <span className="codex-command-row-desc">{selectedAgent.model || t('(provider default)')}</span>
                </div>
              ) : null}
              {isNativeModelProvider(selectedAgent.providerType as ProviderType) ? (
                <div className="codex-list-row">
                  <span className="codex-list-row-name">{t('Auth')}</span>
                  <span className="codex-command-row-desc">
                    {selectedAgent.authSource || defaultAuthSource(selectedAgent.providerType as ProviderType)}
                  </span>
                </div>
              ) : null}
              {selectedAgent.modelReasoningEffort.trim() ? (
                <div className="codex-list-row">
                  <span className="codex-list-row-name">{t('Reasoning effort')}</span>
                  <span className="codex-command-row-desc">{selectedAgent.modelReasoningEffort}</span>
                </div>
              ) : null}
              {selectedAgent.modelServiceTier.trim() ? (
                <div className="codex-list-row">
                  <span className="codex-list-row-name">{t('Service tier')}</span>
                  <span className="codex-command-row-desc">{selectedAgent.modelServiceTier}</span>
                </div>
              ) : null}
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Default workspace directory')}</span>
                <span className="codex-command-row-desc">
                  {selectedAgent.defaultWorkspaceDir.trim() || t('(not set)')}
                </span>
              </div>
              <div style={{ padding: '12px 16px' }}>
                <div className="codex-list-row-name" style={{ marginBottom: 8 }}>{t('System Prompt')}</div>
                <div style={{ whiteSpace: 'pre-wrap', fontSize: 13, lineHeight: 1.6, color: 'var(--color-token-text-secondary)', fontFamily: 'var(--font-mono)' }}>
                  {selectedAgent.systemPrompt || t('(empty)')}
                </div>
              </div>
            </div>
          ) : (
            <div className="codex-empty-state">
              {t('Select an agent from the list to inspect its provider and prompt.')}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
