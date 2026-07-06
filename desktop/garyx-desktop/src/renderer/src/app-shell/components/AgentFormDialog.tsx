import { useRef } from 'react';

import type {
  CreateCustomAgentInput,
  DesktopCustomAgent,
  DesktopProviderModels,
  DesktopWorkspace,
  UpdateCustomAgentInput,
} from '@shared/contracts';

import { Database, Sparkles, Trash, Upload as UploadIcon } from 'lucide-react';
import {
  apiKeyValueFromRows,
  buildProviderEnvPayload,
  envRowsHaveInvalidKey,
  formatEnvText,
  isNativeModelProvider,
  parseEnvText,
  setApiKeyInRows,
} from './agent-env-editor';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
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
import { WorkspacePathPicker } from '../../components/WorkspacePathPicker';
import { useI18n } from '../../i18n';
import { AgentAvatarEditor } from './AgentAvatarEditor';
import {
  AGENT_AVATAR_ACCEPT,
  AGENT_AVATAR_DATA_URL_MAX_LENGTH,
  PROVIDER_DEFAULT_MODEL_VALUE,
  PROVIDER_DEFAULT_REASONING_VALUE,
  PROVIDER_DEFAULT_SERVICE_TIER_VALUE,
  defaultAuthSource,
  providerLabel,
  providerModelsWithCurrent,
  reasoningEffortsWithCurrent,
  serviceTiersWithCurrent,
} from './agents-hub-helpers';
import type { AgentDialogMode, AgentDraft, ProviderType } from './agents-hub-helpers';

type AgentFormDialogProps = {
  agentDialogMode: AgentDialogMode;
  agentDraft: AgentDraft;
  avatarGenerating: boolean;
  closeAgentDialog: () => void;
  ensureProviderModels: (providerType: ProviderType) => Promise<void>;
  envText: string;
  envViewMode: 'form' | 'text';
  handleAvatarFileChange: (
    event: React.ChangeEvent<HTMLInputElement>,
    target: 'agent' | 'team',
  ) => Promise<void>;
  loadData: () => Promise<void>;
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onOpenMemory?: (agent: DesktopCustomAgent) => void;
  onStartThread?: (agentOrTeamId: string) => void;
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
  openCreateTeamDialog: (seedAgentId?: string) => void;
  openEditAgentDialog: (agent: DesktopCustomAgent) => void;
  providerModelsByType: Partial<Record<ProviderType, DesktopProviderModels>>;
  providerModelsLoading: Partial<Record<ProviderType, boolean>>;
  saving: boolean;
  selectedAgent: DesktopCustomAgent | null;
  setAgentDraft: React.Dispatch<React.SetStateAction<AgentDraft>>;
  setAgentIdTouched: React.Dispatch<React.SetStateAction<boolean>>;
  setAvatarStyleDialogOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setAvatarStyleTarget: React.Dispatch<React.SetStateAction<'agent' | 'team'>>;
  setEnvText: React.Dispatch<React.SetStateAction<string>>;
  setEnvViewMode: React.Dispatch<React.SetStateAction<'form' | 'text'>>;
  setSaving: React.Dispatch<React.SetStateAction<boolean>>;
  workspaces: DesktopWorkspace[];
};

