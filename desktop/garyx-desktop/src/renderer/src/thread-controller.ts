import type {
  ThreadTranscript,
  TranscriptMessage,
  DesktopThreadSummary,
  DesktopWorkspace,
  DesktopState,
  GaryxDesktopApi,
} from "@shared/contracts";

import { pickPreferredWorkspace } from "./thread-model";

export function startNewThreadDraft(input: {
  selectableNewThreadWorkspaces: DesktopWorkspace[];
  pendingNewThreadWorkspaceEntry?: DesktopWorkspace | null;
  activeThreadNewThreadWorkspace?: DesktopWorkspace | null;
  selectedNewThreadWorkspaceEntry?: DesktopWorkspace | null;
  workspaceId?: string | null;
  setError: (value: string | null) => void;
  setContentView: (view: "thread") => void;
  setNewThreadDraftActive: (value: boolean) => void;
  setSelectedThreadId: (value: string | null) => void;
  setPendingWorkspaceId: (value: string | null) => void;
  setPendingBotId: (value: string | null) => void;
  setPendingAgentId: (value: string) => void;
  clearComposerDraft: () => void;
  syncComposerPhase: (value: string) => void;
  requestComposerFocus: () => void;
}) {
  const nextWorkspace = input.workspaceId
    ? (input.selectableNewThreadWorkspaces.find(
        (workspace) => workspace.id === input.workspaceId,
      ) ?? null)
    : pickPreferredWorkspace(
        input.selectableNewThreadWorkspaces,
        input.pendingNewThreadWorkspaceEntry,
        input.activeThreadNewThreadWorkspace,
        input.selectedNewThreadWorkspaceEntry,
      );
  input.setError(null);
  input.setContentView("thread");
  input.setNewThreadDraftActive(true);
  input.setSelectedThreadId(null);
  input.setPendingWorkspaceId(nextWorkspace?.id || null);
  input.setPendingBotId(null);
  input.setPendingAgentId("claude");
  input.clearComposerDraft();
  input.syncComposerPhase("");
  input.requestComposerFocus();
}

export async function selectWorkspaceForThread(input: {
  api: GaryxDesktopApi;
  workspaceId: string;
  threadId?: string | null;
  setError: (value: string | null) => void;
  setContentView: (view: "thread") => void;
  setDesktopState: (value: DesktopState) => void;
  setSelectedThreadId: (value: string | null) => void;
  setNewThreadDraftActive: (value: boolean) => void;
  setPendingWorkspaceId: (value: string | null) => void;
  requestComposerFocus: () => void;
}): Promise<void> {
  input.setError(null);
  input.setContentView("thread");
  try {
    const nextState = await input.api.selectWorkspace({
      workspaceId: input.workspaceId,
    });
    input.setDesktopState(nextState);
    if (input.threadId !== undefined) {
      input.setSelectedThreadId(input.threadId);
      input.setNewThreadDraftActive(!input.threadId);
      input.setPendingWorkspaceId(input.threadId ? null : input.workspaceId);
      if (!input.threadId) {
        input.requestComposerFocus();
      }
    }
  } catch (selectionError) {
    input.setError(
      selectionError instanceof Error
        ? selectionError.message
        : "Failed to select workspace",
    );
  }
}

export async function saveThreadTitle(input: {
  api: GaryxDesktopApi;
  activeThread?: Pick<DesktopThreadSummary, "id" | "title"> | null;
  activeAutomationThread: boolean;
  titleDraft: string;
  closeEditor?: boolean;
  defaultTitle: string;
  setError: (value: string | null) => void;
  setSavingTitle: (value: boolean) => void;
  setDesktopState: (value: DesktopState) => void;
  setTitleDraft: (value: string) => void;
  setEditingThreadTitle: (value: boolean) => void;
}): Promise<void> {
  if (!input.activeThread || input.activeAutomationThread) {
    if (input.activeAutomationThread) {
      input.setError("Rename this automation from the Automation view.");
    }
    return;
  }

  const normalizedTitle = input.titleDraft.trim() || input.defaultTitle;
  if (normalizedTitle === (input.activeThread.title || input.defaultTitle)) {
    input.setTitleDraft(normalizedTitle);
    if (input.closeEditor) {
      input.setEditingThreadTitle(false);
    }
    return;
  }

  input.setSavingTitle(true);
  input.setError(null);
  try {
    const nextState = await input.api.renameThread({
      threadId: input.activeThread.id,
      title: normalizedTitle,
    });
    input.setDesktopState(nextState);
    input.setTitleDraft(normalizedTitle);
    if (input.closeEditor) {
      input.setEditingThreadTitle(false);
    }
  } catch (renameError) {
    input.setError(
      renameError instanceof Error
        ? renameError.message
        : "Failed to rename the thread",
    );
  } finally {
    input.setSavingTitle(false);
  }
}

