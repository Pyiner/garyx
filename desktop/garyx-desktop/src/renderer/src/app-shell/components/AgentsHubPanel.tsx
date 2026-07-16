import { useEffect, useMemo, useRef, useState } from 'react';

import type {
  DesktopAgentCatalog,
  DesktopCustomAgent,
  DesktopProviderModels,
  DesktopWorkspace,
} from '@shared/contracts';

import { Plus, Search as SearchIcon, Trash } from 'lucide-react';
import { envRowsFromEnvMap } from './agent-env-editor';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';
import { MoreDotsIcon } from '../icons';
import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../../components/ui/table';
import { Input } from '../../components/ui/input';
import { Switch } from '../../components/ui/switch';
import { useI18n } from '../../i18n';
import { AgentAvatarEditor, AvatarStyleDialog } from './AgentAvatarEditor';
import {
  acceptAvatarCandidate,
  avatarGenerationFailure,
  beginAvatarGeneration,
  cancelAvatarGeneration,
  changeAvatarStyle,
  createAvatarGenerationFlow,
  ownsAvatarGenerationOperation,
  resolveAvatarGeneration,
} from './agent-avatar-flow';
import type {
  AvatarGenerationFlow,
  AvatarGenerationOperation,
} from './agent-avatar-flow';
import {
  customAgentDeleteConfirmationFor,
  runCustomAgentDeleteConfirmation,
  type CustomAgentDeleteConfirmation,
} from './agents-hub-delete-model';
import { AgentFormDialog } from './AgentFormDialog';
import {
  AGENT_AVATAR_MAX_BYTES,
  DEFAULT_AVATAR_STYLE_ID,
  deriveId,
  emptyAgentDraft,
  normalizeAvatarFile,
  providerLabel,
  sortedAgents,
  stopEvent,
} from './agents-hub-helpers';
import {
  agentManagementActionState,
  defaultBadgeForAgent,
} from '../agent-availability-model';
import type {
  AgentDialogMode,
  AgentDraft,
  AvatarStyleId,
  ProviderType,
} from './agents-hub-helpers';

const EMPTY_AGENT_CATALOG: DesktopAgentCatalog = {
  agents: [],
  defaultAgentId: null,
  effectiveDefaultAgentId: null,
};

