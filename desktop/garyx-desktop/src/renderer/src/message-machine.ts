import type {
  MessageFileAttachment,
  MessageImageAttachment,
} from '@shared/contracts';

export type ComposerPhase = 'empty' | 'editing' | 'ime_composing' | 'locked';

export type IntentDispatchMode = 'sync_send' | 'async_steer';

export type IntentState =
  | 'queued_local'
  | 'dispatch_requested'
  | 'dispatching'
  | 'remote_accepted'
  | 'awaiting_response'
  | 'awaiting_history'
  | 'completed'
  | 'failed'
  | 'interrupted'
  | 'cancelled';

export type IntentSource =
  | 'composer_send'
  | 'composer_queue'
  | 'queue_send'
  | 'queue_steer'
  | 'retry';

export interface MessageIntent {
  intentId: string;
  threadId: string;
  text: string;
  images: MessageImageAttachment[];
  files: MessageFileAttachment[];
  createdAt: string;
  updatedAt: string;
  state: IntentState;
  source: IntentSource;
  dispatchMode?: IntentDispatchMode;
  remoteRunId?: string;
  remoteThreadKey?: string;
  pendingInputId?: string;
  responseText?: string;
  error?: string;
}

export type ThreadRuntimeState =
  | 'idle'
  | 'dispatching_sync'
  | 'running_remote'
  | 'reconciling_history'
  | 'interrupting'
  | 'failed';

export interface ThreadRuntime {
  threadId: string;
  state: ThreadRuntimeState;
  activeIntentId?: string;
  remoteRunId?: string;
  lastError?: string;
  updatedAt: string;
}

export interface MessageMachineState {
  composerPhase: ComposerPhase;
  intentsById: Record<string, MessageIntent>;
  queueByThread: Record<string, string[]>;
  threadRuntimeByThread: Record<string, ThreadRuntime>;
}

export type MessageMachineAction =
  | {
      type: 'composer/sync';
      hasText: boolean;
      isComposing: boolean;
      locked: boolean;
    }
  | {
      type: 'intent/created';
      intent: MessageIntent;
      enqueue: boolean;
    }
  | {
      type: 'intent/request-dispatch';
      threadId: string;
      intentId: string;
      mode: IntentDispatchMode;
      source: IntentSource;
      removeFromQueue: boolean;
    }
  | {
      type: 'intent/dispatch-started';
      intentId: string;
    }
  | {
      type: 'intent/remote-accepted';
      intentId: string;
      runId: string;
      threadId: string;
      pendingInputId?: string;
      responseText?: string;
      removeFromQueue: boolean;
    }
  | {
      type: 'intent/awaiting-response';
      intentId: string;
    }
  | {
      type: 'intent/awaiting-history';
      intentId: string;
      responseText?: string;
    }
  | {
      type: 'intent/completed';
      intentId: string;
    }
  | {
      type: 'intent/failed';
      intentId: string;
      error: string;
    }
  | {
      type: 'intent/interrupted';
      intentId: string;
      error?: string;
    }
  | {
      type: 'intent/cancelled';
      threadId: string;
      intentId: string;
    }
  | {
      type: 'intent/requeue-front';
      threadId: string;
      intentId: string;
      error?: string;
      source?: IntentSource;
    }
  | {
      type: 'intent/reorder';
      threadId: string;
      intentId: string;
      toIndex: number;
    }
  | {
      type: 'thread/runtime';
      threadId: string;
      runtimeState: ThreadRuntimeState;
      activeIntentId?: string;
      remoteRunId?: string;
      error?: string;
    }
  | {
      type: 'thread/clear';
      threadId: string;
    }
  | {
      type: 'thread/delete';
      threadId: string;
    };

export const initialMessageMachineState: MessageMachineState = {
  composerPhase: 'empty',
  intentsById: {},
  queueByThread: {},
  threadRuntimeByThread: {},
};

