import { startTransition, useEffect, useState } from 'react';

import type {
  DesktopAutomationActivityFeed,
  DesktopAutomationSchedule,
  DesktopAutomationSummary,
  DesktopCustomAgent,
  DesktopState,
} from '@shared/contracts';

import { selectedAutomation } from '../thread-model';
import {
  requestDesktopState,
  requestDesktopStateResult,
} from '../pinned-order-ingress';
import { buildStandaloneAgentOptions } from './agent-options';
import type { DesktopRoute } from './desktop-route';
import type {
  AutomationDraft,
  AutomationDialogState,
  ContentView,
  PendingAutomationRun,
} from './types';
import {
  automationAgentIdForMutation,
  generatedAutomationAgentError,
  initialAutomationAgentId,
} from './automation-agent-contract';

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

export function buildAutomationDraft(
  workspaces: DesktopState['workspaces'],
  effectiveDefaultAgentId: string | null,
  automation?: DesktopAutomationSummary | null,
): AutomationDraft {
  const targetThreadId = automation?.targetThreadId?.trim() || '';
  const targetMode = targetThreadId ? 'existing_thread' : 'new_thread';
  return {
    label: automation?.label || '',
    prompt: automation?.prompt || '',
    agentId: initialAutomationAgentId({
      targetMode,
      configuredAgentId: automation?.agentId,
      targetEffectiveAgentId: automation?.effectiveAgentId,
      effectiveDefaultAgentId,
    }),
    agentChanged: false,
    initialTargetMode: targetMode,
    targetEffectiveAgentId: automation?.effectiveAgentId?.trim() || '',
    targetMode,
    targetThreadId,
    workspacePath: automation?.workspacePath || workspaces[0]?.path || '',
    schedule: automation?.schedule || defaultAutomationSchedule(),
  };
}

function automationUnreadTimestamp(automation: DesktopAutomationSummary): string | null {
  return automation.unreadHintTimestamp || automation.lastRunAt || null;
}

