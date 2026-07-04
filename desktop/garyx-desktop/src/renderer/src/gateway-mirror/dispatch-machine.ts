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
