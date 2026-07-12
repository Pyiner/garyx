import {
  Fragment,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  type CSSProperties,
  type MutableRefObject,
  type ReactNode,
  type RefObject,
} from "react";
import { ArrowDown, CircleAlert, GitBranch, Repeat2 } from 'lucide-react';

import { Bubble, BubbleContent } from "@/components/ui/bubble";
import { Marker, MarkerContent, MarkerIcon } from "@/components/ui/marker";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "@/components/ui/message-scroller";

import type {
  DesktopApiProviderType,
  BrowserAnnotationCommentRequest,
  DesktopBotConsoleSummary,
  MessageFileAttachment,
  MessageImageAttachment,
  DesktopProviderModels,
  DesktopThreadSummary,
  DesktopWorkspace,
  DesktopWorkspaceMode,
  PendingThreadInput,
  RenderRateLimit,
  RenderState,
  RenderCapsuleCard,
  SlashCommand,
  TranscriptMessage,
} from "@shared/contracts";

import type { MessageIntent } from "../../message-machine";
import type { ComposerPendingUpload } from "../useMessageDispatchController";
import type { ThreadHistoryPaginationState } from "../../gateway-mirror/transcript-materialize";
import {
  TranscriptScrollBridge,
  useThreadTranscriptScroll,
  type TranscriptScrollIntent,
} from "./thread-transcript-scroll";
import {
  ComposerForm,
  type ComposerAgentOption,
} from "../../ComposerForm";
import { ComposerQueue } from "../../ComposerQueue";
import { RateLimitBanner } from "./RateLimitBanner";
import { NewThreadEmptyState } from "../../NewThreadEmptyState";
import {
  RichMessageContent,
  buildOptimisticTranscriptContent,
  splitRichMessageContentIntoBubbleParts,
  type MessageImagePreviewLoader,
} from "../../message-rich-content";
import { parseTaskNotificationText } from "../../task-notification";
import { parseRestartNoticeText } from "../../restart-notice";
import type { ThreadAvatarCatalog } from "../../thread-avatar";
import {
  buildThreadViewRowsWithLocalUsers,
  type MessagesBySeq,
  type RenderTranscriptBlock,
  type TurnRow,
  type UserTurnActivityRow,
} from "../../render-view-model";
import { TurnSummary } from "../../turn-summary";
import { ToolTraceGroup } from "../../tool-trace";
import { CapsuleChatCardList } from "./CapsuleChatCard";
import { ThreadLogDock } from "./ThreadLogDock";
import { ThreadTaskTreePopover } from "./ThreadTaskTreePopover";
import { shouldShowThreadTaskTreePopover } from "./thread-task-tree-popover-model";
import { useI18n } from "../../i18n";
import type {
  PendingAutomationRun,
  UiTranscriptMessage,
} from "../types";
import { RUN_LOADING_LABEL } from "../loading-labels";
import { resolveThreadFilePreviewTarget } from "../workspace-helpers";

function normalizeMessageText(value: string | undefined): string {
  return value?.trim() || "";
}

const LOOP_CONTINUATION_SUMMARY = "In loop, continue.";

function isLoopContinuationMessage(message: TranscriptMessage): boolean {
  return (
    Boolean(message.internal) && message.internalKind === "loop_continuation"
  );
}

function displayTranscriptMessageText(message: UiTranscriptMessage): string {
  if (isLoopContinuationMessage(message) && message.role === "system") {
    return LOOP_CONTINUATION_SUMMARY;
  }
  return message.text;
}

function renderUserMessageBubbleParts({
  keyPrefix,
  text,
  content,
  pending,
  error,
  retryLabel,
  onRetry,
  onLocalFileLinkClick,
  loadImagePreview,
  markUserTurnStart = true,
}: {
  keyPrefix: string;
  text: string;
  content?: unknown;
  pending?: boolean;
  error?: boolean;
  retryLabel?: string;
  onRetry?: () => void;
  onLocalFileLinkClick: (path: string) => void;
  loadImagePreview?: MessageImagePreviewLoader;
  markUserTurnStart?: boolean;
}): ReactNode {
  const parts = splitRichMessageContentIntoBubbleParts({
    altPrefix: "user",
    content,
    text,
  });

  const bubbles = parts.map((part, index) => {
    const userTurnMarker =
      markUserTurnStart && index === 0
        ? { "data-user-turn-start": "true" }
        : {};
    if (part.kind === "image" || part.kind === "file") {
      return (
        <article
          className={`message-attachment-bubble message-attachment-bubble-${part.kind} user ${pending ? "pending" : ""} ${error ? "error" : ""}`}
          key={`${keyPrefix}:${part.key}`}
          {...userTurnMarker}
        >
          <RichMessageContent
            altPrefix="user"
            content={part.content}
            loadImagePreview={loadImagePreview}
            onLocalFileLinkClick={onLocalFileLinkClick}
            text={part.text}
          />
        </article>
      );
    }

    const cardPartClass =
      part.kind !== "text"
        ? ""
        : parseTaskNotificationText(part.text) !== null
          ? "task-notification-message "
          : parseRestartNoticeText(part.text) !== null
            ? "restart-notice-message "
            : "";
    if (cardPartClass) {
      return (
        <article
          className={`message-bubble ${cardPartClass}user ${pending ? "pending" : ""} ${error ? "error" : ""}`}
          key={`${keyPrefix}:${part.key}`}
          {...userTurnMarker}
        >
          <RichMessageContent
            altPrefix="user"
            content={part.content}
            loadImagePreview={loadImagePreview}
            onLocalFileLinkClick={onLocalFileLinkClick}
            text={part.text}
          />
        </article>
      );
    }
    return (
      <Bubble
        align="end"
        className={`max-w-[77%] ${pending ? "opacity-80" : ""}`}
        key={`${keyPrefix}:${part.key}`}
        variant="muted"
        {...userTurnMarker}
      >
        <BubbleContent className="rounded-[20px]">
          <RichMessageContent
            altPrefix="user"
            content={part.content}
            loadImagePreview={loadImagePreview}
            onLocalFileLinkClick={onLocalFileLinkClick}
            text={part.text}
          />
        </BubbleContent>
      </Bubble>
    );
  });

  if (!error) {
    return bubbles;
  }
  return [
    ...bubbles,
    <div className="message-error-note" key={`${keyPrefix}:error-note`} role="status">
      <CircleAlert aria-hidden size={12} strokeWidth={2} />
      {onRetry && retryLabel ? (
        <button className="message-error-note-retry" onClick={onRetry} type="button">
          {retryLabel}
        </button>
      ) : null}
    </div>,
  ];
}