type UseAutomationControllerArgs = {
  contentView: ContentView;
  desktopState: DesktopState | null;
  desktopAgents: DesktopCustomAgent[];
  effectiveDefaultAgentId: string | null;
  /**
   * Route-store version probe for the async guard (6c-2a): selections
   * capture it before awaiting the IPC and drop the landing when a newer
   * route committed meanwhile.
   */
  getRouteVersion: () => number;
  /** Route-store navigation seam (replace semantics, 6c-2a). */
  navigateRoute: (route: DesktopRoute) => void;
  /**
   * State-to-hash sync for the SERVER-confirmed automation selection
   * (6c-2c): selectAutomation's response carries the normalized selection
   * (a missing id falls back server-side), and the hash must follow it
   * without re-triggering the application (sync origin).
   */
  syncAutomationRoute: (automationId: string | null) => void;
  pendingThreadBottomSnapRef: React.MutableRefObject<string | null>;
  selectedThreadId: string | null;
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
  effectiveDefaultAgentId,
  getRouteVersion,
  navigateRoute,
  syncAutomationRoute,
  pendingThreadBottomSnapRef,
  selectedThreadId,
  setDesktopState,
  setError,
  setSelectedThreadId,
  setPendingAutomationRun,
  reconcilePendingAutomationRun,
}: UseAutomationControllerArgs) {
  const [, setAutomationActivityById] = useState<
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
  const automationAgentOptions = buildStandaloneAgentOptions(desktopAgents, {
    labelStyle: 'target',
  });
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
    // The view flips through the committed automation route (6c-2b): the
    // rail select and external hashes navigate before this application
    // runs, so the contentView selector is already 'automation'.
    // Async guard (6c-2a): a slow select must not clobber the state a
    // newer navigation installed while this one awaited the IPC.
    const routeVersion = getRouteVersion();
    try {
      const nextState = await requestDesktopState(() =>
        window.garyxDesktop.selectAutomation({ automationId }),
      );
      if (getRouteVersion() !== routeVersion) {
        return;
      }
      setDesktopState(nextState);
      // Server-owned selection: follow the confirmed (possibly
      // normalized) id so the hash converges even after the fold dies.
      syncAutomationRoute(nextState.selectedAutomationId ?? null);
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
    setError(null);
    setAutomationDialog({
      mode,
      automationId: automation?.id,
      draft: buildAutomationDraft(
        automationWorkspaces,
        effectiveDefaultAgentId,
        automation,
      ),
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
    const targetThreadId = automationDialog.draft.targetMode === 'existing_thread'
      ? automationDialog.draft.targetThreadId.trim()
      : '';
    // A thread-bound automation runs under the thread's own agent; the agent
    // picker only applies to generated-thread automations.
    const agentSelectionError = generatedAutomationAgentError(
      automationDialog.mode,
      automationDialog.draft,
      new Set(automationAgentOptions.map((option) => option.id)),
    );
    if (agentSelectionError) {
      setError(agentSelectionError);
      return;
    }
    const mutationAgentId = automationAgentIdForMutation(
      automationDialog.mode,
      automationDialog.draft,
    );
    const workspacePath = automationDialog.draft.workspacePath.trim();
    if (automationDialog.draft.targetMode === 'existing_thread' && !targetThreadId) {
      setError('Choose the thread this automation should post into.');
      return;
    }
    if (automationDialog.draft.targetMode === 'new_thread' && !workspacePath) {
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
        ? await requestDesktopStateResult(() => window.garyxDesktop.createAutomation({
            label,
            prompt,
            agentId: mutationAgentId,
            workspacePath: targetThreadId ? undefined : workspacePath || undefined,
            targetThreadId: targetThreadId || null,
            schedule: automationDialog.draft.schedule,
          }), (response) => response.state)
        : await requestDesktopStateResult(() => window.garyxDesktop.updateAutomation({
            automationId: automationDialog.automationId || '',
            label,
            prompt,
            agentId: mutationAgentId,
            workspacePath: targetThreadId ? undefined : workspacePath || undefined,
            targetThreadId: targetThreadId || null,
            schedule: automationDialog.draft.schedule,
          }), (response) => response.state);
      setDesktopState(result.state);
      setAutomationDialog(null);
      // Navigate to the SERVER-confirmed selection (create selects the
      // new automation, update selects the edited one — review
      // #TASK-1627); a pre-save closure id would go stale here. Equal-
      // route dedupe makes this free when already on that route.
      navigateRoute({
        kind: 'automation',
        automationId: result.state.selectedAutomationId ?? null,
      });
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
      const result = await requestDesktopStateResult(
        () => window.garyxDesktop.updateAutomation({
          automationId: automation.id,
          enabled,
        }),
        (response) => response.state,
      );
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
      const nextState = await requestDesktopState(() =>
        window.garyxDesktop.deleteAutomation({
          automationId: automation.id,
        }),
      );
      setDesktopState(nextState);
      setAutomationActivityById((current) => {
        const next = { ...current };
        delete next[automation.id];
        return next;
      });
      if (selectedThreadId === automation.threadId || selectedThreadId === automation.targetThreadId) {
        setSelectedThreadId(nextState.threads[0]?.id || null);
      }
      // Deleting the selected automation changes the server selection
      // (possibly to null); follow it in the hash (review #TASK-1627).
      if (isAutomationView) {
        syncAutomationRoute(nextState.selectedAutomationId ?? null);
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
      const result = await requestDesktopStateResult(
        () => window.garyxDesktop.runAutomationNow({
          automationId: automation.id,
        }),
        (response) => response.state,
      );
      const latestThreadId = result.activity.threadId || automation.targetThreadId || automation.threadId;
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
        // Selection + draft exit + view flip is the thread-route
        // application (6c-2a).
        navigateRoute({ kind: 'thread', threadId: latestThreadId });
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
    const latestThreadId = (automation.targetThreadId || automation.threadId).trim();
    if (!latestThreadId) {
      setError('This automation has not produced a latest run thread yet. Run it once first.');
      return;
    }

    setError(null);
    navigateRoute({ kind: 'thread', threadId: latestThreadId });
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
          const nextState = await requestDesktopState(() =>
            window.garyxDesktop.markAutomationSeen({
              automationId: activeAutomation.id,
              seenAt: activeAutomationUnreadAt,
            }),
          );
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
