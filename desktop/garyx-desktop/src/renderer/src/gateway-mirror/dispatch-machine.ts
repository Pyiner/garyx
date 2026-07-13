// Dispatch-machine domain of the GatewayMirror (endgame architecture
// batch 3a): owns the MessageMachineState storage. The reducer itself
// stays in message-machine.ts — this module only stores, dispatches, and
// notifies. Send/steer/interrupt orchestration migrates in batch 3c.
//
// Semantics mirror React's useReducer contract the AppShell previously
// used: one reducer application per action, and a dispatch that produces
// an identical state reference (Object.is bail-out) neither commits nor
// notifies.

import {
  initialMessageMachineState,
  messageMachineReducer,
  type MessageMachineAction,
  type MessageMachineState,
} from "../message-machine.ts";

export type Unsubscribe = () => void;

const TERMINAL_INTENT_STATES = new Set([
  "completed",
  "cancelled",
  "failed",
  "interrupted",
]);

export function collectTerminalThreadIntents(
  state: MessageMachineState,
  threadId: string,
  retainedIntentIds: ReadonlySet<string>,
): MessageMachineState {
  const queuedIntentIds = new Set(state.queueByThread[threadId] ?? []);
  let intentsById: MessageMachineState["intentsById"] | null = null;

  for (const [intentId, intent] of Object.entries(state.intentsById)) {
    if (
      intent.threadId !== threadId ||
      !TERMINAL_INTENT_STATES.has(intent.state) ||
      retainedIntentIds.has(intentId) ||
      queuedIntentIds.has(intentId)
    ) {
      continue;
    }
    intentsById ??= { ...state.intentsById };
    delete intentsById[intentId];
  }

  let queueByThread: MessageMachineState["queueByThread"] | null = null;
  if (
    Object.prototype.hasOwnProperty.call(state.queueByThread, threadId) &&
    state.queueByThread[threadId]?.length === 0
  ) {
    queueByThread = { ...state.queueByThread };
    delete queueByThread[threadId];
  }

  if (!intentsById && !queueByThread) {
    return state;
  }
  return {
    ...state,
    intentsById: intentsById ?? state.intentsById,
    queueByThread: queueByThread ?? state.queueByThread,
  };
}

export class DispatchMachine {
  private state: MessageMachineState = initialMessageMachineState;
  private listeners = new Set<() => void>();

  getState(): MessageMachineState {
    return this.state;
  }

  subscribe(listener: () => void): Unsubscribe {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Apply one action through the shared reducer and return the resulting
   * state. The returned reference is the committed state, so callers that
   * need the post-dispatch value synchronously (the legacy
   * messageStateRef shadow) can use it without re-reading.
   */
  dispatch(action: MessageMachineAction): MessageMachineState {
    const next = messageMachineReducer(this.state, action);
    return this.commit(next);
  }

  /**
   * Desktop storage reclamation layered over the canonical runtime-only clear.
   * The shared action/reducer contract stays unchanged; both state transforms
   * publish as one machine commit so subscribers never observe a half-release.
   */
  releaseThread(
    threadId: string,
    retainedIntentIds: ReadonlySet<string>,
  ): MessageMachineState {
    const cleared = messageMachineReducer(this.state, {
      type: "thread/clear",
      threadId,
    });
    return this.commit(
      collectTerminalThreadIntents(cleared, threadId, retainedIntentIds),
    );
  }

  private commit(next: MessageMachineState): MessageMachineState {
    if (next === this.state) {
      return next;
    }
    this.state = next;
    for (const listener of [...this.listeners]) {
      listener();
    }
    return next;
  }
}