type ThreadPageProps = {
  surfaceVariant?: "default" | "side-chat";
  activeMessages: UiTranscriptMessage[];
  activePendingAckIntents: MessageIntent[];
  agentLabel?: string | null;
  composerAgentOptions?: ComposerAgentOption[];
  activePendingAutomationRun: PendingAutomationRun | null;
  activeToolGroupId: string | null;
  activeQueue: MessageIntent[];
  renderState: RenderState | null;
  activeThreadSummary: DesktopThreadSummary | null;
  activeThreadTitle: string | null;
  activeThreadRunId: string | null;
  availableWorkspaceCount: number;
  composer: string;
  composerAttachmentInputRef: RefObject<HTMLInputElement | null>;
  composerBrowserAnnotations: BrowserAnnotationCommentRequest[];
  composerFiles: MessageFileAttachment[];
  composerHasPayload: boolean;
  composerImages: MessageImageAttachment[];
  composerPendingUploads?: ComposerPendingUpload[];
  composerEditingLocked: boolean;
  composerLocked: boolean;
  composerPlaceholder: string;
  composerProviderType: DesktopApiProviderType;
  composerWorkspaceBranch: string | null;
  composerWorkspaceMode: DesktopWorkspaceMode | null;
  composerResetKey: number;
  activeThreadBot: DesktopBotConsoleSummary | null;
  activeThreadBotId: string | null;
  botBindingDisabled: boolean;
  botGroups: DesktopBotConsoleSummary[];
  slashCommands: SlashCommand[];
  slashCommandsLoaded: boolean;
  slashCommandsLoading: boolean;
  historyLoading: boolean;
  historyLoadingEarlier: boolean;
  inspectorOpen: boolean;
  isActiveSendingThread: boolean;
  canSteerQueuedPrompt: boolean;
  messagesRef: RefObject<HTMLDivElement | null>;
  newThreadSelectedAgentId: string;
  newThreadProviderModels?: DesktopProviderModels | null;
  newThreadAgentConfiguredModel?: string | null;
  newThreadSelectedModel?: string | null;
  newThreadSelectedReasoningEffort?: string | null;
  newThreadSelectedServiceTier?: string | null;
  threadProviderModels?: DesktopProviderModels | null;
  threadEffectiveModel?: string | null;
  threadEffectiveReasoningEffort?: string | null;
  threadEffectiveServiceTier?: string | null;
  threadSelectedModel?: string | null;
  threadSelectedReasoningEffort?: string | null;
  threadSelectedServiceTier?: string | null;
  newThreadWorkspaceEntry: DesktopWorkspace | null;
  newThreadWorkspaceMode: DesktopWorkspaceMode;
  selectableNewThreadWorkspaces: DesktopWorkspace[];
  selectedThreadId: string | null;
  showAutomationRunInitialPlaceholder: boolean;
  showHistoryLoadingPlaceholder: boolean;
  showTailThinking: boolean;
  rateLimit?: RenderRateLimit | null;
  onRateLimitContinue?: () => void | Promise<unknown>;
  taskTreeDocked: boolean;
  threadLayoutRef: RefObject<HTMLDivElement | null>;
  threadLayoutStyle?: CSSProperties;
  threadLogsMaxWidth: number;
  threadLogsDocked: boolean;
  threadLogsOpen: boolean;
  threadLogsPanelWidth: number;
  threadLogsResizing: boolean;
  threadAvatarCatalog: ThreadAvatarCatalog;
  visibleRemoteAwaitingAckInputs: PendingThreadInput[];
  visibleRemotePendingInputs: PendingThreadInput[];
  workspaceMutation: string | null;
  composerTextareaRef: RefObject<HTMLTextAreaElement | null>;
  isComposingRef: MutableRefObject<boolean>;
  ignoreComposerSubmitUntilRef: MutableRefObject<number>;
  onAddWorkspace: () => void;
  onAppendComposerAttachments: (files: File[]) => void;
  onCancelIntent: (threadId: string, intentId: string) => void;
  onComposerChange: (value: string) => void;
  onComposerCompositionEnd: (value: string) => void;
  onComposerCompositionStart: () => void;
  onComposerInterrupt: () => void;
  onComposerSubmit: (options?: {
    useAlternateFollowUpBehavior?: boolean;
  }) => void;
  onLocalWorkspaceFileLinkClick: (path: string) => void;
  /**
   * Side-chat instance only: lightweight scroll handlers. The main
   * instance passes `scrollIntent` instead and the colocated hook owns
   * the container handlers.
   */
  onMessagesScroll?: () => void;
  onMessagesUserScrollIntent?: () => void;
  /** Main transcript instance: the shell-owned scroll intent bundle. */
  scrollIntent?: TranscriptScrollIntent | null;
  activeHistoryPagination?: ThreadHistoryPaginationState | null;
  activeThreadMessageKey?: string | null;
  onMarkIgnoreComposerSubmitWindow: () => void;
  onRemoveComposerFile: (fileId: string) => void;
  onRemoveComposerImage: (imageId: string) => void;
  onRemoveComposerPendingUpload?: (uploadId: string) => void;
  onRemoveComposerBrowserAnnotation: (annotationId: string) => void;
  onReorderQueuedIntent: (
    threadId: string,
    draggedIntentId: string,
    targetIntentId: string,
    position: "before" | "after",
  ) => void;
  onSelectNewThreadAgent: (agentId: string) => void;
  onSelectNewThreadModel?: (model: string | null) => void;
  onSelectNewThreadReasoningEffort?: (effort: string | null) => void;
  onSelectNewThreadServiceTier?: (tier: string | null) => void;
  onSelectThreadModel?: (model: string | null) => void;
  onSelectThreadReasoningEffort?: (effort: string | null) => void;
  onSelectThreadServiceTier?: (tier: string | null) => void;
  onSelectNewThreadWorkspaceMode: (mode: DesktopWorkspaceMode) => void;
  onResumeProviderSession: (sessionId: string) => Promise<void>;
  onRetryFailedMessage?: (message: UiTranscriptMessage) => void;
  onSelectBotBinding: (botId: string | null) => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onThreadLogsUnreadChange: (hasUnread: boolean) => void;
  onThreadLogsResizeKeyDown: (
    event: React.KeyboardEvent<HTMLDivElement>,
  ) => void;
  onThreadLogsResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  onSteerQueuedPrompt: (intent: MessageIntent) => void;
  onOpenThreadById: (threadId: string) => void;
  onOpenCapsule?: (card: RenderCapsuleCard) => void;
  preferredWorkspaceForNewThread: DesktopWorkspace | null;
};

