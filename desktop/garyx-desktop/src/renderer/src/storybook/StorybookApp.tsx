import { useEffect, useMemo, useRef, useState } from 'react';

import { ThreadPage } from '../app-shell/components/ThreadPage';
import { deriveThreadActivityModel } from '../app-shell/thread-activity';
import { isRuntimeBusy } from '../message-machine';
import {
  buildRenderableTranscript,
  buildRenderTranscriptBlocks,
} from '../transcript-render';
import { buildStories, type Story, type StoryStep } from './scenarios';

const PLAY_INTERVAL_MS = 1600;

function noop() {}

/// Mounts the real ThreadPage against one scenario step. Activity flags are
/// derived through the real contract derivation — the storybook only authors
/// machine-level state, never hand-set loading booleans.
function ThreadStage({ step }: { step: StoryStep }) {
  const state = step.state;
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const threadLayoutRef = useRef<HTMLDivElement | null>(null);
  const composerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const composerAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const threadLogsRef = useRef<HTMLDivElement | null>(null);
  const ignoreComposerSubmitUntilRef = useRef(0);
  const isComposingRef = useRef(false);

  const renderableBlocks = useMemo(
    () => buildRenderTranscriptBlocks(buildRenderableTranscript(state.messages)),
    [state.messages],
  );
  const lastBlock = renderableBlocks[renderableBlocks.length - 1] || null;
  const tailToolTraceBlockKey = lastBlock?.kind === 'tool_group' ? lastBlock.key : null;

  const activity = deriveThreadActivityModel({
    messages: state.messages,
    threadInfo: state.activeRunId ? { activeRun: { runId: state.activeRunId } } : null,
    liveStream: state.liveStreamStatus
      ? {
          threadId: 'storybook-thread',
          pendingAckIntentIds: state.pendingAckIntents.map((entry) => entry.intentId),
          streamStatus: state.liveStreamStatus,
        }
      : null,
    runtimeBusy: isRuntimeBusy(state.runtimeState),
    pendingAckIntentCount: state.pendingAckIntents.length,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: state.pendingHistoryIntent,
  });

  return (
    <ThreadPage
      surfaceVariant="side-chat"
      agentLabel="Storybook Agent"
      activeMessages={state.messages}
      activePendingAckIntents={state.pendingAckIntents}
      activePendingAutomationRun={null}
      activeToolTraceLoadingKey={activity.runActive ? tailToolTraceBlockKey : null}
      activeQueue={state.queue}
      activeRenderableBlocks={renderableBlocks}
      activeThreadLogsHasUnread={false}
      activeThreadLogsPath=""
      activeThreadSummary={null}
      activeThreadTitle="Conversation State Storybook"
      activeThreadRunId={state.activeRunId}
      availableWorkspaceCount={1}
      clientThreadLogEntries={[]}
      composer=""
      composerAttachmentInputRef={composerAttachmentInputRef}
      composerBrowserAnnotations={[]}
      composerFiles={[]}
      composerHasPayload={false}
      composerImages={[]}
      composerLocked={false}
      composerPlaceholder="Describe what you want Garyx to build…"
      composerProviderType="claude_code"
      composerResetKey={0}
      composerWorkspaceBranch={null}
      composerWorkspaceMode={null}
      activeThreadBot={null}
      activeThreadBotId={null}
      botBindingDisabled={false}
      botGroups={[]}
      slashCommands={[]}
      slashCommandsLoaded
      slashCommandsLoading={false}
      composerTextareaRef={composerTextareaRef}
      draggedQueueIntentId={null}
      expandedClientLogEntries={{}}
      historyLoading={state.historyLoading}
      historyLoadingEarlier={state.historyLoadingEarlier}
      ignoreComposerSubmitUntilRef={ignoreComposerSubmitUntilRef}
      inspectorOpen={false}
      isActiveSendingThread={activity.runActive || activity.showPendingAckLoading}
      canSteerQueuedPrompt={activity.canSteerQueuedPrompt}
      isComposingRef={isComposingRef}
      messagesRef={messagesRef}
      mobileThreadLogLines={[]}
      newThreadSelectedAgentId="claude"
      newThreadSelectedWorkflowId={null}
      newThreadWorkspaceEntry={null}
      newThreadWorkspaceMode="local"
      onAddWorkspace={noop}
      onAppendComposerAttachments={noop}
      onCancelIntent={noop}
      onComposerChange={noop}
      onComposerCompositionEnd={noop}
      onComposerCompositionStart={noop}
      onComposerInterrupt={noop}
      onComposerSubmit={noop}
      onJumpToLatestThreadLogs={noop}
      onLocalWorkspaceFileLinkClick={noop}
      onMarkIgnoreComposerSubmitWindow={noop}
      onMessagesScroll={noop}
      onMessagesUserScrollIntent={noop}
      onQueueDropTargetChange={noop}
      onRemoveComposerFile={noop}
      onRemoveComposerImage={noop}
      onRemoveComposerBrowserAnnotation={noop}
      onReorderQueuedIntent={noop}
      onSelectNewThreadAgent={noop}
      onSelectNewThreadWorkflow={noop}
      onSelectNewThreadWorkspaceMode={noop}
      onResumeProviderSession={async () => {}}
      onRetryFailedMessage={noop}
      onSelectBotBinding={noop}
      onSelectThreadLogsTab={noop}
      onOpenThreadById={noop}
      onSelectWorkspace={noop}
      onSetDraggedQueueIntentId={noop}
      onSteerQueuedPrompt={noop}
      onThreadLogsContentScroll={noop}
      onThreadLogsResizeKeyDown={noop}
      onThreadLogsResizeStart={noop}
      onToggleClientLogEntry={noop}
      preferredWorkspaceForNewThread={null}
      queueDropTarget={null}
      selectableNewThreadWorkspaces={[]}
      selectedThreadId="storybook-thread"
      showAutomationRunInitialPlaceholder={false}
      showDreams={false}
      showAutomationRunTailLoading={activity.showRunLoading && !tailToolTraceBlockKey}
      showHistoryLoadingPlaceholder={state.showHistoryLoadingPlaceholder}
      showPendingAckLoading={activity.showPendingAckLoading}
      threadLayoutRef={threadLayoutRef}
      threadLogsActiveTab="client"
      threadLogsError={null}
      threadLogsLoading={false}
      threadLogsMaxWidth={0}
      threadLogsOpen={false}
      threadLogsPanelWidth={0}
      threadLogsRef={threadLogsRef}
      threadLogsResizing={false}
      teamAgentDisplayNamesById={{}}
      visibleRemoteAwaitingAckInputs={[]}
      visibleRemotePendingInputs={[]}
      workflowRunContent={null}
      workspaceMutation={null}
    />
  );
}

