import {
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
}: {
  keyPrefix: string;
  text: string;
  content?: unknown;
  pending?: boolean;
  error?: boolean;
  onLocalFileLinkClick: (path: string) => void;
}): ReactNode {
  const parts = splitRichMessageContentIntoBubbleParts({
    altPrefix: "user",
    content,
    text,
  });

  return parts.map((part) => {
    if (part.kind === "image" || part.kind === "file") {
      return (
        <article
          className={`message-attachment-bubble message-attachment-bubble-${part.kind} user ${pending ? "pending" : ""} ${error ? "error" : ""}`}
          key={`${keyPrefix}:${part.key}`}
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
  inspectorOpen: boolean;
  isActiveSendingThread: boolean;
  messagesRef: RefObject<HTMLDivElement | null>;
  mobileThreadLogLines: ThreadLogLine[];
  newThreadSelectedAgentId: string;
  newThreadWorkspaceEntry: DesktopWorkspace | null;
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
  ignoreComposerSubmitUntilRef,
  inspectorOpen,
  isActiveSendingThread,
  isComposingRef,
  messagesRef,
  mobileThreadLogLines,
  newThreadSelectedAgentId,
  newThreadWorkspaceEntry,
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
  const teamView = deriveThreadTeamView(activeThreadSummary);
  const teamSpeakerOptions = {
    leaderAgentId: activeThreadSummary?.team?.leader_agent_id || null,
    agentDisplayNamesById: teamAgentDisplayNamesById,
    childThreadIds: activeThreadSummary?.team?.child_thread_ids || {},
  };

  return (
    <div
      className={`thread-layout ${inspectorOpen ? "with-inspector-panel" : ""} ${threadLogsOpen ? "with-log-panel" : ""} ${threadLogsResizing ? "log-panel-resizing" : ""}`}
      ref={threadLayoutRef}
      style={threadLayoutStyle}
    >
      <div className="thread-main">
        <div className="messages" onScroll={onMessagesScroll} ref={messagesRef}>
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
            ) : (
              <NewThreadEmptyState
                newThreadWorkspaceEntry={newThreadWorkspaceEntry}
                onAddWorkspace={onAddWorkspace}
                onSelectWorkspace={onSelectWorkspace}
                onResumeProviderSession={onResumeProviderSession}
                selectableNewThreadWorkspaces={selectableNewThreadWorkspaces}
                workspaceMutation={workspaceMutation}
              />
            )
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
              <article className="message-bubble user">
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
                <div aria-label={t("Garyx is working")} className="message-loading">
                  <p className="message-loading-label">
                    {t("Garyx is working through the run…")}
                  </p>
                  <span aria-hidden="true" className="message-loading-dots">
                    <span />
                    <span />
                    <span />
                  </span>
                </div>
              </article>
            </>
          ) : null}

          {activeRenderableBlocks.map((block, index) => {
            const speaker = teamView.isTeam
              ? speakerForTranscriptBlock(block, teamSpeakerOptions)
              : null;
            const previousSpeaker =
              teamView.isTeam && index > 0
                ? speakerForTranscriptBlock(
                    activeRenderableBlocks[index - 1]!,
                    teamSpeakerOptions,
                  )
                : null;
            const showSpeakerHeader =
              Boolean(speaker) && speaker!.agentId !== previousSpeaker?.agentId;

            let blockBody: ReactNode;
            if (block.kind === "tool_group") {
              blockBody = (
                <article
                  className="message-bubble tool-cluster"
                  key={`${block.key}:body`}
                >
                  <ToolTraceGroup
                    entries={block.entries}
                    onThreadNavigate={onOpenThreadById}
                  />
                </article>
              );
            } else {
              const entry = block.entry;
              const loopContinuation = isLoopContinuationMessage(entry.message);
              if (entry.message.role === "user" && !loopContinuation) {
                blockBody = renderUserMessageBubbleParts({
                  keyPrefix: `${block.key}:body`,
                  text: displayTranscriptMessageText(entry.message),
                  content: entry.message.content,
                  pending: entry.message.pending,
                  error: entry.message.error,
                  onLocalFileLinkClick: onLocalWorkspaceFileLinkClick,
                });
              } else {
                blockBody = (
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
                          {entry.message.text}
                        </p>
                        <span
                          aria-hidden="true"
                          className="message-loading-dots"
                        >
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
              }
            }

            if (!speaker) {
              return blockBody;
            }

            const speakerHeader = showSpeakerHeader ? (
              speaker.threadId ? (
                <button
                  className="team-agent-speaker"
                  key={`${block.key}:speaker`}
                  onClick={() => onOpenThreadById(speaker.threadId!)}
                  title={t("Open {name} thread", { name: speaker.displayName })}
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
          })}

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
                aria-label={t("Waiting for Garyx to accept queued follow-up")}
                className="message-loading"
              >
                <p className="message-loading-label">
                  {t("Waiting for Garyx to accept the queued follow-up…")}
                </p>
                <span aria-hidden="true" className="message-loading-dots">
                  <span />
                  <span />
                  <span />
                </span>
              </div>
            </article>
          ) : null}

          {showAutomationRunTailLoading ? (
            <article className="message-bubble assistant pending">
              <div aria-label={t("Garyx is working")} className="message-loading">
                <p className="message-loading-label">
                  {t("Garyx is working through the run…")}
                </p>
                <span aria-hidden="true" className="message-loading-dots">
                  <span />
                  <span />
                  <span />
                </span>
              </div>
            </article>
          ) : null}
        </div>

        <div className="composer-shell-wrap" ref={composerShellWrapRef}>
          <div
            className={`composer-shell ${activeQueue.length ? "has-queue" : ""}`}
          >
            <ComposerQueue
              activeQueue={activeQueue}
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
              composerTextareaRef={composerTextareaRef}
              activeThreadBot={activeThreadBot}
              activeThreadBotId={activeThreadBotId}
              botBindingDisabled={botBindingDisabled}
              botGroups={botGroups}
              agentLabel={agentLabel}
              agentOptions={
                !selectedThreadId ? composerAgentOptions : undefined
              }
              selectedAgentId={
                !selectedThreadId ? newThreadSelectedAgentId : undefined
              }
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
