import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import type {
  BrowserAnnotationCommentRequest,
  DesktopApiProviderType,
  DesktopBotConsoleSummary,
  DesktopChannelEndpoint,
  DesktopCustomAgent,
  DesktopSettings,
  DesktopState,
  DesktopThreadSummary,
  DesktopWorkspaceMode,
  MessageFileAttachment,
  MessageImageAttachment,
  RenderState,
  ThreadRuntimeInfo,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";

import type { Translate } from "../i18n";
import {
  buildIntent,
  isRuntimeBusy,
  selectQueueIntentIds,
  selectThreadRuntime,
  type MessageIntent,
  type MessageMachineAction,
  type MessageMachineState,
} from "../message-machine";
import { getDesktopApi } from "../platform/desktop-api";
import { loadThreadHistory } from "../thread-controller";
import {
  automationForLatestThread,
  deriveThreadTeamView,
} from "../thread-model";
import { isRunLoadingPlaceholderMessage } from "./loading-labels";
import {
  visibleRemotePendingInputsForThread,
  type PendingInputOriginRef,
} from "./pending-inputs";
import {
  deriveThreadComposerControlModel,
  deriveThreadActivityModel,
} from "./thread-activity";
import type {
  BoundBot,
  LiveStreamState,
  MessageMap,
  PendingAutomationRun,
  PendingThreadInputMap,
  UiTranscriptMessage,
} from "./types";

type SideComposerDraft = {
  text: string;
  textPresent: boolean;
  images: MessageImageAttachment[];
  files: MessageFileAttachment[];
  browserAnnotations: BrowserAnnotationCommentRequest[];
  resetKey: number;
};

function emptySideComposerDraft(): SideComposerDraft {
  return {
    text: "",
    textPresent: false,
    images: [],
    files: [],
    browserAnnotations: [],
    resetKey: 0,
  };
}

function sideChatThreadStorageKey(sourceThreadId: string): string {
  return `garyx.side-tools.side-chat-thread.${sourceThreadId}`;
}

function readPersistedSideChatThreadId(sourceThreadId: string): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    return window.sessionStorage.getItem(sideChatThreadStorageKey(sourceThreadId)) || null;
  } catch {
    return null;
  }
}

function persistSideChatThreadId(sourceThreadId: string, sideThreadId: string) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.sessionStorage.setItem(
      sideChatThreadStorageKey(sourceThreadId),
      sideThreadId,
    );
  } catch {
    // Side chat can still run from in-memory state if sessionStorage is blocked.
  }
}

const EMPTY_UI_TRANSCRIPT_MESSAGES: UiTranscriptMessage[] = [];

