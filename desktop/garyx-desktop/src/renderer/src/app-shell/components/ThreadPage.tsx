import {
  Fragment,
  useLayoutEffect,
  useMemo,
  useRef,
  type CSSProperties,
  type MutableRefObject,
  type ReactNode,
  type RefObject,
} from "react";

import type {
  DesktopApiProviderType,
  DesktopBotConsoleSummary,
  MessageFileAttachment,
  MessageImageAttachment,
  DesktopThreadSummary,
  DesktopWorkspace,
  DesktopWorkspaceMode,
  PendingThreadInput,
  SlashCommand,
  TranscriptMessage,
} from "@shared/contracts";

import type { MessageIntent } from "../../message-machine";
import { ComposerForm, type ComposerAgentOption } from "../../ComposerForm";
import { ComposerQueue } from "../../ComposerQueue";
import { NewThreadEmptyState } from "../../NewThreadEmptyState";
import {
  RichMessageContent,
  buildOptimisticTranscriptContent,
  splitRichMessageContentIntoBubbleParts,
} from "../../message-rich-content";
import { deriveThreadTeamView } from "../../thread-model";
import {
  buildRenderTranscriptBlocks,
  type RenderTranscriptBlock,
} from "../../transcript-render";
import {
  buildTurnRows,
  type TurnRow,
  type UserTurnActivityRow,
} from "../../turn-render";
import { TurnSummary } from "../../turn-summary";
import { ToolTraceGroup } from "../../tool-trace";
import { AgentAvatar } from "./AgentAvatar";
import { ThreadLogPanel } from "./ThreadLogPanel";
import { useI18n } from "../../i18n";
import type {
  ClientLogEntry,
  PendingAutomationRun,
  ThreadLogLine,
  ThreadLogTab,
  UiTranscriptMessage,
} from "../types";
import { RUN_LOADING_LABEL } from "../loading-labels";

function normalizeMessageText(value: string | undefined): string {
  return value?.trim() || "";
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
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

type TeamSpeaker = {
  agentId: string;
  displayName: string;
  role: "leader" | "member";
  threadId: string | null;
};

function resolveTeamSpeaker(
  metadata: Record<string, unknown> | null | undefined,
  options: {
    leaderAgentId?: string | null;
    agentDisplayNamesById: Record<string, string>;
    childThreadIds: Record<string, string>;
  },
): TeamSpeaker | null {
  const record = asRecord(metadata);
  if (!record) {
    return null;
  }
  const agentId =
    typeof record.agent_id === "string" ? record.agent_id.trim() : "";
  if (!agentId) {
    return null;
  }
  const metadataDisplayName =
    typeof record.agent_display_name === "string"
      ? record.agent_display_name.trim()
      : "";
  return {
    agentId,
    displayName:
      options.agentDisplayNamesById[agentId] || metadataDisplayName || agentId,
    role: agentId === options.leaderAgentId ? "leader" : "member",
    threadId: options.childThreadIds[agentId] || null,
  };
}

function speakerForTranscriptBlock(
  block: RenderTranscriptBlock,
  options: {
    leaderAgentId?: string | null;
    agentDisplayNamesById: Record<string, string>;
    childThreadIds: Record<string, string>;
  },
): TeamSpeaker | null {
  if (block.kind === "message") {
    if (block.entry.message.role !== "assistant") {
      return null;
    }
    return resolveTeamSpeaker(block.entry.message.metadata, options);
  }

  for (const entry of block.entries) {
    const message = entry.toolUse || entry.toolResult;
    const speaker = resolveTeamSpeaker(message?.metadata, options);
    if (speaker) {
      return speaker;
    }
  }
  return null;
}

function renderUserMessageBubbleParts({
  keyPrefix,
  text,
  content,
  pending,
  error,
  onLocalFileLinkClick,
  markUserTurnStart = true,
}: {
  keyPrefix: string;
  text: string;
  content?: unknown;
  pending?: boolean;
  error?: boolean;
  onLocalFileLinkClick: (path: string) => void;
  markUserTurnStart?: boolean;
}): ReactNode {
  const parts = splitRichMessageContentIntoBubbleParts({
    altPrefix: "user",
    content,
    text,
  });

  return parts.map((part, index) => {
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
            onLocalFileLinkClick={onLocalFileLinkClick}
            text={part.text}
          />
        </article>
      );
    }

    return (
      <article
        className={`message-bubble user ${pending ? "pending" : ""} ${error ? "error" : ""}`}
        key={`${keyPrefix}:${part.key}`}
        {...userTurnMarker}
      >
        <RichMessageContent
          altPrefix="user"
          content={part.content}
          onLocalFileLinkClick={onLocalFileLinkClick}
          text={part.text}
        />
      </article>
    );
  });
}