export function buildIntent(input: {
  threadId: string;
  text: string;
  images?: MessageImageAttachment[];
  files?: MessageFileAttachment[];
  source: IntentSource;
  state: IntentState;
  dispatchMode?: IntentDispatchMode;
}): MessageIntent {
  const now = new Date().toISOString();
  return {
    intentId: `intent:${crypto.randomUUID()}`,
    threadId: input.threadId,
    text: input.text,
    images: [...(input.images || [])],
    files: [...(input.files || [])],
    createdAt: now,
    updatedAt: now,
    state: input.state,
    source: input.source,
    dispatchMode: input.dispatchMode,
  };
}

export function nextComposerPhase(input: {
  hasText: boolean;
  isComposing: boolean;
  locked: boolean;
}): ComposerPhase {
  if (input.locked) {
    return 'locked';
  }
  if (input.isComposing) {
    return 'ime_composing';
  }
  return input.hasText ? 'editing' : 'empty';
}

export function isRuntimeBusy(state: ThreadRuntimeState | undefined): boolean {
  return Boolean(
    state &&
      state !== 'idle' &&
      state !== 'failed',
  );
}

export function selectThreadRuntime(
  state: MessageMachineState,
  threadId: string | null,
): ThreadRuntime | null {
  if (!threadId) {
    return null;
  }
  return state.threadRuntimeByThread[threadId] || null;
}

export function selectQueueIntentIds(
  state: MessageMachineState,
  threadId: string | null,
): string[] {
  if (!threadId) {
    return [];
  }
  return state.queueByThread[threadId] || [];
}

export function selectGlobalActiveThreadId(state: MessageMachineState): string | null {
  for (const runtime of Object.values(state.threadRuntimeByThread)) {
    if (isRuntimeBusy(runtime.state)) {
      return runtime.threadId;
    }
  }
  return null;
}

function removeIntentFromQueue(queue: string[] | undefined, intentId: string): string[] {
  return (queue || []).filter((entry) => entry !== intentId);
}

function reorderQueueIntent(
  queue: string[] | undefined,
  intentId: string,
  toIndex: number,
): string[] | null {
  const nextQueue = [...(queue || [])];
  const fromIndex = nextQueue.indexOf(intentId);
  if (fromIndex < 0) {
    return null;
  }

  const boundedIndex = Math.max(0, Math.min(toIndex, nextQueue.length - 1));
  if (boundedIndex === fromIndex) {
    return null;
  }

  nextQueue.splice(fromIndex, 1);
  nextQueue.splice(boundedIndex, 0, intentId);
  return nextQueue;
}

function upsertRuntime(
  state: MessageMachineState,
  threadId: string,
  runtimeState: ThreadRuntimeState,
  extra?: Partial<ThreadRuntime>,
): MessageMachineState {
  const now = new Date().toISOString();
  return {
    ...state,
    threadRuntimeByThread: {
      ...state.threadRuntimeByThread,
      [threadId]: {
        ...state.threadRuntimeByThread[threadId],
        ...extra,
        threadId,
        state: runtimeState,
        updatedAt: now,
      },
    },
  };
}

function patchIntent(
  state: MessageMachineState,
  intentId: string,
  patch: Partial<MessageIntent>,
): MessageMachineState {
  const current = state.intentsById[intentId];
  if (!current) {
    return state;
  }
  return {
    ...state,
    intentsById: {
      ...state.intentsById,
      [intentId]: {
        ...current,
        ...patch,
        updatedAt: new Date().toISOString(),
      },
    },
  };
}