export async function deleteThread(input: {
  api: GaryxDesktopApi;
  desktopState?: DesktopState | null;
  targetThreadId?: string | null;
  targetIsAutomationThread: boolean;
  targetIsBusy: boolean;
  selectedThreadId?: string | null;
  setError: (value: string | null) => void;
  setDeletingThreadId: (
    value: string | ((current: string | null) => string | null),
  ) => void;
  setDesktopState: (value: DesktopState) => void;
  setSelectedThreadId: (value: string | null) => void;
  dispatchDelete: (threadId: string) => void;
}): Promise<void> {
  if (!input.desktopState) {
    return;
  }

  const nextTargetThreadId = input.targetThreadId || null;
  if (!nextTargetThreadId) {
    return;
  }
  if (input.targetIsAutomationThread) {
    input.setError("Delete this automation from the Automation view.");
    return;
  }
  if (input.targetIsBusy) {
    return;
  }

  input.setDeletingThreadId(nextTargetThreadId);
  input.setError(null);
  try {
    const nextState = await input.api.deleteThread({
      threadId: nextTargetThreadId,
    });
    const deletingSelected = nextTargetThreadId === input.selectedThreadId;
    const fallbackThread = deletingSelected
      ? nextState.threads[0] || null
      : null;
    input.setDesktopState(nextState);
    if (deletingSelected) {
      input.setSelectedThreadId(fallbackThread?.id || null);
    }
    input.dispatchDelete(nextTargetThreadId);
  } catch (deleteError) {
    input.setError(
      deleteError instanceof Error
        ? deleteError.message
        : "Failed to delete the thread",
    );
  } finally {
    input.setDeletingThreadId((current) =>
      current === nextTargetThreadId ? null : current,
    );
  }
}

export async function bindEndpointToThread(input: {
  api: GaryxDesktopApi;
  endpointKey: string;
  threadId?: string | null;
  setBindingMutation: (value: string | null) => void;
  setError: (value: string | null) => void;
  setDesktopState: (value: DesktopState) => void;
}): Promise<void> {
  if (!input.threadId) {
    return;
  }

  input.setBindingMutation(`bind:${input.endpointKey}`);
  input.setError(null);
  try {
    const nextState = await input.api.bindChannelEndpoint({
      endpointKey: input.endpointKey,
      threadId: input.threadId,
    });
    input.setDesktopState(nextState);
  } catch (bindingError) {
    input.setError(
      bindingError instanceof Error
        ? bindingError.message
        : "Failed to move endpoint",
    );
  } finally {
    input.setBindingMutation(null);
  }
}

export async function detachEndpointFromThread(input: {
  api: GaryxDesktopApi;
  endpointKey: string;
  setBindingMutation: (value: string | null) => void;
  setError: (value: string | null) => void;
  setDesktopState: (value: DesktopState) => void;
}): Promise<void> {
  input.setBindingMutation(`detach:${input.endpointKey}`);
  input.setError(null);
  try {
    const nextState = await input.api.detachChannelEndpoint({
      endpointKey: input.endpointKey,
    });
    input.setDesktopState(nextState);
  } catch (bindingError) {
    input.setError(
      bindingError instanceof Error
        ? bindingError.message
        : "Failed to detach endpoint",
    );
  } finally {
    input.setBindingMutation(null);
  }
}

export async function updateThreadBotBinding(input: {
  threadId?: string | null;
  botId: string | null;
  setBindingMutation: (value: string | null) => void;
  setError: (value: string | null) => void;
  syncThreadBotBinding: (
    threadId: string,
    botId: string | null,
  ) => Promise<void>;
}): Promise<void> {
  if (!input.threadId) {
    return;
  }

  input.setBindingMutation("bot-binding");
  input.setError(null);
  try {
    await input.syncThreadBotBinding(input.threadId, input.botId);
  } catch (bindError) {
    input.setError(
      bindError instanceof Error
        ? bindError.message
        : "Failed to update bot binding",
    );
  } finally {
    input.setBindingMutation(null);
  }
}

export async function ensureWorkspaceForNewThread(input: {
  api: GaryxDesktopApi;
  preferredWorkspaceId?: string | null;
  selectableWorkspaceCount: number;
  setWorkspaceMutation: (
    value: "assign" | "add" | "relink" | "remove" | null,
  ) => void;
  setDesktopState: (value: DesktopState) => void;
  setError: (value: string | null) => void;
}): Promise<string | null> {
  if (input.preferredWorkspaceId) {
    return input.preferredWorkspaceId;
  }

  if (input.selectableWorkspaceCount === 0) {
    input.setWorkspaceMutation("add");
    try {
      const result = await input.api.addWorkspace();
      input.setDesktopState(result.state);
      if (result.cancelled || !result.workspace) {
        return null;
      }
      return result.workspace.id;
    } catch (workspaceError) {
      input.setError(
        workspaceError instanceof Error
          ? workspaceError.message
          : "Failed to add workspace",
      );
      return null;
    } finally {
      input.setWorkspaceMutation(null);
    }
  }

  input.setError("Choose an available folder before creating a thread.");
  return null;
}

