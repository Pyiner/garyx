import { useEffect, useMemo, useRef, useState } from 'react';

import type {
  DesktopCustomAgent,
  DesktopProviderModels,
  DesktopWorkflowDefinition,
  DesktopWorkflowSourceDocument,
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
import { useI18n } from '../../i18n';
import { AgentAvatarEditor, AvatarStyleDialog } from './AgentAvatarEditor';
import {
  customAgentDeleteConfirmationFor,
  runCustomAgentDeleteConfirmation,
  type CustomAgentDeleteConfirmation,
} from './agents-hub-delete-model';
import { AgentFormDialog } from './AgentFormDialog';
import { WorkflowViewDialog } from './WorkflowViewDialog';
import {
  AGENT_AVATAR_MAX_BYTES,
  DEFAULT_AVATAR_STYLE_ID,
  defaultAuthSource,
  deriveId,
  emptyAgentDraft,
  normalizeAvatarFile,
  previewText,
  providerLabel,
  sortedAgents,
  sortedWorkflows,
  stopEvent,
  workflowDefaultWorkspace,
} from './agents-hub-helpers';
import type {
  AgentDialogMode,
  AgentDraft,
  AvatarStyleId,
  ProviderType,
  WorkflowDialogMode,
} from './agents-hub-helpers';

type HubTab = 'agents' | 'workflows';

type AgentsHubPanelProps = {
  initialTab?: HubTab;
  workspaces?: DesktopWorkspace[];
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onRefreshAgentTargets?: () => Promise<void>;
  onStartThread?: (agentId: string) => void;
  onOpenMemory?: (agent: DesktopCustomAgent) => void;
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

export function AgentsHubPanel({
  initialTab = 'agents',
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
  const [agents, setAgents] = useState<DesktopCustomAgent[]>([]);
  const [workflows, setWorkflows] = useState<DesktopWorkflowDefinition[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [activeTab, setActiveTab] = useState<HubTab>(initialTab);

  const [agentDialogMode, setAgentDialogMode] = useState<AgentDialogMode>(null);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [agentDeleteConfirmation, setAgentDeleteConfirmation] =
    useState<CustomAgentDeleteConfirmation | null>(null);
  const [agentDraft, setAgentDraft] = useState<AgentDraft>(() => emptyAgentDraft());
  const [envViewMode, setEnvViewMode] = useState<'form' | 'text'>('form');
  const [envText, setEnvText] = useState('');
  const [agentIdTouched, setAgentIdTouched] = useState(false);
  const [avatarGenerating, setAvatarGenerating] = useState(false);
  const [avatarStyleDialogOpen, setAvatarStyleDialogOpen] = useState(false);
  const [avatarStyleId, setAvatarStyleId] = useState<AvatarStyleId>(DEFAULT_AVATAR_STYLE_ID);
  const [customAvatarStyle, setCustomAvatarStyle] = useState('');
  const workflowSourceRequestId = useRef(0);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<ProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<ProviderType, boolean>>
  >({});

  const [workflowDialogMode, setWorkflowDialogMode] = useState<WorkflowDialogMode>(null);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string | null>(null);
  const [workflowSource, setWorkflowSource] = useState<DesktopWorkflowSourceDocument | null>(null);
  const [workflowSourceLoading, setWorkflowSourceLoading] = useState(false);
  const [workflowSourceError, setWorkflowSourceError] = useState<string | null>(null);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

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
      const [agentsResult, workflowsResult] = await Promise.allSettled([
        window.garyxDesktop.listCustomAgents(),
        window.garyxDesktop.listWorkflowDefinitions(),
      ]);

      if (agentsResult.status === 'fulfilled') {
        setAgents(sortedAgents(agentsResult.value));
      } else if (!silent) {
        setAgents([]);
      }
      if (workflowsResult.status === 'fulfilled') {
        setWorkflows(sortedWorkflows(workflowsResult.value));
      } else if (!silent) {
        setWorkflows([]);
      }

      const failures = [
        agentsResult.status === 'rejected' ? 'agents' : null,
        workflowsResult.status === 'rejected' ? 'workflows' : null,
      ].filter(Boolean);

      if (failures.length && !silent) {
        const message = `Failed to fully load ${failures.join(' and ')}.`;
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
  const selectedWorkflow = useMemo(
    () => workflows.find((workflow) => workflow.workflowId === selectedWorkflowId) || null,
    [selectedWorkflowId, workflows],
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

  const filteredWorkflows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return workflows;
    }
    return workflows.filter((workflow) => {
      return [
        workflow.name,
        workflow.workflowId,
        workflow.description,
        workflow.packageDir || '',
        workflowDefaultWorkspace(workflow),
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [search, workflows]);

  function closeAgentDialog() {
    setAgentDialogMode(null);
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setAgentIdTouched(false);
    setAvatarStyleDialogOpen(false);
    setAvatarStyleId(DEFAULT_AVATAR_STYLE_ID);
    setCustomAvatarStyle('');
  }

  function closeWorkflowDialog() {
    workflowSourceRequestId.current += 1;
    setWorkflowDialogMode(null);
    setSelectedWorkflowId(null);
    setWorkflowSource(null);
    setWorkflowSourceLoading(false);
    setWorkflowSourceError(null);
  }

  async function loadWorkflowSource(workflowId: string) {
    const requestId = workflowSourceRequestId.current + 1;
    workflowSourceRequestId.current = requestId;
    setWorkflowSource(null);
    setWorkflowSourceError(null);
    setWorkflowSourceLoading(true);
    try {
      const source = await window.garyxDesktop.getWorkflowDefinitionSource({ workflowId });
      if (workflowSourceRequestId.current === requestId) {
        setWorkflowSource(source);
      }
    } catch (error) {
      if (workflowSourceRequestId.current === requestId) {
        setWorkflowSourceError(error instanceof Error ? error.message : t('Failed to load workflow source'));
      }
    } finally {
      if (workflowSourceRequestId.current === requestId) {
        setWorkflowSourceLoading(false);
      }
    }
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
      authSource: agent.authSource || defaultAuthSource(agent.providerType as ProviderType),
      baseUrl: agent.baseUrl || '',
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
      authSource: agent.authSource || defaultAuthSource(agent.providerType as ProviderType),
      baseUrl: agent.baseUrl || '',
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

  function openViewWorkflowDialog(workflow: DesktopWorkflowDefinition) {
    setWorkflowDialogMode('view');
    setSelectedWorkflowId(workflow.workflowId);
    void loadWorkflowSource(workflow.workflowId);
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

  async function handleGenerateAvatar(stylePrompt: string) {
    const displayName = agentDraft.displayName.trim();
    const agentId = agentDraft.agentId.trim();
    if (!displayName && !agentId) {
      onToast?.(t('Name is required.'), 'error');
      return;
    }
    setAvatarGenerating(true);
    try {
      const result = await window.garyxDesktop.generateCustomAgentAvatar({
        agentId,
        displayName: displayName || agentId,
        stylePrompt,
      });
      setAgentDraft((current) => ({
        ...current,
        avatarDataUrl: result.avatarDataUrl,
      }));
      setAvatarStyleDialogOpen(false);
      onToast?.(t('Avatar generated'), 'success');
    } catch {
      onToast?.(t('Failed to generate avatar'), 'error');
    } finally {
      setAvatarGenerating(false);
    }
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

  const showingAgents = activeTab === 'agents';
  const showingWorkflows = activeTab === 'workflows';
  const visibleAgents = filteredAgents;
  const visibleWorkflows = filteredWorkflows;

  return (
    <div className="agents-hub">
      <div className="mgmt-page-header agents-hub-page-header">
        <div className="mgmt-page-title-block">
          <h1 className="mgmt-page-title">{t('Agents')}</h1>
          <p className="mgmt-page-subtitle">
            {t('{count} total', {
              count: showingAgents ? agents.length : workflows.length,
            })}
          </p>
        </div>
        {!showingWorkflows ? (
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
        ) : null}
      </div>
      <div className="agents-hub-hero">
        <div className="agents-hub-tabs" role="tablist" aria-label={t("Agent registry sections")}>
          <button
            className={`agents-hub-tab ${showingAgents ? 'active' : ''}`}
            onClick={() => {
              setActiveTab('agents');
            }}
            role="tab"
            type="button"
          >
            <span>{t("Agent")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{agents.length}</Badge>
          </button>
          <button
            className={`agents-hub-tab ${showingWorkflows ? 'active' : ''}`}
            onClick={() => {
              setActiveTab('workflows');
            }}
            role="tab"
            type="button"
          >
            <span>{t("Workflow")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{workflows.length}</Badge>
          </button>
        </div>

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
              <TableHead style={{ width: '40%' }}>{t('Name')}</TableHead>
              <TableHead style={{ width: '20%' }}>
                {showingAgents ? t('Provider') : t('Version')}
              </TableHead>
              <TableHead style={{ width: '20%' }}>
                {showingAgents ? t('Type') : t('Workspace')}
              </TableHead>
              <TableHead style={{ width: '20%' }} className="text-right">
                {showingWorkflows ? t('Package') : t('Actions')}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {showingAgents ? (
              visibleAgents.length ? (
                visibleAgents.map((agent) => (
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
                          <div className="agents-hub-cell-name">{agent.displayName}</div>
                          <div className="agents-hub-cell-id">{agent.agentId}</div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell>{providerLabel(agent.providerType)}</TableCell>
                    <TableCell>
                      <Badge variant="outline">{agent.builtIn ? t('Built-in') : t('Custom')}</Badge>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="agents-hub-row-actions">
                        <Button
                          onClick={(e) => { stopEvent(e); onStartThread?.(agent.agentId); }}
                          size="sm"
                          variant="outline"
                        >
                          {t('Chat')}
                        </Button>
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
                ))
              ) : search.trim() ? (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No agents matching "{query}"', { query: search.trim() })}
                  </TableCell>
                </TableRow>
              ) : null
            ) : (
              visibleWorkflows.length ? (
                visibleWorkflows.map((workflow) => {
                  const workspace = workflowDefaultWorkspace(workflow);
                  return (
                    <TableRow
                      className="cursor-pointer"
                      key={workflow.workflowId}
                      onClick={() => openViewWorkflowDialog(workflow)}
                    >
                      <TableCell>
                        <div className="agents-hub-name-cell">
                          <span className="agents-hub-avatar-sm workflow">WF</span>
                          <div>
                            <div className="agents-hub-cell-name">{workflow.name}</div>
                            <div className="agents-hub-cell-id">{workflow.workflowId}</div>
                            {workflow.description ? (
                              <div className="agents-hub-cell-description">
                                {previewText(workflow.description, '')}
                              </div>
                            ) : null}
                          </div>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">v{workflow.version}</Badge>
                      </TableCell>
                      <TableCell>
                        <span className="agents-hub-cell-id">
                          {workspace || t('Task workspace')}
                        </span>
                      </TableCell>
                      <TableCell className="text-right">
                        <span
                          className="agents-hub-cell-id agents-hub-package-path"
                          title={workflow.packageDir || undefined}
                        >
                          {workflow.packageDir ? t('File package') : t('Installed')}
                        </span>
                      </TableCell>
                    </TableRow>
                  );
                })
              ) : search.trim() ? (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No workflows matching "{query}"', { query: search.trim() })}
                  </TableCell>
                </TableRow>
              ) : (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No workflow definitions installed')}
                    <div className="agents-hub-install-hint">
                      <code>garyx workflow definition upsert --file &lt;path&gt;</code>
                    </div>
                  </TableCell>
                </TableRow>
              )
            )}
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
        avatarGenerating={avatarGenerating}
        closeAgentDialog={closeAgentDialog}
        ensureProviderModels={ensureProviderModels}
        envText={envText}
        envViewMode={envViewMode}
        handleAvatarFileChange={handleAvatarFileChange}
        loadData={loadData}
        onAddWorkspace={onAddWorkspace}
        onOpenMemory={onOpenMemory}
        onStartThread={onStartThread}
        onToast={onToast}
        openEditAgentDialog={openEditAgentDialog}
        providerModelsByType={providerModelsByType}
        providerModelsLoading={providerModelsLoading}
        saving={saving}
        selectedAgent={selectedAgent}
        setAgentDraft={setAgentDraft}
        setAgentIdTouched={setAgentIdTouched}
        setAvatarStyleDialogOpen={setAvatarStyleDialogOpen}
        setEnvText={setEnvText}
        setEnvViewMode={setEnvViewMode}
        setSaving={setSaving}
        workspaces={workspaces}
      />

      <AvatarStyleDialog
        avatarGenerating={avatarGenerating}
        avatarStyleDialogOpen={avatarStyleDialogOpen}
        avatarStyleId={avatarStyleId}
        customAvatarStyle={customAvatarStyle}
        handleGenerateAvatar={handleGenerateAvatar}
        setAvatarStyleDialogOpen={setAvatarStyleDialogOpen}
        setAvatarStyleId={setAvatarStyleId}
        setCustomAvatarStyle={setCustomAvatarStyle}
      />

      <WorkflowViewDialog
        closeWorkflowDialog={closeWorkflowDialog}
        loadWorkflowSource={loadWorkflowSource}
        selectedWorkflow={selectedWorkflow}
        workflowDialogMode={workflowDialogMode}
        workflowSource={workflowSource}
        workflowSourceError={workflowSourceError}
        workflowSourceLoading={workflowSourceLoading}
      />
    </div>
  );
}