function MachineBadges({ step }: { step: StoryStep }) {
  const state = step.state;
  const activity = deriveThreadActivityModel({
    messages: state.messages,
    threadInfo: state.activeRunId ? { activeRun: { runId: state.activeRunId } } : null,
    liveStream: state.liveStreamStatus
      ? {
          threadId: 'storybook-thread',
          pendingAckIntentIds: state.pendingAckIntents.map((entry) => entry.intentId),
          streamStatus: state.liveStreamStatus,
        }
      : null,
    runtimeBusy: isRuntimeBusy(state.runtimeState),
    pendingAckIntentCount: state.pendingAckIntents.length,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: state.pendingHistoryIntent,
  });
  const badges: Array<{ label: string; value: string; tone: 'state' | 'derived-on' | 'derived-off' }> = [
    { label: 'intent', value: state.intentState ?? '—', tone: 'state' },
    { label: 'runtime', value: state.runtimeState, tone: 'state' },
    { label: 'stream', value: state.liveStreamStatus ?? '—', tone: 'state' },
    {
      label: 'runActive',
      value: String(activity.runActive),
      tone: activity.runActive ? 'derived-on' : 'derived-off',
    },
    {
      label: 'showRunLoading',
      value: String(activity.showRunLoading),
      tone: activity.showRunLoading ? 'derived-on' : 'derived-off',
    },
    {
      label: 'showPendingAckLoading',
      value: String(activity.showPendingAckLoading),
      tone: activity.showPendingAckLoading ? 'derived-on' : 'derived-off',
    },
    {
      label: 'canSteerQueuedPrompt',
      value: String(activity.canSteerQueuedPrompt),
      tone: activity.canSteerQueuedPrompt ? 'derived-on' : 'derived-off',
    },
  ];
  return (
    <div className="storybook-badges">
      {badges.map((badge) => (
        <span className={`storybook-badge is-${badge.tone}`} key={badge.label}>
          <span className="storybook-badge-label">{badge.label}</span>
          <span className="storybook-badge-value">{badge.value}</span>
        </span>
      ))}
    </div>
  );
}

