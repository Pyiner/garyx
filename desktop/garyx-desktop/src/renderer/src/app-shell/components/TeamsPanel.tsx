import { useEffect, useMemo, useState } from 'react';

import type {
  CreateTeamInput,
  DesktopTeam,
  DesktopCustomAgent,
  UpdateTeamInput,
} from '@shared/contracts';

import { Checkbox } from '../../components/ui/checkbox';
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

type TeamsPanelProps = {
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

type EditorMode = 'inspect' | 'create' | 'edit';

type TeamDraft = {
  teamId: string;
  displayName: string;
  leaderAgentId: string;
  memberAgentIds: string[];
  workflowText: string;
};

function emptyDraft(): TeamDraft {
  return {
    teamId: '',
    displayName: '',
    leaderAgentId: '',
    memberAgentIds: [],
    workflowText: '',
  };
}

function deriveTeamId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}

const plusIcon = (
  <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
    <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
  </svg>
);

export function TeamsPanel({ onToast }: TeamsPanelProps) {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [teams, setTeams] = useState<DesktopTeam[]>([]);
  const [agents, setAgents] = useState<DesktopCustomAgent[]>([]);
  const [teamsLoadError, setTeamsLoadError] = useState<string | null>(null);
  const [agentsLoadError, setAgentsLoadError] = useState<string | null>(null);
  const [selectedTeamId, setSelectedTeamId] = useState<string | null>(null);
  const [editorMode, setEditorMode] = useState<EditorMode>('inspect');
  const [draft, setDraft] = useState<TeamDraft>(() => emptyDraft());
  const [draftIdTouched, setDraftIdTouched] = useState(false);

  async function loadData(preferredTeamId?: string | null) {
    setLoading(true);
    setTeamsLoadError(null);
    setAgentsLoadError(null);
    try {
      const [teamsResult, agentsResult] = await Promise.allSettled([
        window.garyxDesktop.listTeams(),
        window.garyxDesktop.listCustomAgents(),
      ]);

      if (teamsResult.status === 'fulfilled') {
        const nextTeams = [...teamsResult.value];
        nextTeams.sort((left, right) => left.displayName.localeCompare(right.displayName));
        setTeams(nextTeams);
        setSelectedTeamId((current) => {
          const requestedTeamId = preferredTeamId || current;
          if (requestedTeamId && nextTeams.some((team) => team.teamId === requestedTeamId)) {
            return requestedTeamId;
          }
          return nextTeams[0]?.teamId || null;
        });
      } else {
        const message =
          teamsResult.reason instanceof Error
            ? teamsResult.reason.message
            : t('Failed to load teams');
        setTeams([]);
        setSelectedTeamId(null);
        setTeamsLoadError(message);
        onToast?.(message, 'error');
      }

      if (agentsResult.status === 'fulfilled') {
        const nextAgents = [...agentsResult.value];
        nextAgents.sort((left, right) => left.displayName.localeCompare(right.displayName));
        setAgents(nextAgents);
      } else {
        const message =
          agentsResult.reason instanceof Error
            ? agentsResult.reason.message
            : t('Failed to load custom agents');
        setAgents([]);
        setAgentsLoadError(message);
        onToast?.(message, 'error');
      }
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadData();
  }, []);

  useEffect(() => {
    if (editorMode !== 'create' || draftIdTouched) {
      return;
    }
    const nextId = deriveTeamId(draft.displayName);
    setDraft((current) => (current.teamId === nextId ? current : { ...current, teamId: nextId }));
  }, [draft.displayName, draftIdTouched, editorMode]);

  const selectedTeam = useMemo(
    () => teams.find((team) => team.teamId === selectedTeamId) || null,
    [teams, selectedTeamId],
  );

  const agentMap = useMemo(() => {
    const next = new Map<string, DesktopCustomAgent>();
    for (const agent of agents) {
      next.set(agent.agentId, agent);
    }
    return next;
  }, [agents]);

  const knownSelectedAgentCount = useMemo(
    () => agents.filter((agent) => draft.memberAgentIds.includes(agent.agentId)).length,
    [agents, draft.memberAgentIds],
  );

  const allAgentsSelected = agents.length > 0 && knownSelectedAgentCount === agents.length;

  const memberSelectionState =
    allAgentsSelected
      ? true
      : knownSelectedAgentCount > 0
        ? 'indeterminate'
        : false;

  function toggleMember(agentId: string) {
    setDraft((current) => {
      const exists = current.memberAgentIds.includes(agentId);
      const memberAgentIds = exists
        ? current.memberAgentIds.filter((entry) => entry !== agentId)
        : [...current.memberAgentIds, agentId];
      const leaderAgentId = memberAgentIds.includes(current.leaderAgentId) ? current.leaderAgentId : '';
      return { ...current, memberAgentIds, leaderAgentId };
    });
  }

  function selectLeader(agentId: string) {
    setDraft((current) => {
      const memberAgentIds = current.memberAgentIds.includes(agentId)
        ? current.memberAgentIds
        : [agentId, ...current.memberAgentIds];
      return { ...current, leaderAgentId: agentId, memberAgentIds };
    });
  }

  function setAllMembers(nextChecked: boolean) {
    setDraft((current) => {
      const preservedLeaderIds = current.leaderAgentId ? [current.leaderAgentId] : [];
      return {
        ...current,
        memberAgentIds: nextChecked
          ? Array.from(new Set([...preservedLeaderIds, ...agents.map((agent) => agent.agentId)]))
          : preservedLeaderIds,
      };
    });
  }

  function openCreateEditor() {
    setEditorMode('create');
    setDraft(emptyDraft());
    setDraftIdTouched(false);
  }

  function openEditEditor(team: DesktopTeam) {
    setEditorMode('edit');
    setDraft({
      teamId: team.teamId,
      displayName: team.displayName,
      leaderAgentId: team.leaderAgentId,
      memberAgentIds: [...team.memberAgentIds],
      workflowText: team.workflowText,
    });
    setDraftIdTouched(true);
  }

  async function handleDelete(team: DesktopTeam) {
    setSaving(true);
    try {
      await window.garyxDesktop.deleteTeam({ teamId: team.teamId });
      onToast?.(t('Agent team deleted'), 'success');
      setEditorMode('inspect');
      await loadData(teams.find((item) => item.teamId !== team.teamId)?.teamId || null);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete team'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      const payload: CreateTeamInput = {
        teamId: draft.teamId.trim(),
        displayName: draft.displayName.trim(),
        leaderAgentId: draft.leaderAgentId.trim(),
        memberAgentIds: draft.memberAgentIds,
        workflowText: draft.workflowText.trim(),
      };
      let saved: DesktopTeam;
      if (editorMode === 'create') {
        saved = await window.garyxDesktop.createTeam(payload);
        onToast?.(t('Agent team created'), 'success');
      } else {
        const updatePayload: UpdateTeamInput = {
          ...payload,
          currentTeamId: selectedTeam?.teamId || payload.teamId,
        };
        saved = await window.garyxDesktop.updateTeam(updatePayload);
        onToast?.(t('Agent team updated'), 'success');
      }
      setEditorMode('inspect');
      setDraft(emptyDraft());
      setDraftIdTouched(false);
      await loadData(saved.teamId);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save team'), 'error');
    } finally {
      setSaving(false);
    }
  }

  const validationError =
    !draft.displayName.trim()
      ? t('Team name is required.')
      : !draft.teamId.trim()
        ? t('Team ID is required.')
        : draft.memberAgentIds.length === 0
          ? t('Select at least one member.')
          : !draft.leaderAgentId.trim()
            ? t('Select a leader.')
            : !draft.memberAgentIds.includes(draft.leaderAgentId)
              ? t('Leader must be included in the member list.')
              : !draft.workflowText.trim()
                ? t('Workflow is required.')
                : null;

  const showingEditor = editorMode === 'create' || editorMode === 'edit';

  return (
    <div className="grid h-full min-h-0 w-full gap-6" style={{ gridTemplateColumns: '340px minmax(0,1fr)' }}>
      {/* ── Left column: team list ── */}
      <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
        <div className="codex-section">
          <div className="codex-section-header">
            <span className="codex-section-title">{t('Teams')}</span>
            <button className="codex-section-action" onClick={openCreateEditor} type="button">
              {plusIcon} {t('New')}
            </button>
          </div>
        </div>
        {teamsLoadError ? (
          <div className="codex-inline-hint" style={{ color: '#9b4b4b' }}>{teamsLoadError}</div>
        ) : null}
        {loading ? (
          <div className="codex-empty-state">{t('Loading teams...')}</div>
        ) : teams.length ? (
          <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
            {teams.map((team) => {
              const active = team.teamId === selectedTeamId && editorMode === 'inspect';
              return (
                <button
                  key={team.teamId}
                  className={`codex-list-row w-full text-left ${active ? 'codex-list-row-active' : ''}`}
                  onClick={() => {
                    setSelectedTeamId(team.teamId);
                    setEditorMode('inspect');
                  }}
                  type="button"
                >
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                    <span className="codex-list-row-name">{team.displayName}</span>
                    <span className="codex-command-row-desc">{team.teamId}</span>
                  </div>
                  <div className="codex-list-row-actions">
                    <span className="codex-sync-pill ok">
                      {t('{count} members', { count: team.memberAgentIds.length })}
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        ) : (
          <div className="codex-empty-state">{t('No teams yet.')}</div>
        )}
      </div>

      {/* ── Right column: inspect or edit ── */}
      {showingEditor ? (
        <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
          <div className="codex-section">
            <div className="codex-section-header">
              <span className="codex-section-title">
                {editorMode === 'create' ? t('New Team') : t('Edit Team')}
              </span>
            </div>
          </div>
          <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
            <form onSubmit={handleSubmit}>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="team-display-name">{t('Name')}</Label>
                <Input
                  id="team-display-name"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, displayName: event.target.value }));
                  }}
                  value={draft.displayName}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="team-id">{t('Team ID')}</Label>
                <Input
                  disabled={editorMode === 'edit'}
                  id="team-id"
                  onChange={(event) => {
                    setDraftIdTouched(true);
                    setDraft((current) => ({ ...current, teamId: event.target.value }));
                  }}
                  value={draft.teamId}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label">{t('Leader Agent')}</Label>
                <Select
                  onValueChange={selectLeader}
                  value={draft.leaderAgentId}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t('Choose the leader agent')} />
                  </SelectTrigger>
                  <SelectContent>
                    {agents.length ? (
                      agents.map((agent) => (
                        <SelectItem key={agent.agentId} value={agent.agentId}>
                          {agent.displayName}
                        </SelectItem>
                      ))
                    ) : (
                      <SelectItem disabled value="no-agents">
                        {t('No agents available')}
                      </SelectItem>
                    )}
                  </SelectContent>
                </Select>
                <span className="codex-form-hint">
                  {t('The leader receives the brief first and is automatically included in the member set.')}
                </span>
              </div>
              <div className="codex-form-field">
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
                  <Label className="codex-form-label">{t('Members')}</Label>
                  <span className="codex-form-hint">
                    {t('{count} selected', { count: draft.memberAgentIds.length })}
                  </span>
                </div>
                {agentsLoadError ? (
                  <div className="codex-inline-hint" style={{ color: '#9b4b4b' }}>{agentsLoadError}</div>
                ) : null}
                <div className="codex-list-card" style={{ maxHeight: 280, overflowY: 'auto' }}>
                  {agents.length ? (
                    <>
                      <div className="codex-list-row" style={{ position: 'sticky', top: 0, zIndex: 10, background: 'var(--color-token-bg-primary, #fcfcfa)' }}>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                          <Checkbox
                            aria-label={t('Select all agents')}
                            checked={memberSelectionState}
                            onCheckedChange={(checked) => {
                              setAllMembers(checked === true);
                            }}
                          />
                          <span className="codex-list-row-name" style={{ fontSize: 12 }}>{t('Select All')}</span>
                        </div>
                      </div>
                      {agents.map((agent) => {
                        const selected = draft.memberAgentIds.includes(agent.agentId);
                        const leader = draft.leaderAgentId === agent.agentId;

                        return (
                          <div
                            className={`codex-list-row ${selected ? 'codex-list-row-active' : ''}`}
                            key={agent.agentId}
                          >
                            <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0 }}>
                              <Checkbox
                                aria-label={t('Select {name}', { name: agent.displayName })}
                                checked={selected}
                                disabled={leader}
                                onCheckedChange={(checked) => {
                                  const nextChecked = checked === true;
                                  if (nextChecked !== selected) {
                                    toggleMember(agent.agentId);
                                  }
                                }}
                              />
                              <span className="codex-list-row-name">{agent.displayName}</span>
                            </div>
                            <div className="codex-list-row-actions">
                              {leader ? (
                                <span className="codex-sync-pill ok">{t('Leader')}</span>
                              ) : null}
                              {agent.builtIn ? (
                                <span className="codex-sync-pill ok">{t('Built-in')}</span>
                              ) : null}
                            </div>
                          </div>
                        );
                      })}
                    </>
                  ) : (
                    <div className="codex-empty-state">{t('No custom agents available yet.')}</div>
                  )}
                </div>
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="team-workflow">{t('Workflow')}</Label>
                <Textarea
                  className="min-h-[220px]"
                  id="team-workflow"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, workflowText: event.target.value }));
                  }}
                  value={draft.workflowText}
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
                    {saving ? t('Saving...') : editorMode === 'create' ? t('Create Team') : t('Save Team')}
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
              <span className="codex-section-title">{selectedTeam?.displayName || t('Team')}</span>
              {selectedTeam ? (
                <div className="codex-list-row-actions">
                  <button className="codex-section-action" onClick={() => openEditEditor(selectedTeam)} type="button">
                    {t('Edit')}
                  </button>
                  <button
                    className="codex-section-action"
                    onClick={() => { void handleDelete(selectedTeam); }}
                    style={{ color: '#ef4444' }}
                    type="button"
                  >
                    {t('Delete')}
                  </button>
                </div>
              ) : null}
            </div>
          </div>
          {selectedTeam ? (
            <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Team ID')}</span>
                <span className="codex-command-row-desc">{selectedTeam.teamId}</span>
              </div>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Leader')}</span>
                <span className="codex-command-row-desc">
                  {agentMap.get(selectedTeam.leaderAgentId)?.displayName || selectedTeam.leaderAgentId}
                </span>
              </div>
              <div style={{ padding: '12px 16px' }}>
                <div className="codex-list-row-name" style={{ marginBottom: 8 }}>{t('Members')}</div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                  {selectedTeam.memberAgentIds.map((agentId: string) => (
                    <span
                      key={agentId}
                      className="codex-sync-pill ok"
                      style={agentId === selectedTeam.leaderAgentId ? { fontWeight: 500 } : undefined}
                    >
                      {agentMap.get(agentId)?.displayName || agentId}
                    </span>
                  ))}
                </div>
              </div>
              <div style={{ padding: '12px 16px' }}>
                <div className="codex-list-row-name" style={{ marginBottom: 8 }}>{t('Workflow')}</div>
                <div style={{ whiteSpace: 'pre-wrap', fontSize: 13, lineHeight: 1.6, color: 'var(--color-token-text-secondary)' }}>
                  {selectedTeam.workflowText}
                </div>
              </div>
            </div>
          ) : (
            <div className="codex-empty-state">
              {t('Select a team to inspect its leader, members, and workflow.')}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
