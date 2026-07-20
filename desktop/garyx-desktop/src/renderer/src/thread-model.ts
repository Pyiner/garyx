import type {
  DesktopAutomationSummary,
  DesktopChannelEndpoint,
  DesktopThreadSummary,
  DesktopState,
  DesktopWorkspace,
  DraftWorkspaceSelection,
  ThreadWorktreeInfo,
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

function worktreeInfosEqual(
  left: ThreadWorktreeInfo | null | undefined,
  right: ThreadWorktreeInfo | null | undefined,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return !left && !right;
  }
  return (
    (left.mode ?? null) === (right.mode ?? null)
    && (left.enabled ?? null) === (right.enabled ?? null)
    && (left.branch ?? null) === (right.branch ?? null)
    && (left.sourceBranch ?? null) === (right.sourceBranch ?? null)
    && (left.path ?? null) === (right.path ?? null)
    && (left.worktreeDir ?? null) === (right.worktreeDir ?? null)
    && (left.sourceWorkspaceDir ?? null) === (right.sourceWorkspaceDir ?? null)
    && (left.sourceRepoRoot ?? null) === (right.sourceRepoRoot ?? null)
  );
}

/**
 * Structural equality for thread summaries. Transcript caching uses this to
 * keep `desktopState` referentially stable when a re-fetched summary carries
 * no new information; effects keyed on `desktopState` identity must not
 * re-fire for idempotent cache writes.
 */
export function threadSummariesEquivalent(
  left: DesktopThreadSummary,
  right: DesktopThreadSummary,
): boolean {
  return (
    left.id === right.id
    && left.title === right.title
    && (left.threadType ?? null) === (right.threadType ?? null)
    && left.createdAt === right.createdAt
    && left.updatedAt === right.updatedAt
    && left.lastMessagePreview === right.lastMessagePreview
    && (left.workspacePath ?? null) === (right.workspacePath ?? null)
    && (left.messageCount ?? null) === (right.messageCount ?? null)
    && (left.agentId ?? null) === (right.agentId ?? null)
    && (left.recentRunId ?? null) === (right.recentRunId ?? null)
    && worktreeInfosEqual(left.worktree, right.worktree)
  );
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
    state.sessions.find((entry) => entry.id === threadId) ||
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

function workspaceNameFromPath(path: string): string {
  const trimmed = path.trim().replace(/[\\/]+$/, '');
  if (!trimmed) {
    return 'Workspace';
  }
  const segments = trimmed.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] || trimmed;
}

export function workspaceSuggestionFromPath(
  path?: string | null,
  timestamps?: { createdAt?: string | null; updatedAt?: string | null },
): DesktopWorkspace | null {
  const workspacePath = path?.trim() || '';
  if (!workspacePath) {
    return null;
  }
  const createdAt =
    timestamps?.createdAt?.trim() ||
    timestamps?.updatedAt?.trim() ||
    '1970-01-01T00:00:00.000Z';
  return {
    name: workspaceNameFromPath(workspacePath),
    path: workspacePath,
    kind: 'local',
    createdAt,
    updatedAt: timestamps?.updatedAt?.trim() || createdAt,
    available: true,
    managed: true,
    pinned: false,
    threadCount: 0,
    lastActivityAt: null,
    gitRepo: false,
  };
}

export function isAvailableWorkspace(
  workspace: DesktopWorkspace | null | undefined,
): workspace is DesktopWorkspace {
  return Boolean(workspace?.available);
}

export function isSelectableNewThreadWorkspace(
  workspace: DesktopWorkspace | null | undefined,
): workspace is DesktopWorkspace {
  return Boolean(workspace?.available && workspace?.path && workspace.kind === 'local');
}

function workspacePathKey(path?: string | null): string {
  return path?.trim().toLowerCase() || '';
}

function isManagedWorktreePath(path?: string | null): boolean {
  const normalized = workspacePathKey(path).replace(/\\/g, '/');
  return normalized.includes('/.garyx/worktrees/') ||
    normalized.includes('/.codex/worktrees/');
}

