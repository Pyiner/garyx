import { startTransition, useEffect, useState } from 'react';

import type {
  DesktopAutomationActivityFeed,
  DesktopAutomationSchedule,
  DesktopAutomationSummary,
  DesktopCustomAgent,
  DesktopState,
  DesktopTeam,
} from '@shared/contracts';

import { selectedAutomation } from '../thread-model';
import type {
  AutomationDraft,
  AutomationAgentOption,
  AutomationDialogState,
  ContentView,
  PendingAutomationRun,
} from './types';

function isValidOnceScheduleInput(value: string): boolean {
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value.trim());
}

function defaultAutomationSchedule(): DesktopAutomationSchedule {
  return {
    kind: 'daily',
    time: '09:00',
    weekdays: ['mo', 'tu', 'we', 'th', 'fr'],
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
  };
}

function buildAutomationDraft(
  workspaces: DesktopState['workspaces'],
  agents: DesktopCustomAgent[],
  automation?: DesktopAutomationSummary | null,
): AutomationDraft {
  const standaloneAgents = agents.filter((agent) => agent.standalone);
  const defaultAgentId = standaloneAgents.find((agent) => agent.agentId === 'claude')?.agentId
    || standaloneAgents[0]?.agentId
    || 'claude';
  return {
    label: automation?.label || '',
    prompt: automation?.prompt || '',
    agentId: automation?.agentId || defaultAgentId,
    workspacePath: automation?.workspacePath || workspaces[0]?.path || '',
    schedule: automation?.schedule || defaultAutomationSchedule(),
  };
}

function formatAutomationAgentOptions(
  agents: DesktopCustomAgent[],
  teams: DesktopTeam[],
): AutomationAgentOption[] {
  const teamOptions = [...teams]
    .sort((left, right) => {
      return left.displayName.localeCompare(right.displayName) || left.teamId.localeCompare(right.teamId);
    })
    .map((team) => ({
      id: team.teamId,
      label:
        team.displayName.trim() === team.teamId.trim()
          ? `${team.displayName} (team)`
          : `${team.displayName} (${team.teamId}, team)`,
      kind: 'team' as const,
    }));
  const agentOptions = agents
    .filter((agent) => agent.standalone)
    .sort((left, right) => {
      if (left.builtIn !== right.builtIn) {
        return left.builtIn ? -1 : 1;
      }
      return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
    })
    .map((agent) => ({
      id: agent.agentId,
      label:
        agent.displayName.trim() === agent.agentId.trim()
          ? `${agent.displayName} (agent${agent.builtIn ? ', built-in' : ''})`
          : `${agent.displayName} (${agent.agentId}, agent${agent.builtIn ? ', built-in' : ''})`,
      kind: 'agent' as const,
    }));
  return [...teamOptions, ...agentOptions];
}

function automationUnreadTimestamp(automation: DesktopAutomationSummary): string | null {
  return automation.unreadHintTimestamp || automation.lastRunAt || null;
}

type UseAutomationControllerArgs = {
  contentView: ContentView;
  desktopState: DesktopState | null;
  desktopAgents: DesktopCustomAgent[];
  desktopTeams: DesktopTeam[];
  pendingThreadBottomSnapRef: React.MutableRefObject<string | null>;
  selectedThreadId: string | null;
  setContentView: React.Dispatch<React.SetStateAction<ContentView>>;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setNewThreadDraftActive: React.Dispatch<React.SetStateAction<boolean>>;
  setSelectedThreadId: React.Dispatch<React.SetStateAction<string | null>>;
  setPendingAutomationRun: (threadId: string, run: PendingAutomationRun | null) => void;
  reconcilePendingAutomationRun: (threadId: string, run: PendingAutomationRun) => void;
};

