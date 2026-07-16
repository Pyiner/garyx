// SideChatPanel: the colocated side-chat feature component (endgame batch
// 5b-7b, docs/design/appshell-sidechat-colocation.md). Owns the render
// derivations, the composer behavior, and the side ThreadPage instance
// that used to be assembled inside AppShell from useSideChatController's
// 45-member return surface.
//
// Sources of truth (C3 — each domain single-sourced):
// - session store (bindings/drafts/transients): its own uSES subscription
// - mirror domains (transcript maps, machine, live streams): panel-local
//   uSES subscriptions over the SAME GatewayMirror the shell uses
// - root state (desktopState): useGatewayRoot
// - shell-truth chrome (active thread, bot groups, workspace pickers,
//   avatar catalog, slash commands, i18n, IPC-backed handlers): props
//
// The session lifetime stays in the shell store (5b-7a); this panel can
// unmount freely with the dock. ensureSideChatThread and
// ensureSideChatThread lives in side-chat-ops.ts because it is also dispatched
// while this panel is not mounted; the panel builds the same ops context for
// its submit path.

import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from "react";

import type {
  DesktopApiProviderType,
  DesktopChannelEndpoint,
  DesktopCustomAgent,
  DesktopProviderModels,
  DesktopSettings,
  DesktopState,
  DesktopThreadSummary,
  DesktopWorkspaceMode,
  MessageFileAttachment,
  MessageImageAttachment,
  SlashCommand,
  ThreadRuntimeInfo,
} from "@shared/contracts";

import type { Translate } from "../../i18n";
import {
  buildIntent,
  isRuntimeBusy,
  selectQueueIntentIds,
  selectThreadRuntime,
  type MessageIntent,
} from "../../message-machine";
import { automationForLatestThread } from "../../thread-model";
import {
  useGatewayMirror,
  useGatewayRoot,
  useThreadMirror,
} from "../../gateway-mirror/react";
import {
  pendingAckIntentsNotRepresented,
  representedUserIntentIds,
} from "../pending-ack-intents";
import {
  visibleRemotePendingInputsForThread,
  type PendingInputOriginRef,
} from "../pending-inputs";
import {
  deriveThreadComposerControlModel,
  deriveThreadActivityModel,
} from "../thread-activity";
import {
  browserAnnotationScreenshotImages,
  composePromptWithBrowserAnnotations,
} from "../useMessageDispatchController";
import {
  emptySideComposerDraft,
  type SideChatSessions,
  type SideComposerDraft,
} from "../side-chat-sessions";
import {
  ensureSideChatThread,
  type SideChatOpsContext,
} from "../side-chat-ops";
import {
  messagesNearBottom,
  messageTailSignature,
  scrollMessagesToLatest,
} from "./thread-transcript-scroll";
import { messagesNearEarlierUserTurnBoundary } from "../../gateway-mirror/transcript-materialize";
import type {
  BoundBot,
  LiveStreamState,
  UiTranscriptMessage,
} from "../types";
import { ThreadPage } from "./ThreadPage";
import type { ComponentProps } from "react";

const EMPTY_UI_TRANSCRIPT_MESSAGES: UiTranscriptMessage[] = [];

type ThreadPageProps = ComponentProps<typeof ThreadPage>;