function worktreeWorkspacePathKeys(threads: DesktopThreadSummary[]): Set<string> {
  const keys = new Set<string>();
  for (const thread of threads) {
    if (!thread.worktree) {
      continue;
    }
    for (const path of [
      thread.workspacePath,
      thread.worktree.worktreeDir,
      thread.worktree.path,
    ]) {
      const key = workspacePathKey(path);
      if (key) {
        keys.add(key);
      }
    }
  }
  return keys;
}

/**
 * Resolve the default draft workspace once, at draft creation. Latest
 * thread activity wins; workspaces without activity fall back to the
 * gateway's total order (the list is server-sorted); an empty list means
 * an implicit No-workspace draft. Never called on list refresh — a live
 * draft's selection must not drift.
 */
export function resolveDefaultDraftWorkspace(
  workspaces: DesktopWorkspace[],
): DraftWorkspaceSelection {
  const candidates = workspaces.filter(
    (workspace) => workspace.available && Boolean(workspace.path),
  );
  if (candidates.length === 0) {
    return { kind: 'none' };
  }
  let best = candidates[0];
  for (const workspace of candidates.slice(1)) {
    const bestActivity = best.lastActivityAt || '';
    const activity = workspace.lastActivityAt || '';
    if (activity > bestActivity) {
      best = workspace;
    }
  }
  return { kind: 'path', path: best.path as string };
}

export function visibleWorkspaceList(state: DesktopState | null): DesktopWorkspace[] {
  if (!state) {
    return [];
  }

  const worktreeKeys = worktreeWorkspacePathKeys(state.threads);
  return state.workspaces.filter((workspace) => {
    const key = workspacePathKey(workspace.path);
    return Boolean(
      workspace.kind === 'local' &&
        key &&
        !workspace.managed &&
        !isManagedWorktreePath(workspace.path) &&
        !worktreeKeys.has(key),
    );
  });
}

export function newThreadWorkspaceOptions(
  savedWorkspaces: DesktopWorkspace[],
): DesktopWorkspace[] {
  const options: DesktopWorkspace[] = [];
  const seenKeys = new Set<string>();
  const append = (workspace: DesktopWorkspace | null | undefined) => {
    if (!isSelectableNewThreadWorkspace(workspace)) {
      return;
    }
    const key = workspacePathKey(workspace.path);
    if (!key || seenKeys.has(key)) {
      return;
    }
    seenKeys.add(key);
    options.push(workspace);
  };

  for (const workspace of savedWorkspaces) {
    append(workspace);
  }

  return options;
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

  const threadsByWorkspacePath = new Map<string, DesktopThreadSummary[]>();
  for (const thread of input.state.threads) {
    // Server-derived membership: worktree threads group under their source
    // workspace, implicit threads under none. workspacePath is only a
    // fallback for rows from gateways predating root_workspace_path.
    const key = workspacePathKey(thread.rootWorkspacePath ?? thread.workspacePath);
    if (!key) {
      continue;
    }
    const threads = threadsByWorkspacePath.get(key);
    if (threads) {
      threads.push(thread);
    } else {
      threadsByWorkspacePath.set(key, [thread]);
    }
  }

  const automationCountByWorkspacePath = new Map<string, number>();
  for (const automation of input.state.automations) {
    const key = workspacePathKey(automation.workspacePath);
    if (!key) {
      continue;
    }
    automationCountByWorkspacePath.set(
      key,
      (automationCountByWorkspacePath.get(key) || 0) + 1,
    );
  }

  return visibleWorkspaceList(input.state).map((workspace) => {
    const workspacePath = workspace.path || '';
    const workspacePathKey = workspacePath.trim().toLowerCase();
    const threads = threadsByWorkspacePath.get(workspacePathKey) || [];
    const automationCount = automationCountByWorkspacePath.get(workspacePathKey) || 0;

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
      canManageWorkspace: true,
    };
  });
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

/**
 * True when the desktop state already knows this thread id (threads,
 * hidden sessions, or an automation's bound thread). Pure helper shared
 * by the route bridge and the mirror's transcript lifecycle.
 */
export function isKnownThreadId(
  state: DesktopState | null,
  threadId: string | null,
): boolean {
  if (!state || !threadId) {
    return false;
  }
  return (
    state.threads.some((thread) => thread.id === threadId) ||
    state.sessions.some((thread) => thread.id === threadId) ||
    state.automations.some((automation) => automation.threadId === threadId)
  );
}
