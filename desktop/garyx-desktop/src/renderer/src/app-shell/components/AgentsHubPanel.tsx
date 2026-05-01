import { useEffect, useMemo, useState } from 'react';
import {
  IconBolt,
  IconCheck,
  IconPlus,
  IconRobot,
  IconSearch,
  IconSparkles,
  IconUsersGroup,
  IconX,
} from '@tabler/icons-react';

import type {
  CreateCustomAgentInput,
  CreateTeamInput,
  DesktopCustomAgent,
  DesktopProviderModels,
  DesktopTeam,
  UpdateCustomAgentInput,
  UpdateTeamInput,
} from '@shared/contracts';

import { Badge } from '../../components/ui/badge';
import { Button } from '../../components/ui/button';
import { Checkbox } from '../../components/ui/checkbox';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../../components/ui/table';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Textarea } from '../../components/ui/textarea';
import { useI18n } from '../../i18n';

type ProviderType = 'claude_code' | 'codex_app_server' | 'gemini_cli';
type HubTab = 'agents' | 'teams';
type AgentDialogMode = 'create' | 'edit' | 'view' | null;
type TeamDialogMode = 'create' | 'edit' | 'view' | null;

type AgentDraft = {
  agentId: string;
  displayName: string;
  providerType: ProviderType;
  model: string;
  systemPrompt: string;
};

const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default__';

type TeamDraft = {
  teamId: string;
  displayName: string;
  leaderAgentId: string;
  memberAgentIds: string[];
  workflowText: string;
};

type AgentsHubPanelProps = {
  initialTab?: HubTab;
  onStartThread?: (agentOrTeamId: string) => void;
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

function emptyAgentDraft(): AgentDraft {
  return {
    agentId: '',
    displayName: '',
    providerType: 'claude_code',
    model: '',
    systemPrompt: '',
  };
}

function emptyTeamDraft(): TeamDraft {
  return {
    teamId: '',
    displayName: '',
    leaderAgentId: '',
    memberAgentIds: [],
    workflowText: '',
  };
}

function deriveId(name: string): string {
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
  if (value === 'gemini_cli') {
    return 'Gemini';
  }
  return 'Claude';
}

function previewText(value: string | null | undefined, fallback: string): string {
  const normalized = value?.replace(/\s+/g, ' ').trim() || '';
  if (!normalized) {
    return fallback;
  }
  return normalized.length > 140 ? `${normalized.slice(0, 137)}...` : normalized;
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

function avatarLabel(value: string): string {
  return value
    .split(/\s+/)
    .map((part) => part[0] || '')
    .join('')
    .slice(0, 2)
    .toUpperCase();
}

function buildSuggestedWorkflow(
  agents: DesktopCustomAgent[],
  leaderAgentId: string,
  memberAgentIds: string[],
): string {
  const nameById = new Map(agents.map((agent) => [agent.agentId, agent.displayName] as const));
  const leaderName = nameById.get(leaderAgentId) || leaderAgentId || 'Leader';
  const memberNames = memberAgentIds
    .map((agentId) => nameById.get(agentId) || agentId)
    .filter(Boolean);

  return [
    `${leaderName} receives the brief first, breaks the work into clear subtasks, and coordinates the team response.`,
    '',
    memberNames.length
      ? `Selected members: ${memberNames.join(', ')}.`
      : 'Selected members should explore focused slices of the task in parallel.',
    '',
    'Have members surface tradeoffs early, then merge the strongest ideas into one final answer with clear acceptance checks.',
  ].join('\n');
}

function sortedAgents(value: DesktopCustomAgent[]): DesktopCustomAgent[] {
  return [...value]
    .filter((agent) => agent.standalone)
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
    });
}

function sortedTeams(value: DesktopTeam[]): DesktopTeam[] {
  return [...value].sort((left, right) => {
    return left.displayName.localeCompare(right.displayName) || left.teamId.localeCompare(right.teamId);
  });
}

