import type {
  DesktopAutomationSummary,
  DesktopChannelEndpoint,
  DesktopThreadSummary,
  DesktopState,
  DesktopWorkspace,
  ThreadTeamBlock,
} from '@shared/contracts';

export interface WorkspaceThreadGroup {
  workspace: DesktopWorkspace;
  threads: DesktopThreadSummary[];
  automationCount: number;
  status: string | null;
  preferredThreadId: string | null;
  isSelected: boolean;
  canManageWorkspace: boolean;
}

export interface WorkspaceThreadRow {
  thread: DesktopThreadSummary;
  isActive: boolean;
  isDeleting: boolean;
  isBusy: boolean;
  isAutomationThread: boolean;
  deleteDisabled: boolean;
  deleteTitle?: string;
}

export function endpointThreadTitle(
  state: DesktopState | null,
  endpoint: DesktopChannelEndpoint,
): string {
  if (!endpoint.threadId) {
    return 'Unbound';
  }
  return endpoint.threadLabel?.trim()
    || state?.threads.find((thread) => thread.id === endpoint.threadId)?.title
    || state?.sessions.find((session) => session.id === endpoint.threadId)?.title
    || endpoint.threadId;
}

export function mergeThread(
  threads: DesktopThreadSummary[],
  thread: DesktopThreadSummary,
): DesktopThreadSummary[] {
  return [thread, ...threads.filter((entry) => entry.id !== thread.id)].sort((left, right) => {
    return Date.parse(right.updatedAt) - Date.parse(left.updatedAt);
  });
}

/**
 * Structural equality for two team blocks. Used by `applyRemoteTranscript`
 * to avoid re-assigning an identical block (which would churn React identity
 * and re-trigger dependent effects).
 *
 * The static team fields (`team_id`, `display_name`, `leader_agent_id`,
 * `member_agent_ids`) change very rarely; `child_thread_ids` grows as
 * sub-agents get dispatched, so it's the field most worth diffing cheaply.
 */
export function teamBlocksEqual(
  left: ThreadTeamBlock | null,
  right: ThreadTeamBlock | null,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }
  if (
    left.team_id !== right.team_id
    || left.display_name !== right.display_name
    || left.leader_agent_id !== right.leader_agent_id
  ) {
    return false;
  }
  if (left.member_agent_ids.length !== right.member_agent_ids.length) {
    return false;
  }
  for (let index = 0; index < left.member_agent_ids.length; index += 1) {
    if (left.member_agent_ids[index] !== right.member_agent_ids[index]) {
      return false;
    }
  }
  const leftKeys = Object.keys(left.child_thread_ids);
  const rightKeys = Object.keys(right.child_thread_ids);
  if (leftKeys.length !== rightKeys.length) {
    return false;
  }
  for (const key of leftKeys) {
    if (left.child_thread_ids[key] !== right.child_thread_ids[key]) {
      return false;
    }
  }
  return true;
}

export function automationForLatestThread(
  state: DesktopState | null,
  threadId: string | null,
): DesktopAutomationSummary | null {
  if (!state || !threadId) {
    return null;
  }
  return state.automations.find((entry) => entry.threadId === threadId) || null;
}

export function latestAutomationThreadSummary(
  state: DesktopState | null,
  threadId: string,
): DesktopThreadSummary | null {
  const automation = automationForLatestThread(state, threadId);
  if (!automation) {
    return null;
  }

  return {
    id: automation.threadId,
    title: automation.label,
    createdAt: automation.lastRunAt || automation.nextRun,
    updatedAt: automation.lastRunAt || automation.nextRun,
    lastMessagePreview: automation.prompt,
    workspacePath: automation.workspacePath,
  };
}

export function selectedThread(
  state: DesktopState | null,
  threadId: string | null,
): DesktopThreadSummary | null {
  if (!state || !threadId) {
    return null;
  }
  return (
    state.threads.find((entry) => entry.id === threadId) ||
    latestAutomationThreadSummary(state, threadId) ||
    null
  );
}

export function selectedAutomation(
  state: DesktopState | null,
  automationId: string | null,
): DesktopAutomationSummary | null {
  if (!state || !automationId) {
    return null;
  }
  return state.automations.find((entry) => entry.id === automationId) || null;
}

export function selectedWorkspace(
  state: DesktopState | null,
  workspacePath: string | null,
): DesktopWorkspace | null {
  if (!state || !workspacePath) {
    return null;
  }
  const pathKey = workspacePath.trim().toLowerCase();
  return state.workspaces.find((entry) => entry.path?.trim().toLowerCase() === pathKey) || null;
}

export function workspaceForThread(
  state: DesktopState | null,
  threadId: string | null,
): DesktopWorkspace | null {
  const thread = selectedThread(state, threadId);
  if (!thread) {
    return null;
  }
  return selectedWorkspace(state, thread.workspacePath || null);
}

export function isAvailableWorkspace(
  workspace: DesktopWorkspace | null | undefined,
): workspace is DesktopWorkspace {
  return Boolean(workspace?.available);
}

export function isSelectableNewThreadWorkspace(
  workspace: DesktopWorkspace | null | undefined,
): workspace is DesktopWorkspace {
  return Boolean(workspace?.available && workspace?.path);
}