export function AgentFormDialog({
  agentDialogMode,
  agentDraft,
  avatarGenerating,
  closeAgentDialog,
  ensureProviderModels,
  envText,
  envViewMode,
  handleAvatarFileChange,
  loadData,
  onAddWorkspace,
  onOpenMemory,
  onStartThread,
  onToast,
  openCreateTeamDialog,
  openEditAgentDialog,
  providerModelsByType,
  providerModelsLoading,
  saving,
  selectedAgent,
  setAgentDraft,
  setAgentIdTouched,
  setAvatarStyleDialogOpen,
  setAvatarStyleTarget,
  setEnvText,
  setEnvViewMode,
  setSaving,
  workspaces,
}: AgentFormDialogProps) {
  const { t } = useI18n();
  const avatarFileInputRef = useRef<HTMLInputElement | null>(null);

  const activeAgentProviderModels = providerModelsByType[agentDraft.providerType];
  const agentProviderModelsLoading = providerModelsLoading[agentDraft.providerType] === true;
  const agentModelOptions = providerModelsWithCurrent(activeAgentProviderModels, agentDraft.model);
  const agentSupportsModelSelection =
    activeAgentProviderModels?.supportsModelSelection === true && agentModelOptions.length > 0;
  const agentReasoningEffortOptions = reasoningEffortsWithCurrent(
    activeAgentProviderModels,
    agentDraft.model || activeAgentProviderModels?.defaultModel || '',
    agentDraft.modelReasoningEffort,
  );
  const agentSupportsReasoningEffortSelection =
    activeAgentProviderModels?.supportsReasoningEffortSelection === true
    && agentReasoningEffortOptions.length > 0;
  const agentServiceTierOptions = serviceTiersWithCurrent(
    activeAgentProviderModels,
    agentDraft.model || activeAgentProviderModels?.defaultModel || '',
    agentDraft.modelServiceTier,
  );
  const agentSupportsServiceTierSelection =
    agentDraft.providerType === 'gpt'
    && (activeAgentProviderModels?.supportsServiceTierSelection === true || agentDraft.modelServiceTier.trim().length > 0)
    && agentServiceTierOptions.length > 0;
  const agentModelStatus =
    agentDraft.providerType === 'gemini_cli' && !agentSupportsModelSelection
      ? agentProviderModelsLoading
        ? t('Loading models from local Gemini ACP...')
        : activeAgentProviderModels?.error
          ? t('Local Gemini ACP does not expose a model list yet.')
          : null
      : null;
  const viewAgentProviderModels = selectedAgent
    ? providerModelsByType[selectedAgent.providerType]
    : undefined;
  const viewAgentModelId = selectedAgent?.model.trim() || '';
  const viewAgentModelLabel = viewAgentModelId
    ? viewAgentProviderModels?.models.find((option) => option.id === viewAgentModelId)?.label
      || viewAgentModelId
    : '';
  const viewAgentEffortId = selectedAgent?.modelReasoningEffort.trim() || '';
  const viewAgentEffortLabel = viewAgentEffortId
    ? reasoningEffortsWithCurrent(viewAgentProviderModels, viewAgentModelId, viewAgentEffortId)
      .find((option) => option.id === viewAgentEffortId)?.label || viewAgentEffortId
    : '';

  async function handleAgentSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const avatarDataUrl = agentDraft.avatarDataUrl.trim();
    if (avatarDataUrl.length > AGENT_AVATAR_DATA_URL_MAX_LENGTH) {
      onToast?.(t('Avatar image is too large.'), 'error');
      return;
    }
    setSaving(true);
    try {
      const nativeProvider = isNativeModelProvider(agentDraft.providerType);
      // The KV editor is the single source of env (the API-key field writes into
      // it). Send the whole map, which preserves keys the API-key shortcut used
      // to drop.
      const providerEnv = buildProviderEnvPayload(agentDraft.env);
      const payload: CreateCustomAgentInput = {
        agentId: agentDraft.agentId.trim(),
        displayName: agentDraft.displayName.trim(),
        providerType: agentDraft.providerType,
        model: agentSupportsModelSelection ? agentDraft.model.trim() : '',
        modelReasoningEffort: agentSupportsReasoningEffortSelection ? agentDraft.modelReasoningEffort.trim() : '',
        modelServiceTier: agentSupportsServiceTierSelection ? agentDraft.modelServiceTier.trim() : '',
        providerEnv,
        authSource: nativeProvider
          ? (agentDraft.authSource.trim() || defaultAuthSource(agentDraft.providerType))
          : null,
        baseUrl: nativeProvider ? agentDraft.baseUrl.trim() : null,
        defaultWorkspaceDir: agentDraft.defaultWorkspaceDir.trim(),
        avatarDataUrl,
        systemPrompt: agentDraft.systemPrompt.trim(),
      };

      if (agentDialogMode === 'create') {
        await window.garyxDesktop.createCustomAgent(payload);
        onToast?.(t('Custom agent created'), 'success');
      } else {
        const updatePayload: UpdateCustomAgentInput = {
          ...payload,
          currentAgentId: selectedAgent?.agentId || payload.agentId,
        };
        await window.garyxDesktop.updateCustomAgent(updatePayload);
        onToast?.(t('Custom agent updated'), 'success');
      }

      closeAgentDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  const agentValidationError =
    !agentDraft.displayName.trim()
      ? t('Name is required.')
      : !agentDraft.agentId.trim()
        ? t('Agent ID is required.')
        : envRowsHaveInvalidKey(agentDraft.env)
          ? t('Environment variable names must match [A-Za-z_][A-Za-z0-9_]*.')
          : null;

  return (
    <Dialog
      open={Boolean(agentDialogMode)}
      onOpenChange={(open) => {
        if (!open) {
          closeAgentDialog();
        }
      }}
    >
      <DialogContent aria-describedby={undefined} className="agents-hub-agent-dialog" size="form">
        <DialogHeader className="agents-hub-dialog-header">
          <DialogTitle className="agents-hub-dialog-title">
            {agentDialogMode === 'create'
              ? t('Create agent')
              : agentDialogMode === 'edit'
                ? t('Edit agent')
                : selectedAgent?.displayName || t('Agent')}
          </DialogTitle>
        </DialogHeader>

        {agentDialogMode === 'create' || agentDialogMode === 'edit' ? (
          <form className="agents-hub-dialog-form" onSubmit={handleAgentSubmit}>
            <div className="agents-hub-avatar-editor">
              <AgentAvatarEditor
                agentId={agentDialogMode === 'edit' ? agentDraft.agentId : undefined}
                avatarDataUrl={agentDraft.avatarDataUrl}
                builtIn={agentDialogMode === 'edit' ? selectedAgent?.builtIn : undefined}
                className="agents-hub-avatar-centered large"
                label={agentDraft.displayName || agentDraft.agentId || 'A'}
                providerType={agentDraft.providerType}
              />
              <div className="agents-hub-avatar-editor-actions">
                <input
                  accept={AGENT_AVATAR_ACCEPT}
                  className="sr-only"
                  onChange={(event) => {
                    void handleAvatarFileChange(event, 'agent');
                  }}
                  ref={avatarFileInputRef}
                  type="file"
                />
                <Button
                  onClick={() => avatarFileInputRef.current?.click()}
                  type="button"
                  variant="outline"
                >
                  <UploadIcon aria-hidden size={15} strokeWidth={1.8} />
                  {t('Upload avatar')}
                </Button>
                <Button
                  disabled={avatarGenerating}
                  onClick={() => {
                    setAvatarStyleTarget('agent');
                    setAvatarStyleDialogOpen(true);
                  }}
                  type="button"
                  variant="outline"
                >
                  <Sparkles aria-hidden size={15} strokeWidth={1.8} />
                  {avatarGenerating ? t('Generating...') : t('Generate avatar')}
                </Button>
                {agentDraft.avatarDataUrl ? (
                  <Button
                    onClick={() => {
                      setAgentDraft((current) => ({ ...current, avatarDataUrl: '' }));
                    }}
                    type="button"
                    variant="ghost"
                  >
                    {t('Clear')}
                  </Button>
                ) : null}
              </div>
            </div>

            <div className="agents-hub-form-grid">
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-dialog-name">{t('Name')}</Label>
                <Input
                  id="agent-dialog-name"
                  onChange={(event) => {
                    setAgentDraft((current) => ({ ...current, displayName: event.target.value }));
                  }}
                  value={agentDraft.displayName}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-dialog-id">{t('Agent ID')}</Label>
                <Input
                  disabled={agentDialogMode === 'edit'}
                  id="agent-dialog-id"
                  onChange={(event) => {
                    setAgentIdTouched(true);
                    setAgentDraft((current) => ({ ...current, agentId: event.target.value }));
                  }}
                  value={agentDraft.agentId}
                />
              </div>
            </div>

            <div className="agents-hub-model-row">
              <div className="codex-form-field">
                <Label className="codex-form-label">{t('Provider')}</Label>
                <Select
                  onValueChange={(value: ProviderType) => {
                    setAgentDraft((current) => ({
                      ...current,
                      providerType: value,
                      model: '',
                      modelReasoningEffort: value === 'codex_app_server' || value === 'traex' || isNativeModelProvider(value)
                        ? current.modelReasoningEffort
                        : '',
                      modelServiceTier: value === 'gpt' ? current.modelServiceTier : '',
                      authSource: isNativeModelProvider(value) ? defaultAuthSource(value) : '',
                      baseUrl: '',
                    }));
                    void ensureProviderModels(value);
                  }}
                  value={agentDraft.providerType}
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
                {agentModelStatus ? <span className="codex-form-hint">{agentModelStatus}</span> : null}
              </div>

              {agentSupportsModelSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-model">{t('Model')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setAgentDraft((current) => ({
                        ...current,
                        model: value === PROVIDER_DEFAULT_MODEL_VALUE ? '' : value,
                        modelServiceTier: '',
                      }));
                    }}
                    value={agentDraft.model.trim() || PROVIDER_DEFAULT_MODEL_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-model">
                      <SelectValue placeholder={t('Select model')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_MODEL_VALUE}>{t('Provider default')}</SelectItem>
                        {agentModelOptions.map((model) => (
                          <SelectItem key={model.id} value={model.id}>
                            {model.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
              ) : null}

              {agentSupportsReasoningEffortSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-reasoning-effort">{t('Reasoning effort')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setAgentDraft((current) => ({
                        ...current,
                        modelReasoningEffort: value === PROVIDER_DEFAULT_REASONING_VALUE ? '' : value,
                      }));
                    }}
                    value={agentDraft.modelReasoningEffort.trim() || PROVIDER_DEFAULT_REASONING_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-reasoning-effort">
                      <SelectValue placeholder={t('Select reasoning effort')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_REASONING_VALUE}>{t('Provider default')}</SelectItem>
                        {agentReasoningEffortOptions.map((effort) => (
                          <SelectItem key={effort.id} value={effort.id}>
                            {effort.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
              ) : null}

              {agentSupportsServiceTierSelection ? (
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-service-tier">{t('Service tier')}</Label>
                  <Select
                    onValueChange={(value) => {
                      setAgentDraft((current) => ({
                        ...current,
                        modelServiceTier: value === PROVIDER_DEFAULT_SERVICE_TIER_VALUE ? '' : value,
                      }));
                    }}
                    value={agentDraft.modelServiceTier.trim() || PROVIDER_DEFAULT_SERVICE_TIER_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-service-tier">
                      <SelectValue placeholder={t('Select service tier')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={PROVIDER_DEFAULT_SERVICE_TIER_VALUE}>{t('Provider default')}</SelectItem>
                        {agentServiceTierOptions.map((tier) => (
                          <SelectItem key={tier.id} value={tier.id}>
                            {tier.label}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
              ) : null}
            </div>

            {isNativeModelProvider(agentDraft.providerType) ? (
              <>
                {agentDraft.providerType === 'gpt' ? (
                  <div className="codex-form-field">
                    <Label className="codex-form-label">{t('GPT auth')}</Label>
                    <Select
                      onValueChange={(value) => {
                        setAgentDraft((current) => ({
                          ...current,
                          authSource: value,
                          env:
                            value === 'codex'
                              ? setApiKeyInRows(current.env, current.providerType, '')
                              : current.env,
                        }));
                      }}
                      value={agentDraft.authSource || 'codex'}
                    >
                      <SelectTrigger className="agents-hub-model-select">
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

                {agentDraft.providerType !== 'gpt' || agentDraft.authSource === 'api_key' ? (
                  <div className="codex-form-field">
                    <Label className="codex-form-label" htmlFor="agent-dialog-api-key">
                      {t('API Key')}
                    </Label>
                    <Input
                      autoCapitalize="off"
                      autoComplete="off"
                      id="agent-dialog-api-key"
                      onChange={(event) => {
                        const nextApiKey = event.target.value;
                        setAgentDraft((current) => ({
                          ...current,
                          env: setApiKeyInRows(current.env, current.providerType, nextApiKey),
                        }));
                      }}
                      placeholder={
                        agentDraft.providerType === 'anthropic' || agentDraft.providerType === 'claude_llm'
                          ? 'ANTHROPIC_API_KEY'
                          : agentDraft.providerType === 'google' || agentDraft.providerType === 'gemini_llm'
                            ? 'GEMINI_API_KEY'
                            : 'OPENAI_API_KEY'
                      }
                      spellCheck={false}
                      type="text"
                      value={apiKeyValueFromRows(agentDraft.env, agentDraft.providerType)}
                    />
                    <span className="codex-form-hint">
                      {t('Stored on this custom provider config.')}
                    </span>
                  </div>
                ) : null}

                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="agent-dialog-base-url">
                    {t('Base URL')}
                  </Label>
                  <Input
                    autoCapitalize="off"
                    autoComplete="off"
                    id="agent-dialog-base-url"
                    onChange={(event) => {
                      setAgentDraft((current) => ({ ...current, baseUrl: event.target.value }));
                    }}
                    placeholder={t('(provider default)')}
                    spellCheck={false}
                    value={agentDraft.baseUrl}
                  />
                </div>
              </>
            ) : null}

            <div className="codex-form-field">
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: '8px',
                }}
              >
                <Label className="codex-form-label">{t('Environment Variables')}</Label>
                <div style={{ display: 'flex', gap: '4px' }}>
                  <Button
                    onClick={() => {
                      setEnvViewMode('form');
                    }}
                    size="sm"
                    type="button"
                    variant={envViewMode === 'form' ? 'secondary' : 'ghost'}
                  >
                    {t('Form')}
                  </Button>
                  <Button
                    onClick={() => {
                      setEnvText(formatEnvText(agentDraft.env));
                      setEnvViewMode('text');
                    }}
                    size="sm"
                    type="button"
                    variant={envViewMode === 'text' ? 'secondary' : 'ghost'}
                  >
                    {t('Text')}
                  </Button>
                </div>
              </div>
              {envViewMode === 'text' ? (
                <>
                  <Textarea
                    autoCapitalize="off"
                    autoComplete="off"
                    className="mono"
                    onChange={(event) => {
                      const nextText = event.target.value;
                      setEnvText(nextText);
                      const nextRows = parseEnvText(nextText);
                      setAgentDraft((current) => ({ ...current, env: nextRows }));
                    }}
                    placeholder={'KEY=value'}
                    rows={Math.min(12, Math.max(4, agentDraft.env.length + 1))}
                    spellCheck={false}
                    value={envText}
                  />
                  <span className="codex-form-hint">
                    {t('One KEY=value per line. Values are passed verbatim—no quoting is added, numbers stay plain. Lines starting with # are ignored.')}
                  </span>
                </>
              ) : (
                <>
                  {agentDraft.env.map((row, index) => (
                    <div
                      key={index}
                      style={{ display: 'flex', gap: '8px', alignItems: 'center', marginBottom: '8px' }}
                    >
                      <Input
                        autoCapitalize="off"
                        autoComplete="off"
                        onChange={(event) => {
                          const nextKey = event.target.value;
                          setAgentDraft((current) => ({
                            ...current,
                            env: current.env.map((entry, entryIndex) =>
                              entryIndex === index ? { ...entry, key: nextKey } : entry,
                            ),
                          }));
                        }}
                        placeholder={t('KEY')}
                        spellCheck={false}
                        style={{ flex: '0 0 40%' }}
                        value={row.key}
                      />
                      <Input
                        autoCapitalize="off"
                        autoComplete="off"
                        onChange={(event) => {
                          const nextValue = event.target.value;
                          setAgentDraft((current) => ({
                            ...current,
                            env: current.env.map((entry, entryIndex) =>
                              entryIndex === index ? { ...entry, value: nextValue } : entry,
                            ),
                          }));
                        }}
                        placeholder={t('value')}
                        spellCheck={false}
                        style={{ flex: 1 }}
                        type="text"
                        value={row.value}
                      />
                      <Button
                        aria-label={t('Remove variable')}
                        onClick={() => {
                          setAgentDraft((current) => ({
                            ...current,
                            env: current.env.filter((_, entryIndex) => entryIndex !== index),
                          }));
                        }}
                        size="icon"
                        type="button"
                        variant="ghost"
                      >
                        <Trash size={14} />
                      </Button>
                    </div>
                  ))}
                  <Button
                    onClick={() => {
                      setAgentDraft((current) => ({
                        ...current,
                        env: [...current.env, { key: '', value: '' }],
                      }));
                    }}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    {t('Add variable')}
                  </Button>
                </>
              )}
              <span className="codex-form-hint">
                {t('Environment variables are passed to this agent’s provider runs. They may appear in command output or logs—avoid secrets you can’t rotate.')}
              </span>
            </div>

            <div className="codex-form-field">
              <Label className="codex-form-label" htmlFor="agent-dialog-default-workspace">
                {t('Default workspace directory')}
              </Label>
              <WorkspacePathPicker
                id="agent-dialog-default-workspace"
                onChange={(value) => {
                  setAgentDraft((current) => ({ ...current, defaultWorkspaceDir: value }));
                }}
                onAddWorkspace={onAddWorkspace}
                placeholder={t('/path/to/project')}
                triggerClassName="agents-hub-workspace-trigger"
                value={agentDraft.defaultWorkspaceDir}
                workspaces={workspaces}
              />
            </div>

            <div className="codex-form-field">
              <Label className="codex-form-label" htmlFor="agent-dialog-prompt">{t('System Prompt')}</Label>
              <Textarea
                className="agents-hub-system-prompt"
                id="agent-dialog-prompt"
                onChange={(event) => {
                  setAgentDraft((current) => ({ ...current, systemPrompt: event.target.value }));
                }}
                value={agentDraft.systemPrompt}
              />
            </div>

            <DialogFooter className="agents-hub-dialog-footer">
              <div className="agents-hub-dialog-status">{agentValidationError}</div>
              <div className="agents-hub-dialog-actions">
                <Button
                  disabled={saving}
                  onClick={closeAgentDialog}
                  type="button"
                  variant="outline"
                >
                  {t('Cancel')}
                </Button>
                <Button disabled={Boolean(agentValidationError) || saving} type="submit">
                  {saving ? t('Saving...') : agentDialogMode === 'create' ? t('Create Agent') : t('Save Agent')}
                </Button>
              </div>
            </DialogFooter>
          </form>
        ) : (
          <div className="agents-hub-dialog-stack">
            <div className="agents-hub-detail-header">
              <AgentAvatarEditor
                agentId={selectedAgent?.agentId}
                avatarDataUrl={selectedAgent?.avatarDataUrl}
                builtIn={selectedAgent?.builtIn}
                className="agents-hub-avatar-centered large"
                label={selectedAgent?.displayName || selectedAgent?.agentId || 'A'}
                providerIcon={selectedAgent?.providerIcon}
                providerType={selectedAgent?.providerType}
              />
              <div className="agents-hub-detail-copy">
                <div className="agents-hub-card-badges">
                  <Badge variant="outline">{selectedAgent?.builtIn ? t('Built-in') : t('Custom')}</Badge>
                  {selectedAgent ? <Badge variant="outline">{providerLabel(selectedAgent.providerType)}</Badge> : null}
                </div>
                <p className="agents-hub-detail-id">{selectedAgent?.agentId || ''}</p>
              </div>
            </div>

            <div className="agents-hub-detail-scroll">
              <div className="agents-hub-detail-grid agents-hub-detail-facts">
                <div className="agents-hub-detail-item">
                  <div className="agents-hub-detail-term">{t('Model')}</div>
                  <div className="agents-hub-detail-value" title={viewAgentModelId || undefined}>
                    {viewAgentModelLabel || t('(provider default)')}
                  </div>
                </div>
                <div className="agents-hub-detail-item">
                  <div className="agents-hub-detail-term">{t('Thinking level')}</div>
                  <div className="agents-hub-detail-value">
                    {viewAgentEffortLabel || t('(provider default)')}
                  </div>
                </div>
                {selectedAgent && isNativeModelProvider(selectedAgent.providerType as ProviderType) ? (
                  <div className="agents-hub-detail-item">
                    <div className="agents-hub-detail-term">{t('Auth')}</div>
                    <div className="agents-hub-detail-value">
                      {selectedAgent.authSource || defaultAuthSource(selectedAgent.providerType as ProviderType)}
                    </div>
                  </div>
                ) : null}
                {selectedAgent?.modelServiceTier.trim() ? (
                  <div className="agents-hub-detail-item">
                    <div className="agents-hub-detail-term">{t('Service tier')}</div>
                    <div className="agents-hub-detail-value">{selectedAgent.modelServiceTier}</div>
                  </div>
                ) : null}
                <div className="agents-hub-detail-item agents-hub-detail-item-full">
                  <div className="agents-hub-detail-term">{t('Default workspace directory')}</div>
                  <div
                    className="agents-hub-detail-value mono"
                    title={selectedAgent?.defaultWorkspaceDir.trim() || undefined}
                  >
                    {selectedAgent?.defaultWorkspaceDir.trim() || t('(not set)')}
                  </div>
                </div>
              </div>

              {selectedAgent && Object.keys(selectedAgent.providerEnv || {}).length > 0 ? (
                <div className="agents-hub-detail-block">
                  <div className="agents-hub-detail-label">{t('Environment Variables')}</div>
                  <div className="agents-hub-detail-body mono">
                    {Object.keys(selectedAgent.providerEnv)
                      .sort()
                      .map((key) => (
                        <div key={key} style={{ wordBreak: 'break-all' }}>
                          {key}={selectedAgent.providerEnv[key]}
                        </div>
                      ))}
                  </div>
                </div>
              ) : null}

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('System Prompt')}</div>
                <div className="agents-hub-detail-body mono agents-hub-detail-prompt">
                  {selectedAgent?.systemPrompt || t('Provider default')}
                </div>
              </div>
            </div>

            <DialogFooter className="agents-hub-dialog-actions">
              {selectedAgent ? (
                <Button
                  onClick={() => {
                    closeAgentDialog();
                    onStartThread?.(selectedAgent.agentId);
                  }}
                  type="button"
                  variant="outline"
                >
                  {t('Chat')}
                </Button>
              ) : null}
              {selectedAgent ? (
                <Button
                  onClick={() => {
                    closeAgentDialog();
                    openCreateTeamDialog(selectedAgent.agentId);
                  }}
                  type="button"
                  variant="outline"
                >
                  {t('Create Team')}
                </Button>
              ) : null}
              {selectedAgent && !selectedAgent.builtIn ? (
                <Button
                  onClick={() => {
                    closeAgentDialog();
                    onOpenMemory?.(selectedAgent);
                  }}
                  type="button"
                  variant="outline"
                >
                  <Database aria-hidden size={15} strokeWidth={1.8} />
                  {t('Memory')}
                </Button>
              ) : null}
              {selectedAgent && !selectedAgent.builtIn ? (
                <Button
                  onClick={() => {
                    openEditAgentDialog(selectedAgent);
                  }}
                  type="button"
                >
                  {t('Edit Agent')}
                </Button>
              ) : null}
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
