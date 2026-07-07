import { useRef } from 'react';

import type {
  CreateTeamInput,
  DesktopCustomAgent,
  DesktopTeam,
  UpdateTeamInput,
} from '@shared/contracts';

import { Check, Plus, Sparkles, Upload as UploadIcon, X } from 'lucide-react';
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
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import { Textarea } from '../../components/ui/textarea';
import { useI18n } from '../../i18n';
import { AgentAvatarEditor } from './AgentAvatarEditor';
import {
  AGENT_AVATAR_ACCEPT,
  AGENT_AVATAR_DATA_URL_MAX_LENGTH,
  buildSuggestedWorkflow,
  previewText,
} from './agents-hub-helpers';
import type { TeamDialogMode, TeamDraft } from './agents-hub-helpers';

type TeamFormDialogProps = {
  agentMap: Map<string, DesktopCustomAgent>;
  agents: DesktopCustomAgent[];
  avatarGenerating: boolean;
  closeTeamDialog: () => void;
  handleAvatarFileChange: (
    event: React.ChangeEvent<HTMLInputElement>,
    target: 'agent' | 'team',
  ) => Promise<void>;
  loadData: () => Promise<void>;
  onStartThread?: (agentOrTeamId: string) => void;
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
  openEditTeamDialog: (team: DesktopTeam) => void;
  saving: boolean;
  selectedTeam: DesktopTeam | null;
  setAvatarStyleDialogOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setAvatarStyleTarget: React.Dispatch<React.SetStateAction<'agent' | 'team'>>;
  setSaving: React.Dispatch<React.SetStateAction<boolean>>;
  setTeamDraft: React.Dispatch<React.SetStateAction<TeamDraft>>;
  teamDialogMode: TeamDialogMode;
  teamDraft: TeamDraft;
};

export function TeamFormDialog({
  agentMap,
  agents,
  avatarGenerating,
  closeTeamDialog,
  handleAvatarFileChange,
  loadData,
  onStartThread,
  onToast,
  openEditTeamDialog,
  saving,
  selectedTeam,
  setAvatarStyleDialogOpen,
  setAvatarStyleTarget,
  setSaving,
  setTeamDraft,
  teamDialogMode,
  teamDraft,
}: TeamFormDialogProps) {
  const { t } = useI18n();
  const teamAvatarFileInputRef = useRef<HTMLInputElement | null>(null);

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

  async function handleTeamSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const avatarDataUrl = teamDraft.avatarDataUrl.trim();
    if (avatarDataUrl.length > AGENT_AVATAR_DATA_URL_MAX_LENGTH) {
      onToast?.(t('Avatar image is too large.'), 'error');
      return;
    }
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
        avatarDataUrl,
      };

      if (teamDialogMode === 'create') {
        await window.garyxDesktop.createTeam(payload);
        onToast?.(t('Agent team created'), 'success');
      } else {
        const updatePayload: UpdateTeamInput = {
          ...payload,
          currentTeamId: selectedTeam?.teamId || payload.teamId,
          expectedUpdatedAt: selectedTeam?.updatedAt || '',
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

  return (
    <Dialog
      open={Boolean(teamDialogMode)}
      onOpenChange={(open) => {
        if (!open) {
          closeTeamDialog();
        }
      }}
    >
      <DialogContent className="sm:max-w-[720px] team-builder-dialog" size="wide">
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
            <div className="agents-hub-avatar-editor team-builder-avatar-editor">
              <AgentAvatarEditor
                avatarDataUrl={teamDraft.avatarDataUrl}
                className="agents-hub-avatar-centered large"
                label={teamDraft.displayName || teamDraft.teamId || 'T'}
                team
              />
              <div className="agents-hub-avatar-editor-actions">
                <input
                  accept={AGENT_AVATAR_ACCEPT}
                  className="sr-only"
                  onChange={(event) => {
                    void handleAvatarFileChange(event, 'team');
                  }}
                  ref={teamAvatarFileInputRef}
                  type="file"
                />
                <Button
                  onClick={() => teamAvatarFileInputRef.current?.click()}
                  type="button"
                  variant="outline"
                >
                  <UploadIcon aria-hidden size={15} strokeWidth={1.8} />
                  {t('Upload avatar')}
                </Button>
                <Button
                  disabled={avatarGenerating}
                  onClick={() => {
                    setAvatarStyleTarget('team');
                    setAvatarStyleDialogOpen(true);
                  }}
                  type="button"
                  variant="outline"
                >
                  <Sparkles aria-hidden size={15} strokeWidth={1.8} />
                  {avatarGenerating ? t('Generating...') : t('Generate avatar')}
                </Button>
                {teamDraft.avatarDataUrl ? (
                  <Button
                    onClick={() => {
                      setTeamDraft((current) => ({ ...current, avatarDataUrl: '' }));
                    }}
                    type="button"
                    variant="ghost"
                  >
                    {t('Clear')}
                  </Button>
                ) : null}
              </div>
            </div>
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
                        <AgentAvatarEditor
                          agentId={agent.agentId}
                          avatarDataUrl={agent.avatarDataUrl}
                          builtIn={agent.builtIn}
                          className="agents-hub-avatar-centered small"
                          label={agent.displayName || agent.agentId}
                          providerIcon={agent.providerIcon}
                          providerType={agent.providerType}
                        />
                        <div className="team-builder-agent-info">
                          <span className="team-builder-agent-name">{agent.displayName}</span>
                          <span className="team-builder-agent-desc">
                            {previewText(agent.systemPrompt, agent.builtIn ? t('Built-in agent') : t('Custom agent'))}
                          </span>
                        </div>
                        <span className={`team-builder-toggle-btn ${selected ? 'checked' : ''}`}>
                          {selected
                            ? <Check aria-hidden size={14} strokeWidth={2.5} />
                            : <Plus aria-hidden size={14} strokeWidth={2} />}
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
                        <AgentAvatarEditor
                          agentId={agent?.agentId || agentId}
                          avatarDataUrl={agent?.avatarDataUrl}
                          builtIn={agent?.builtIn}
                          className="agents-hub-avatar-centered small"
                          label={agent?.displayName || agentId}
                          providerIcon={agent?.providerIcon}
                          providerType={agent?.providerType}
                        />
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
                            <X aria-hidden size={14} strokeWidth={2} />
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
              <AgentAvatarEditor
                avatarDataUrl={selectedTeam?.avatarDataUrl}
                className="agents-hub-avatar-centered large"
                label={selectedTeam?.displayName || selectedTeam?.teamId || 'T'}
                team
              />
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
  );
}