export function pickPreferredWorkspace(
  workspaces: DesktopWorkspace[],
  ...candidates: Array<DesktopWorkspace | null | undefined>
): DesktopWorkspace | null {
  return candidates.find(isAvailableWorkspace)
    || workspaces.find((workspace) => workspace.available)
    || candidates.find((workspace): workspace is DesktopWorkspace => Boolean(workspace))
    || workspaces[0]
    || null;
}

export function buildWorkspaceThreadGroups(input: {
  state: DesktopState | null;
  activeThread: DesktopThreadSummary | null;
  selectedThreadId: string | null;
  workspaceSelectionEntry: DesktopWorkspace | null;
}): WorkspaceThreadGroup[] {
  if (!input.state) {
    return [];
  }

  return input.state.workspaces.map((workspace) => {
    const workspacePath = workspace.path || '';
    const workspacePathKey = workspacePath.trim().toLowerCase();
    const threads = input.state!.threads.filter((thread) => {
      return (thread.workspacePath || '').trim().toLowerCase() === workspacePathKey;
    });
    const automationCount = input.state!.automations.filter((automation) => {
      return automation.workspacePath.trim().toLowerCase() === workspacePathKey;
    }).length;
    const canManageWorkspace = false;

    return {
      workspace,
      threads,
      automationCount,
      status: !workspace.available ? 'Unavailable' : null,
      preferredThreadId:
        (input.activeThread?.workspacePath || '').trim().toLowerCase() === workspacePathKey
          ? input.selectedThreadId
          : threads[0]?.id || null,
      isSelected:
        (input.workspaceSelectionEntry?.path || '').trim().toLowerCase() === workspacePathKey,
      canManageWorkspace,
    };
  });
}

export interface SubAgentThreadLink {
  agentId: string;
  threadId: string;
}

export interface ThreadTeamView {
  isTeam: boolean;
  teamDisplayName: string | undefined;
  subAgentThreads: SubAgentThreadLink[];
}

/**
 * Derive team-branding info for a thread from its summary.
 *
 * The full `team` block (including `child_thread_ids`) flows from the gateway
 * through `DesktopThreadSummary.team` and is now supplied by both the list
 * endpoint (`/api/threads`) and the detail endpoints. The `teamId` hint is
 * kept as a belt-and-suspenders fallback for snapshots that were cached
 * before the list endpoint started emitting `team` (and for any future
 * entry point that only populates the hints).
 *
 * Sub-agent peek tabs require the full `team.child_thread_ids` map, so they
 * only light up once at least one sub-agent has been dispatched.
 */
export function deriveThreadTeamView(
  summary: DesktopThreadSummary | null | undefined,
): ThreadTeamView {
  if (!summary) {
    return { isTeam: false, teamDisplayName: undefined, subAgentThreads: [] };
  }
  const teamBlock: ThreadTeamBlock | null | undefined = summary.team;
  const hasTeamBlock = Boolean(teamBlock && teamBlock.team_id);
  const hasTeamIdHint = Boolean(summary.teamId && summary.teamId.trim());
  const isTeam = hasTeamBlock || hasTeamIdHint;
  const teamDisplayName =
    (teamBlock?.display_name && teamBlock.display_name.trim()) ||
    (summary.teamName && summary.teamName.trim()) ||
    undefined;
  const subAgentThreads: SubAgentThreadLink[] = teamBlock
    ? Object.entries(teamBlock.child_thread_ids || {})
        .filter(([agentId, threadId]) => Boolean(agentId) && Boolean(threadId))
        .map(([agentId, threadId]) => ({ agentId, threadId }))
        // Deterministic order by agentId to prevent flicker until the Group's
        // own ordering becomes available through the detail endpoint.
        .sort((left, right) => left.agentId.localeCompare(right.agentId))
    : [];
  return { isTeam, teamDisplayName, subAgentThreads };
}

export function buildWorkspaceThreadRows(input: {
  state: DesktopState | null;
  threads: DesktopThreadSummary[];
  selectedThreadId: string | null;
  deletingThreadId: string | null;
  isThreadRuntimeBusy: (threadId: string) => boolean;
}): WorkspaceThreadRow[] {
  // A thread is considered "bound" only while its bot is still enabled. Once
  // the bot is deleted or disabled, the endpoint becomes effectively orphaned,
  // and the thread should be deletable again — otherwise there is no way to
  // clean up transcripts left behind by inactive bots.
  const liveBotKeys = new Set(
    (input.state?.configuredBots ?? []).map((bot) => `${bot.channel}::${bot.accountId}`),
  );
  const boundThreadIds = new Set(
    (input.state?.endpoints ?? [])
      .filter((ep) => ep.threadId && liveBotKeys.has(`${ep.channel}::${ep.accountId}`))
      .map((ep) => ep.threadId as string),
  );

  return input.threads.map((thread) => {
    const isAutomationThread = Boolean(automationForLatestThread(input.state, thread.id));
    const isDeleting = input.deletingThreadId === thread.id;
    const isBusy = input.isThreadRuntimeBusy(thread.id);
    const hasBotBinding = boundThreadIds.has(thread.id);
    const deleteDisabled =
      isDeleting || isBusy || isAutomationThread || hasBotBinding;

    return {
      thread,
      isActive: thread.id === input.selectedThreadId,
      isDeleting,
      isBusy,
      isAutomationThread,
      deleteDisabled,
    };
  });
}