export function StorybookApp() {
  const stories = useMemo(buildStories, []);
  const [storyId, setStoryId] = useState<string>(stories[0]?.id ?? '');
  const [stepIndex, setStepIndex] = useState(0);
  const [playing, setPlaying] = useState(false);

  const story: Story = stories.find((entry) => entry.id === storyId) ?? stories[0];
  const boundedStepIndex = Math.min(stepIndex, story.steps.length - 1);
  const step = story.steps[boundedStepIndex];

  useEffect(() => {
    if (!playing) {
      return;
    }
    const timer = window.setInterval(() => {
      setStepIndex((current) => {
        if (current >= story.steps.length - 1) {
          setPlaying(false);
          return current;
        }
        return current + 1;
      });
    }, PLAY_INTERVAL_MS);
    return () => {
      window.clearInterval(timer);
    };
  }, [playing, story]);

  function selectStory(id: string) {
    setStoryId(id);
    setStepIndex(0);
    setPlaying(false);
  }

  return (
    <div className="storybook-shell">
      <aside className="storybook-sidebar">
        <header className="storybook-header">
          <span className="eyebrow">Conversation State</span>
          <h1>消息列表 Storybook</h1>
          <p>
            场景由共享状态契约（docs/agents/conversation-state.md）的词汇驱动，渲染走真实的
            ThreadPage 组件与派生逻辑。
          </p>
        </header>
        <nav className="storybook-story-list">
          {stories.map((entry) => (
            <button
              className={`storybook-story ${entry.id === story.id ? 'is-active' : ''}`}
              key={entry.id}
              onClick={() => selectStory(entry.id)}
              type="button"
            >
              <strong>{entry.name}</strong>
              <span>{entry.description}</span>
            </button>
          ))}
        </nav>
      </aside>
      <main className="storybook-main">
        <div className="storybook-controls">
          <div className="storybook-transport">
            <button
              disabled={boundedStepIndex === 0}
              onClick={() => {
                setPlaying(false);
                setStepIndex(0);
              }}
              type="button"
            >
              ⟲ 重放
            </button>
            <button
              disabled={boundedStepIndex === 0}
              onClick={() => {
                setPlaying(false);
                setStepIndex((current) => Math.max(0, current - 1));
              }}
              type="button"
            >
              ← 上一步
            </button>
            <button
              className="storybook-play"
              onClick={() => {
                if (boundedStepIndex >= story.steps.length - 1) {
                  setStepIndex(0);
                }
                setPlaying((current) => !current);
              }}
              type="button"
            >
              {playing ? '⏸ 暂停' : '▶ 播放'}
            </button>
            <button
              disabled={boundedStepIndex >= story.steps.length - 1}
              onClick={() => {
                setPlaying(false);
                setStepIndex((current) => Math.min(story.steps.length - 1, current + 1));
              }}
              type="button"
            >
              下一步 →
            </button>
          </div>
          <div className="storybook-timeline">
            {story.steps.map((entry, index) => (
              <button
                className={`storybook-timeline-step ${index === boundedStepIndex ? 'is-active' : ''} ${index < boundedStepIndex ? 'is-passed' : ''}`}
                key={entry.label}
                onClick={() => {
                  setPlaying(false);
                  setStepIndex(index);
                }}
                title={entry.label}
                type="button"
              >
                <span className="storybook-timeline-dot" />
                <span className="storybook-timeline-label">{entry.label}</span>
              </button>
            ))}
          </div>
        </div>
        <div className="storybook-step-meta">
          <h2>{step.label}</h2>
          <p>{step.description}</p>
          <MachineBadges step={step} />
        </div>
        <section className="storybook-stage">
          <ThreadStage key={`${story.id}:${boundedStepIndex}`} step={step} />
        </section>
      </main>
    </div>
  );
}