type UseSideChatControllerArgs = {
  activeThread: DesktopThreadSummary | null;
  applyRemoteTranscript: (
    threadId: string,
    transcript: ThreadTranscript,
    options?: {
      persist?: boolean;
      syncRunState?: boolean;
    },
  ) => void;
  botGroups: DesktopBotConsoleSummary[];
  boundBotsForThread: (endpoints: DesktopChannelEndpoint[]) => BoundBot[];
  browserAnnotationScreenshotImages: (
    requests: BrowserAnnotationCommentRequest[],
  ) => MessageImageAttachment[];
  composePromptWithBrowserAnnotations: (
    prompt: string,
    requests: BrowserAnnotationCommentRequest[],
    t: Translate,
  ) => string;
  composerProviderType: DesktopApiProviderType;
  deferredQueueDrainByThreadRef: React.MutableRefObject<
    Record<string, boolean>
  >;
  desktopAgentMap: Map<string, DesktopCustomAgent>;
  desktopAgents: DesktopCustomAgent[];
  desktopState: DesktopState | null;
  dispatchMessageState: (action: MessageMachineAction) => void;
  ensureThreadOpenable: (threadId: string) => Promise<boolean>;
  getLiveStreamState: (threadId: string) => LiveStreamState | null;
  historyPaginationByThread: Record<
    string,
    {
      hasMoreBefore: boolean;
      nextBeforeIndex: number | null;
      loadingBefore: boolean;
    }
  >;
  inferProviderTypeForThread: (
    threadId: string,
    threadInfoByThread: Record<string, ThreadRuntimeInfo | null>,
    desktopState: DesktopState | null,
    desktopAgents: DesktopCustomAgent[],
  ) => DesktopApiProviderType | null;
  liveStreamStateByThread: Record<string, LiveStreamState>;
  messageState: MessageMachineState;
  messageStateRef: React.MutableRefObject<MessageMachineState>;
  messageTailSignature: (messages: UiTranscriptMessage[]) => string;
  messagesByThread: MessageMap;
  messagesByThreadRef: React.MutableRefObject<MessageMap>;
  pendingAgentId: string;
  pendingInputOriginRefsForThread: (
    intentsById: Record<string, MessageIntent>,
    threadId: string | null,
  ) => PendingInputOriginRef[];
  pendingRemoteInputsByThread: PendingThreadInputMap;
  prepareAttachmentUploads: (files: File[]) => Promise<
    Array<{
      id: string;
      kind: "image" | "file";
      name: string;
      mediaType: string;
      dataBase64: string;
    }>
  >;
  queueDrainInFlightByThreadRef: React.MutableRefObject<
    Record<string, boolean>
  >;
  renderStateByThread: Record<string, RenderState>;
  runQueuedBatch: (threadId: string, initialIntentId?: string) => Promise<void>;
  scrollMessagesToLatest: (
    node: HTMLDivElement | null,
    behavior?: ScrollBehavior,
  ) => void;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setPendingAutomationRun: (
    threadId: string,
    run: PendingAutomationRun | null,
  ) => void;
  settingsDraft: DesktopSettings;
  startCommittedThreadStream: (
    threadId: string,
    transcript: ThreadTranscript,
    consumerId: string,
  ) => Promise<void>;
  steerQueuedIntent: (
    latestIntent: MessageIntent,
    options?: { canSteer?: boolean },
  ) => Promise<void>;
  t: Translate;
  threadInfoByThread: Record<string, ThreadRuntimeInfo | null>;
  threadSummaryById: Map<string, DesktopThreadSummary>;
  transcriptHasAutomationResponse: (messages: TranscriptMessage[]) => boolean;
  transcriptMessageMatchesIntent: (
    message: TranscriptMessage,
    intent: MessageIntent,
  ) => boolean;
  updateMessagesByThread: (
    updater: (current: MessageMap) => MessageMap,
  ) => MessageMap;
};

