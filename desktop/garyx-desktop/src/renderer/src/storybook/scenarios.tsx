import type { RenderState, TranscriptMessage } from '@shared/contracts';

import type { IntentState, MessageIntent, ThreadRuntimeState } from '../message-machine';
import type { LiveStreamStatus, UiTranscriptMessage } from '../app-shell/types';

// Conversation state storybook scenarios. Every step pins the contract
// vocabulary (docs/agents/conversation-state.md): the message list is built
// from real UiTranscriptMessage values and the activity flags are derived by
// the real deriveThreadActivityModel at render time — the storybook host
// never hand-sets a loading flag the contract can derive.

export type StoryState = {
  messages: UiTranscriptMessage[];
  renderState: RenderState | null;
  liveStreamStatus: LiveStreamStatus | null;
  runtimeState: ThreadRuntimeState;
  activeRunId: string | null;
  pendingAckIntents: MessageIntent[];
  queue: MessageIntent[];
  pendingHistoryIntent: boolean;
  historyLoading: boolean;
  historyLoadingEarlier: boolean;
  showHistoryLoadingPlaceholder: boolean;
  intentState: IntentState | null;
};

export type StoryStep = {
  label: string;
  description: string;
  state: StoryState;
};

export type Story = {
  id: string;
  name: string;
  description: string;
  steps: StoryStep[];
};

// Anchor fixture timestamps just before now so elapsed-time labels read naturally.
const BASE_TIME = Date.now() - 180_000;

function timestamp(offsetSeconds: number): string {
  return new Date(BASE_TIME + offsetSeconds * 1000).toISOString();
}

let messageSerial = 0;

function nextId(prefix: string): string {
  messageSerial += 1;
  return `${prefix}-${messageSerial}`;
}

function message(
  partial: Partial<TranscriptMessage> & Pick<TranscriptMessage, 'role' | 'text'>,
  ui?: Partial<UiTranscriptMessage>,
): UiTranscriptMessage {
  messageSerial += 1;
  return {
    id: partial.id ?? `m-${messageSerial}`,
    timestamp: timestamp(messageSerial),
    ...partial,
    ...ui,
  } as UiTranscriptMessage;
}

function userMessage(text: string, ui?: Partial<UiTranscriptMessage>): UiTranscriptMessage {
  return message({ role: 'user', text }, ui);
}

function assistantMessage(text: string, ui?: Partial<UiTranscriptMessage>): UiTranscriptMessage {
  return message({ role: 'assistant', text }, ui);
}

function assistantStreaming(text: string): UiTranscriptMessage {
  return message({ role: 'assistant', text, pending: true }, { localState: 'remote_partial' });
}

function toolPair(
  toolName: string,
  input: string,
  result?: { text: string; isError?: boolean },
): UiTranscriptMessage[] {
  const toolUseId = nextId('tool-use');
  const calls: UiTranscriptMessage[] = [
    message({ role: 'tool_use', text: input, toolUseId, toolName }),
  ];
  if (result) {
    calls.push(
      message({
        role: 'tool_result',
        text: result.text,
        toolUseId,
        toolName,
        isError: result.isError ?? false,
      }),
    );
  }
  return calls;
}

function intent(
  text: string,
  state: IntentState,
  extra?: Partial<MessageIntent>,
): MessageIntent {
  messageSerial += 1;
  return {
    intentId: `intent-${messageSerial}`,
    threadId: 'storybook-thread',
    text,
    images: [],
    files: [],
    createdAt: timestamp(messageSerial),
    updatedAt: timestamp(messageSerial),
    state,
    source: 'composer_queue',
    ...extra,
  };
}

const idleState: Omit<StoryState, 'messages'> = {
  renderState: null,
  liveStreamStatus: null,
  runtimeState: 'idle',
  activeRunId: null,
  pendingAckIntents: [],
  queue: [],
  pendingHistoryIntent: false,
  historyLoading: false,
  historyLoadingEarlier: false,
  showHistoryLoadingPlaceholder: false,
  intentState: null,
};