export async function ensureThread(input: {
  api: GaryxDesktopApi;
  selectedThreadId?: string | null;
  pendingWorkspaceId?: string | null;
  pendingAgentId?: string | null;
  preferredWorkspaceId?: string | null;
  selectableWorkspaceCount: number;
  setWorkspaceMutation: (
    value: "assign" | "add" | "relink" | "remove" | null,
  ) => void;
  setDesktopState: (value: DesktopState) => void;
  setSelectedThreadId: (value: string | null) => void;
  initializeThreadMessages: (threadId: string) => void;
  setNewThreadDraftActive: (value: boolean) => void;
  setPendingWorkspaceId: (value: string | null) => void;
  setPendingBotId: (value: string | null) => void;
  setPendingAgentId: (value: string) => void;
  setError: (value: string | null) => void;
}): Promise<string | null> {
  let threadId = input.selectedThreadId || null;
  if (threadId) {
    return threadId;
  }

  const agentId = input.pendingAgentId?.trim() || null;
  const workspaceId =
    input.pendingWorkspaceId ||
    (await ensureWorkspaceForNewThread({
      api: input.api,
      preferredWorkspaceId: input.preferredWorkspaceId,
      selectableWorkspaceCount: input.selectableWorkspaceCount,
      setWorkspaceMutation: input.setWorkspaceMutation,
      setDesktopState: input.setDesktopState,
      setError: input.setError,
    }));
  if (!workspaceId) {
    return null;
  }

  try {
    const created = await input.api.createThread({
      workspaceId,
      agentId,
    });
    input.setDesktopState(created.state);
    input.setSelectedThreadId(created.thread.id);
    input.initializeThreadMessages(created.thread.id);
    threadId = created.thread.id;
    input.setNewThreadDraftActive(false);
    input.setPendingWorkspaceId(null);
    input.setPendingBotId(null);
    input.setPendingAgentId("claude");
    return threadId;
  } catch (creationError) {
    input.setError(
      creationError instanceof Error
        ? creationError.message
        : "Failed to create a thread",
    );
    return null;
  }
}

export async function loadThreadHistory(input: {
  api: GaryxDesktopApi;
  threadId?: string | null;
  onBeforeLoad?: (threadId: string) => void;
  onTranscript: (threadId: string, transcript: ThreadTranscript) => void;
  onAutomationResponseDetected?: (threadId: string) => void;
  hasAutomationResponse?: (transcript: TranscriptMessage[]) => boolean;
  setHistoryLoading: (value: boolean) => void;
  setError: (value: string | null) => void;
}): Promise<void> {
  const threadId = input.threadId || null;
  if (!threadId) {
    return;
  }

  input.onBeforeLoad?.(threadId);
  input.setHistoryLoading(true);
  try {
    const transcript = await input.api.getThreadHistory(threadId);
    input.onTranscript(threadId, transcript);
    if (input.hasAutomationResponse?.(transcript.messages)) {
      input.onAutomationResponseDetected?.(threadId);
    }
  } catch (historyError) {
    input.setError(
      historyError instanceof Error
        ? historyError.message
        : "Failed to load thread history",
    );
  } finally {
    input.setHistoryLoading(false);
  }
}

export function scheduleThreadHistoryRefresh(input: {
  api: GaryxDesktopApi;
  threadId: string;
  attempts?: number;
  delayMs?: number;
  canonical?: boolean;
  shouldContinue: (threadId: string) => boolean;
  onCanonicalTranscript: (
    threadId: string,
    transcript: ThreadTranscript,
  ) => void;
  onRemoteTranscript: (threadId: string, transcript: ThreadTranscript) => void;
  onExhausted?: (threadId: string) => void;
}): void {
  const attempts = input.attempts ?? 4;
  const delayMs = input.delayMs ?? 1200;
  const canonical = input.canonical ?? false;

  window.setTimeout(() => {
    void (async () => {
      try {
        const transcript = await input.api.getThreadHistory(input.threadId);
        if (canonical) {
          input.onCanonicalTranscript(input.threadId, transcript);
        } else {
          input.onRemoteTranscript(input.threadId, transcript);
        }
      } catch {
        // Best-effort reconcile loop for async steer.
      } finally {
        if (!input.shouldContinue(input.threadId)) {
          return;
        }
        if (attempts > 1) {
          scheduleThreadHistoryRefresh({
            ...input,
            attempts: attempts - 1,
            delayMs: Math.min(delayMs * 2, 5000),
            canonical,
          });
        } else {
          input.onExhausted?.(input.threadId);
        }
      }
    })();
  }, delayMs);
}