type AgentsHubPanelProps = {
  gatewayScope?: string;
  workspaces?: DesktopWorkspace[];
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onRefreshAgentTargets?: () => Promise<void>;
  onStartThread?: (agentId: string) => void;
  onOpenMemory?: (agent: DesktopCustomAgent) => void;
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

export function AgentsHubPanel({
  gatewayScope = '',
  workspaces = [],
  onAddWorkspace,
  onRefreshAgentTargets,
  onStartThread,
  onOpenMemory,
  onToast,
}: AgentsHubPanelProps) {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [catalog, setCatalog] = useState<DesktopAgentCatalog>(EMPTY_AGENT_CATALOG);
  const agents = catalog.agents;
  const [availabilityMutationAgentId, setAvailabilityMutationAgentId] =
    useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [search, setSearch] = useState('');

  const [agentDialogMode, setAgentDialogMode] = useState<AgentDialogMode>(null);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [agentDeleteConfirmation, setAgentDeleteConfirmation] =
    useState<CustomAgentDeleteConfirmation | null>(null);
  const [agentDraft, setAgentDraft] = useState<AgentDraft>(() => emptyAgentDraft());
  const [envViewMode, setEnvViewMode] = useState<'form' | 'text'>('form');
  const [envText, setEnvText] = useState('');
  const [agentIdTouched, setAgentIdTouched] = useState(false);
  const [avatarStyleDialogOpen, setAvatarStyleDialogOpen] = useState(false);
  const [avatarStyleId, setAvatarStyleId] = useState<AvatarStyleId>(DEFAULT_AVATAR_STYLE_ID);
  const [customAvatarStyle, setCustomAvatarStyle] = useState('');
  const [avatarFlow, setAvatarFlow] = useState<AvatarGenerationFlow>(() => (
    createAvatarGenerationFlow('')
  ));
  const avatarFlowRef = useRef(avatarFlow);
  const avatarGenerationEpochRef = useRef(0);
  const previousGatewayScopeRef = useRef(gatewayScope);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<ProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<ProviderType, boolean>>
  >({});

  async function loadData(options: { silent?: boolean } = {}) {
    // A silent refresh (e.g. when the window regains focus) must not flash the
    // loading state, blank the lists, or toast on a transient failure. It only
    // swaps in fresh data for the fetches that actually succeed and otherwise
    // leaves the currently displayed data untouched.
    const silent = options.silent ?? false;
    if (!silent) {
      setLoading(true);
      setLoadError(null);
    }
    try {
      const nextCatalog = await window.garyxDesktop.listCustomAgents();
      setCatalog({ ...nextCatalog, agents: sortedAgents(nextCatalog.agents) });
    } catch (error) {
      if (!silent) {
        setCatalog(EMPTY_AGENT_CATALOG);
        const message = error instanceof Error ? error.message : 'Failed to load agents.';
        setLoadError(message);
        onToast?.(message, 'error');
      }
    } finally {
      if (!silent) {
        setLoading(false);
      }
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
    void loadData();
  }, []);

  useEffect(() => () => {
    avatarGenerationEpochRef.current += 1;
    const requestId = avatarFlowRef.current.requestId;
    if (requestId) {
      void window.garyxDesktop
        .cancelCustomAgentAvatarGeneration({ requestId })
        .catch(() => {});
    }
  }, []);

  useEffect(() => {
    if (previousGatewayScopeRef.current === gatewayScope) {
      return;
    }
    previousGatewayScopeRef.current = gatewayScope;
    if (avatarStyleDialogOpen) {
      closeAvatarStyleDialog();
    }
  }, [gatewayScope, avatarStyleDialogOpen]);

  // Re-fetch when the user returns to the app, so changes made on another
  // surface (e.g. editing an agent's model on mobile) show up without a manual
  // reload. `focus` covers switching apps while the window stays visible;
  // `visibilitychange` covers minimize/occlusion. The refresh is silent so it
  // never flickers the panel.
  useEffect(() => {
    let refreshing = false;
    const refreshOnReturn = () => {
      if (document.hidden || refreshing) {
        return;
      }
      refreshing = true;
      void loadData({ silent: true }).finally(() => {
        refreshing = false;
      });
    };
    window.addEventListener('focus', refreshOnReturn);
    document.addEventListener('visibilitychange', refreshOnReturn);
    return () => {
      window.removeEventListener('focus', refreshOnReturn);
      document.removeEventListener('visibilitychange', refreshOnReturn);
    };
  }, []);

  useEffect(() => {
    if (agentDialogMode === 'create' || agentDialogMode === 'edit') {
      void ensureProviderModels(agentDraft.providerType);
    }
  }, [agentDialogMode, agentDraft.providerType]);

  useEffect(() => {
    // The view dialog resolves model/effort ids to catalog labels.
    if (agentDialogMode === 'view' && selectedAgentId) {
      const agent = agents.find((entry) => entry.agentId === selectedAgentId);
      if (agent) {
        void ensureProviderModels(agent.providerType as ProviderType);
      }
    }
  }, [agentDialogMode, selectedAgentId, agents]);

  useEffect(() => {
    if (agentDialogMode !== 'create' || agentIdTouched) {
      return;
    }
    const nextId = deriveId(agentDraft.displayName);
    setAgentDraft((current) => (current.agentId === nextId ? current : { ...current, agentId: nextId }));
  }, [agentDialogMode, agentDraft.displayName, agentIdTouched]);

  const selectedAgent = useMemo(
    () => agents.find((agent) => agent.agentId === selectedAgentId) || null,
    [agents, selectedAgentId],
  );
  const filteredAgents = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return agents;
    }
    return agents.filter((agent) => {
      return [
        agent.displayName,
        agent.agentId,
        providerLabel(agent.providerType),
        agent.systemPrompt,
        agent.builtIn ? 'built-in' : 'custom',
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [agents, search]);

  function closeAgentDialog() {
    closeAvatarStyleDialog();
    setAgentDialogMode(null);
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setAgentIdTouched(false);
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openCreateAgentDialog() {
    setAgentDialogMode('create');
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setEnvViewMode('form');
    setEnvText('');
    setAgentIdTouched(false);
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openViewAgentDialog(agent: DesktopCustomAgent) {
    setAgentDialogMode('view');
    setSelectedAgentId(agent.agentId);
    setAgentDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      modelReasoningEffort: agent.modelReasoningEffort,
      modelServiceTier: agent.modelServiceTier,
      defaultWorkspaceDir: agent.defaultWorkspaceDir,
      avatarDataUrl: agent.avatarDataUrl,
      env: envRowsFromEnvMap(agent.providerEnv),
      systemPrompt: agent.systemPrompt,
    });
    setEnvViewMode('form');
    setEnvText('');
    setAgentIdTouched(true);
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function openEditAgentDialog(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      openViewAgentDialog(agent);
      return;
    }
    setAgentDialogMode('edit');
    setSelectedAgentId(agent.agentId);
    setAgentDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      modelReasoningEffort: agent.modelReasoningEffort,
      modelServiceTier: agent.modelServiceTier,
      defaultWorkspaceDir: agent.defaultWorkspaceDir,
      avatarDataUrl: agent.avatarDataUrl,
      env: envRowsFromEnvMap(agent.providerEnv),
      systemPrompt: agent.systemPrompt,
    });
    setEnvViewMode('form');
    setEnvText('');
    setAgentIdTouched(true);
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  async function handleAvatarFileChange(
    event: React.ChangeEvent<HTMLInputElement>,
  ) {
    const file = event.target.files?.[0] || null;
    event.target.value = '';
    if (!file) {
      return;
    }
    if (file.size > AGENT_AVATAR_MAX_BYTES) {
      onToast?.(t('Avatar image is too large.'), 'error');
      return;
    }
    if (!file.type.startsWith('image/')) {
      onToast?.(t('Choose an image file.'), 'error');
      return;
    }
    try {
      const avatarDataUrl = await normalizeAvatarFile(file);
      setAgentDraft((current) => ({ ...current, avatarDataUrl }));
    } catch (error) {
      const message = error instanceof Error && error.message === 'Avatar image is too large.'
        ? error.message
        : 'Failed to read avatar image';
      onToast?.(t(message), 'error');
    }
  }

  function commitAvatarFlow(
    next: AvatarGenerationFlow | ((current: AvatarGenerationFlow) => AvatarGenerationFlow),
  ) {
    const value = typeof next === 'function' ? next(avatarFlowRef.current) : next;
    avatarFlowRef.current = value;
    setAvatarFlow(value);
  }

  function openAvatarStyleDialog() {
    commitAvatarFlow(createAvatarGenerationFlow(agentDraft.avatarDataUrl));
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
    setAvatarStyleDialogOpen(true);
  }

  function closeAvatarStyleDialog() {
    const shouldRestoreFocus = avatarStyleDialogOpen;
    avatarGenerationEpochRef.current += 1;
    const requestId = avatarFlowRef.current.requestId;
    if (requestId) {
      void window.garyxDesktop
        .cancelCustomAgentAvatarGeneration({ requestId })
        .catch(() => {});
      commitAvatarFlow((current) => cancelAvatarGeneration(current, requestId));
    }
    setAvatarStyleDialogOpen(false);
    if (shouldRestoreFocus) {
      window.requestAnimationFrame(() => {
        document.getElementById('agent-avatar-generate-trigger')?.focus();
      });
    }
  }

  async function handleGenerateAvatar(stylePrompt: string) {
    if (avatarFlowRef.current.phase === 'generating') {
      return;
    }
    const displayName = agentDraft.displayName.trim();
    const agentId = agentDraft.agentId.trim();
    if (!displayName) {
      onToast?.(t('Name is required.'), 'error');
      return;
    }
    avatarGenerationEpochRef.current += 1;
    const operation: AvatarGenerationOperation = {
      epoch: avatarGenerationEpochRef.current,
      requestId: crypto.randomUUID(),
    };
    commitAvatarFlow((current) => beginAvatarGeneration(current, operation.requestId));
    try {
      const result = await window.garyxDesktop.generateCustomAgentAvatar({
        requestId: operation.requestId,
        agentId,
        displayName,
        stylePrompt,
      });
      if (!ownsAvatarGenerationOperation(
        avatarGenerationEpochRef.current,
        avatarFlowRef.current.requestId,
        operation,
      )) {
        return;
      }
      if (result.status === 'success') {
        commitAvatarFlow((current) => resolveAvatarGeneration(
          current,
          operation.requestId,
          { status: 'success', avatarDataUrl: result.avatarDataUrl },
        ));
      } else if (result.status === 'failure') {
        commitAvatarFlow((current) => resolveAvatarGeneration(
          current,
          operation.requestId,
          {
            status: 'failure',
            failure: avatarGenerationFailure(result.category, result.message),
          },
        ));
      } else {
        commitAvatarFlow((current) => resolveAvatarGeneration(
          current,
          operation.requestId,
          { status: 'cancelled' },
        ));
      }
    } catch {
      if (ownsAvatarGenerationOperation(
        avatarGenerationEpochRef.current,
        avatarFlowRef.current.requestId,
        operation,
      )) {
        commitAvatarFlow((current) => resolveAvatarGeneration(
          current,
          operation.requestId,
          { status: 'failure', failure: avatarGenerationFailure('unknown') },
        ));
      }
    }
  }

  function handleUseGeneratedAvatar() {
    const accepted = acceptAvatarCandidate(avatarFlowRef.current);
    if (!accepted.avatarDataUrl) {
      return;
    }
    commitAvatarFlow(accepted.flow);
    setAgentDraft((current) => ({
      ...current,
      avatarDataUrl: accepted.avatarDataUrl || current.avatarDataUrl,
    }));
    closeAvatarStyleDialog();
    onToast?.(t('New avatar selected'), 'success');
  }

  function openAgentDeleteConfirmation(agent: DesktopCustomAgent) {
    const confirmation = customAgentDeleteConfirmationFor(agent);
    if (!confirmation) {
      return;
    }
    closeAgentDialog();
    setAgentDeleteConfirmation(confirmation);
  }

  async function handleConfirmDeleteAgent() {
    if (!agentDeleteConfirmation) {
      return;
    }
    setSaving(true);
    try {
      await runCustomAgentDeleteConfirmation({
        confirmation: agentDeleteConfirmation,
        deleteCustomAgent: (input) => window.garyxDesktop.deleteCustomAgent(input),
        closeConfirmation: () => setAgentDeleteConfirmation(null),
        closeAgentDialog,
        loadData,
        refreshAgentTargets: onRefreshAgentTargets,
      });
      onToast?.(t('Custom agent deleted'), 'success');
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleToggleAgent(agent: DesktopCustomAgent, enabled: boolean) {
    setAvailabilityMutationAgentId(agent.agentId);
    try {
      await window.garyxDesktop.toggleCustomAgent({
        agentId: agent.agentId,
        enabled,
      });
      await loadData({ silent: true });
      await onRefreshAgentTargets?.();
      onToast?.(enabled ? t('Agent enabled') : t('Agent disabled'), 'success');
    } catch (error) {
      onToast?.(
        error instanceof Error ? error.message : t('Failed to update agent'),
        'error',
      );
    } finally {
      setAvailabilityMutationAgentId(null);
    }
  }

  async function handleSetDefaultAgent(agent: DesktopCustomAgent) {
    setAvailabilityMutationAgentId(agent.agentId);
    try {
      await window.garyxDesktop.setDefaultCustomAgent({ agentId: agent.agentId });
      await loadData({ silent: true });
      await onRefreshAgentTargets?.();
      onToast?.(t('Default agent updated'), 'success');
    } catch (error) {
      onToast?.(
        error instanceof Error ? error.message : t('Failed to update default agent'),
        'error',
      );
    } finally {
      setAvailabilityMutationAgentId(null);
    }
  }

  const visibleAgents = filteredAgents;

  return (
    <div className="agents-hub">
      <div className="mgmt-page-header agents-hub-page-header">
        <div className="mgmt-page-title-block">
          <h1 className="mgmt-page-title">{t('Agents')}</h1>
          <p className="mgmt-page-subtitle">
            {t('{count} total', { count: agents.length })}
          </p>
        </div>
        <div className="mgmt-page-actions">
          <button
            className="mgmt-primary-button"
            onClick={openCreateAgentDialog}
            type="button"
          >
            <Plus aria-hidden size={15} strokeWidth={2} />
            {t('New Agent')}
          </button>
        </div>
      </div>
      <div className="agents-hub-hero">
        <div className="agents-hub-controls">
          <div className="agents-hub-search">
            <SearchIcon aria-hidden size={16} strokeWidth={1.8} />
            <Input
              className="agents-hub-search-input"
              onChange={(event) => {
                setSearch(event.target.value);
              }}
              placeholder={t("Search...")}
              value={search}
            />
          </div>

        </div>
      </div>

      {loadError ? (
        <div className="codex-inline-hint" style={{ color: 'var(--color-token-error-foreground)' }}>{loadError}</div>
      ) : null}

      {loading ? (
        <div className="agents-hub-empty-state">{t('Loading...')}</div>
      ) : (
        <Table className="agents-hub-table">
          <TableHeader>
            <TableRow>
              <TableHead style={{ width: '34%' }}>{t('Name')}</TableHead>
              <TableHead style={{ width: '16%' }}>{t('Provider')}</TableHead>
              <TableHead style={{ width: '14%' }}>{t('Type')}</TableHead>
              <TableHead style={{ width: '12%' }}>{t('Enabled')}</TableHead>
              <TableHead style={{ width: '24%' }} className="text-right">
                {t('Actions')}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visibleAgents.length ? (
              visibleAgents.map((agent) => {
                const defaultBadge = defaultBadgeForAgent(catalog, agent);
                const actionState = agentManagementActionState(catalog, agent);
                const mutatingAvailability = availabilityMutationAgentId === agent.agentId;
                return (
                  <TableRow
                    className="cursor-pointer"
                    key={agent.agentId}
                    onClick={() => openViewAgentDialog(agent)}
                  >
                    <TableCell>
                      <div className="agents-hub-name-cell">
                        <AgentAvatarEditor
                          agentId={agent.agentId}
                          avatarDataUrl={agent.avatarDataUrl}
                          builtIn={agent.builtIn}
                          className="agents-hub-avatar-sm"
                          label={agent.displayName || agent.agentId}
                          providerIcon={agent.providerIcon}
                          providerType={agent.providerType}
                        />
                        <div>
                          <div className="agents-hub-cell-name">
                            {agent.displayName}
                            {defaultBadge ? (
                              <Badge className="agents-hub-default-badge" variant="outline">
                                {defaultBadge === 'default'
                                  ? t('Default')
                                  : defaultBadge === 'default-inactive'
                                    ? t('Default (inactive)')
                                    : defaultBadge === 'acting-default'
                                      ? t('Acting default')
                                      : t('Default (auto)')}
                              </Badge>
                            ) : null}
                          </div>
                          <div className="agents-hub-cell-id">{agent.agentId}</div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell>{providerLabel(agent.providerType)}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{agent.builtIn ? t('Built-in') : t('Custom')}</Badge>
                    </TableCell>
                    <TableCell>
                      <Switch
                        aria-label={t('{name} enabled', { name: agent.displayName || agent.agentId })}
                        checked={agent.enabled}
                        disabled={mutatingAvailability}
                        onCheckedChange={(enabled) => {
                          void handleToggleAgent(agent, enabled);
                        }}
                        onClick={stopEvent}
                      />
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="agents-hub-row-actions">
                        <Button
                          disabled={!actionState.chatEnabled}
                          onClick={(e) => { stopEvent(e); onStartThread?.(agent.agentId); }}
                          size="sm"
                          variant="outline"
                        >
                          {t('Chat')}
                        </Button>
                        {actionState.setDefaultVisible ? (
                          <Button
                            disabled={mutatingAvailability}
                            onClick={(event) => {
                              stopEvent(event);
                              void handleSetDefaultAgent(agent);
                            }}
                            size="sm"
                            variant="ghost"
                          >
                            {t('Set default')}
                          </Button>
                        ) : null}
                        {!agent.builtIn ? (
                          <Button
                            onClick={(e) => { stopEvent(e); openEditAgentDialog(agent); }}
                            size="sm"
                            variant="ghost"
                          >
                            {t('Edit')}
                          </Button>
                        ) : null}
                        {!agent.builtIn ? (
                          <Button
                            onClick={(e) => { stopEvent(e); onOpenMemory?.(agent); }}
                            size="sm"
                            variant="ghost"
                          >
                            {t('Memory')}
                          </Button>
                        ) : null}
                        {!agent.builtIn ? (
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <button
                                aria-label={t('More actions for {name}', { name: agent.displayName || agent.agentId })}
                                className="bot-table-action-button"
                                onClick={stopEvent}
                                type="button"
                              >
                                <MoreDotsIcon size={14} />
                              </button>
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end" sideOffset={4}>
                              <DropdownMenuItem
                                disabled={saving}
                                onClick={(event) => {
                                  event.stopPropagation();
                                }}
                                onSelect={(event) => {
                                  event.stopPropagation();
                                  openAgentDeleteConfirmation(agent);
                                }}
                                variant="destructive"
                              >
                                <Trash aria-hidden />
                                {t('Delete')}
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })
            ) : search.trim() ? (
              <TableRow>
                <TableCell className="text-center text-muted-foreground" colSpan={5}>
                  {t('No agents matching "{query}"', { query: search.trim() })}
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      )}

      <Dialog
        open={Boolean(agentDeleteConfirmation)}
        onOpenChange={(open) => {
          if (!open && !saving) {
            setAgentDeleteConfirmation(null);
          }
        }}
      >
        <DialogContent
          aria-describedby="agent-delete-confirmation-description"
          role="alertdialog"
          showCloseButton={false}
          size="compact"
        >
          <DialogHeader>
            <DialogTitle>
              {t('Delete "{name}"?', { name: agentDeleteConfirmation?.displayName || '' })}
            </DialogTitle>
            <DialogDescription id="agent-delete-confirmation-description">
              {t('This permanently deletes this custom agent. This action cannot be undone.')}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              disabled={saving}
              onClick={() => setAgentDeleteConfirmation(null)}
              type="button"
              variant="outline"
            >
              {t('Cancel')}
            </Button>
            <Button
              disabled={saving}
              onClick={() => { void handleConfirmDeleteAgent(); }}
              type="button"
              variant="destructive"
            >
              {saving ? t('Deleting') : t('Delete agent')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AgentFormDialog
        agentDialogMode={agentDialogMode}
        agentDraft={agentDraft}
        closeAgentDialog={closeAgentDialog}
        ensureProviderModels={ensureProviderModels}
        envText={envText}
        envViewMode={envViewMode}
        handleAvatarFileChange={handleAvatarFileChange}
        loadData={async () => {
          await loadData();
          await onRefreshAgentTargets?.();
        }}
        onAddWorkspace={onAddWorkspace}
        onOpenMemory={onOpenMemory}
        onStartThread={onStartThread}
        onToast={onToast}
        openEditAgentDialog={openEditAgentDialog}
        providerModelsByType={providerModelsByType}
        saving={saving}
        selectedAgent={selectedAgent}
        setAgentDraft={setAgentDraft}
        setAgentIdTouched={setAgentIdTouched}
        openAvatarStyleDialog={openAvatarStyleDialog}
        setEnvText={setEnvText}
        setEnvViewMode={setEnvViewMode}
        setSaving={setSaving}
        workspaces={workspaces}
      />

      <AvatarStyleDialog
        agentId={agentDraft.agentId}
        avatarStyleDialogOpen={avatarStyleDialogOpen}
        avatarStyleId={avatarStyleId}
        builtIn={selectedAgent?.builtIn}
        customAvatarStyle={customAvatarStyle}
        displayName={agentDraft.displayName}
        flow={avatarFlow}
        handleGenerateAvatar={handleGenerateAvatar}
        onCancel={closeAvatarStyleDialog}
        onChangeStyle={() => commitAvatarFlow(changeAvatarStyle(avatarFlowRef.current))}
        onUseAvatar={handleUseGeneratedAvatar}
        providerType={agentDraft.providerType}
        setAvatarStyleId={setAvatarStyleId}
        setCustomAvatarStyle={setCustomAvatarStyle}
      />

    </div>
  );
}