function step(
  label: string,
  description: string,
  state: Partial<StoryState> & Pick<StoryState, 'messages'>,
): StoryStep {
  return {
    label,
    description,
    state: { ...idleState, ...state },
  };
}

export function buildStories(): Story[] {
  messageSerial = 0;

  // Shared transcript fragments, built once so message identity is stable
  // across steps — exactly how the apps preserve row identity on reconcile.
  const q1 = userMessage('Summarize the failing tests and propose a fix.', {
    localState: 'optimistic',
    intentId: 'intent-happy',
  });
  const q1Final = { ...q1, localState: 'remote_final' as const };
  const a1Partial = assistantStreaming('Looking at the failing suite, the timeout comes from');
  const a1MorePartial = {
    ...a1Partial,
    text: 'Looking at the failing suite, the timeout comes from the retry loop never observing the cancelled flag.',
  };
  const happyTools = [
    ...toolPair('Bash', 'swift test --filter RetryLoopTests', {
      text: 'Executed 12 tests, with 2 failures',
    }),
    ...toolPair('Read', 'Sources/RetryLoop.swift', {
      text: '120 lines',
    }),
  ];
  const a1Final = assistantMessage(
    [
      'The retry loop never observes the cancelled flag.',
      '',
      '**Fix**',
      '',
      '```swift',
      'while !Task.isCancelled && attempt < limit {',
      '    try await poll()',
      '}',
      '```',
      '',
      'Both failures pass locally after the change.',
    ].join('\n'),
  );

  const happyPath: Story = {
    id: 'sync-send',
    name: '同步发送 · 完整生命周期',
    description:
      '一次 sync_send 从 optimistic 到 remote_final 的完整状态轨迹：排队、派发、流式、工具调用、终态对账。',
    steps: [
      step('空线程 · idle', '没有任何消息，runtime idle。', { messages: [] }),
      step(
        'optimistic · dispatching_sync',
        '用户消息以 optimistic 本地态立即上屏；发送态由 runtime busy 驱动，远端 tail 渲染交给 render_state。',
        {
          messages: [q1],
          runtimeState: 'dispatching_sync',
          intentState: 'dispatching',
        },
      ),
      step(
        'remote_accepted · connecting',
        '网关接受运行；流处于 connecting。',
        {
          messages: [q1],
          runtimeState: 'running_remote',
          liveStreamStatus: 'connecting',
          activeRunId: 'run-1',
          intentState: 'remote_accepted',
        },
      ),
      step(
        'streaming · remote_partial',
        '助手增量文本以 remote_partial 流入；可见 rows/tail/tool 状态由 server render_state 决定。',
        {
          messages: [q1, a1Partial],
          runtimeState: 'running_remote',
          liveStreamStatus: 'streaming',
          activeRunId: 'run-1',
          intentState: 'remote_accepted',
        },
      ),
      step(
        '工具调用 · tool_use/tool_result',
        '工具组实时展开；尾部工具组承担运行指示（activeToolTraceLoadingKey）。',
        {
          messages: [q1, a1MorePartial, ...happyTools],
          runtimeState: 'running_remote',
          liveStreamStatus: 'streaming',
          activeRunId: 'run-1',
          intentState: 'remote_accepted',
        },
      ),
      step(
        'awaiting_history · reconciling',
        '流结束，等待终态转录确认；turn 折叠为 "Worked"，final answer 外置。',
        {
          messages: [q1, ...happyTools, a1Final],
          runtimeState: 'reconciling_history',
          liveStreamStatus: 'reconciling',
          intentState: 'awaiting_history',
        },
      ),
      step(
        'completed · idle',
        '终态转录对账完成：用户消息物化为 remote_final，行身份保持稳定（不闪烁）。',
        {
          messages: [q1Final, ...happyTools, a1Final],
          intentState: 'completed',
        },
      ),
    ],
  };

  const q2 = userMessage('Deploy the staging build.', {
    localState: 'optimistic',
    intentId: 'intent-failed',
  });
  const q2Failed = { ...q2, error: true, localState: 'error' as const };

  const failurePath: Story = {
    id: 'failure',
    name: '失败与中断',
    description: 'dispatch 失败的 error 本地态、非瞬态流错误，以及 interrupted 终态。',
    steps: [
      step('optimistic 发送中', '消息已上屏，HTTP 派发在途。', {
        messages: [q2],
        runtimeState: 'dispatching_sync',
        intentState: 'dispatching',
      }),
      step(
        'failed · error 本地态',
        '网关拒绝；消息保留并标记 error（移动端为 statusText 叠加，桌面为 error 标记）。',
        {
          messages: [q2Failed],
          intentState: 'failed',
        },
      ),
      step(
        '重试后被中断 · interrupted',
        '重试的运行被用户中断；intent 进入 interrupted 终态，runtime 回到 idle。',
        {
          messages: [
            q2Failed,
            userMessage('Deploy the staging build.', { localState: 'remote_final' }),
            assistantMessage('Starting the staging deploy…'),
            message(
              { role: 'system', text: 'Run interrupted by user.' },
              { localState: 'interrupted' },
            ),
          ],
          intentState: 'interrupted',
        },
      ),
    ],
  };

  const q3 = userMessage('Refactor the cache layer.', { localState: 'remote_final' });
  const a3Partial = assistantStreaming('Working through the cache invalidation paths now —');
  const steerText = 'Also add metrics for cache hit rate, please.';

  const steerPath: Story = {
    id: 'steer-queue',
    name: '运行中追加 · 队列与 steer',
    description:
      'async_steer 生命周期：本地排队（queued_local）、等待 provider ack（awaiting_provider_ack）、user_ack 后物化。',
    steps: [
      step('运行进行中', '主运行正在流式输出。', {
        messages: [q3, a3Partial],
        runtimeState: 'running_remote',
        liveStreamStatus: 'streaming',
        activeRunId: 'run-9',
      }),
      step(
        'queued_local · 本地队列',
        '追加输入先进入本地队列，可重排、可撤销。',
        {
          messages: [q3, a3Partial],
          runtimeState: 'running_remote',
          liveStreamStatus: 'streaming',
          activeRunId: 'run-9',
          queue: [intent(steerText, 'queued_local')],
          intentState: 'queued_local',
        },
      ),
      step(
        'awaiting_provider_ack',
        '队列项已发往网关等待 user_ack；composer 显示等待回执的本地加载态。',
        {
          messages: [q3, a3Partial],
          runtimeState: 'running_remote',
          liveStreamStatus: 'streaming',
          activeRunId: 'run-9',
          pendingAckIntents: [
            intent(steerText, 'awaiting_provider_ack', { pendingInputId: 'p-1' }),
          ],
          intentState: 'awaiting_provider_ack',
        },
      ),
      step(
        'user_ack · 物化为消息',
        'provider 确认后，追加输入物化为转录消息，保持行身份。',
        {
          messages: [
            q3,
            { ...a3Partial, pending: false, localState: 'remote_final' as const },
            userMessage(steerText, { localState: 'remote_final' }),
            assistantStreaming('Adding a hit-rate counter to the cache layer…'),
          ],
          runtimeState: 'running_remote',
          liveStreamStatus: 'streaming',
          activeRunId: 'run-9',
          intentState: 'remote_accepted',
        },
      ),
    ],
  };

  const historyPath: Story = {
    id: 'history',
    name: '历史加载与分页',
    description: '初次进入的历史占位、向上翻页的 earlier loading，以及加载完成态。',
    steps: [
      step('加载历史占位', '空消息 + historyLoading → 显示加载占位气泡。', {
        messages: [],
        historyLoading: true,
        showHistoryLoadingPlaceholder: true,
      }),
      step('历史就绪', '第一页（canonical 100 条上限）渲染完成。', {
        messages: [
          userMessage('How does the retry budget work?', { localState: 'remote_final' }),
          assistantMessage('Each request gets three attempts with exponential backoff.'),
          userMessage('And the budget resets per session?', { localState: 'remote_final' }),
          assistantMessage('Yes — the budget is scoped to the provider session.'),
        ],
      }),
      step('向上翻页', '滚动接近用户回合边界触发 before_index 翻页；顶部出现 earlier spinner。', {
        messages: [
          userMessage('How does the retry budget work?', { localState: 'remote_final' }),
          assistantMessage('Each request gets three attempts with exponential backoff.'),
          userMessage('And the budget resets per session?', { localState: 'remote_final' }),
          assistantMessage('Yes — the budget is scoped to the provider session.'),
        ],
        historyLoadingEarlier: true,
      }),
    ],
  };

  const longToolRun: Story = {
    id: 'tool-collapse',
    name: '多步工具回合 · 折叠与 final answer',
    description:
      '一个多步工具回合在运行中保持展开计数，结束后折叠为 "Worked"，最终回答外置在折叠之外。',
    steps: [
      step('多工具运行中', '连续 tool_use/tool_result 组成工具组。', {
        messages: [
          userMessage('Audit the workspace for unused dependencies.', {
            localState: 'remote_final',
          }),
          ...toolPair('Bash', 'npm ls --depth=0', { text: '42 packages' }),
          ...toolPair('Grep', 'import .* from', { text: '318 matches' }),
          ...toolPair('Read', 'package.json', { text: '88 lines' }),
        ],
        runtimeState: 'running_remote',
        liveStreamStatus: 'streaming',
        activeRunId: 'run-5',
      }),
      step('回合完成 · 折叠 + 外置回答', '工具步骤收进折叠组，最终回答独立呈现。', {
        messages: [
          userMessage('Audit the workspace for unused dependencies.', {
            localState: 'remote_final',
          }),
          ...toolPair('Bash', 'npm ls --depth=0', { text: '42 packages' }),
          ...toolPair('Grep', 'import .* from', { text: '318 matches' }),
          ...toolPair('Read', 'package.json', { text: '88 lines' }),
          assistantMessage(
            [
              'Three packages are unused and safe to remove:',
              '',
              '- `left-pad-utils`',
              '- `legacy-polyfills`',
              '- `moment-timezone-lite`',
            ].join('\n'),
          ),
        ],
      }),
    ],
  };

  const richContent: Story = {
    id: 'rich-content',
    name: '消息形态 · Markdown 与错误工具',
    description: '富文本、代码块、列表、失败的工具调用与 error 气泡的静态形态。',
    steps: [
      step('富文本与失败工具', '渲染层的各种消息形态一览。', {
        messages: [
          userMessage('Show me the migration plan.', { localState: 'remote_final' }),
          assistantMessage(
            [
              '## Migration plan',
              '',
              '1. Freeze writes on the legacy table',
              '2. Backfill into the new schema',
              '3. Flip the read path behind a flag',
              '',
              '| Phase | Owner | Risk |',
              '| --- | --- | --- |',
              '| Freeze | infra | low |',
              '| Backfill | data | medium |',
              '',
              'Inline `code`, **bold**, and a [link](https://example.test/docs).',
            ].join('\n'),
          ),
          ...toolPair('Bash', 'psql -c "SELECT count(*) FROM legacy"', {
            text: 'connection refused',
            isError: true,
          }),
          assistantMessage('The database is unreachable from this workspace.', {
            localState: 'error',
          } as Partial<UiTranscriptMessage>),
        ],
      }),
    ],
  };

  const markdownParity: Story = {
    id: 'markdown-parity',
    name: '消息排版 · 阅读基线',
    description: '固定宽度与固定内容，用于核对正文、列表、强调和行内代码的像素级排版。',
    steps: [
      step('Markdown 阅读基线', '直接渲染 RichMessageText，隔离消息状态与工具行。', {
        messages: [],
      }),
    ],
  };

  const taskNotificationUserText = Array.from(
    { length: 18 },
    (_, index) =>
      `Width owner validation sentence ${index + 1} keeps this ordinary user bubble at the shared trailing cap.`,
  ).join(' ');
  const taskNotificationTutorial = [
    'View details: garyx task get #TASK-528',
    '',
    'Review next:',
    'If changes are needed, move the task back to in progress and send feedback to the task thread:',
    'garyx task update #TASK-528 --status in_progress --note "needs changes: summary"',
    '',
    'If approved, mark it done:',
    'garyx task update #TASK-528 --status done --note "approved by reviewer"',
  ].join('\n');
  const taskNotificationStep = (
    label: string,
    description: string,
    body: string,
    seq: number,
  ): StoryStep => {
    const userSeq = seq - 1;
    const taskText = [
      '<garyx_task_notification event="ready_for_review" task_id="#TASK-528" status="in_review" title="MCP tool review">',
      body,
      '</garyx_task_notification>',
      '',
      taskNotificationTutorial,
    ].join('\n');
    return step(label, description, {
      messages: [
        userMessage(taskNotificationUserText, {
          id: `task-notification-width-owner-${userSeq}`,
          localState: 'remote_final',
          seq: userSeq,
        }),
        userMessage(taskText, {
          id: `task-notification-story-row-${seq}`,
          localState: 'remote_final',
          seq,
        }),
      ],
      renderState: {
        based_on_seq: seq,
        rows: [
          {
            kind: 'user_turn',
            id: `user_turn:seq:${userSeq}`,
            user: {
              id: `seq:${userSeq}`,
              seq: userSeq,
              role: 'user',
            },
            activity: [],
            started_at: null,
            finished_at: null,
            capsule_cards: [],
          },
          {
            kind: 'user_turn',
            id: `user_turn:seq:${seq}`,
            user: {
              id: `seq:${seq}`,
              seq,
              role: 'user',
              presentation: {
                kind: 'task_notification',
                event: 'ready_for_review',
                status: 'in_review',
                task_id: '#TASK-528',
                title: 'MCP tool review',
              },
            },
            activity: [],
            started_at: null,
            finished_at: null,
            capsule_cards: [],
          },
        ],
        tailActivity: 'none',
        activeToolGroupId: null,
        progress_locus: 'none',
        filtered_placeholders: [],
      },
    });
  };

  const taskNotification: Story = {
    id: 'task-notification',
    name: '任务通知 · 待审查卡片',
    description: 'garyx_task_notification 结构化通知渲染为专用审查卡片，而不是裸 XML。',
    steps: [
      taskNotificationStep(
        '短正文 · 无展开',
        '短正文用于确认无 overflow 时没有展开 affordance，并与长用户消息共享宽度 owner。',
        'All focused tests pass.',
        528,
      ),
      taskNotificationStep(
        '单行长文本 · 响应宽度',
        '一个源文本行在窄宽度折行溢出，在加宽后重新测量并恢复 fit。',
        Array.from(
          { length: 6 },
          (_, index) =>
            `Wrapping segment ${index + 1} preserves a single source line while exercising measured line boxes across width changes.`,
        ).join(' '),
        530,
      ),
      taskNotificationStep(
        '十一显式行 · clamp',
        '十一行正文必须超过十个当前字体 line-boxes。',
        Array.from({ length: 11 }, (_, index) => `Explicit line ${index + 1}.  `).join('\n'),
        532,
      ),
      taskNotificationStep(
        '富 Markdown · 交互共存',
        '列表、代码、表格和链接使用真实 RichMessageText 布局；链接不得触发展开。',
        [
          '[Review documentation](https://example.test/task-notification-docs)',
          '',
          '- Manifest discovery passed',
          '- Enable and disable passed',
          '- Login-state end-to-end path passed',
          '',
          '```ts',
          'const manifest = await discoverTools();',
          'await verify(manifest);',
          '```',
          '',
          '| Surface | Result |',
          '| --- | --- |',
          '| Desktop | pass |',
          '| iOS | pass |',
        ].join('\n'),
        534,
      ),
      taskNotificationStep(
        '延迟图片 · intrinsic settling',
        '远端图片晚到后 ResizeObserver/load 重新测量，并从 fit 变为 overflow。',
        [
          'The image below settles after the initial text layout.',
          '',
          '![Late intrinsic task result](https://example.test/task-notification-late-image.svg)',
        ].join('\n'),
        536,
      ),
    ],
  };

  return [
    happyPath,
    failurePath,
    steerPath,
    historyPath,
    longToolRun,
    markdownParity,
    richContent,
    taskNotification,
  ];
}