function stopEvent(event: React.MouseEvent<HTMLElement>) {
  event.preventDefault();
  event.stopPropagation();
}

export function AgentsHubPanel({
  initialTab = 'agents',
  onStartThread,
  onToast,
}: AgentsHubPanelProps) {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [agents, setAgents] = useState<DesktopCustomAgent[]>([]);
  const [teams, setTeams] = useState<DesktopTeam[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [activeTab, setActiveTab] = useState<HubTab>(initialTab);

  const [agentDialogMode, setAgentDialogMode] = useState<AgentDialogMode>(null);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [agentDraft, setAgentDraft] = useState<AgentDraft>(() => emptyAgentDraft());
  const [agentIdTouched, setAgentIdTouched] = useState(false);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<ProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<ProviderType, boolean>>
  >({});

  const [teamDialogMode, setTeamDialogMode] = useState<TeamDialogMode>(null);
  const [selectedTeamId, setSelectedTeamId] = useState<string | null>(null);
  const [teamDraft, setTeamDraft] = useState<TeamDraft>(() => emptyTeamDraft());
  const [teamIdTouched, setTeamIdTouched] = useState(false);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  async function loadData() {
    setLoading(true);
    setLoadError(null);
    try {
      const [agentsResult, teamsResult] = await Promise.allSettled([
        window.garyxDesktop.listCustomAgents(),
        window.garyxDesktop.listTeams(),
      ]);

      const nextAgents = agentsResult.status === 'fulfilled' ? sortedAgents(agentsResult.value) : [];
      const nextTeams = teamsResult.status === 'fulfilled' ? sortedTeams(teamsResult.value) : [];
      setAgents(nextAgents);
      setTeams(nextTeams);

      const failures = [
        agentsResult.status === 'rejected' ? 'agents' : null,
        teamsResult.status === 'rejected' ? 'teams' : null,
      ].filter(Boolean);

      if (failures.length) {
        const message = `Failed to fully load ${failures.join(' and ')}.`;
        setLoadError(message);
        onToast?.(message, 'error');
      }
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
    void loadData();
  }, []);

  useEffect(() => {
    if (agentDialogMode === 'create' || agentDialogMode === 'edit') {
      void ensureProviderModels(agentDraft.providerType);
    }
  }, [agentDialogMode, agentDraft.providerType]);

  useEffect(() => {
    if (agentDialogMode !== 'create' || agentIdTouched) {
      return;
    }
    const nextId = deriveId(agentDraft.displayName);
    setAgentDraft((current) => (current.agentId === nextId ? current : { ...current, agentId: nextId }));
  }, [agentDialogMode, agentDraft.displayName, agentIdTouched]);

  useEffect(() => {
    if (teamDialogMode !== 'create' || teamIdTouched) {
      return;
    }
    const nextId = deriveId(teamDraft.displayName);
    setTeamDraft((current) => (current.teamId === nextId ? current : { ...current, teamId: nextId }));
  }, [teamDialogMode, teamDraft.displayName, teamIdTouched]);

  const agentMap = useMemo(() => {
    return new Map(agents.map((agent) => [agent.agentId, agent] as const));
  }, [agents]);

  const selectedAgent = useMemo(
    () => agents.find((agent) => agent.agentId === selectedAgentId) || null,
    [agents, selectedAgentId],
  );
  const selectedTeam = useMemo(
    () => teams.find((team) => team.teamId === selectedTeamId) || null,
    [teams, selectedTeamId],
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

  const filteredTeams = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) {
      return teams;
    }
    return teams.filter((team) => {
      const memberLabels = team.memberAgentIds
        .map((agentId) => agentMap.get(agentId)?.displayName || agentId)
        .join(' ');
      return [
        team.displayName,
        team.teamId,
        team.workflowText,
        agentMap.get(team.leaderAgentId)?.displayName || team.leaderAgentId,
        memberLabels,
      ].some((value) => value.toLowerCase().includes(needle));
    });
  }, [agentMap, search, teams]);

  const teamSelectionCount = useMemo(() => {
    return agents.filter((agent) => teamDraft.memberAgentIds.includes(agent.agentId)).length;
  }, [agents, teamDraft.memberAgentIds]);

  const allAgentsSelected = agents.length > 0 && teamSelectionCount === agents.length;
  const teamMemberSelectionState = allAgentsSelected
    ? true
    : teamSelectionCount > 0
      ? 'indeterminate'
      : false;
  const activeAgentProviderModels = providerModelsByType[agentDraft.providerType];
  const agentProviderModelsLoading = providerModelsLoading[agentDraft.providerType] === true;
  const agentModelOptions = providerModelsWithCurrent(activeAgentProviderModels, agentDraft.model);
  const agentSupportsModelSelection =
    activeAgentProviderModels?.supportsModelSelection === true && agentModelOptions.length > 0;
  const agentModelStatus =
    agentDraft.providerType === 'gemini_cli' && !agentSupportsModelSelection
      ? agentProviderModelsLoading
        ? t('Loading models from local Gemini ACP...')
        : activeAgentProviderModels?.error
          ? t('Local Gemini ACP does not expose a model list yet.')
          : null
      : null;

  function closeAgentDialog() {
    setAgentDialogMode(null);
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setAgentIdTouched(false);
  }

  function closeTeamDialog() {
    setTeamDialogMode(null);
    setSelectedTeamId(null);
    setTeamDraft(emptyTeamDraft());
    setTeamIdTouched(false);
  }

  function openCreateAgentDialog() {
    setAgentDialogMode('create');
    setSelectedAgentId(null);
    setAgentDraft(emptyAgentDraft());
    setAgentIdTouched(false);
  }

  function openViewAgentDialog(agent: DesktopCustomAgent) {
    setAgentDialogMode('view');
    setSelectedAgentId(agent.agentId);
    setAgentDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      systemPrompt: agent.systemPrompt,
    });
    setAgentIdTouched(true);
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
      systemPrompt: agent.systemPrompt,
    });
    setAgentIdTouched(true);
  }

  function openCreateTeamDialog(seedAgentId?: string) {
    const seedAgent = seedAgentId ? agentMap.get(seedAgentId) || null : null;
    const nextDisplayName = seedAgent ? `${seedAgent.displayName} Team` : '';
    const nextLeaderAgentId = seedAgent?.agentId || '';
    const nextMemberAgentIds = seedAgent ? [seedAgent.agentId] : [];
    setTeamDialogMode('create');
    setSelectedTeamId(null);
    setTeamDraft({
      teamId: '',
      displayName: nextDisplayName,
      leaderAgentId: nextLeaderAgentId,
      memberAgentIds: nextMemberAgentIds,
      workflowText: buildSuggestedWorkflow(agents, nextLeaderAgentId, nextMemberAgentIds),
    });
    setTeamIdTouched(false);
    setActiveTab('teams');
  }

  function openViewTeamDialog(team: DesktopTeam) {
    setTeamDialogMode('view');
    setSelectedTeamId(team.teamId);
    setTeamDraft({
      teamId: team.teamId,
      displayName: team.displayName,
      leaderAgentId: team.leaderAgentId,
      memberAgentIds: [...team.memberAgentIds],
      workflowText: team.workflowText,
    });
    setTeamIdTouched(true);
  }

  function openEditTeamDialog(team: DesktopTeam) {
    setTeamDialogMode('edit');
    setSelectedTeamId(team.teamId);
    setTeamDraft({
      teamId: team.teamId,
      displayName: team.displayName,
      leaderAgentId: team.leaderAgentId,
      memberAgentIds: [...team.memberAgentIds],
      workflowText: team.workflowText,
    });
    setTeamIdTouched(true);
    setActiveTab('teams');
  }

  function toggleTeamMember(agentId: string) {
    setTeamDraft((current) => {
      const exists = current.memberAgentIds.includes(agentId);
      const memberAgentIds = exists
        ? current.memberAgentIds.filter((entry) => entry !== agentId)
        : [...current.memberAgentIds, agentId];
      // If leader was removed or no leader set, default to first member
      const leaderAgentId = memberAgentIds.includes(current.leaderAgentId)
        ? current.leaderAgentId
        : memberAgentIds[0] || '';
      return { ...current, memberAgentIds, leaderAgentId };
    });
  }

  function selectTeamLeader(agentId: string) {
    setTeamDraft((current) => {
      const memberAgentIds = current.memberAgentIds.includes(agentId)
        ? current.memberAgentIds
        : [agentId, ...current.memberAgentIds];
      return { ...current, leaderAgentId: agentId, memberAgentIds };
    });
  }

  function selectAllTeamMembers(nextChecked: boolean) {
    setTeamDraft((current) => {
      const preservedLeaderIds = current.leaderAgentId ? [current.leaderAgentId] : [];
      return {
        ...current,
        memberAgentIds: nextChecked
          ? Array.from(new Set([...preservedLeaderIds, ...agents.map((agent) => agent.agentId)]))
          : preservedLeaderIds,
      };
    });
  }

  async function handleAgentSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      const payload: CreateCustomAgentInput = {
        agentId: agentDraft.agentId.trim(),
        displayName: agentDraft.displayName.trim(),
        providerType: agentDraft.providerType,
        model: agentSupportsModelSelection ? agentDraft.model.trim() : '',
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

  async function handleDeleteAgent(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      return;
    }
    setSaving(true);
    try {
      await window.garyxDesktop.deleteCustomAgent({ agentId: agent.agentId });
      onToast?.(t('Custom agent deleted'), 'success');
      closeAgentDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleTeamSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      const workflowText = teamDraft.workflowText.trim()
        || buildSuggestedWorkflow(agents, teamDraft.leaderAgentId, teamDraft.memberAgentIds);
      const payload: CreateTeamInput = {
        teamId: teamDraft.teamId.trim(),
        displayName: teamDraft.displayName.trim(),
        leaderAgentId: teamDraft.leaderAgentId.trim(),
        memberAgentIds: teamDraft.memberAgentIds,
        workflowText,
      };

      if (teamDialogMode === 'create') {
        await window.garyxDesktop.createTeam(payload);
        onToast?.(t('Agent team created'), 'success');
      } else {
        const updatePayload: UpdateTeamInput = {
          ...payload,
          currentTeamId: selectedTeam?.teamId || payload.teamId,
        };
        await window.garyxDesktop.updateTeam(updatePayload);
        onToast?.(t('Agent team updated'), 'success');
      }

      closeTeamDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save team'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteTeam(team: DesktopTeam) {
    setSaving(true);
    try {
      await window.garyxDesktop.deleteTeam({ teamId: team.teamId });
      onToast?.(t('Agent team deleted'), 'success');
      closeTeamDialog();
      await loadData();
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete team'), 'error');
    } finally {
      setSaving(false);
    }
  }

  const agentValidationError =
    !agentDraft.displayName.trim()
      ? t('Name is required.')
      : !agentDraft.agentId.trim()
        ? t('Agent ID is required.')
        : !agentDraft.systemPrompt.trim()
          ? t('System prompt is required.')
          : null;

  const teamValidationError =
    !teamDraft.displayName.trim()
      ? t('Team name is required.')
      : !teamDraft.teamId.trim()
        ? t('Team ID is required.')
        : teamDraft.memberAgentIds.length === 0
          ? t('Select at least one member.')
          : !teamDraft.leaderAgentId.trim()
            ? t('Select a leader.')
            : !teamDraft.memberAgentIds.includes(teamDraft.leaderAgentId)
              ? t('Leader must be part of the team.')
              : null;

  const showingAgents = activeTab === 'agents';
  const visibleAgents = filteredAgents;
  const visibleTeams = filteredTeams;

  return (
    <div className="agents-hub">
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
            <span>{t("My Agents")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{agents.length}</Badge>
          </button>
          <button
            className={`agents-hub-tab ${!showingAgents ? 'active' : ''}`}
            onClick={() => {
              setActiveTab('teams');
            }}
            role="tab"
            type="button"
          >
            <span>{t("Teams")}</span>
            <Badge className="agents-hub-tab-badge" variant="outline">{teams.length}</Badge>
          </button>
        </div>

        <div className="agents-hub-controls">
          <div className="agents-hub-search">
            <IconSearch aria-hidden size={16} stroke={1.8} />
            <Input
              className="agents-hub-search-input"
              onChange={(event) => {
                setSearch(event.target.value);
              }}
              placeholder={t("Search...")}
              value={search}
            />
          </div>

          <Button
            onClick={showingAgents ? openCreateAgentDialog : () => openCreateTeamDialog()}
            size="sm"
          >
            <IconPlus aria-hidden size={15} stroke={2} />
            {showingAgents ? t('New Agent') : t('New Team')}
          </Button>
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
              <TableHead style={{ width: '20%' }}>{showingAgents ? t('Provider') : t('Leader')}</TableHead>
              <TableHead style={{ width: '20%' }}>{showingAgents ? t('Type') : t('Members')}</TableHead>
              <TableHead style={{ width: '20%' }} className="text-right">{t('Actions')}</TableHead>
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
                        <span className={`agents-hub-avatar-sm ${agent.builtIn ? 'builtin' : ''}`}>
                          {avatarLabel(agent.displayName || agent.agentId)}
                        </span>
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
                        <Button
                          onClick={(e) => { stopEvent(e); openCreateTeamDialog(agent.agentId); }}
                          size="sm"
                          variant="ghost"
                        >
                          {t('Team')}
                        </Button>
                        {!agent.builtIn ? (
                          <Button
                            disabled={saving}
                            onClick={(e) => { stopEvent(e); void handleDeleteAgent(agent); }}
                            size="sm"
                            variant="ghost"
                            className="text-destructive"
                          >
                            {t('Delete')}
                          </Button>
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
              visibleTeams.length ? (
                visibleTeams.map((team) => {
                  const leaderLabel = agentMap.get(team.leaderAgentId)?.displayName || team.leaderAgentId;
                  return (
                    <TableRow
                      className="cursor-pointer"
                      key={team.teamId}
                      onClick={() => openViewTeamDialog(team)}
                    >
                      <TableCell>
                        <div className="agents-hub-name-cell">
                          <span className="agents-hub-avatar-sm team">
                            {avatarLabel(team.displayName || team.teamId)}
                          </span>
                          <div>
                            <div className="agents-hub-cell-name">{team.displayName}</div>
                            <div className="agents-hub-cell-id">{team.teamId}</div>
                          </div>
                        </div>
                      </TableCell>
                      <TableCell>{leaderLabel}</TableCell>
                      <TableCell>
                        {team.memberAgentIds
                          .slice(0, 3)
                          .map((id) => agentMap.get(id)?.displayName || id)
                          .join(', ')}
                        {team.memberAgentIds.length > 3 ? ` +${team.memberAgentIds.length - 3}` : ''}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="agents-hub-row-actions">
                          <Button
                            onClick={(e) => { stopEvent(e); onStartThread?.(team.teamId); }}
                            size="sm"
                            variant="outline"
                          >
                            {t('Chat')}
                          </Button>
                          <Button
                            onClick={(e) => { stopEvent(e); openEditTeamDialog(team); }}
                            size="sm"
                            variant="ghost"
                          >
                            {t('Edit')}
                          </Button>
                          <Button
                            disabled={saving}
                            onClick={(e) => { stopEvent(e); void handleDeleteTeam(team); }}
                            size="sm"
                            variant="ghost"
                            className="text-destructive"
                          >
                            {t('Delete')}
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              ) : search.trim() ? (
                <TableRow>
                  <TableCell className="text-center text-muted-foreground" colSpan={4}>
                    {t('No teams matching "{query}"', { query: search.trim() })}
                  </TableCell>
                </TableRow>
              ) : null
            )}
          </TableBody>
        </Table>
      )}

      <Dialog
        open={Boolean(agentDialogMode)}
        onOpenChange={(open) => {
          if (!open) {
            closeAgentDialog();
          }
        }}
      >
        <DialogContent className="agents-hub-agent-dialog">
          <DialogHeader className="agents-hub-dialog-header">
            <DialogDescription className="agents-hub-dialog-kicker">
              {t('Agent')}
            </DialogDescription>
            <DialogTitle className="agents-hub-dialog-title">
              {agentDialogMode === 'create'
                ? t('Create agent')
                : agentDialogMode === 'edit'
                  ? t('Edit agent')
                  : selectedAgent?.displayName || t('Agent')}
            </DialogTitle>
            <DialogDescription className="agents-hub-dialog-description">
              {agentDialogMode === 'create'
                ? t('Create a reusable agent identity with its own provider and system prompt.')
                : t('Inspect or adjust how this agent shows up in the desktop app.')}
            </DialogDescription>
          </DialogHeader>

          {agentDialogMode === 'create' || agentDialogMode === 'edit' ? (
            <form className="agents-hub-dialog-form" onSubmit={handleAgentSubmit}>
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

              <div className="codex-form-field">
                <Label className="codex-form-label">{t('Provider')}</Label>
                <Select
                  onValueChange={(value: ProviderType) => {
                    setAgentDraft((current) => ({
                      ...current,
                      providerType: value,
                      model: '',
                    }));
                    void ensureProviderModels(value);
                  }}
                  value={agentDraft.providerType}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t('Select provider')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="claude_code">Claude</SelectItem>
                    <SelectItem value="codex_app_server">Codex</SelectItem>
                    <SelectItem value="gemini_cli">Gemini</SelectItem>
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
                      }));
                    }}
                    value={agentDraft.model.trim() || PROVIDER_DEFAULT_MODEL_VALUE}
                  >
                    <SelectTrigger className="agents-hub-model-select" id="agent-dialog-model">
                      <SelectValue placeholder={t('Select model')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={PROVIDER_DEFAULT_MODEL_VALUE}>{t('Provider default')}</SelectItem>
                      {agentModelOptions.map((model) => (
                        <SelectItem key={model.id} value={model.id}>
                          {model.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <span className="codex-form-hint">
                    {activeAgentProviderModels?.defaultModel
                      ? t('Gateway default: {model}', { model: activeAgentProviderModels.defaultModel })
                      : t('Optional. Leave empty to use the provider default.')}
                  </span>
                </div>
              ) : null}

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
                <span className={`agents-hub-avatar-centered large ${selectedAgent?.builtIn ? 'builtin' : ''}`}>
                  {avatarLabel(selectedAgent?.displayName || selectedAgent?.agentId || 'A')}
                </span>
                <div className="agents-hub-detail-copy">
                <div className="agents-hub-card-badges">
                  <Badge variant="outline">{selectedAgent?.builtIn ? t('Built-in') : t('Custom')}</Badge>
                  {selectedAgent ? <Badge variant="outline">{providerLabel(selectedAgent.providerType)}</Badge> : null}
                </div>
                <h3>{selectedAgent?.displayName || t('Agent')}</h3>
                <p>{selectedAgent?.agentId || ''}</p>
                {selectedAgent && (selectedAgent.providerType === 'gemini_cli' || selectedAgent.model.trim()) ? (
                  <p>{selectedAgent.model || t('(provider default)')}</p>
                ) : null}
              </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('System Prompt')}</div>
                <div className="agents-hub-detail-body mono">
                  {selectedAgent?.systemPrompt || t('(empty)')}
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

      <Dialog
        open={Boolean(teamDialogMode)}
        onOpenChange={(open) => {
          if (!open) {
            closeTeamDialog();
          }
        }}
      >
        <DialogContent className="sm:max-w-[720px] team-builder-dialog">
          <DialogHeader>
            <DialogTitle>
              {teamDialogMode === 'create'
                ? t('Create team')
                : teamDialogMode === 'edit'
                  ? t('Edit team')
                  : selectedTeam?.displayName || t('Team')}
            </DialogTitle>
            <DialogDescription>
              {t('Set team name and add at least one agent. You can set one as TL (Team Lead).')}
            </DialogDescription>
          </DialogHeader>

          {teamDialogMode === 'create' || teamDialogMode === 'edit' ? (
            <form className="agents-hub-dialog-form" onSubmit={handleTeamSubmit}>
              <div className="team-builder-form-grid">
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="team-dialog-name">{t('Team name')}</Label>
                  <Input
                    id="team-dialog-name"
                    onChange={(event) => {
                      setTeamDraft((current) => ({ ...current, displayName: event.target.value }));
                    }}
                    placeholder={t('e.g. dev-team')}
                    value={teamDraft.displayName}
                  />
                </div>
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="team-dialog-workflow">{t('Workflow')}</Label>
                  <Textarea
                    id="team-dialog-workflow"
                    onChange={(event) => {
                      setTeamDraft((current) => ({ ...current, workflowText: event.target.value }));
                    }}
                    placeholder={t('How should the team coordinate? (optional)')}
                    rows={2}
                    value={teamDraft.workflowText}
                  />
                </div>
              </div>

              <div className="team-builder-body">
                <div className="team-builder-left">
                  <div className="team-builder-agent-count">{t('ALL AGENTS')} ({agents.length})</div>
                  <div className="team-builder-agent-list">
                    {agents.map((agent) => {
                      const selected = teamDraft.memberAgentIds.includes(agent.agentId);
                      return (
                        <button
                          className={`team-builder-agent-row ${selected ? 'selected' : ''}`}
                          key={agent.agentId}
                          onClick={() => {
                            toggleTeamMember(agent.agentId);
                          }}
                          type="button"
                        >
                          <span className={`agents-hub-avatar-centered small ${agent.builtIn ? 'builtin' : ''}`}>
                            {avatarLabel(agent.displayName || agent.agentId)}
                          </span>
                          <div className="team-builder-agent-info">
                            <span className="team-builder-agent-name">{agent.displayName}</span>
                            <span className="team-builder-agent-desc">
                              {previewText(agent.systemPrompt, agent.builtIn ? t('Built-in agent') : t('Custom agent'))}
                            </span>
                          </div>
                          <span className={`team-builder-toggle-btn ${selected ? 'checked' : ''}`}>
                            {selected
                              ? <IconCheck aria-hidden size={14} stroke={2.5} />
                              : <IconPlus aria-hidden size={14} stroke={2} />}
                          </span>
                        </button>
                      );
                    })}
                    {!agents.length ? (
                      <div className="codex-empty-state">{t('No agents available yet.')}</div>
                    ) : null}
                  </div>
                </div>

                <div className="team-builder-right">
                  <div className="team-builder-agent-count">{t('SELECTED MEMBERS')} ({teamDraft.memberAgentIds.length} / {agents.length})</div>
                  <div className="team-builder-selected-list">
                    {teamDraft.memberAgentIds.map((agentId) => {
                      const agent = agentMap.get(agentId);
                      const isLeader = teamDraft.leaderAgentId === agentId;
                      return (
                        <div className="team-builder-selected-row" key={agentId}>
                          <span className={`agents-hub-avatar-centered small ${agent?.builtIn ? 'builtin' : ''}`}>
                            {avatarLabel(agent?.displayName || agentId)}
                          </span>
                          <div className="team-builder-agent-info">
                            <span className="team-builder-agent-name">{agent?.displayName || agentId}</span>
                            <span className="team-builder-agent-desc">
                              {previewText(agent?.systemPrompt || '', t('Agent'))}
                            </span>
                          </div>
                          <div className="team-builder-selected-actions">
                            {isLeader ? (
                              <span className="team-builder-tl-badge active">TL</span>
                            ) : (
                              <button
                                className="team-builder-tl-badge"
                                onClick={() => {
                                  selectTeamLeader(agentId);
                                }}
                                title={t('Set as Team Lead')}
                                type="button"
                              >
                                TL
                              </button>
                            )}
                            <button
                              className="team-builder-remove-btn"
                              onClick={() => {
                                toggleTeamMember(agentId);
                              }}
                              title={t('Remove')}
                              type="button"
                            >
                              <IconX aria-hidden size={14} stroke={2} />
                            </button>
                          </div>
                        </div>
                      );
                    })}
                    {!teamDraft.memberAgentIds.length ? (
                      <div style={{ padding: '32px 16px', color: 'var(--color-token-description-foreground)', fontSize: 'var(--text-sm)', textAlign: 'center' }}>
                        {t('Select agents from the left to add them.')}
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>

              <DialogFooter className="team-builder-footer">
                {teamValidationError ? (
                  <span className="agents-hub-dialog-status" style={{ marginRight: 'auto' }}>{teamValidationError}</span>
                ) : null}
                <Button
                  disabled={saving}
                  onClick={closeTeamDialog}
                  type="button"
                  variant="outline"
                >
                  {t('Cancel')}
                </Button>
                <Button className="team-builder-create-team-btn" disabled={Boolean(teamValidationError) || saving} type="submit">
                  {saving ? t('Saving...') : teamDialogMode === 'create' ? t('Create team') : t('Save team')}
                </Button>
              </DialogFooter>
            </form>
          ) : (
            <div className="agents-hub-dialog-stack">
              <div className="agents-hub-detail-header">
                <span className="agents-hub-avatar-centered large team">
                  {avatarLabel(selectedTeam?.displayName || selectedTeam?.teamId || 'T')}
                </span>
                <div className="agents-hub-detail-copy">
                  <div className="agents-hub-card-badges">
                    <Badge variant="outline">
                      {t('Lead')}: {agentMap.get(selectedTeam?.leaderAgentId || '')?.displayName || selectedTeam?.leaderAgentId}
                    </Badge>
                    <Badge variant="outline">{t('{count} members', { count: selectedTeam?.memberAgentIds.length || 0 })}</Badge>
                  </div>
                  <h3>{selectedTeam?.displayName || t('Team')}</h3>
                  <p>{selectedTeam?.teamId || ''}</p>
                </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('Members')}</div>
                <div className="agents-hub-chip-list">
                  {(selectedTeam?.memberAgentIds || []).map((agentId) => (
                    <Badge key={agentId} variant="outline">
                      {agentMap.get(agentId)?.displayName || agentId}
                    </Badge>
                  ))}
                </div>
              </div>

              <div className="agents-hub-detail-block">
                <div className="agents-hub-detail-label">{t('Workflow')}</div>
                <div className="agents-hub-detail-body">{selectedTeam?.workflowText || t('(empty)')}</div>
              </div>

              <DialogFooter className="agents-hub-dialog-actions">
                {selectedTeam ? (
                  <Button
                    onClick={() => {
                      onStartThread?.(selectedTeam.teamId);
                    }}
                    type="button"
                    variant="outline"
                  >
                    {t('Chat')}
                  </Button>
                ) : null}
                {selectedTeam ? (
                  <Button
                    onClick={() => {
                      openEditTeamDialog(selectedTeam);
                    }}
                    type="button"
                  >
                    {t('Edit Team')}
                  </Button>
                ) : null}
              </DialogFooter>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