export interface SideChatPanelProps {
  sessions: SideChatSessions;
  activeThread: DesktopThreadSummary | null;
  /** Shell-truth chrome shared with the main ThreadPage instance. */
  composerAgentOptions: ThreadPageProps["composerAgentOptions"];
  availableWorkspaceCount: number;
  newThreadWorkspaceEntry: ThreadPageProps["newThreadWorkspaceEntry"];
  newThreadWorkspaceMode: DesktopWorkspaceMode;
  preferredWorkspaceForNewThread: ThreadPageProps["preferredWorkspaceForNewThread"];
  selectableNewThreadWorkspaces: ThreadPageProps["selectableNewThreadWorkspaces"];
  threadAvatarCatalog: ThreadPageProps["threadAvatarCatalog"];
  newThreadProviderModels?: DesktopProviderModels | null;
  botGroups: ThreadPageProps["botGroups"];
  botBindingDisabled: boolean;
  workspaceMutation: ThreadPageProps["workspaceMutation"];
  slashCommands: SlashCommand[];
  slashCommandsLoaded: boolean;
  slashCommandsLoading: boolean;
  loadSlashCommands: () => Promise<void> | void;
  composerProviderType: DesktopApiProviderType;
  pendingAgentId: string | null;
  settingsDraft: DesktopSettings;
  desktopAgents: DesktopCustomAgent[];
  desktopAgentMap: Map<string, DesktopCustomAgent>;
  threadSummaryById: Map<string, DesktopThreadSummary>;
  /** Shell helper fns (module-level in AppShell, stable identities). */
  boundBotsForThread: (endpoints: DesktopChannelEndpoint[]) => BoundBot[];
  inferProviderTypeForThread: (
    threadId: string,
    threadInfo: ThreadRuntimeInfo | null,
    desktopState: DesktopState | null,
    desktopAgents: DesktopCustomAgent[],
  ) => DesktopApiProviderType | null;
  pendingInputOriginRefsForThread: (
    intentsById: Record<string, MessageIntent>,
    threadId: string | null,
  ) => PendingInputOriginRef[];
  prepareAttachmentUploads: (files: File[]) => Promise<
    Array<{
      id: string;
      kind: "image" | "file";
      name: string;
      mediaType: string;
      dataBase64: string;
    }>
  >;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setError: (error: string | null) => void;
  /** Shell-owned refs shared with the always-on shell effects. */
  sideChatMessagesRef: React.MutableRefObject<HTMLDivElement | null>;
  deferredQueueDrainByThreadRef: React.MutableRefObject<
    Record<string, boolean>
  >;
  /** Shell handlers reused verbatim from the main instance. */
  onAddWorkspace: () => void;
  onLocalWorkspaceFileLinkClick: (path: string) => void;
  onResumeProviderSession: ThreadPageProps["onResumeProviderSession"];
  onRetryFailedMessage: (message: UiTranscriptMessage) => void;
  onOpenThreadById: (threadId: string) => void;
  onOpenCapsule: ThreadPageProps["onOpenCapsule"];
  onReorderQueuedIntent: ThreadPageProps["onReorderQueuedIntent"];
  syncThreadBotBinding: (threadId: string, botId: string | null) => Promise<void>;
  t: Translate;
}