export function useSideChatController({
  activeThread,
  applyRemoteTranscript,
  botGroups,
  boundBotsForThread,
  browserAnnotationScreenshotImages,
  composePromptWithBrowserAnnotations,
  composerProviderType,
  deferredQueueDrainByThreadRef,
  desktopAgentMap,
  desktopAgents,
  desktopState,
  dispatchMessageState,
  ensureThreadOpenable,
  getLiveStreamState,
  historyPaginationByThread,
  inferProviderTypeForThread,
  liveStreamStateByThread,
  messageState,
  messageStateRef,
  messageTailSignature,
  messagesByThread,
  messagesByThreadRef,
  pendingAgentId,
  pendingInputOriginRefsForThread,
  pendingRemoteInputsByThread,
  prepareAttachmentUploads,
  queueDrainInFlightByThreadRef,
  renderStateByThread,
  runQueuedBatch,
  scrollMessagesToLatest,
  setDesktopState,
  setError,
  setPendingAutomationRun,
  settingsDraft,
  startCommittedThreadStream,
  steerQueuedIntent,
  t,
  threadInfoByThread,
  threadSummaryById,
  transcriptHasAutomationResponse,
  transcriptMessageMatchesIntent,
  updateMessagesByThread,
}: UseSideChatControllerArgs) {
  const [sideComposerBySource, setSideComposerBySource] = useState<
    Record<string, SideComposerDraft>
  >({});
  const [sideComposerAttachmentUploadCount, setSideComposerAttachmentUploadCount] =
    useState(0);
  const sideComposerAttachmentUploadPending = sideComposerAttachmentUploadCount > 0;
  const [sideChatThreadBySource, setSideChatThreadBySource] = useState<
    Record<string, string>
  >({});
  const [sideChatCreatingBySource, setSideChatCreatingBySource] = useState<
    Record<string, boolean>
  >({});
  const [sideChatErrorBySource, setSideChatErrorBySource] = useState<
    Record<string, string>
  >({});
  const [sideChatHistoryLoading, setSideChatHistoryLoading] = useState(false);
  const sideComposerAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const sideComposerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const sideChatMessagesRef = useRef<HTMLDivElement | null>(null);
  const sideChatThreadLayoutRef = useRef<HTMLDivElement | null>(null);
  const sideChatThreadIdRef = useRef<string | null>(null);
  const sideChatThreadIdsRef = useRef<Set<string>>(new Set());
  const sideChatCreationBySourceRef = useRef<
    Record<string, Promise<string | null>>
  >({});
  const sideIsComposingRef = useRef(false);
  const sideIgnoreComposerSubmitUntilRef = useRef(0);

  const sideChatSourceThreadId = activeThread?.id?.trim() || null;
  const sideChatThreadId = sideChatSourceThreadId
    ? sideChatThreadBySource[sideChatSourceThreadId] || null
    : null;
  const sideChatCreating = sideChatSourceThreadId
    ? Boolean(sideChatCreatingBySource[sideChatSourceThreadId])
    : false;
  const sideChatError = sideChatSourceThreadId
    ? sideChatErrorBySource[sideChatSourceThreadId] || null
    : null;
  const sideComposerDraft = sideChatSourceThreadId
    ? sideComposerDraftForSource(sideChatSourceThreadId)
    : emptySideComposerDraft();

  useEffect(() => {
    if (!sideChatSourceThreadId) {
      return;
    }
    const persistedThreadId = readPersistedSideChatThreadId(sideChatSourceThreadId);
    if (!persistedThreadId) {
      return;
    }
    setSideChatThreadBySource((current) =>
      current[sideChatSourceThreadId]
        ? current
        : {
            ...current,
            [sideChatSourceThreadId]: persistedThreadId,
          },
    );
  }, [sideChatSourceThreadId]);

  useEffect(() => {
    sideChatThreadIdRef.current = sideChatThreadId;
  }, [sideChatThreadId]);

  useEffect(() => {
    sideChatThreadIdsRef.current = new Set(Object.values(sideChatThreadBySource));
  }, [sideChatThreadBySource]);

  function rememberSideChatThreadId(threadId: string) {
    sideChatThreadIdsRef.current = new Set([
      ...sideChatThreadIdsRef.current,
      threadId,
    ]);
  }

  const sideChatThreadSummary = sideChatThreadId
    ? threadSummaryById.get(sideChatThreadId) || null
    : null;
  const sideChatThreadTeamView = deriveThreadTeamView(sideChatThreadSummary);
  const sideChatAgent =
    sideChatThreadSummary?.agentId
      ? desktopAgentMap.get(sideChatThreadSummary.agentId) || null
      : null;
  const sideChatAgentLabel =
    sideChatThreadTeamView.teamDisplayName ||
    sideChatAgent?.displayName ||
    sideChatThreadSummary?.agentId ||
    null;
  const sideChatComposerProviderType: DesktopApiProviderType =
    sideChatThreadId
      ? inferProviderTypeForThread(
          sideChatThreadId,
          threadInfoByThread,
          desktopState,
          desktopAgents,
        ) || "claude_code"
      : composerProviderType;
  const sideChatRawMessages = sideChatThreadId
    ? messagesByThread[sideChatThreadId] || EMPTY_UI_TRANSCRIPT_MESSAGES
    : EMPTY_UI_TRANSCRIPT_MESSAGES;
  const sideChatMessages = useMemo(
    () =>
      sideChatRawMessages.filter(
        (message) => !isRunLoadingPlaceholderMessage(message),
      ),
    [sideChatRawMessages],
  );
  const sideChatMessageTailSignature = messageTailSignature(sideChatMessages);
  const sideChatThreadInfo = sideChatThreadId
    ? threadInfoByThread[sideChatThreadId] || null
    : null;
  const sideChatThreadWorktree =
    sideChatThreadInfo?.worktree || sideChatThreadSummary?.worktree || null;
  const sideChatComposerWorkspaceMode: DesktopWorkspaceMode | null =
    sideChatThreadId && sideChatThreadWorktree ? "worktree" : null;
  const sideChatComposerWorkspaceBranch =
    sideChatThreadWorktree?.branch?.trim() || null;
  const sideChatRenderState = sideChatThreadId
    ? renderStateByThread[sideChatThreadId] || null
    : null;
  const sideChatQueue = sideChatThreadId
    ? selectQueueIntentIds(messageState, sideChatThreadId)
        .map((intentId) => messageState.intentsById[intentId])
        .filter((intent): intent is MessageIntent => Boolean(intent))
    : [];
  const sideChatRuntime = selectThreadRuntime(messageState, sideChatThreadId);
  const sideChatLiveStream = sideChatThreadId
    ? liveStreamStateByThread[sideChatThreadId] || null
    : null;
  const sideChatPendingAckIntents = (
    sideChatLiveStream?.pendingAckIntentIds || []
  )
    .map((intentId) => messageState.intentsById[intentId])
    .filter((intent): intent is MessageIntent => Boolean(intent));
  const sideChatVisiblePendingAckIntents = sideChatPendingAckIntents.filter(
    (intent) => {
      return !sideChatMessages.some((message) => {
        return (
          message.role === "user" &&
          (message.intentId === intent.intentId ||
            transcriptMessageMatchesIntent(message, intent))
        );
      });
    },
  );
  const sideChatRemotePendingInputs = sideChatThreadId
    ? pendingRemoteInputsByThread[sideChatThreadId] || []
    : [];
  const sideChatPendingInputOriginRefs = useMemo(
    () =>
      pendingInputOriginRefsForThread(
        messageState.intentsById,
        sideChatThreadId,
      ),
    [messageState.intentsById, sideChatThreadId],
  );
  const sideChatVisibleRemotePendingInputs = visibleRemotePendingInputsForThread(
    {
      activeMessages: sideChatMessages,
      visiblePendingAckIntentCount: sideChatVisiblePendingAckIntents.length,
      remotePendingInputs: sideChatRemotePendingInputs,
      pendingInputOriginRefs: sideChatPendingInputOriginRefs,
    },
  );
  const sideChatPendingHistoryIntent = sideChatThreadId
    ? Object.values(messageState.intentsById).some((intent) => {
        return (
          intent.threadId === sideChatThreadId &&
          [
            "dispatching",
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_response",
            "awaiting_history",
          ].includes(intent.state)
        );
      })
    : false;
  const sideChatRuntimeBusy = Boolean(
    sideChatRuntime && isRuntimeBusy(sideChatRuntime.state),
  );
  const sideChatThreadActivity = deriveThreadActivityModel({
    messages: sideChatMessages,
    runtimeBusy: sideChatRuntimeBusy,
    pendingAckIntentCount: sideChatPendingAckIntents.length,
    remoteAwaitingAckInputCount: sideChatVisibleRemotePendingInputs.length,
    pendingHistoryIntent: sideChatPendingHistoryIntent,
    renderTailActivity: sideChatRenderState?.tailActivity ?? null,
    renderActiveToolGroupId: sideChatRenderState?.activeToolGroupId ?? null,
  });
  const sideChatShowPendingAckLoading =
    sideChatThreadActivity.showPendingAckLoading;
  const sideChatCanSteerQueuedPrompt =
    sideChatThreadActivity.canSteerQueuedPrompt;
  const { isActiveSendingThread: sideChatIsSendingThread } =
    deriveThreadComposerControlModel({
      hasThread: Boolean(sideChatThreadId),
      runtimeBusy: sideChatRuntimeBusy,
      showPendingAckLoading: sideChatShowPendingAckLoading,
      renderTailActivity: sideChatRenderState?.tailActivity ?? null,
      renderActiveToolGroupId: sideChatRenderState?.activeToolGroupId ?? null,
    });
  const sideChatHistoryPagination = sideChatThreadId
    ? historyPaginationByThread[sideChatThreadId] || null
    : null;
  const sideChatThreadEndpoints =
    sideChatThreadId && !automationForLatestThread(desktopState, sideChatThreadId)
      ? (desktopState?.endpoints || []).filter(
          (endpoint) => endpoint.threadId === sideChatThreadId,
        )
      : [];
  const sideChatThreadBots = boundBotsForThread(sideChatThreadEndpoints);
  const sideChatMappedThreadBotId = sideChatThreadId
    ? (Object.entries(desktopState?.botMainThreads || {}).find(
        ([, threadId]) => threadId === sideChatThreadId,
      )?.[0] ?? null)
    : null;
  const sideChatThreadBotId =
    sideChatMappedThreadBotId ||
    (sideChatThreadBots.length === 1 ? sideChatThreadBots[0]?.id ?? null : null);
  const sideChatThreadBot = sideChatThreadBotId
    ? botGroups.find((group) => group.id === sideChatThreadBotId) || null
    : null;
  const sideChatComposerHasPayload =
    sideComposerDraft.textPresent ||
    sideComposerDraft.images.length > 0 ||
    sideComposerDraft.files.length > 0 ||
    sideComposerDraft.browserAnnotations.length > 0;
  const sideChatComposerLocked =
    sideComposerAttachmentUploadPending || sideChatCreating;
  const sideChatComposerEditingLocked = sideChatCreating;
  const sideChatComposerPlaceholder =
    sideChatIsSendingThread || sideChatQueue.length > 0
      ? "Queue another follow-up for Garyx..."
      : "Ask in side chat";
  const sideChatActiveToolGroupId =
    sideChatRenderState?.activeToolGroupId ?? null;
  const sideChatShowTailThinking = Boolean(
    sideChatRenderState?.tailActivity === "thinking" ||
      sideChatShowPendingAckLoading,
  );

  function sideComposerDraftForSource(sourceThreadId: string): SideComposerDraft {
    return sideComposerBySource[sourceThreadId] || emptySideComposerDraft();
  }

  function updateSideComposerDraft(
    sourceThreadId: string,
    updater: (current: SideComposerDraft) => SideComposerDraft,
  ) {
    setSideComposerBySource((current) => {
      const previous = current[sourceThreadId] || emptySideComposerDraft();
      return {
        ...current,
        [sourceThreadId]: updater(previous),
      };
    });
  }

  function resetSideComposerAttachmentPicker() {
    if (sideComposerAttachmentInputRef.current) {
      sideComposerAttachmentInputRef.current.value = "";
    }
  }

  function clearSideComposerDraft(sourceThreadId: string) {
    updateSideComposerDraft(sourceThreadId, (current) => ({
      ...current,
      text: "",
      textPresent: false,
      images: [],
      files: [],
      browserAnnotations: [],
      resetKey: current.resetKey + 1,
    }));
    resetSideComposerAttachmentPicker();
  }

  function removeSideComposerImage(sourceThreadId: string, imageId: string) {
    updateSideComposerDraft(sourceThreadId, (current) => ({
      ...current,
      images: current.images.filter((image) => image.id !== imageId),
    }));
  }

  function removeSideComposerFile(sourceThreadId: string, fileId: string) {
    updateSideComposerDraft(sourceThreadId, (current) => ({
      ...current,
      files: current.files.filter((file) => file.id !== fileId),
    }));
  }

  function removeSideComposerBrowserAnnotation(
    sourceThreadId: string,
    annotationId: string,
  ) {
    updateSideComposerDraft(sourceThreadId, (current) => ({
      ...current,
      browserAnnotations: current.browserAnnotations.filter(
        (annotation) => annotation.id !== annotationId,
      ),
    }));
  }

  function appendSideComposerFile(
    sourceThreadId: string,
    file: MessageFileAttachment,
  ) {
    updateSideComposerDraft(sourceThreadId, (current) => {
      if (current.files.some((entry) => entry.path === file.path)) {
        return current;
      }
      return {
        ...current,
        files: [...current.files, file],
      };
    });
  }

  async function appendSideComposerAttachments(
    sourceThreadId: string,
    files: File[],
  ) {
    if (!files.length) {
      return;
    }

    setSideComposerAttachmentUploadCount((count) => count + 1);
    try {
      const prepared = await prepareAttachmentUploads(files);
      if (!prepared.length) {
        setError("No attachments could be loaded.");
        return;
      }
      const uploaded = await window.garyxDesktop.uploadChatAttachments({
        files: prepared.map((file) => ({
          kind: file.kind,
          name: file.name,
          mediaType: file.mediaType,
          dataBase64: file.dataBase64,
        })),
      });
      if (uploaded.files.length !== prepared.length) {
        throw new Error("Gateway returned an incomplete attachment upload result.");
      }

      const nextImages: MessageImageAttachment[] = [];
      const nextFiles: MessageFileAttachment[] = [];
      prepared.forEach((file, index) => {
        const stored = uploaded.files[index];
        if (!stored?.path?.trim()) {
          return;
        }
        if (file.kind === "image") {
          nextImages.push({
            id: file.id,
            name: stored.name,
            mediaType: stored.mediaType || file.mediaType,
            path: stored.path,
            data: file.dataBase64,
          });
          return;
        }
        nextFiles.push({
          id: file.id,
          name: stored.name,
          mediaType: stored.mediaType || file.mediaType,
          path: stored.path,
        });
      });

      if (!nextImages.length && !nextFiles.length) {
        throw new Error("Gateway did not return any uploaded attachments.");
      }
      updateSideComposerDraft(sourceThreadId, (current) => ({
        ...current,
        images: nextImages.length ? [...current.images, ...nextImages] : current.images,
        files: nextFiles.length ? [...current.files, ...nextFiles] : current.files,
      }));
      setError(null);
    } catch (attachmentError) {
      setError(
        attachmentError instanceof Error
          ? attachmentError.message
          : "Failed to load attachment",
      );
    } finally {
      setSideComposerAttachmentUploadCount((count) => count - 1);
      resetSideComposerAttachmentPicker();
    }
  }

  // Load the side thread transcript once per side thread (after state
  // hydration). Depending on `desktopState` identity here is unsafe: applying
  // a transcript can rewrite `desktopState.sessions`, which would re-fire
  // this effect in a fetch loop. Steady-state sync comes from the per-thread
  // committed stream started after the initial committed cursor is known.
  const desktopStateHydrated = Boolean(desktopState);
  useEffect(() => {
    if (!sideChatThreadId || !desktopStateHydrated) {
      return;
    }

    let cancelled = false;
    let latestTranscript: ThreadTranscript | null = null;
    const consumerId = sideChatStreamConsumerId(sideChatThreadId);
    void loadThreadHistory({
      api: getDesktopApi(),
      threadId: sideChatThreadId,
      onBeforeLoad: (threadId) => {
        if (!(messagesByThreadRef.current[threadId] || []).length) {
          scrollMessagesToLatest(sideChatMessagesRef.current);
        }
      },
      onTranscript: (threadId, transcript) => {
        if (cancelled) {
          return;
        }
        latestTranscript = transcript;
        applyRemoteTranscript(threadId, transcript);
      },
      onAutomationResponseDetected: (threadId) => {
        setPendingAutomationRun(threadId, null);
      },
      hasAutomationResponse: transcriptHasAutomationResponse,
      setHistoryLoading: setSideChatHistoryLoading,
      setError,
    }).then(() => {
      if (cancelled || !latestTranscript) {
        return;
      }
      void startCommittedThreadStream(
        sideChatThreadId,
        latestTranscript,
        consumerId,
      );
    });

    return () => {
      cancelled = true;
      void window.garyxDesktop.stopThreadStream({
        threadId: sideChatThreadId,
        consumerId,
      });
    };
  }, [desktopStateHydrated, sideChatThreadId]);

  useEffect(() => {
    if (!sideChatThreadId) {
      return;
    }
    if (sideChatQueue.length === 0) {
      delete deferredQueueDrainByThreadRef.current[sideChatThreadId];
      delete queueDrainInFlightByThreadRef.current[sideChatThreadId];
      return;
    }
    if (
      sideChatIsSendingThread ||
      !deferredQueueDrainByThreadRef.current[sideChatThreadId] ||
      queueDrainInFlightByThreadRef.current[sideChatThreadId]
    ) {
      return;
    }

    deferredQueueDrainByThreadRef.current[sideChatThreadId] = false;
    queueDrainInFlightByThreadRef.current[sideChatThreadId] = true;
    void runQueuedBatch(sideChatThreadId).finally(() => {
      delete queueDrainInFlightByThreadRef.current[sideChatThreadId];
    });
  }, [sideChatIsSendingThread, sideChatQueue.length, sideChatThreadId]);

  useLayoutEffect(() => {
    if (!sideChatThreadId || sideChatHistoryLoading) {
      return;
    }
    scrollMessagesToLatest(sideChatMessagesRef.current);
  }, [
    sideChatHistoryLoading,
    sideChatThreadId,
    sideChatMessageTailSignature,
  ]);

  async function ensureSideChatThread(): Promise<string | null> {
    const sourceThreadId = sideChatSourceThreadId;
    if (!sourceThreadId) {
      return null;
    }

    const existingThreadId =
      sideChatThreadBySource[sourceThreadId] ||
      readPersistedSideChatThreadId(sourceThreadId);
    if (existingThreadId) {
      try {
        if (await ensureThreadOpenable(existingThreadId)) {
          rememberSideChatThreadId(existingThreadId);
          setSideChatThreadBySource((current) =>
            current[sourceThreadId] === existingThreadId
              ? current
              : {
                  ...current,
                  [sourceThreadId]: existingThreadId,
                },
          );
          setSideChatErrorBySource((current) => {
            if (!(sourceThreadId in current)) {
              return current;
            }
            const next = { ...current };
            delete next[sourceThreadId];
            return next;
          });
          return existingThreadId;
        }
      } catch {
        setSideChatThreadBySource((current) => {
          if (current[sourceThreadId] !== existingThreadId) {
            return current;
          }
          const next = { ...current };
          delete next[sourceThreadId];
          return next;
        });
      }
    }

    const inFlight = sideChatCreationBySourceRef.current[sourceThreadId];
    if (inFlight) {
      return inFlight;
    }

    const creation = (async () => {
      setSideChatCreatingBySource((current) => ({
        ...current,
        [sourceThreadId]: true,
      }));
      setSideChatErrorBySource((current) => {
        if (!(sourceThreadId in current)) {
          return current;
        }
        const next = { ...current };
        delete next[sourceThreadId];
        return next;
      });

      try {
        const sourceThread =
          threadSummaryById.get(sourceThreadId) || activeThread || null;
        const created = await window.garyxDesktop.createThread({
          title: "Side chat",
          agentId: sourceThread?.agentId || pendingAgentId || "claude",
          forkFromThreadId: sourceThreadId,
          metadata: {
            source: "side_chat",
            hidden: true,
            exclude_from_recent: true,
            side_chat_parent_thread_id: sourceThreadId,
          },
        });
        setDesktopState(created.state);
        updateMessagesByThread((current) => ({
          ...current,
          [created.thread.id]: current[created.thread.id] || [],
        }));
        rememberSideChatThreadId(created.thread.id);
        setSideChatThreadBySource((current) => ({
          ...current,
          [sourceThreadId]: created.thread.id,
        }));
        persistSideChatThreadId(sourceThreadId, created.thread.id);
        return created.thread.id;
      } catch (createError) {
        const message =
          createError instanceof Error
            ? createError.message
            : "Failed to start side chat.";
        setSideChatErrorBySource((current) => ({
          ...current,
          [sourceThreadId]: message,
        }));
        setError(message);
        return null;
      } finally {
        setSideChatCreatingBySource((current) => {
          if (!current[sourceThreadId]) {
            return current;
          }
          const next = { ...current };
          delete next[sourceThreadId];
          return next;
        });
        delete sideChatCreationBySourceRef.current[sourceThreadId];
      }
    })();

    sideChatCreationBySourceRef.current[sourceThreadId] = creation;
    return creation;
  }

  async function openTaskThreadInSidePanel(threadId: string): Promise<void> {
    const sourceThreadId = sideChatSourceThreadId;
    const targetThreadId = threadId.trim();
    if (!sourceThreadId || !targetThreadId) {
      return;
    }

    setSideChatCreatingBySource((current) => ({
      ...current,
      [sourceThreadId]: true,
    }));
    setSideChatErrorBySource((current) => {
      if (!(sourceThreadId in current)) {
        return current;
      }
      const next = { ...current };
      delete next[sourceThreadId];
      return next;
    });

    try {
      if (!(await ensureThreadOpenable(targetThreadId))) {
        throw new Error(`Thread not found: ${targetThreadId}`);
      }
      rememberSideChatThreadId(targetThreadId);
      setSideChatThreadBySource((current) =>
        current[sourceThreadId] === targetThreadId
          ? current
          : {
              ...current,
              [sourceThreadId]: targetThreadId,
            },
      );
      persistSideChatThreadId(sourceThreadId, targetThreadId);
    } catch (openError) {
      const message =
        openError instanceof Error
          ? openError.message
          : `Failed to open thread: ${targetThreadId}`;
      setSideChatErrorBySource((current) => ({
        ...current,
        [sourceThreadId]: message,
      }));
      setError(message);
      throw openError;
    } finally {
      setSideChatCreatingBySource((current) => {
        if (!current[sourceThreadId]) {
          return current;
        }
        const next = { ...current };
        delete next[sourceThreadId];
        return next;
      });
    }
  }

  async function handleSideComposerSubmit(options?: {
    useAlternateFollowUpBehavior?: boolean;
  }) {
    const sourceThreadId = sideChatSourceThreadId;
    if (!sourceThreadId) {
      setError("Open a thread before starting side chat.");
      return;
    }
    if (sideComposerAttachmentUploadPending) {
      setError("Attachments are still uploading to gateway.");
      return;
    }

    const draft = sideComposerDraftForSource(sourceThreadId);
    const prompt = composePromptWithBrowserAnnotations(
      draft.text,
      draft.browserAnnotations,
      t,
    );
    const promptImages = [
      ...draft.images,
      ...browserAnnotationScreenshotImages(draft.browserAnnotations),
    ];
    const promptFiles = [...draft.files];
    const hasPromptPayload =
      Boolean(prompt) ||
      promptImages.length > 0 ||
      promptFiles.length > 0 ||
      draft.browserAnnotations.length > 0;
    if (!hasPromptPayload) {
      return;
    }

    const threadId = await ensureSideChatThread();
    if (!threadId) {
      return;
    }

    const runtime = selectThreadRuntime(messageStateRef.current, threadId);
    const liveStream = getLiveStreamState(threadId);
    const streamBusy = Boolean(
      liveStream &&
        ["connecting", "streaming", "reconciling"].includes(
          liveStream.streamStatus,
        ),
    );
    const sendingThread =
      (sideChatThreadId === threadId && sideChatIsSendingThread) ||
      Boolean(runtime && isRuntimeBusy(runtime.state)) ||
      streamBusy;

    if (sendingThread) {
      const intent = buildIntent({
        threadId,
        text: prompt,
        images: promptImages,
        files: promptFiles,
        source: "composer_queue",
        state: "queued_local",
      });
      dispatchMessageState({
        type: "intent/created",
        intent,
        enqueue: true,
      });
      deferredQueueDrainByThreadRef.current[threadId] = true;
      clearSideComposerDraft(sourceThreadId);
      setError(null);

      const followUpBehavior = options?.useAlternateFollowUpBehavior
        ? settingsDraft.followUpBehavior === "steer"
          ? "queue"
          : "steer"
        : settingsDraft.followUpBehavior;
      if (followUpBehavior === "steer" && sideChatCanSteerQueuedPrompt) {
        await steerQueuedIntent(intent, { canSteer: sideChatCanSteerQueuedPrompt });
      }
      return;
    }

    const intent = buildIntent({
      threadId,
      text: prompt,
      images: promptImages,
      files: promptFiles,
      source: "composer_send",
      state: "dispatch_requested",
      dispatchMode: "sync_send",
    });
    dispatchMessageState({
      type: "intent/created",
      intent,
      enqueue: false,
    });
    clearSideComposerDraft(sourceThreadId);
    setError(null);
    void runQueuedBatch(threadId, intent.intentId);
  }

  function sideChatStreamConsumerId(threadId: string): string {
    return `side-chat:${threadId}`;
  }

  return {
    appendSideComposerAttachments,
    ensureSideChatThread,
    handleSideComposerSubmit,
    openTaskThreadInSidePanel,
    removeSideComposerBrowserAnnotation,
    removeSideComposerFile,
    removeSideComposerImage,
    sideChatActiveToolGroupId,
    sideChatAgentLabel,
    sideChatCanSteerQueuedPrompt,
    sideChatComposerEditingLocked,
    sideChatComposerHasPayload,
    sideChatComposerLocked,
    sideChatComposerPlaceholder,
    sideChatComposerProviderType,
    sideChatComposerWorkspaceBranch,
    sideChatComposerWorkspaceMode,
    sideChatCreating,
    sideChatError,
    sideChatHistoryLoading,
    sideChatHistoryPagination,
    sideChatIsSendingThread,
    sideChatLiveStream,
    sideChatMessages,
    sideChatMessagesRef,
    sideChatQueue,
    sideChatRenderState,
    sideChatShowTailThinking,
    sideChatSourceThreadId,
    sideChatStreamConsumerId,
    sideChatThreadBot,
    sideChatThreadBotId,
    sideChatThreadId,
    sideChatThreadIdRef,
    sideChatThreadIdsRef,
    sideChatThreadLayoutRef,
    sideChatThreadSummary,
    sideChatVisiblePendingAckIntents,
    sideChatVisibleRemotePendingInputs,
    sideComposerAttachmentInputRef,
    sideComposerDraft,
    sideComposerTextareaRef,
    sideIgnoreComposerSubmitUntilRef,
    sideIsComposingRef,
    updateSideComposerDraft,
  };
}