export function messageMachineReducer(
  state: MessageMachineState,
  action: MessageMachineAction,
): MessageMachineState {
  switch (action.type) {
    case 'composer/sync': {
      const composerPhase = nextComposerPhase(action);
      return composerPhase === state.composerPhase
        ? state
        : {
            ...state,
            composerPhase,
          };
    }

    case 'intent/created': {
      const queue = state.queueByThread[action.intent.threadId] || [];
      return {
        ...state,
        intentsById: {
          ...state.intentsById,
          [action.intent.intentId]: action.intent,
        },
        queueByThread: action.enqueue
          ? {
              ...state.queueByThread,
              [action.intent.threadId]: [...queue, action.intent.intentId],
            }
          : state.queueByThread,
      };
    }

    case 'intent/request-dispatch': {
      const current = state.intentsById[action.intentId];
      if (!current) {
        return state;
      }
      const queue = action.removeFromQueue
        ? removeIntentFromQueue(state.queueByThread[action.threadId], action.intentId)
        : state.queueByThread[action.threadId] || [];
      const next = patchIntent(state, action.intentId, {
        state: 'dispatch_requested',
        dispatchMode: action.mode,
        source: action.source,
        error: undefined,
      });
      return {
        ...next,
        queueByThread: {
          ...next.queueByThread,
          [action.threadId]: queue,
        },
      };
    }

    case 'intent/dispatch-started':
      return patchIntent(state, action.intentId, {
        state: 'dispatching',
        error: undefined,
      });

    case 'intent/awaiting-response':
      return patchIntent(state, action.intentId, {
        state: 'awaiting_response',
      });

    case 'intent/remote-accepted': {
      const next = patchIntent(state, action.intentId, {
        state: 'remote_accepted',
        remoteRunId: action.runId,
        remoteThreadKey: action.threadId,
        pendingInputId: action.pendingInputId,
        responseText: action.responseText,
        error: undefined,
      });
      if (!action.removeFromQueue) {
        return next;
      }
      const current = next.intentsById[action.intentId];
      if (!current) {
        return next;
      }
      return {
        ...next,
        queueByThread: {
          ...next.queueByThread,
          [current.threadId]: removeIntentFromQueue(next.queueByThread[current.threadId], action.intentId),
        },
      };
    }

    case 'intent/awaiting-history':
      return patchIntent(state, action.intentId, {
        state: 'awaiting_history',
        responseText: action.responseText,
      });

    case 'intent/completed':
      return patchIntent(state, action.intentId, {
        state: 'completed',
        error: undefined,
      });

    case 'intent/failed':
      return patchIntent(state, action.intentId, {
        state: 'failed',
        error: action.error,
      });

    case 'intent/interrupted':
      return patchIntent(state, action.intentId, {
        state: 'interrupted',
        error: action.error,
      });

    case 'intent/cancelled': {
      const next = patchIntent(state, action.intentId, {
        state: 'cancelled',
      });
      return {
        ...next,
        queueByThread: {
          ...next.queueByThread,
          [action.threadId]: removeIntentFromQueue(next.queueByThread[action.threadId], action.intentId),
        },
      };
    }

    case 'intent/requeue-front': {
      const next = patchIntent(state, action.intentId, {
        state: 'queued_local',
        dispatchMode: undefined,
        remoteRunId: undefined,
        remoteThreadKey: undefined,
        responseText: undefined,
        error: action.error,
        source: action.source || 'queue_send',
      });
      const queue = next.queueByThread[action.threadId] || [];
      return {
        ...next,
        queueByThread: {
          ...next.queueByThread,
          [action.threadId]: queue.includes(action.intentId)
            ? queue
            : [action.intentId, ...queue],
        },
      };
    }

    case 'intent/reorder': {
      const queue = reorderQueueIntent(
        state.queueByThread[action.threadId],
        action.intentId,
        action.toIndex,
      );
      if (!queue) {
        return state;
      }
      return {
        ...state,
        queueByThread: {
          ...state.queueByThread,
          [action.threadId]: queue,
        },
      };
    }

    case 'thread/runtime':
      return upsertRuntime(state, action.threadId, action.runtimeState, {
        activeIntentId: action.activeIntentId,
        remoteRunId: action.remoteRunId,
        lastError: action.error,
      });

    case 'thread/clear': {
      const next = { ...state.threadRuntimeByThread };
      delete next[action.threadId];
      return {
        ...state,
        threadRuntimeByThread: next,
      };
    }

    case 'thread/delete': {
      const intentsById = { ...state.intentsById };
      for (const intent of Object.values(state.intentsById)) {
        if (intent.threadId === action.threadId) {
          delete intentsById[intent.intentId];
        }
      }
      const queueByThread = { ...state.queueByThread };
      delete queueByThread[action.threadId];
      const threadRuntimeByThread = { ...state.threadRuntimeByThread };
      delete threadRuntimeByThread[action.threadId];
      return {
        ...state,
        intentsById,
        queueByThread,
        threadRuntimeByThread,
      };
    }

    default:
      return state;
  }
}