export function useAutomationController({
  contentView,
  desktopState,
  desktopAgents,
  desktopTeams,
  pendingThreadBottomSnapRef,
  selectedThreadId,
  setContentView,
  setDesktopState,
  setError,
  setNewThreadDraftActive,
  setSelectedThreadId,
  setPendingAutomationRun,
  reconcilePendingAutomationRun,
}: UseAutomationControllerArgs) {
  const [automationActivityById, setAutomationActivityById] = useState<
    Record<string, DesktopAutomationActivityFeed>
  >({});
  const [automationActivityLoadingId, setAutomationActivityLoadingId] = useState<string | null>(
    null,
  );
  const [automationDialog, setAutomationDialog] = useState<AutomationDialogState | null>(null);
  const [automationMutation, setAutomationMutation] = useState<string | null>(null);
  const [automationStatus, setAutomationStatus] = useState<string | null>(null);

  const isAutomationView = contentView === 'automation';
  const automations = desktopState?.automations || [];
  const selectedAutomationId = desktopState?.selectedAutomationId || null;
  const activeAutomation = selectedAutomation(desktopState, selectedAutomationId);
  const automationAgentOptions = formatAutomationAgentOptions(desktopAgents, desktopTeams);
  const automationWorkspaces = (desktopState?.workspaces || []).filter((workspace) => {
    return Boolean(workspace.path) && workspace.available;
  });
  const activeAutomationUnreadAt = activeAutomation ? automationUnreadTimestamp(activeAutomation) : null;
  const activeAutomationSeenAt = activeAutomation
    ? desktopState?.lastSeenRunAtByAutomation?.[activeAutomation.id] || null
    : null;

  async function loadAutomationActivity(automationId: string) {
    const feed = await window.garyxDesktop.getAutomationActivity(automationId);
    startTransition(() => {
      setAutomationActivityById((current) => ({
        ...current,
        [automationId]: feed,
      }));
    });
    return feed;
  }

  async function handleSelectAutomation(automationId: string | null) {
    setError(null);
    setContentView('automation');
    try {
      const nextState = await window.garyxDesktop.selectAutomation({ automationId });
      setDesktopState(nextState);
    } catch (selectionError) {
      setError(
        selectionError instanceof Error
          ? selectionError.message
          : 'Failed to select the automation',
      );
    }
  }

  function openAutomationDialog(
    mode: 'create' | 'edit',
    automation?: DesktopAutomationSummary | null,
  ) {
    if (!automationAgentOptions.length && mode === 'create') {
      setError('Add or restore an agent or team before creating an automation.');
      return;
    }

    setError(null);
    setAutomationDialog({
      mode,
      automationId: automation?.id,
      draft: buildAutomationDraft(automationWorkspaces, desktopAgents, automation),
    });
  }

  function updateAutomationDialogDraft(mutator: (draft: AutomationDraft) => AutomationDraft) {
    setAutomationDialog((current) => {
      if (!current) {
        return current;
      }
      return {
        ...current,
        draft: mutator(current.draft),
      };
    });
  }

  async function handleSubmitAutomationDialog(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!automationDialog) {
      return;
    }

    const label = automationDialog.draft.label.trim();
    const prompt = automationDialog.draft.prompt.trim();
    if (!label) {
      setError('Automation name is required.');
      return;
    }
    if (!prompt) {
      setError('Automation prompt is required.');
      return;
    }
    if (!automationDialog.draft.agentId.trim()) {
      setError('Choose an agent or team for this automation.');
      return;
    }
    const workspacePath = automationDialog.draft.workspacePath.trim();
    if (!workspacePath) {
      setError('Choose a directory for this automation.');
      return;
    }
    if (
      automationDialog.draft.schedule.kind === 'interval'
      && automationDialog.draft.schedule.hours <= 0
    ) {
      setError('Interval hours must be greater than zero.');
      return;
    }
    if (
      automationDialog.draft.schedule.kind === 'once'
      && !isValidOnceScheduleInput(automationDialog.draft.schedule.at)
    ) {
      setError('Choose a valid one-time run date and time.');
      return;
    }

    const mutationKey = automationDialog.mode === 'create'
      ? 'create'
      : `edit:${automationDialog.automationId || ''}`;
    setAutomationMutation(mutationKey);
    setError(null);

    try {
      const result = automationDialog.mode === 'create'
          ? await window.garyxDesktop.createAutomation({
            label,
            prompt,
            agentId: automationDialog.draft.agentId,
            workspacePath,
            schedule: automationDialog.draft.schedule,
          })
        : await window.garyxDesktop.updateAutomation({
            automationId: automationDialog.automationId || '',
            label,
            prompt,
            agentId: automationDialog.draft.agentId,
            workspacePath,
            schedule: automationDialog.draft.schedule,
          });
      setDesktopState(result.state);
      setAutomationDialog(null);
      setContentView('automation');
    } catch (automationError) {
      setError(
        automationError instanceof Error
          ? automationError.message
          : 'Failed to save the automation',
      );
    } finally {
      setAutomationMutation((current) => (current === mutationKey ? null : current));
    }
  }

  async function handleToggleAutomationEnabled(
    automation: DesktopAutomationSummary,
    enabled: boolean,
  ) {
    const mutationKey = `toggle:${automation.id}`;
    setAutomationMutation(mutationKey);
    setError(null);
    try {
      const result = await window.garyxDesktop.updateAutomation({
        automationId: automation.id,
        enabled,
      });
      setDesktopState(result.state);
    } catch (automationError) {
      setError(
        automationError instanceof Error
          ? automationError.message
          : 'Failed to update the automation',
      );
    } finally {
      setAutomationMutation((current) => (current === mutationKey ? null : current));
    }
  }

  async function handleDeleteAutomation(automation?: DesktopAutomationSummary | null) {
    if (!automation) {
      return;
    }

    const mutationKey = `delete:${automation.id}`;
    setAutomationMutation(mutationKey);
    setError(null);
    try {
      const nextState = await window.garyxDesktop.deleteAutomation({
        automationId: automation.id,
      });
      setDesktopState(nextState);
      setAutomationActivityById((current) => {
        const next = { ...current };
        delete next[automation.id];
        return next;
      });
      if (selectedThreadId === automation.threadId) {
        setSelectedThreadId(nextState.threads[0]?.id || null);
      }
    } catch (automationError) {
      setError(
        automationError instanceof Error
          ? automationError.message
          : 'Failed to delete the automation',
      );
    } finally {
      setAutomationMutation((current) => (current === mutationKey ? null : current));
    }
  }

  async function handleRunAutomationNow(automation?: DesktopAutomationSummary | null) {
    if (!automation) {
      return;
    }

    const mutationKey = `run:${automation.id}`;
    setAutomationMutation(mutationKey);
    setError(null);
    setAutomationStatus(null);
    try {
      const result = await window.garyxDesktop.runAutomationNow({
        automationId: automation.id,
      });
      const latestThreadId = result.activity.threadId || automation.threadId;
      setDesktopState({
        ...result.state,
        automations: result.state.automations.map((entry) => {
          if (entry.id !== automation.id) {
            return entry;
          }

          return {
            ...entry,
            lastRunAt: result.activity.startedAt,
            lastStatus: result.activity.status,
            unreadHintTimestamp: result.activity.finishedAt || result.activity.startedAt,
            threadId: latestThreadId || entry.threadId,
          };
        }),
      });
      setAutomationActivityById((current) => {
        const existing = current[automation.id];
        return {
          ...current,
          [automation.id]: {
            automationId: automation.id,
            threadId: latestThreadId,
            count: Math.max(1, existing?.count || 0),
            items: [
              result.activity,
              ...(existing?.items || []).filter((entry) => entry.runId !== result.activity.runId),
            ],
          },
        };
      });
      if (latestThreadId) {
        const pendingRun: PendingAutomationRun = {
          threadId: latestThreadId,
          runId:
            result.activity.runId || `automation:${automation.id}:${result.activity.startedAt}`,
          prompt: automation.prompt,
        };
        setPendingAutomationRun(latestThreadId, pendingRun);
        reconcilePendingAutomationRun(latestThreadId, pendingRun);
        pendingThreadBottomSnapRef.current = latestThreadId;
        setNewThreadDraftActive(false);
        setSelectedThreadId(latestThreadId);
        setContentView('thread');
      }
      setAutomationStatus(`Ran ${automation.label} just now.`);
      window.setTimeout(() => {
        void loadAutomationActivity(automation.id).catch(() => {});
      }, 350);
    } catch (automationError) {
      setError(
        automationError instanceof Error
          ? automationError.message
          : 'Failed to run the automation',
      );
    } finally {
      setAutomationMutation((current) => (current === mutationKey ? null : current));
    }
  }

  async function handleOpenAutomationThread(automation?: DesktopAutomationSummary | null) {
    if (!automation) {
      return;
    }
    const latestThreadId = automation.threadId.trim();
    if (!latestThreadId) {
      setError('This automation has not produced a latest run thread yet. Run it once first.');
      return;
    }

    setError(null);
    setNewThreadDraftActive(false);
    setSelectedThreadId(latestThreadId);
    setContentView('thread');
  }

  useEffect(() => {
    if (!isAutomationView || !activeAutomation) {
      return;
    }

    let cancelled = false;
    setAutomationActivityLoadingId(activeAutomation.id);
    void (async () => {
      try {
        await loadAutomationActivity(activeAutomation.id);
        if (
          !cancelled
          && activeAutomationUnreadAt
          && (!activeAutomationSeenAt || activeAutomationUnreadAt > activeAutomationSeenAt)
        ) {
          const nextState = await window.garyxDesktop.markAutomationSeen({
            automationId: activeAutomation.id,
            seenAt: activeAutomationUnreadAt,
          });
          if (!cancelled) {
            startTransition(() => {
              setDesktopState(nextState);
            });
          }
        }
      } catch (activityError) {
        if (!cancelled) {
          setError(
            activityError instanceof Error
              ? activityError.message
              : 'Failed to load automation activity',
          );
        }
      } finally {
        if (!cancelled) {
          setAutomationActivityLoadingId((current) => {
            return current === activeAutomation.id ? null : current;
          });
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    activeAutomation,
    activeAutomationSeenAt,
    activeAutomationUnreadAt,
    isAutomationView,
    setDesktopState,
    setError,
  ]);

  return {
    activeAutomation,
    automationActivityLoadingId,
    automationDialog,
    automationMutation,
    automationStatus,
    automationAgentOptions,
    automationWorkspaces,
    automations,
    handleDeleteAutomation,
    handleOpenAutomationThread,
    handleRunAutomationNow,
    handleSelectAutomation,
    handleSubmitAutomationDialog,
    handleToggleAutomationEnabled,
    openAutomationDialog,
    selectedAutomationId,
    setAutomationDialog,
    setAutomationStatus,
    updateAutomationDialogDraft,
  };
}
