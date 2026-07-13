import type {
  GaryxDesktopApi,
  WindowLayoutCommandResult,
  WindowLayoutSnapshotUpdate,
} from "../../../shared/contracts.ts";

import type { HorizontalLayoutFrameStore } from "./horizontal-layout-frame-store";
import type {
  BoundsRejectionReason,
  HorizontalLayoutEvent,
  LayoutMachineEffect,
  SessionCheckpointRejectionReason,
} from "./responsive-layout-model";

type WindowLayoutApi = Pick<
  GaryxDesktopApi,
  | "executeWindowLayoutCommand"
  | "subscribeWindowLayoutSnapshots"
  | "unsubscribeWindowLayoutSnapshots"
>;

export type HorizontalLayoutEffectRunner = Readonly<{
  dispatch(event: HorizontalLayoutEvent): void;
  run(effects: readonly LayoutMachineEffect[]): void;
  stop(): void;
}>;

function checkpointReason(
  result: Extract<WindowLayoutCommandResult, { accepted: false }>,
): SessionCheckpointRejectionReason {
  return result.reason === "stale" || result.reason === "superseded"
    ? result.reason
    : "invalid";
}

function boundsReason(
  result: Extract<WindowLayoutCommandResult, { accepted: false }>,
): BoundsRejectionReason {
  return result.reason;
}

export function createHorizontalLayoutEffectRunner({
  api,
  store,
  scheduleTimeout = (callback, delay) => window.setTimeout(callback, delay),
  cancelTimeout = (handle) => window.clearTimeout(handle),
  scheduleFrame = (callback) => window.requestAnimationFrame(callback),
  cancelFrame = (handle) => window.cancelAnimationFrame(handle),
}: {
  api: WindowLayoutApi;
  store: HorizontalLayoutFrameStore;
  scheduleTimeout?: (callback: () => void, delay: number) => number;
  cancelTimeout?: (handle: number) => void;
  scheduleFrame?: (callback: () => void) => number;
  cancelFrame?: (handle: number) => void;
}): HorizontalLayoutEffectRunner {
  let stopped = false;
  const timeoutHandles = new Set<number>();
  const frameHandles = new Set<number>();

  const dispatch = (event: HorizontalLayoutEvent) => {
    if (stopped) {
      return;
    }
    run(store.dispatch(event));
  };

  const commandFailed = (
    effect: Extract<
      LayoutMachineEffect,
      {
        type:
          | "window-layout-session"
          | "window-bounds"
          | "claim-initial-layout";
      }
    >,
  ): WindowLayoutCommandResult => ({
    accepted: false,
    commandType: effect.command.type,
    reason: "invalid",
    acknowledgedSession: store.getState().acknowledgedSession,
    snapshot: store.getState().snapshot,
  });

  const applyCommandResult = (
    effect: Extract<
      LayoutMachineEffect,
      {
        type:
          | "window-layout-session"
          | "window-bounds"
          | "claim-initial-layout";
      }
    >,
    result: WindowLayoutCommandResult,
  ) => {
    if (stopped) {
      return;
    }
    const rendererEpoch = effect.command.rendererEpoch;
    if (effect.type === "window-layout-session") {
      dispatch(
        result.accepted
          ? {
              type: "WINDOW_LAYOUT_SESSION_APPLIED",
              rendererEpoch,
              transactionId: effect.command.transactionId,
              acknowledgedSession: result.acknowledgedSession,
            }
          : {
              type: "WINDOW_LAYOUT_SESSION_REJECTED",
              rendererEpoch,
              transactionId: effect.command.transactionId,
              reason: checkpointReason(result),
              acknowledgedSession: result.acknowledgedSession,
            },
      );
      return;
    }
    if (effect.type === "claim-initial-layout") {
      dispatch(
        result.accepted
          ? {
              type: "CLAIM_INITIAL_LAYOUT_APPLIED",
              rendererEpoch,
              acknowledgedSession: result.acknowledgedSession,
              snapshot: result.snapshot,
            }
          : {
              type: "CLAIM_INITIAL_LAYOUT_REJECTED",
              rendererEpoch,
              reason: checkpointReason(result),
              acknowledgedSession: result.acknowledgedSession,
              snapshot: result.snapshot,
            },
      );
      return;
    }
    dispatch(
      result.accepted
        ? {
            type: "WINDOW_BOUNDS_APPLIED",
            rendererEpoch,
            transactionId: effect.command.transactionId,
            sequence: effect.command.sequence,
            acknowledgedSession: result.acknowledgedSession,
            snapshot: result.snapshot,
          }
        : {
            type: "WINDOW_BOUNDS_REJECTED",
            rendererEpoch,
            transactionId: effect.command.transactionId,
            sequence: effect.command.sequence,
            reason: boundsReason(result),
            acknowledgedSession: result.acknowledgedSession,
            snapshot: result.snapshot,
          },
    );
  };

  const executeCommand = (
    effect: Extract<
      LayoutMachineEffect,
      {
        type:
          | "window-layout-session"
          | "window-bounds"
          | "claim-initial-layout";
      }
    >,
  ) => {
    void api
      .executeWindowLayoutCommand(effect.command)
      .then((result) => applyCommandResult(effect, result))
      .catch(() => applyCommandResult(effect, commandFailed(effect)));
  };

  const run = (effects: readonly LayoutMachineEffect[]) => {
    if (stopped) {
      return;
    }
    for (const effect of effects) {
      switch (effect.type) {
        case "window-layout-session":
        case "window-bounds":
        case "claim-initial-layout":
          executeCommand(effect);
          break;
        case "schedule-deadline": {
          const delay = effect.deadline === "open" ? 100 : 420;
          const handle = scheduleTimeout(() => {
            timeoutHandles.delete(handle);
            dispatch({
              type:
                effect.deadline === "open"
                  ? "OPEN_DEADLINE_EXPIRED"
                  : "CLOSE_DEADLINE_EXPIRED",
              transactionId: effect.transactionId,
            });
          }, delay);
          timeoutHandles.add(handle);
          break;
        }
        case "request-frame-commit": {
          const handle = scheduleFrame(() => {
            frameHandles.delete(handle);
            dispatch({
              type: "FRAME_COMMITTED",
              transactionId: effect.transactionId,
            });
          });
          frameHandles.add(handle);
          break;
        }
        case "diagnostic":
          console.error(
            "horizontal layout protocol diagnostic",
            effect.code,
            effect.transactionId ?? "",
          );
          break;
        case "persist-preference":
          // The controller owns the existing settings/localStorage writers.
          break;
      }
    }
  };

  const handleSnapshot = (update: WindowLayoutSnapshotUpdate) => {
    dispatch({
      type:
        update.snapshot.origin === "panel-machine"
          ? "VIEWPORT_RESIZED_DURING_NATIVE_SESSION"
          : "WINDOW_SNAPSHOT_CHANGED",
      snapshot: update.snapshot,
      acknowledgedSession: update.acknowledgedSession,
    });
  };
  api.subscribeWindowLayoutSnapshots(handleSnapshot);

  return {
    dispatch,
    run,
    stop() {
      if (stopped) {
        return;
      }
      stopped = true;
      api.unsubscribeWindowLayoutSnapshots(handleSnapshot);
      for (const handle of timeoutHandles) {
        cancelTimeout(handle);
      }
      timeoutHandles.clear();
      for (const handle of frameHandles) {
        cancelFrame(handle);
      }
      frameHandles.clear();
    },
  };
}