export function SideChatPanel({
  sessions,
  activeThread,
  composerAgentOptions,
  availableWorkspaceCount,
  newThreadWorkspaceEntry,
  newThreadWorkspaceMode,
  preferredWorkspaceForNewThread,
  selectableNewThreadWorkspaces,
  threadAvatarCatalog,
  botGroups,
  botBindingDisabled,
  workspaceMutation,
  slashCommands,
  slashCommandsLoaded,
  slashCommandsLoading,
  loadSlashCommands,
  composerProviderType,
  pendingAgentId,
  settingsDraft,
  desktopAgents,
  desktopAgentMap,
  threadSummaryById,
  boundBotsForThread,
  inferProviderTypeForThread,
  pendingInputOriginRefsForThread,
  prepareAttachmentUploads,
  setDesktopState,
  setError,
  sideChatMessagesRef,
  deferredQueueDrainByThreadRef,
  onAddWorkspace,
  onLocalWorkspaceFileLinkClick,
  onResumeProviderSession,
  onRetryFailedMessage,
  onOpenThreadById,
  onOpenCapsule,
  onReorderQueuedIntent,
  syncThreadBotBinding,
  t,
}: SideChatPanelProps) {
  const mirror = useGatewayMirror();
  const root = useGatewayRoot();
  const desktopState = root.desktopState;
  const sessionsSnapshot = useSyncExternalStore(
    sessions.subscribe,
    sessions.getSnapshot,
  );
  const messageState = useSyncExternalStore(
    useCallback(
      (onChange: () => void) => mirror.subscribeMachine(onChange),
      [mirror],
    ),
    () => mirror.getMachineState(),
  );
  const sideComposerAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const sideComposerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const sideIsComposingRef = useRef(false);
  const sideIgnoreComposerSubmitUntilRef = useRef(0);

  const sideComposerBySource = sessionsSnapshot.composerBySource;
  const sideComposerAttachmentUploadPending =
    sessionsSnapshot.attachmentUploadCount > 0;
  const sideChatThreadBySource = sessionsSnapshot.threadBySource;
  const sideChatCreatingBySource = sessionsSnapshot.creatingBySource;
  const sideChatErrorBySource = sessionsSnapshot.errorBySource;
  const sideChatHistoryLoading = sessionsSnapshot.historyLoading;

  const sideChatSourceThreadId = activeThread?.id?.trim() || null;
  const sideChatThreadId = sideChatSourceThreadId
    ? sideChatThreadBySource[sideChatSourceThreadId] || null
    : null;
  const sideChatMirror = useThreadMirror(sideChatThreadId);
  const sideChatCreating = sideChatSourceThreadId
    ? Boolean(sideChatCreatingBySource[sideChatSourceThreadId])
    : false;
  const sideChatError = sideChatSourceThreadId
    ? sideChatErrorBySource[sideChatSourceThreadId] || null
    : null;
  const sideComposerDraft = sideChatSourceThreadId
    ? sideComposerBySource[sideChatSourceThreadId] || emptySideComposerDraft()
    : emptySideComposerDraft();

  const sideChatThreadSummary = sideChatThreadId
    ? threadSummaryById.get(sideChatThreadId) || null
    : null;
  const sideChatAgent =
    sideChatThreadSummary?.agentId
      ? desktopAgentMap.get(sideChatThreadSummary.agentId) || null
      : null;
  const sideChatAgentLabel =
    sideChatAgent?.displayName ||
    sideChatThreadSummary?.agentId ||
    null;
  const sideChatComposerProviderType: DesktopApiProviderType =
    sideChatThreadId
      ? inferProviderTypeForThread(
          sideChatThreadId,
          sideChatMirror?.threadInfo || null,
          desktopState,
          desktopAgents,
        ) || "claude_code"
      : composerProviderType;
  const sideChatRawMessages = sideChatMirror?.messages ||
    EMPTY_UI_TRANSCRIPT_MESSAGES;
  const sideChatMessages = useMemo(
    () => [...sideChatRawMessages],
    [sideChatRawMessages],
  );
  const sideChatThreadInfo = sideChatMirror?.threadInfo || null;
  const sideChatThreadWorktree =
    sideChatThreadInfo?.worktree || sideChatThreadSummary?.worktree || null;
  const sideChatComposerWorkspaceMode: DesktopWorkspaceMode | null =
    sideChatThreadId && sideChatThreadWorktree ? "worktree" : null;
  const sideChatComposerWorkspaceBranch =
    sideChatThreadWorktree?.branch?.trim() || null;
  const sideChatRenderState = sideChatMirror?.renderState || null;
  const sideChatQueue = sideChatThreadId
    ? selectQueueIntentIds(messageState, sideChatThreadId)
        .map((intentId) => messageState.intentsById[intentId])
        .filter((intent): intent is MessageIntent => Boolean(intent))
    : [];
  const sideChatRuntime = selectThreadRuntime(messageState, sideChatThreadId);
  const sideChatLiveStream: LiveStreamState | null =
    sideChatMirror?.liveStream || null;
  const sideChatPendingAckIntents = useMemo(
    () =>
      (sideChatLiveStream?.pendingAckIntentIds || [])
        .map((intentId) => messageState.intentsById[intentId])
        .filter((intent): intent is MessageIntent => Boolean(intent)),
    [sideChatLiveStream?.pendingAckIntentIds, messageState.intentsById],
  );
  const representedSideChatIntentIds = useMemo(
    () => representedUserIntentIds(sideChatMessages),
    [sideChatMessages],
  );
  const sideChatVisiblePendingAckIntents = useMemo(
    () =>
      pendingAckIntentsNotRepresented(
        sideChatPendingAckIntents,
        representedSideChatIntentIds,
      ),
    [sideChatPendingAckIntents, representedSideChatIntentIds],
  );
  const sideChatRemotePendingInputs = sideChatThreadId
    ? sideChatMirror?.pendingRemoteInputs || []
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
  const sideChatHistoryPagination = sideChatMirror?.historyPagination || null;
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

  const sideChatMessageTailSignature = messageTailSignature(sideChatMessages);
  // Opening a side chat (or finishing its history load) always lands at the
  // latest message.
  useLayoutEffect(() => {
    if (!sideChatThreadId || sideChatHistoryLoading) {
      return;
    }
    scrollMessagesToLatest(sideChatMessagesRef.current);
  }, [sideChatHistoryLoading, sideChatThreadId]);
  // Tail growth only follows while the reader is already at the bottom —
  // never yank someone who scrolled up mid-stream.
  useLayoutEffect(() => {
    if (!sideChatThreadId || sideChatHistoryLoading) {
      return;
    }
    if (messagesNearBottom(sideChatMessagesRef.current)) {
      scrollMessagesToLatest(sideChatMessagesRef.current);
    }
  }, [sideChatMessageTailSignature]);

  function updateSideComposerDraft(
    sourceThreadId: string,
    updater: (current: SideComposerDraft) => SideComposerDraft,
  ) {
    sessions.updateDraft(sourceThreadId, updater);
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

  async function appendSideComposerAttachments(
    sourceThreadId: string,
    files: File[],
  ) {
    if (!files.length) {
      return;
    }

    sessions.beginAttachmentUpload();
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
      sessions.endAttachmentUpload();
      resetSideComposerAttachmentPicker();
    }
  }

  function opsContext(): SideChatOpsContext {
    return {
      sessions,
      mirror,
      sourceThreadId: sideChatSourceThreadId,
      activeThread,
      threadSummaryById,
      setDesktopState,
      setError,
    };
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

    const draft = sessions.draftFor(sourceThreadId);
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

    const threadId = await ensureSideChatThread(opsContext());
    if (!threadId) {
      return;
    }

    const machineNow = mirror.getMachineState();
    const runtime = selectThreadRuntime(machineNow, threadId);
    const liveStream = mirror.getThreadLiveStream(threadId);
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
      mirror.dispatchMachineAction({
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
        await mirror.steerQueuedIntent(intent, {
          canSteer: sideChatCanSteerQueuedPrompt,
        });
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
    mirror.dispatchMachineAction({
      type: "intent/created",
      intent,
      enqueue: false,
    });
    clearSideComposerDraft(sourceThreadId);
    setError(null);
    void mirror.runQueuedBatch(threadId, intent.intentId);
  }

  if (!sideChatSourceThreadId) {
    return (
      <div className="side-tool-empty">
        {t("Open a thread before starting side chat.")}
      </div>
    );
  }
  if (!sideChatThreadId) {
    return (
      <div className="side-tool-empty">
        {sideChatCreating
          ? t("Starting…")
          : sideChatError || t("Start a focused side thread.")}
      </div>
    );
  }
  return (
    <ThreadPage
      surfaceVariant="side-chat"
      taskTreeDocked={false}
      agentLabel={sideChatAgentLabel}
      composerAgentOptions={composerAgentOptions}
      activeMessages={sideChatMessages}
      activePendingAckIntents={sideChatVisiblePendingAckIntents}
      activePendingAutomationRun={null}
      activeToolGroupId={sideChatActiveToolGroupId}
      activeQueue={sideChatQueue}
      renderState={sideChatRenderState}
      activeThreadSummary={sideChatThreadSummary}
      activeThreadTitle={sideChatThreadSummary?.title || null}
      activeThreadRunId={
        sideChatLiveStream?.runId || sideChatThreadSummary?.recentRunId || null
      }
      availableWorkspaceCount={availableWorkspaceCount}
      composer={sideComposerDraft.text}
      composerAttachmentInputRef={sideComposerAttachmentInputRef}
      composerBrowserAnnotations={sideComposerDraft.browserAnnotations}
      composerFiles={sideComposerDraft.files}
      composerHasPayload={sideChatComposerHasPayload}
      composerImages={sideComposerDraft.images}
      composerEditingLocked={sideChatComposerEditingLocked}
      composerLocked={sideChatComposerLocked}
      composerPlaceholder={sideChatComposerPlaceholder}
      composerProviderType={sideChatComposerProviderType}
      composerResetKey={sideComposerDraft.resetKey}
      composerWorkspaceBranch={sideChatComposerWorkspaceBranch}
      composerWorkspaceMode={sideChatComposerWorkspaceMode}
      activeThreadBot={sideChatThreadBot}
      activeThreadBotId={sideChatThreadBotId}
      botBindingDisabled={botBindingDisabled}
      botGroups={botGroups}
      slashCommands={slashCommands}
      slashCommandsLoaded={slashCommandsLoaded}
      slashCommandsLoading={slashCommandsLoading}
      composerTextareaRef={sideComposerTextareaRef}
      historyLoading={sideChatHistoryLoading}
      historyLoadingEarlier={Boolean(sideChatHistoryPagination?.loadingBefore)}
      ignoreComposerSubmitUntilRef={sideIgnoreComposerSubmitUntilRef}
      inspectorOpen={false}
      isActiveSendingThread={sideChatIsSendingThread}
      canSteerQueuedPrompt={sideChatCanSteerQueuedPrompt}
      isComposingRef={sideIsComposingRef}
      messagesRef={sideChatMessagesRef}
      newThreadSelectedAgentId={sideChatThreadSummary?.agentId || pendingAgentId}
      newThreadWorkspaceEntry={newThreadWorkspaceEntry}
      newThreadWorkspaceMode={newThreadWorkspaceMode}
      onAddWorkspace={onAddWorkspace}
      onAppendComposerAttachments={(files) => {
        void appendSideComposerAttachments(sideChatSourceThreadId, files);
      }}
      onCancelIntent={(threadId, intentId) => {
        mirror.dispatchMachineAction({
          type: "intent/cancelled",
          threadId,
          intentId,
        });
      }}
      onComposerChange={(value) => {
        updateSideComposerDraft(sideChatSourceThreadId, (current) => ({
          ...current,
          text: value,
          textPresent: value.trim().length > 0,
        }));
        if (
          /^\/[a-z0-9_]*$/i.test(value) &&
          !slashCommandsLoaded &&
          !slashCommandsLoading
        ) {
          void loadSlashCommands();
        }
      }}
      onComposerCompositionEnd={(value) => {
        sideIsComposingRef.current = false;
        updateSideComposerDraft(sideChatSourceThreadId, (current) => ({
          ...current,
          text: value,
          textPresent: value.trim().length > 0,
        }));
        sideIgnoreComposerSubmitUntilRef.current = performance.now() + 80;
      }}
      onComposerCompositionStart={() => {
        sideIsComposingRef.current = true;
      }}
      onComposerInterrupt={() => {
        const current = sessions.sideChatThreadIdRef.current;
        if (current) {
          void mirror.interruptThread(current);
        }
      }}
      onComposerSubmit={handleSideComposerSubmit}
      onLocalWorkspaceFileLinkClick={onLocalWorkspaceFileLinkClick}
      onMarkIgnoreComposerSubmitWindow={() => {
        sideIgnoreComposerSubmitUntilRef.current = performance.now() + 80;
      }}
      onMessagesScroll={() => {
        const node = sideChatMessagesRef.current;
        if (
          sideChatThreadId &&
          node &&
          messagesNearEarlierUserTurnBoundary(node)
        ) {
          void mirror.loadOlderThreadHistoryPage(sideChatThreadId);
        }
      }}
      onMessagesUserScrollIntent={() => {}}
      onRemoveComposerFile={(fileId) => {
        removeSideComposerFile(sideChatSourceThreadId, fileId);
      }}
      onRemoveComposerImage={(imageId) => {
        removeSideComposerImage(sideChatSourceThreadId, imageId);
      }}
      onRemoveComposerBrowserAnnotation={(annotationId) => {
        removeSideComposerBrowserAnnotation(sideChatSourceThreadId, annotationId);
      }}
      onReorderQueuedIntent={onReorderQueuedIntent}
      onSelectNewThreadAgent={() => {}}
      onSelectNewThreadWorkspaceMode={() => {}}
      onResumeProviderSession={onResumeProviderSession}
      onRetryFailedMessage={(message) => {
        onRetryFailedMessage(message);
      }}
      onSelectBotBinding={(botId) => {
        if (sideChatThreadId) {
          void syncThreadBotBinding(sideChatThreadId, botId);
        }
      }}
      onOpenThreadById={(threadId) => {
        onOpenThreadById(threadId);
      }}
      onOpenCapsule={onOpenCapsule}
      onSelectWorkspace={() => {}}
      onSteerQueuedPrompt={(item) => {
        void mirror.steerQueuedIntent(item, {
          canSteer: sideChatCanSteerQueuedPrompt,
        });
      }}
      preferredWorkspaceForNewThread={preferredWorkspaceForNewThread}
      selectableNewThreadWorkspaces={selectableNewThreadWorkspaces}
      selectedThreadId={sideChatThreadId}
      showAutomationRunInitialPlaceholder={false}
      // Side chats fork the provider session without importing visible
      // history, so there is never parent history to wait for — the panel
      // opens as an empty thread instead of a loading placeholder.
      showHistoryLoadingPlaceholder={false}
      showTailThinking={sideChatShowTailThinking}
      threadAvatarCatalog={threadAvatarCatalog}
      visibleRemoteAwaitingAckInputs={sideChatVisibleRemotePendingInputs}
      visibleRemotePendingInputs={sideChatVisibleRemotePendingInputs}
      workspaceMutation={workspaceMutation}
    />
  );
}