type QueueDropTarget = {
  intentId: string;
  position: "before" | "after";
} | null;

type ThreadPageProps = {
  activeMessages: UiTranscriptMessage[];
  activePendingAckIntents: MessageIntent[];
  agentLabel?: string | null;
  composerAgentOptions?: ComposerAgentOption[];
  activePendingAutomationRun: PendingAutomationRun | null;
  activeToolTraceLoadingKey: string | null;
  activeQueue: MessageIntent[];
  activeRenderableBlocks: ReturnType<typeof buildRenderTranscriptBlocks>;
  activeThreadLogsHasUnread: boolean;
  activeThreadLogsPath: string;
  activeThreadSummary: DesktopThreadSummary | null;
  activeThreadTitle: string | null;
  activeThreadRunId: string | null;
  availableWorkspaceCount: number;
  clientThreadLogEntries: ClientLogEntry[];
  composer: string;
  composerAttachmentInputRef: RefObject<HTMLInputElement | null>;
  composerFiles: MessageFileAttachment[];
  composerHasPayload: boolean;
  composerImages: MessageImageAttachment[];
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
  draggedQueueIntentId: string | null;
  expandedClientLogEntries: Record<string, boolean>;
  historyLoading: boolean;
  historyLoadingEarlier: boolean;
  inspectorOpen: boolean;
  isActiveSendingThread: boolean;
  canSteerQueuedPrompt: boolean;
  messagesRef: RefObject<HTMLDivElement | null>;
  mobileThreadLogLines: ThreadLogLine[];
  newThreadSelectedAgentId: string;
  newThreadWorkspaceEntry: DesktopWorkspace | null;
  newThreadWorkspaceMode: DesktopWorkspaceMode;
  queueDropTarget: QueueDropTarget;
  selectableNewThreadWorkspaces: DesktopWorkspace[];
  selectedThreadId: string | null;
  showAutomationRunInitialPlaceholder: boolean;
  showAutomationRunTailLoading: boolean;
  showHistoryLoadingPlaceholder: boolean;
  showPendingAckLoading: boolean;
  threadLayoutRef: RefObject<HTMLDivElement | null>;
  threadLayoutStyle?: CSSProperties;
  threadLogsActiveTab: ThreadLogTab;
  threadLogsError: string | null;
  threadLogsLoading: boolean;
  threadLogsMaxWidth: number;
  threadLogsOpen: boolean;
  threadLogsPanelWidth: number;
  threadLogsRef: RefObject<HTMLDivElement | null>;
  threadLogsResizing: boolean;
  teamAgentDisplayNamesById: Record<string, string>;
  visibleRemoteAwaitingAckInputs: PendingThreadInput[];
  visibleRemotePendingInputs: PendingThreadInput[];
  workspaceDirectoryPanel: ReactNode;
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
  onComposerSubmit: () => void;
  onJumpToLatestThreadLogs: () => void;
  onLocalWorkspaceFileLinkClick: (path: string) => void;
  onMessagesScroll: () => void;
  onMarkIgnoreComposerSubmitWindow: () => void;
  onQueueDropTargetChange: (target: QueueDropTarget) => void;
  onRemoveComposerFile: (fileId: string) => void;
  onRemoveComposerImage: (imageId: string) => void;
  onReorderQueuedIntent: (
    threadId: string,
    draggedIntentId: string,
    targetIntentId: string,
    position: "before" | "after",
  ) => void;
  onSelectNewThreadAgent: (agentId: string) => void;
  onSelectNewThreadWorkspaceMode: (mode: DesktopWorkspaceMode) => void;
  onResumeProviderSession: (sessionId: string) => Promise<void>;
  onSelectThreadLogsTab: (tab: ThreadLogTab) => void;
  onSelectBotBinding: (botId: string | null) => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onSetDraggedQueueIntentId: (intentId: string | null) => void;
  onThreadLogsContentScroll: () => void;
  onThreadLogsResizeKeyDown: (
    event: React.KeyboardEvent<HTMLDivElement>,
  ) => void;
  onThreadLogsResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  onToggleClientLogEntry: (entryKey: string) => void;
  onSteerQueuedPrompt: (intent: MessageIntent) => void;
  onOpenThreadById: (threadId: string) => void;
  preferredWorkspaceForNewThread: DesktopWorkspace | null;
};

