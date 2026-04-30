import type {
  AddWorkspaceByPathInput,
  DesktopBotConsoleSummary,
  DesktopChannelEndpoint,
  DesktopState,
  WorkspaceMutationResult,
} from '@shared/contracts';

import { buildBotGroups, primaryBotEndpoint } from './bot-console-model';

export interface BotConsolePlatform {
  getState: () => Promise<DesktopState>;
  addWorkspaceByPath: (
    input: AddWorkspaceByPathInput,
  ) => Promise<WorkspaceMutationResult>;
}

async function refreshBotState(
  platform: BotConsolePlatform,
  onState: (state: DesktopState) => void,
): Promise<DesktopState | null> {
  try {
    const nextState = await platform.getState();
    onState(nextState);
    return nextState;
  } catch {
    return null;
  }
}

function resolveBotGroupById(
  desktopState: DesktopState | null,
  groupId: string,
): DesktopBotConsoleSummary | null {
  if (!desktopState) {
    return null;
  }
  return buildBotGroups(
    desktopState.endpoints || [],
    desktopState.configuredBots || [],
    desktopState.botMainThreads || {},
    desktopState.botConsoles || [],
  ).find((group) => group.id === groupId) || null;
}

export function resolveBotWorkspacePath(
  desktopState: DesktopState | null,
  group: DesktopBotConsoleSummary,
): string | null {
  if (!group.workspaceDir) {
    return null;
  }
  const dir = group.workspaceDir.toLowerCase();
  const allWorkspaces = desktopState?.workspaces || [];
  const match = allWorkspaces.find(
    (workspace) => workspace.path && workspace.path.toLowerCase() === dir,
  );
  return match?.path ?? null;
}

export async function ensureBotWorkspacePath(
  platform: BotConsolePlatform,
  desktopState: DesktopState | null,
  group: DesktopBotConsoleSummary,
  onState: (state: DesktopState) => void,
): Promise<string | null> {
  const existing = resolveBotWorkspacePath(desktopState, group);
  if (existing) {
    return existing;
  }
  if (!group.workspaceDir) {
    return null;
  }
  try {
    const result = await platform.addWorkspaceByPath({ path: group.workspaceDir });
    if (result.workspace) {
      onState(result.state);
      return result.workspace.path;
    }
  } catch (error) {
    throw new Error(
      error instanceof Error
        ? error.message
        : 'Bot directory is unavailable.',
    );
  }
  throw new Error('Bot directory is unavailable.');
}

function botWorkspaceErrorMessage(group: DesktopBotConsoleSummary, error: unknown): string {
  const reason = error instanceof Error ? error.message : String(error || 'Unknown error');
  return group.workspaceDir
    ? `Bot directory is unavailable: ${group.workspaceDir}. ${reason}`
    : reason;
}

function resolveMainThreadId(group: DesktopBotConsoleSummary): {
  mainThreadId: string | null;
  primaryEndpoint: DesktopChannelEndpoint | null;
} {
  const primaryEndpoint = primaryBotEndpoint(group);
  const mainThreadId = group.rootBehavior === 'expand_only'
    ? (group.mainThreadId || primaryEndpoint?.threadId || null)
    : (group.defaultOpenThreadId || group.mainThreadId || primaryEndpoint?.threadId || null);
  return { mainThreadId, primaryEndpoint: primaryEndpoint ?? null };
}

export async function activateBotDraftThread(input: {
  platform: BotConsolePlatform;
  desktopState: DesktopState | null;
  group: DesktopBotConsoleSummary;
  onState: (state: DesktopState) => void;
  onOpenExistingThread: (endpoint: DesktopChannelEndpoint) => void;
  onOpenThreadById: (threadId: string) => void;
  setError: (value: string | null) => void;
  setContentView: (view: 'thread') => void;
  setNewThreadDraftActive: (value: boolean) => void;
  setSelectedThreadId: (value: string | null) => void;
  setPendingWorkspacePath: (value: string | null) => void;
  setPendingBotId: (value: string | null) => void;
  clearComposerDraft: () => void;
  syncComposerPhase: (value: string) => void;
  requestComposerFocus: () => void;
}): Promise<void> {
  // Fast path: navigate immediately using whatever state we already have and
  // reconcile in the background. getState() hits the gateway for threads /
  // endpoints / bots / automations and easily costs several hundred ms.
  const currentGroup = resolveBotGroupById(input.desktopState, input.group.id) || input.group;
  const current = resolveMainThreadId(currentGroup);
  if (current.mainThreadId) {
    input.onOpenThreadById(current.mainThreadId);
    void refreshBotState(input.platform, input.onState);
    return;
  }
  if (current.primaryEndpoint?.threadId) {
    input.onOpenExistingThread(current.primaryEndpoint);
    void refreshBotState(input.platform, input.onState);
    return;
  }

  // Slow path: no known thread — refresh so we don't create a duplicate draft
  // when a main thread actually exists remotely.
  const refreshedState = await refreshBotState(input.platform, input.onState);
  const nextDesktopState = refreshedState || input.desktopState;
  const nextGroup = resolveBotGroupById(nextDesktopState, input.group.id) || input.group;
  const resolved = resolveMainThreadId(nextGroup);
  if (resolved.mainThreadId) {
    input.onOpenThreadById(resolved.mainThreadId);
    return;
  }
  if (resolved.primaryEndpoint?.threadId) {
    input.onOpenExistingThread(resolved.primaryEndpoint);
    return;
  }

  let workspacePath: string | null;
  try {
    workspacePath = await ensureBotWorkspacePath(
      input.platform,
      nextDesktopState,
      nextGroup,
      input.onState,
    );
  } catch (error) {
    input.setError(botWorkspaceErrorMessage(nextGroup, error));
    return;
  }
  input.setError(null);
  input.setContentView('thread');
  input.setNewThreadDraftActive(true);
  input.setSelectedThreadId(null);
  input.setPendingWorkspacePath(workspacePath);
  input.setPendingBotId(nextGroup.id);
  input.clearComposerDraft();
  input.syncComposerPhase('');
  input.requestComposerFocus();
}

export function openThreadFromEndpoint(input: {
  endpoint: DesktopChannelEndpoint;
  setError: (value: string | null) => void;
  setContentView: (view: 'thread') => void;
  setNewThreadDraftActive: (value: boolean) => void;
  setSelectedThreadId: (value: string | null) => void;
}): void {
  if (!input.endpoint.threadId) {
    input.setError(
      'This endpoint is currently unbound. Open a thread and use Take Over Here to attach it.',
    );
    return;
  }

  input.setError(null);
  input.setContentView('thread');
  input.setNewThreadDraftActive(false);
  input.setSelectedThreadId(input.endpoint.threadId);
}