export function ThreadPage({
  surfaceVariant = "default",
  agentLabel,
  composerAgentOptions,
  activeMessages,
  activePendingAckIntents,
  activePendingAutomationRun,
  activeToolGroupId,
  activeQueue,
  renderState,
  activeThreadSummary,
  activeThreadTitle,
  activeThreadRunId,
  availableWorkspaceCount,
  composer,
  composerAttachmentInputRef,
  composerBrowserAnnotations,
  composerFiles,
  composerHasPayload,
  composerImages,
  composerPendingUploads,
  composerEditingLocked,
  composerLocked,
  composerPlaceholder,
  composerProviderType,
  composerWorkspaceBranch,
  composerWorkspaceMode,
  composerResetKey,
  activeThreadBot,
  activeThreadBotId,
  botBindingDisabled,
  botGroups,
  slashCommands,
  slashCommandsLoaded,
  slashCommandsLoading,
  composerTextareaRef,
  historyLoading,
  historyLoadingEarlier,
  ignoreComposerSubmitUntilRef,
  inspectorOpen,
  isActiveSendingThread,
  canSteerQueuedPrompt,
  isComposingRef,
  messagesRef,
  newThreadSelectedAgentId,
  newThreadProviderModels,
  newThreadAgentConfiguredModel,
  newThreadSelectedModel,
  newThreadSelectedReasoningEffort,
  newThreadSelectedServiceTier,
  threadProviderModels,
  threadEffectiveModel,
  threadEffectiveReasoningEffort,
  threadEffectiveServiceTier,
  threadSelectedModel,
  threadSelectedReasoningEffort,
  threadSelectedServiceTier,
  newThreadWorkspaceEntry,
  newThreadWorkspaceMode,
  onAddWorkspace,
  onAppendComposerAttachments,
  onCancelIntent,
  onComposerChange,
  onComposerCompositionEnd,
  onComposerCompositionStart,
  onComposerInterrupt,
  onComposerSubmit,
  onLocalWorkspaceFileLinkClick,
  onMessagesScroll,
  onMessagesUserScrollIntent,
  scrollIntent = null,
  activeHistoryPagination = null,
  activeThreadMessageKey = null,
  onMarkIgnoreComposerSubmitWindow,
  onRemoveComposerFile,
  onRemoveComposerImage,
  onRemoveComposerPendingUpload,
  onRemoveComposerBrowserAnnotation,
  onReorderQueuedIntent,
  onSelectNewThreadAgent,
  onSelectNewThreadModel,
  onSelectNewThreadReasoningEffort,
  onSelectNewThreadServiceTier,
  onSelectThreadModel,
  onSelectThreadReasoningEffort,
  onSelectThreadServiceTier,
  onSelectNewThreadWorkspaceMode,
  onResumeProviderSession,
  onRetryFailedMessage,
  onSelectBotBinding,
  onSelectWorkspace,
  onSteerQueuedPrompt,
  onOpenThreadById,
  onOpenCapsule,
  onThreadLogsUnreadChange,
  onThreadLogsResizeKeyDown,
  onThreadLogsResizeStart,
  preferredWorkspaceForNewThread,
  selectableNewThreadWorkspaces,
  selectedThreadId,
  showAutomationRunInitialPlaceholder,
  showHistoryLoadingPlaceholder,
  showTailThinking,
  rateLimit,
  onRateLimitContinue,
  taskTreeDocked,
  threadLayoutRef,
  threadLayoutStyle,
  threadLogsMaxWidth,
  threadLogsDocked,
  threadLogsOpen,
  threadLogsPanelWidth,
  threadLogsResizing,
  threadAvatarCatalog,
  visibleRemoteAwaitingAckInputs,
  visibleRemotePendingInputs,
  workspaceMutation,
}: ThreadPageProps) {
  const { t } = useI18n();
  const loadTranscriptImagePreview = useCallback<MessageImagePreviewLoader>(
    async (path) => {
      const target = resolveThreadFilePreviewTarget(
        activeThreadSummary?.workspacePath,
        path,
      );
      if (!target) {
        return null;
      }
      const preview = await window.garyxDesktop.previewWorkspaceFile({
        workspacePath: target.workspacePath,
        filePath: target.filePath,
      });
      if (preview.previewKind !== "image" || !preview.dataBase64?.trim()) {
        return null;
      }
      return {
        src: `data:${preview.mediaType || "image/png"};base64,${preview.dataBase64}`,
        alt: preview.name,
      };
    },
    [activeThreadSummary?.workspacePath],
  );
  // Colocated transcript scroll (endgame batch 5b): stick-to-bottom,
  // prepend anchoring, and scroll-triggered older-page loads. Null for
  // the side-chat instance, which passes lightweight handlers instead.
  const transcriptScroll = useThreadTranscriptScroll({
    activeHistoryPagination,
    activeMessages,
    activeThreadMessageKey,
    historyLoading,
    messagesRef,
    scrollIntent,
  });
  const handleMessagesScroll =
    transcriptScroll?.onMessagesScroll ?? onMessagesScroll;
  const handleMessagesUserScrollIntent =
    transcriptScroll?.onMessagesUserScrollIntent ?? onMessagesUserScrollIntent;
  const composerShellWrapRef = useRef<HTMLDivElement | null>(null);
  const threadMainRef = useRef<HTMLDivElement | null>(null);
  const isSideChatSurface = surfaceVariant === "side-chat";
  // Resolve render_state seq refs against committed bodies by the raw record
  // seq stamped at the wire boundary (TranscriptMessage.seq). The message id is
  // unreliable here — optimistic reconciliation rewrites it to a stable id.
  const messagesBySeq = useMemo<MessagesBySeq>(() => {
    const map = new Map<number, TranscriptMessage>();
    for (const message of activeMessages) {
      if (typeof message.seq === "number") {
        map.set(message.seq, message);
      }
    }
    return map;
  }, [activeMessages]);
  const turnRows = useMemo(
    () => buildThreadViewRowsWithLocalUsers(
      renderState,
      messagesBySeq,
      activeMessages,
    ),
    [renderState, messagesBySeq, activeMessages],
  );
  const composerSelectedAgentId = selectedThreadId
    ? activeThreadSummary?.agentId?.trim() || undefined
    : newThreadSelectedAgentId;
  const emptyNewThread =
    !selectedThreadId &&
    !activeMessages.length &&
    !historyLoading &&
    !showAutomationRunInitialPlaceholder;
  const newThreadPromptTitle = "What do you want Garyx to build?";
  const composerContext = selectedThreadId && composerWorkspaceMode ? (
    <div
      aria-label={t("Workspace mode")}
      className="thread-composer-status"
    >
      <span className="thread-composer-status-pill thread-composer-status-branch">
        <GitBranch aria-hidden size={14} strokeWidth={1.65} />
        <span>
          {composerWorkspaceBranch?.trim() || t("Worktree")}
        </span>
      </span>
    </div>
  ) : null;
  useLayoutEffect(() => {
    const threadMain = threadMainRef.current;
    const composerShellWrap = composerShellWrapRef.current;
    if (!threadMain || !composerShellWrap) {
      return;
    }

    const syncComposerHeight = () => {
      const composerHeight = Math.ceil(
        composerShellWrap.getBoundingClientRect().height,
      );
      threadMain.style.setProperty(
        "--composer-overlay-height",
        `${composerHeight}px`,
      );
      // Offset scroll-clip above half the composer so the runtime value
      // lands on 72px at the default composer height (avoids the
      // unlucky 4-bearing values 66 and 70±). The visible gap above
      // the composer is `2 × scroll-clip + clearance − composerHeight`;
      // this +6 offset adds 12 to that gap and is paired with
      // `--composer-message-clearance` chosen in styles.css to keep
      // the visible gap at the desired 48px (+50% of the legacy 32).
      threadMain.style.setProperty(
        "--composer-scroll-clip-height",
        `${Math.ceil(composerHeight / 2) + 6}px`,
      );
    };

    syncComposerHeight();
    if (typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(syncComposerHeight);
    observer.observe(composerShellWrap);
    return () => {
      observer.disconnect();
    };
  }, []);

  return (
    <div
      className={`thread-layout ${isSideChatSurface ? "thread-layout--side-chat" : ""} ${inspectorOpen ? "with-inspector-panel" : ""} ${threadLogsOpen ? `with-log-panel ${threadLogsDocked ? "log-panel-docked" : "log-panel-overlay"}` : ""} ${threadLogsResizing ? "log-panel-resizing" : ""}`}
      ref={threadLayoutRef}
      style={threadLayoutStyle}
    >
      <div
        className={`thread-main ${isSideChatSurface ? "thread-main--side-chat" : ""} ${emptyNewThread ? "new-thread-centered" : ""}`}
        ref={threadMainRef}
      >
        {shouldShowThreadTaskTreePopover({
          inspectorOpen,
          isSideChatSurface,
          selectedThreadId,
          threadLogsOpen,
        }) && selectedThreadId ? (
          <ThreadTaskTreePopover
            taskTreeDocked={taskTreeDocked}
            threadId={selectedThreadId}
            threadAvatarCatalog={threadAvatarCatalog}
            onOpenThread={onOpenThreadById}
          />
        ) : null}
        <MessageScrollerProvider autoScroll>
          <MessageScroller className="messages-scroller">
          <MessageScrollerViewport
            className="messages"
            onPointerDown={handleMessagesUserScrollIntent}
            onScroll={handleMessagesScroll}
            onTouchStart={handleMessagesUserScrollIntent}
            onWheel={handleMessagesUserScrollIntent}
            preserveScrollOnPrepend
            ref={messagesRef}
          >
          <MessageScrollerContent className="messages-content">
          {historyLoadingEarlier ? (
            <Marker
              aria-label={t("Loading earlier messages")}
              className="min-h-7 py-0.5"
              role="status"
              variant="separator"
            >
              <MarkerIcon>
                <span aria-hidden="true" className="message-history-page-spinner" />
              </MarkerIcon>
            </Marker>
          ) : null}

          {!activeMessages.length &&
          !historyLoading &&
          !showAutomationRunInitialPlaceholder ? (
            selectedThreadId ? (
              <div
                className={`empty-state ${isSideChatSurface ? "empty-state--side-chat" : ""}`}
              >
                <span className="eyebrow">
                  {t(isSideChatSurface ? "Side Chat" : "Ready")}
                </span>
                <h3>
                  {t(
                    isSideChatSurface
                      ? "Ask without changing the main thread"
                      : "Continue the current thread",
                  )}
                </h3>
                <p>
                  {t(
                    isSideChatSurface
                      ? "This hidden fork keeps its own messages while the main conversation stays untouched."
                      : "Every thread is replayable from gateway history and can continue on this Mac.",
                  )}
                </p>
              </div>
            ) : null
          ) : null}

          {showHistoryLoadingPlaceholder ? (
            <Bubble
              className="w-fit max-w-[min(100%,736px)] self-start text-[color:var(--color-token-text-tertiary,var(--color-token-description-foreground))]"
              variant="ghost"
            >
              <BubbleContent>
                <div
                  aria-label={t("Loading thread history")}
                  className="message-loading"
                >
                  <p className="message-loading-label">{t("Loading thread history…")}</p>
                  <span aria-hidden="true" className="message-loading-dots">
                    <span />
                    <span />
                    <span />
                  </span>
                </div>
              </BubbleContent>
            </Bubble>
          ) : null}

          {showAutomationRunInitialPlaceholder && activePendingAutomationRun ? (
            <>
              <Bubble
                align="end"
                className="max-w-[77%]"
                data-user-turn-start="true"
                variant="muted"
              >
                <BubbleContent className="rounded-[20px]">
                  <RichMessageContent
                    altPrefix="user"
                    content={buildOptimisticTranscriptContent(
                      activePendingAutomationRun.prompt,
                      [],
                      [],
                    )}
                    loadImagePreview={loadTranscriptImagePreview}
                    onLocalFileLinkClick={onLocalWorkspaceFileLinkClick}
                    text={activePendingAutomationRun.prompt}
                  />
                </BubbleContent>
              </Bubble>
              <Bubble
                className="w-fit max-w-[min(100%,736px)] self-start text-[color:var(--color-token-text-tertiary,var(--color-token-description-foreground))]"
                variant="ghost"
              >
                <BubbleContent>
                  <div
                    aria-label={t("Garyx is working")}
                    className="message-loading"
                  >
                    <p className="message-loading-label message-loading-label--thinking">
                      {t(RUN_LOADING_LABEL)}
                    </p>
                  </div>
                </BubbleContent>
              </Bubble>
            </>
          ) : null}

          {(() => {
            const renderBlockBody = (
              block: RenderTranscriptBlock,
              options: { markUserTurnStart?: boolean } = {},
            ): ReactNode => {
              if (block.kind === "tool_group") {
                return (
                  <article
                    className="message-bubble tool-cluster"
                    key={`${block.key}:body`}
                  >
                    <ToolTraceGroup
                      active={block.key === activeToolGroupId}
                      defaultExpanded={block.defaultExpanded}
                      entries={block.entries}
                      loadImagePreview={loadTranscriptImagePreview}
                      onThreadNavigate={onOpenThreadById}
                    />
                  </article>
                );
              }
              const entry = block.entry;
              const loopContinuation = isLoopContinuationMessage(entry.message);
              const displayText = displayTranscriptMessageText(entry.message);
              const cardMessageClass =
                entry.message.pending || loopContinuation
                  ? null
                  : parseTaskNotificationText(displayText) !== null
                    ? "task-notification-message"
                    : parseRestartNoticeText(displayText) !== null
                      ? "restart-notice-message"
                      : null;
              if (cardMessageClass) {
                return (
                  <article
                    key={`${block.key}:body`}
                    className={`message-bubble ${cardMessageClass} ${entry.message.role} ${entry.message.error ? "error" : ""}`}
                  >
                    <RichMessageContent
                      altPrefix={entry.message.role}
                      content={entry.message.content}
                      loadImagePreview={loadTranscriptImagePreview}
                      onLocalFileLinkClick={onLocalWorkspaceFileLinkClick}
                      text={displayText}
                    />
                  </article>
                );
              }
              if (entry.message.role === "user" && !loopContinuation) {
                return renderUserMessageBubbleParts({
                  keyPrefix: `${block.key}:body`,
                  text: displayText,
                  content: entry.message.content,
                  pending: entry.message.pending,
                  error: entry.message.error,
                  loadImagePreview: loadTranscriptImagePreview,
                  retryLabel: t("Retry"),
                  onRetry:
                    onRetryFailedMessage &&
                    (entry.message as UiTranscriptMessage).intentId
                      ? () => onRetryFailedMessage(entry.message as UiTranscriptMessage)
                      : undefined,
                  onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
                  markUserTurnStart: options.markUserTurnStart !== false,
                });
              }
              if (loopContinuation) {
                return (
                  <Marker
                    className="my-0.5"
                    key={`${block.key}:body`}
                    variant="separator"
                  >
                    <MarkerIcon>
                      <Repeat2 aria-hidden size={14} strokeWidth={1.8} />
                    </MarkerIcon>
                    <MarkerContent>{LOOP_CONTINUATION_SUMMARY}</MarkerContent>
                  </Marker>
                );
              }
              if (entry.message.role === "assistant" || entry.message.error) {
                return (
                  <Bubble
                    className={
                      entry.message.error
                        ? "max-w-[min(77%,680px)] self-start"
                        : `w-[min(100%,736px)] self-start ${entry.message.pending ? "text-[color:var(--color-token-text-tertiary,var(--color-token-description-foreground))]" : ""}`
                    }
                    key={`${block.key}:body`}
                    variant={entry.message.error ? "destructive" : "ghost"}
                  >
                    <BubbleContent
                      className={entry.message.error ? "rounded-[20px]" : "border-0"}
                    >
                      <RichMessageContent
                        altPrefix={entry.message.role}
                        content={entry.message.content}
                        loadImagePreview={loadTranscriptImagePreview}
                        onLocalFileLinkClick={onLocalWorkspaceFileLinkClick}
                        text={displayText}
                      />
                    </BubbleContent>
                  </Bubble>
                );
              }
              return (
                <article
                  key={`${block.key}:body`}
                  className={`message-bubble ${entry.message.role} ${entry.message.pending ? "pending" : ""} ${entry.message.error ? "error" : ""}`}
                >
                  <RichMessageContent
                    altPrefix={entry.message.role}
                    content={entry.message.content}
                    loadImagePreview={loadTranscriptImagePreview}
                    onLocalFileLinkClick={onLocalWorkspaceFileLinkClick}
                    text={displayText}
                  />
                </article>
              );
            };

            // Running/elapsed state comes from render_state (`step.running`,
            // which already defers the final answer while the run is busy), so
            // the view only maps the server view model.
            const renderActivityRow = (
              activityRow: UserTurnActivityRow,
            ): ReactNode => {
              if (activityRow.kind === "flat") {
                return (
                  <Fragment key={activityRow.key}>
                    {renderBlockBody(activityRow.block)}
                  </Fragment>
                );
              }
              const turn: TurnRow = activityRow;
              return (
                <Fragment key={turn.key}>
                  <TurnSummary turn={turn}>
                    {turn.steps.map((step) => (
                      <Fragment key={step.key}>
                        {renderBlockBody(step)}
                      </Fragment>
                    ))}
                  </TurnSummary>
                  {turn.finalBlock ? renderBlockBody(turn.finalBlock) : null}
                </Fragment>
              );
            };

            // Each transcript row rides its own MessageScrollerItem so
            // off-screen turns skip layout via content-visibility while
            // the primitive tracks them for prepend anchoring.
            const renderTurnRowBody = (
              row: (typeof turnRows)[number],
            ): ReactNode => {
              if (row.kind === "flat") {
                return renderBlockBody(row.block);
              }
              if (row.kind === "turn") {
                return renderActivityRow(row);
              }
              if (row.kind === "capsule_only") {
                return (
                  <CapsuleChatCardList
                    cards={row.capsuleCards}
                    onOpenCapsule={onOpenCapsule}
                  />
                );
              }
              return (
                <>
                  {renderBlockBody(row.userBlock, {
                    markUserTurnStart: true,
                  })}
                  {row.activityRows.map((activityRow) =>
                    renderActivityRow(activityRow),
                  )}
                  {row.capsuleCards.length ? (
                    <CapsuleChatCardList
                      cards={row.capsuleCards}
                      onOpenCapsule={onOpenCapsule}
                    />
                  ) : null}
                </>
              );
            };

            return turnRows.map((row) => (
              <MessageScrollerItem
                className="messages-item"
                key={row.key}
                messageId={row.key}
              >
                {renderTurnRowBody(row)}
              </MessageScrollerItem>
            ));
          })()}

          {activePendingAckIntents.map((intent) =>
            renderUserMessageBubbleParts({
              keyPrefix: `pending-ack:${intent.intentId}`,
              text: intent.text,
              content: buildOptimisticTranscriptContent(
                intent.text,
                intent.images,
                intent.files,
              ),
              loadImagePreview: loadTranscriptImagePreview,
              onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
            }),
          )}

          {visibleRemotePendingInputs.map((input) =>
            renderUserMessageBubbleParts({
              keyPrefix: `remote-pending:${input.id}`,
              text: input.text,
              content: input.content,
              loadImagePreview: loadTranscriptImagePreview,
              onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
            }),
          )}

          {showTailThinking ? (
            <Bubble
              className="w-fit max-w-[min(100%,736px)] self-start text-[color:var(--color-token-text-tertiary,var(--color-token-description-foreground))]"
              variant="ghost"
            >
              <BubbleContent>
                <div
                  aria-label={t("Garyx is working")}
                  className="message-loading"
                >
                  <p className="message-loading-label message-loading-label--thinking">
                    {t(RUN_LOADING_LABEL)}
                  </p>
                </div>
              </BubbleContent>
            </Bubble>
          ) : null}

          <RateLimitBanner onContinue={onRateLimitContinue} rateLimit={rateLimit} />
          </MessageScrollerContent>
          </MessageScrollerViewport>
          <MessageScrollerButton behavior="smooth" className="rounded-full shadow-sm">
            <ArrowDown aria-hidden size={16} strokeWidth={2} />
            <span className="sr-only">{t("Scroll to latest")}</span>
          </MessageScrollerButton>
          <TranscriptScrollBridge
            activeMessages={activeMessages}
            activeThreadMessageKey={activeThreadMessageKey}
            historyLoading={historyLoading}
            scrollIntent={scrollIntent}
          />
          </MessageScroller>
          </MessageScrollerProvider>

        <div className="composer-shell-wrap" ref={composerShellWrapRef}>
            {emptyNewThread ? (
              <h1 className="new-thread-prompt-title">
                {newThreadPromptTitle}
              </h1>
            ) : null}
            <div
              className={`composer-shell ${activeQueue.length ? "has-queue" : ""}`}
            >
            <ComposerQueue
              activeQueue={activeQueue}
              canSteerQueuedPrompt={canSteerQueuedPrompt}
              isActiveSendingThread={isActiveSendingThread}
              onCancelIntent={onCancelIntent}
              onReorderQueuedIntent={onReorderQueuedIntent}
              onSteerQueuedPrompt={onSteerQueuedPrompt}
            />

            <ComposerForm
              activeQueueLength={activeQueue.length}
              composer={composer}
              composerContext={composerContext}
              composerAttachmentInputRef={composerAttachmentInputRef}
              composerBrowserAnnotations={composerBrowserAnnotations}
              composerFiles={composerFiles}
              composerHasPayload={composerHasPayload}
              composerImages={composerImages}
              composerPendingUploads={composerPendingUploads}
              composerEditingLocked={composerEditingLocked}
              composerLocked={composerLocked}
              composerPlaceholder={composerPlaceholder}
              composerProviderType={composerProviderType}
              composerResetKey={composerResetKey}
              composerTextareaRef={composerTextareaRef}
              activeThreadBot={activeThreadBot}
              activeThreadBotId={activeThreadBotId}
              botBindingDisabled={botBindingDisabled}
              botGroups={botGroups}
              agentLabel={agentLabel}
              agentOptions={composerAgentOptions}
              selectedAgentId={composerSelectedAgentId}
              onSelectAgent={
                !selectedThreadId ? onSelectNewThreadAgent : undefined
              }
              newThreadProviderModels={
                !selectedThreadId ? newThreadProviderModels : null
              }
              newThreadAgentConfiguredModel={
                !selectedThreadId ? newThreadAgentConfiguredModel : null
              }
              newThreadSelectedModel={
                !selectedThreadId ? newThreadSelectedModel : null
              }
              newThreadSelectedReasoningEffort={
                !selectedThreadId ? newThreadSelectedReasoningEffort : null
              }
              newThreadSelectedServiceTier={
                !selectedThreadId ? newThreadSelectedServiceTier : null
              }
              threadProviderModels={
                selectedThreadId ? threadProviderModels : null
              }
              threadEffectiveModel={
                selectedThreadId ? threadEffectiveModel : null
              }
              threadEffectiveReasoningEffort={
                selectedThreadId ? threadEffectiveReasoningEffort : null
              }
              threadEffectiveServiceTier={
                selectedThreadId ? threadEffectiveServiceTier : null
              }
              threadSelectedModel={
                selectedThreadId ? threadSelectedModel : null
              }
              threadSelectedReasoningEffort={
                selectedThreadId ? threadSelectedReasoningEffort : null
              }
              threadSelectedServiceTier={
                selectedThreadId ? threadSelectedServiceTier : null
              }
              onSelectNewThreadModel={
                !selectedThreadId ? onSelectNewThreadModel : undefined
              }
              onSelectNewThreadReasoningEffort={
                !selectedThreadId ? onSelectNewThreadReasoningEffort : undefined
              }
              onSelectNewThreadServiceTier={
                !selectedThreadId ? onSelectNewThreadServiceTier : undefined
              }
              onSelectThreadModel={
                selectedThreadId ? onSelectThreadModel : undefined
              }
              onSelectThreadReasoningEffort={
                selectedThreadId ? onSelectThreadReasoningEffort : undefined
              }
              onSelectThreadServiceTier={
                selectedThreadId ? onSelectThreadServiceTier : undefined
              }
              isActiveSendingThread={isActiveSendingThread}
              onAppendComposerAttachments={onAppendComposerAttachments}
              onComposerChange={onComposerChange}
              onComposerCompositionEnd={onComposerCompositionEnd}
              onComposerCompositionStart={onComposerCompositionStart}
              onComposerKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  const isImeComposing =
                    event.nativeEvent.isComposing ||
                    isComposingRef.current ||
                    event.keyCode === 229;
                  if (isImeComposing) {
                    onMarkIgnoreComposerSubmitWindow();
                    return;
                  }
                  event.preventDefault();
                  onComposerSubmit({
                    useAlternateFollowUpBehavior:
                      event.metaKey || event.ctrlKey,
                  });
                }
              }}
              onComposerPasteFiles={onAppendComposerAttachments}
              onInterrupt={onComposerInterrupt}
              onRemoveComposerFile={onRemoveComposerFile}
              onRemoveComposerImage={onRemoveComposerImage}
              onRemoveComposerPendingUpload={onRemoveComposerPendingUpload}
              onRemoveComposerBrowserAnnotation={onRemoveComposerBrowserAnnotation}
              onSelectBotBinding={onSelectBotBinding}
              onSubmit={(event) => {
                event.preventDefault();
                if (
                  isComposingRef.current ||
                  performance.now() < ignoreComposerSubmitUntilRef.current
                ) {
                  return;
                }
                onComposerSubmit();
              }}
              slashPanelContainerRef={composerShellWrapRef}
              slashCommands={slashCommands}
              slashCommandsLoaded={slashCommandsLoaded}
              slashCommandsLoading={slashCommandsLoading}
            />
            {!selectedThreadId &&
            !activeMessages.length &&
            !historyLoading &&
            !showAutomationRunInitialPlaceholder ? (
              <NewThreadEmptyState
                newThreadWorkspaceEntry={newThreadWorkspaceEntry}
                onAddWorkspace={onAddWorkspace}
                onSelectWorkspace={onSelectWorkspace}
                onWorkspaceModeChange={onSelectNewThreadWorkspaceMode}
                onResumeProviderSession={onResumeProviderSession}
                selectableNewThreadWorkspaces={selectableNewThreadWorkspaces}
                workspaceMode={newThreadWorkspaceMode}
                workspaceMutation={workspaceMutation}
              />
            ) : null}
            </div>
          </div>
      </div>

      {threadLogsOpen ? (
        <>
          <div
            aria-label={t("Resize logs panel")}
            aria-orientation="vertical"
            aria-valuemax={threadLogsMaxWidth}
            aria-valuemin={280}
            aria-valuenow={threadLogsPanelWidth}
            className="thread-log-resizer"
            onKeyDown={onThreadLogsResizeKeyDown}
            onPointerDown={onThreadLogsResizeStart}
            role="separator"
            tabIndex={0}
          />
          <ThreadLogDock
            activeThreadTitle={activeThreadTitle}
            onUnreadChange={onThreadLogsUnreadChange}
            threadId={selectedThreadId}
          />
        </>
      ) : null}
    </div>
  );
}