export function ThreadPage({
  agentLabel,
  composerAgentOptions,
  activeMessages,
  activePendingAckIntents,
  activePendingAutomationRun,
  activeToolTraceLoadingKey,
  activeQueue,
  activeRenderableBlocks,
  activeThreadLogsHasUnread,
  activeThreadLogsPath,
  activeThreadSummary,
  activeThreadTitle,
  activeThreadRunId,
  availableWorkspaceCount,
  clientThreadLogEntries,
  composer,
  composerAttachmentInputRef,
  composerFiles,
  composerHasPayload,
  composerImages,
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
  draggedQueueIntentId,
  expandedClientLogEntries,
  historyLoading,
  historyLoadingEarlier,
  ignoreComposerSubmitUntilRef,
  inspectorOpen,
  isActiveSendingThread,
  canSteerQueuedPrompt,
  isComposingRef,
  messagesRef,
  mobileThreadLogLines,
  newThreadSelectedAgentId,
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
  onJumpToLatestThreadLogs,
  onLocalWorkspaceFileLinkClick,
  onMessagesScroll,
  onMarkIgnoreComposerSubmitWindow,
  onQueueDropTargetChange,
  onRemoveComposerFile,
  onRemoveComposerImage,
  onReorderQueuedIntent,
  onSelectNewThreadAgent,
  onSelectNewThreadWorkspaceMode,
  onResumeProviderSession,
  onSelectBotBinding,
  onSelectThreadLogsTab,
  onSelectWorkspace,
  onSetDraggedQueueIntentId,
  onSteerQueuedPrompt,
  onOpenThreadById,
  onThreadLogsContentScroll,
  onThreadLogsResizeKeyDown,
  onThreadLogsResizeStart,
  onToggleClientLogEntry,
  preferredWorkspaceForNewThread,
  queueDropTarget,
  selectableNewThreadWorkspaces,
  selectedThreadId,
  showAutomationRunInitialPlaceholder,
  showAutomationRunTailLoading,
  showHistoryLoadingPlaceholder,
  showPendingAckLoading,
  threadLayoutRef,
  threadLayoutStyle,
  threadLogsActiveTab,
  threadLogsError,
  threadLogsLoading,
  threadLogsMaxWidth,
  threadLogsOpen,
  threadLogsPanelWidth,
  threadLogsRef,
  threadLogsResizing,
  teamAgentDisplayNamesById,
  visibleRemoteAwaitingAckInputs,
  visibleRemotePendingInputs,
  workspaceDirectoryPanel,
  workspaceMutation,
}: ThreadPageProps) {
  const { t } = useI18n();
  const composerShellWrapRef = useRef<HTMLDivElement | null>(null);
  const threadMainRef = useRef<HTMLDivElement | null>(null);
  const teamView = useMemo(
    () => deriveThreadTeamView(activeThreadSummary),
    [activeThreadSummary],
  );
  const teamSpeakerOptions = useMemo(
    () => ({
      leaderAgentId: activeThreadSummary?.team?.leader_agent_id || null,
      agentDisplayNamesById: teamAgentDisplayNamesById,
      childThreadIds: activeThreadSummary?.team?.child_thread_ids || {},
    }),
    [
      activeThreadSummary?.team?.child_thread_ids,
      activeThreadSummary?.team?.leader_agent_id,
      teamAgentDisplayNamesById,
    ],
  );
  const turnRows = useMemo(
    () =>
      teamView.isTeam
        ? []
        : buildTurnRows(activeRenderableBlocks, {
            deferTrailingFinalAssistant: isActiveSendingThread,
          }),
    [activeRenderableBlocks, isActiveSendingThread, teamView.isTeam],
  );
  const composerSelectedAgentId = selectedThreadId
    ? teamView.isTeam
      ? activeThreadSummary?.team?.team_id?.trim() ||
        activeThreadSummary?.teamId?.trim() ||
        activeThreadSummary?.agentId?.trim() ||
        undefined
      : activeThreadSummary?.agentId?.trim() || undefined
    : newThreadSelectedAgentId;
  const emptyNewThread =
    !selectedThreadId &&
    !activeMessages.length &&
    !historyLoading &&
    !showAutomationRunInitialPlaceholder;
  const newThreadWorkspaceName = newThreadWorkspaceEntry?.name?.trim() || "";
  const newThreadPromptTitle = newThreadWorkspaceName
    ? `What do you want Garyx to build in ${newThreadWorkspaceName}?`
    : "What do you want Garyx to build?";

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
      className={`thread-layout ${inspectorOpen ? "with-inspector-panel" : ""} ${threadLogsOpen ? "with-log-panel" : ""} ${threadLogsResizing ? "log-panel-resizing" : ""}`}
      ref={threadLayoutRef}
      style={threadLayoutStyle}
    >
      <div
        className={`thread-main ${emptyNewThread ? "new-thread-centered" : ""}`}
        ref={threadMainRef}
      >
        <div className="messages" onScroll={onMessagesScroll} ref={messagesRef}>
          {historyLoadingEarlier ? (
            <div
              aria-label={t("Loading earlier messages")}
              className="message-history-page-loader"
            >
              <span aria-hidden="true" className="message-history-page-spinner" />
            </div>
          ) : null}

          {!activeMessages.length &&
          !historyLoading &&
          !showAutomationRunInitialPlaceholder ? (
            selectedThreadId ? (
              <div className="empty-state">
                <span className="eyebrow">{t("Ready")}</span>
                <h3>{t("Continue the current thread")}</h3>
                <p>
                  {t("Every thread is replayable from gateway history and can continue on this Mac.")}
                </p>
              </div>
            ) : null
          ) : null}

          {showHistoryLoadingPlaceholder ? (
            <article className="message-bubble assistant pending">
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
            </article>
          ) : null}

          {showAutomationRunInitialPlaceholder && activePendingAutomationRun ? (
            <>
              <article className="message-bubble user" data-user-turn-start="true">
                <RichMessageContent
                  altPrefix="user"
                  content={buildOptimisticTranscriptContent(
                    activePendingAutomationRun.prompt,
                    [],
                    [],
                  )}
                  onLocalFileLinkClick={onLocalWorkspaceFileLinkClick}
                  text={activePendingAutomationRun.prompt}
                />
              </article>
              <article className="message-bubble assistant pending">
                <div
                  aria-label={t("Garyx is working")}
                  className="message-loading"
                >
                  <p className="message-loading-label message-loading-label--thinking">
                    {t(RUN_LOADING_LABEL)}
                  </p>
                </div>
              </article>
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
                      active={block.key === activeToolTraceLoadingKey}
                      defaultExpanded={block.defaultExpanded}
                      entries={block.entries}
                      onThreadNavigate={onOpenThreadById}
                    />
                  </article>
                );
              }
              const entry = block.entry;
              const loopContinuation = isLoopContinuationMessage(entry.message);
              if (entry.message.role === "user" && !loopContinuation) {
                return renderUserMessageBubbleParts({
                  keyPrefix: `${block.key}:body`,
                  text: displayTranscriptMessageText(entry.message),
                  content: entry.message.content,
                  pending: entry.message.pending,
                  error: entry.message.error,
                  onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
                  markUserTurnStart: options.markUserTurnStart !== false,
                });
              }
              return (
                <article
                  key={`${block.key}:body`}
                  className={`message-bubble ${entry.message.role} ${entry.message.pending ? "pending" : ""} ${entry.message.error ? "error" : ""} ${loopContinuation ? "loop-continuation" : ""}`}
                >
                  {entry.message.role === "assistant" &&
                  entry.message.pending ? (
                    <div
                      aria-label={t("Garyx is working")}
                      className="message-loading"
                    >
                      <p className="message-loading-label">
                        {displayTranscriptMessageText(entry.message)}
                      </p>
                      <span aria-hidden="true" className="message-loading-dots">
                        <span />
                        <span />
                        <span />
                      </span>
                    </div>
                  ) : (
                    <RichMessageContent
                      altPrefix={entry.message.role}
                      content={
                        loopContinuation
                          ? LOOP_CONTINUATION_SUMMARY
                          : entry.message.content
                      }
                      onLocalFileLinkClick={onLocalWorkspaceFileLinkClick}
                      text={displayTranscriptMessageText(entry.message)}
                    />
                  )}
                </article>
              );
            };

            // Team mode keeps the per-block iteration so we can still emit
            // speaker headers between consecutive agents. Solo threads route
            // through `buildTurnRows` so each multi-step assistant turn ends
            // up behind a Codex-style "Worked for X" collapsible.
            if (teamView.isTeam) {
              return activeRenderableBlocks.map((block, index) => {
                const speaker = speakerForTranscriptBlock(
                  block,
                  teamSpeakerOptions,
                );
                const previousSpeaker =
                  index > 0
                    ? speakerForTranscriptBlock(
                        activeRenderableBlocks[index - 1]!,
                        teamSpeakerOptions,
                      )
                    : null;
                const showSpeakerHeader =
                  Boolean(speaker) &&
                  speaker!.agentId !== previousSpeaker?.agentId;
                const blockBody = renderBlockBody(block);
                if (!speaker) {
                  return blockBody;
                }
                const speakerHeader = showSpeakerHeader ? (
                  speaker.threadId ? (
                    <button
                      className="team-agent-speaker"
                      key={`${block.key}:speaker`}
                      onClick={() => onOpenThreadById(speaker.threadId!)}
                      title={t("Open {name} thread", {
                        name: speaker.displayName,
                      })}
                      type="button"
                    >
                      <AgentAvatar
                        agentId={speaker.agentId}
                        displayName={speaker.displayName}
                        role={speaker.role}
                        size={28}
                      />
                      <span className="team-agent-speaker-name">
                        {speaker.displayName}
                      </span>
                    </button>
                  ) : (
                    <div
                      className="team-agent-speaker"
                      key={`${block.key}:speaker`}
                    >
                      <AgentAvatar
                        agentId={speaker.agentId}
                        displayName={speaker.displayName}
                        role={speaker.role}
                        size={28}
                      />
                      <span className="team-agent-speaker-name">
                        {speaker.displayName}
                      </span>
                    </div>
                  )
                ) : null;
                return (
                  <div
                    key={block.key}
                    className={`team-agent-block ${showSpeakerHeader ? "with-speaker-header" : "continued-speaker"}`}
                  >
                    {speakerHeader}
                    <div className="team-agent-block-body">{blockBody}</div>
                  </div>
                );
              });
            }

            return turnRows.map((row, idx) => {
              const renderActivityRow = (
                activityRow: UserTurnActivityRow,
                forceRunning: boolean,
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
                    <TurnSummary turn={turn} forceRunning={forceRunning}>
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

              if (row.kind === "flat") {
                return (
                  <Fragment key={row.key}>
                    {renderBlockBody(row.block)}
                  </Fragment>
                );
              }
              if (row.kind === "turn") {
                return (
                  <Fragment key={row.key}>
                    {renderActivityRow(row, false)}
                  </Fragment>
                );
              }
              // Bridge the gap where the assistant message is no longer
              // pending=true but the thread run is still active (e.g.
              // tool call in flight): force only the bottom-most activity
              // in the bottom-most user turn to read as running.
              const isLastUserTurn = idx === turnRows.length - 1;
              const lastActivityIndex = row.activityRows.length - 1;
              return (
                <Fragment key={row.key}>
                  {renderBlockBody(row.userBlock, {
                    markUserTurnStart: true,
                  })}
                  {row.activityRows.map((activityRow, activityIndex) =>
                    renderActivityRow(
                      activityRow,
                      isLastUserTurn &&
                        activityIndex === lastActivityIndex &&
                        isActiveSendingThread &&
                        activityRow.kind === "turn" &&
                        activityRow.finalBlock === null,
                    ),
                  )}
                </Fragment>
              );
            });
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
              onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
            }),
          )}

          {visibleRemotePendingInputs.map((input) =>
            renderUserMessageBubbleParts({
              keyPrefix: `remote-pending:${input.id}`,
              text: input.text,
              content: input.content,
              onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
            }),
          )}

          {showPendingAckLoading ? (
            <article className="message-bubble assistant pending">
              <div
                aria-label={t("Garyx is working")}
                className="message-loading"
              >
                <p className="message-loading-label message-loading-label--thinking">
                  {t(RUN_LOADING_LABEL)}
                </p>
              </div>
            </article>
          ) : null}

          {showAutomationRunTailLoading ? (
            <article className="message-bubble assistant pending">
              <div
                aria-label={t("Garyx is working")}
                className="message-loading"
              >
                <p className="message-loading-label message-loading-label--thinking">
                  {t(RUN_LOADING_LABEL)}
                </p>
              </div>
            </article>
          ) : null}
        </div>

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
              draggedQueueIntentId={draggedQueueIntentId}
              isActiveSendingThread={isActiveSendingThread}
              onCancelIntent={onCancelIntent}
              onQueueDropTargetChange={onQueueDropTargetChange}
              onReorderQueuedIntent={onReorderQueuedIntent}
              onSetDraggedQueueIntentId={onSetDraggedQueueIntentId}
              onSteerQueuedPrompt={onSteerQueuedPrompt}
              queueDropTarget={queueDropTarget}
            />

            <ComposerForm
              activeQueueLength={activeQueue.length}
              composer={composer}
              composerAttachmentInputRef={composerAttachmentInputRef}
              composerFiles={composerFiles}
              composerHasPayload={composerHasPayload}
              composerImages={composerImages}
              composerLocked={composerLocked}
              composerPlaceholder={composerPlaceholder}
              composerProviderType={composerProviderType}
              composerResetKey={composerResetKey}
              composerTextareaRef={composerTextareaRef}
              activeThreadBot={activeThreadBot}
              activeThreadBotId={activeThreadBotId}
              botBindingDisabled={botBindingDisabled}
              botGroups={botGroups}
              composerWorkspaceBranch={composerWorkspaceBranch}
              composerWorkspaceMode={composerWorkspaceMode}
              agentLabel={agentLabel}
              agentOptions={composerAgentOptions}
              selectedAgentId={composerSelectedAgentId}
              onSelectAgent={
                !selectedThreadId ? onSelectNewThreadAgent : undefined
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
                  onComposerSubmit();
                }
              }}
              onComposerPasteFiles={onAppendComposerAttachments}
              onInterrupt={onComposerInterrupt}
              onRemoveComposerFile={onRemoveComposerFile}
              onRemoveComposerImage={onRemoveComposerImage}
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

      {inspectorOpen && workspaceDirectoryPanel ? (
        <aside className="workspace-directory-panel">
          {workspaceDirectoryPanel}
        </aside>
      ) : null}

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
          <ThreadLogPanel
            activeThreadLogsHasUnread={activeThreadLogsHasUnread}
            activeThreadLogsPath={activeThreadLogsPath}
            activeThreadTitle={activeThreadTitle}
            clientThreadLogEntries={clientThreadLogEntries}
            expandedClientLogEntries={expandedClientLogEntries}
            mobileThreadLogLines={mobileThreadLogLines}
            onContentScroll={onThreadLogsContentScroll}
            onJumpToLatest={onJumpToLatestThreadLogs}
            onSelectTab={onSelectThreadLogsTab}
            onToggleClientLogEntry={onToggleClientLogEntry}
            selectedThreadId={selectedThreadId}
            threadLogsActiveTab={threadLogsActiveTab}
            threadLogsError={threadLogsError}
            threadLogsLoading={threadLogsLoading}
            threadLogsRef={threadLogsRef}
          />
        </>
      ) : null}
    </div>
  );
}
