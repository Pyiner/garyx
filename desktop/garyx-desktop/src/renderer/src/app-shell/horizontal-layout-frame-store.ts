import {
  createHorizontalLayoutState,
  projectHorizontalLayout,
  reduceHorizontalLayout,
  stableFrameFromProjection,
  type CreateHorizontalLayoutStateInput,
  type HorizontalLayoutEvent,
  type HorizontalLayoutFrame,
  type HorizontalLayoutState,
  type LayoutMachineEffect,
  type LayoutPolicyName,
  type StableHorizontalLayoutFrame,
} from "./responsive-layout-model.ts";

export type HorizontalLayoutFrameRoot = Pick<
  HTMLElement,
  "style" | "setAttribute" | "removeAttribute"
>;

export const HORIZONTAL_LAYOUT_FRAME_VARIABLES = [
  "--gx-sidebar-preferred-width",
  "--gx-conversation-rail-preferred-width",
  "--gx-side-tools-preferred-width",
  "--gx-thread-logs-preferred-width",
  "--gx-sidebar-width",
  "--gx-conversation-rail-width",
  "--gx-shell-main-width",
  "--gx-conversation-width",
  "--gx-right-resizer-width",
  "--gx-right-panel-width",
  "--gx-thread-main-width",
  "--gx-thread-log-resizer-width",
  "--gx-thread-log-panel-width",
] as const;

export const HORIZONTAL_LAYOUT_FRAME_ATTRIBUTES = [
  "data-layout-policy",
  "data-sidebar-state",
  "data-conversation-rail-state",
  "data-side-tools-state",
  "data-thread-logs-presentation",
  "data-task-tree-presentation",
  "data-header-density",
  "data-layout-revision",
] as const;

function requiredPaintFrame(
  projection: HorizontalLayoutFrame,
): StableHorizontalLayoutFrame {
  const frame = stableFrameFromProjection(projection);
  if (!frame) {
    throw new Error(
      `horizontal layout must project a paintable frame, received ${projection.kind}`,
    );
  }
  return frame;
}

/**
 * Synchronously publishes one complete horizontal frame. Browsers cannot
 * paint in the middle of this call; the revision marker is written last so
 * observers also have an explicit commit boundary for the variables and
 * presentation attributes that precede it.
 */
export function applyFrame(
  root: HorizontalLayoutFrameRoot,
  projection: HorizontalLayoutFrame,
): StableHorizontalLayoutFrame {
  const frame = requiredPaintFrame(projection);
  for (const [name, value] of Object.entries(frame.cssVariables)) {
    root.style.setProperty(name, value);
  }
  for (const [name, value] of Object.entries(frame.dataAttributes)) {
    if (name !== "data-layout-revision") {
      root.setAttribute(name, value);
    }
  }
  root.setAttribute(
    "data-layout-revision",
    frame.dataAttributes["data-layout-revision"],
  );
  return frame;
}

export function clearFrame(root: HorizontalLayoutFrameRoot): void {
  for (const variable of HORIZONTAL_LAYOUT_FRAME_VARIABLES) {
    root.style.removeProperty(variable);
  }
  for (const attribute of HORIZONTAL_LAYOUT_FRAME_ATTRIBUTES) {
    root.removeAttribute(attribute);
  }
}

export type HorizontalLayoutFrameStore = Readonly<{
  attachRoot(root: HorizontalLayoutFrameRoot | null): void;
  dispatch(event: HorizontalLayoutEvent): readonly LayoutMachineEffect[];
  getSnapshot(): StableHorizontalLayoutFrame;
  getState(): HorizontalLayoutState;
  subscribe(listener: () => void): () => void;
}>;

export type LegacyHorizontalLayoutFrameStore = HorizontalLayoutFrameStore;

export function createHorizontalLayoutFrameStore(
  input: Omit<CreateHorizontalLayoutStateInput, "policy"> & {
    policy: LayoutPolicyName;
  },
): HorizontalLayoutFrameStore {
  let state = createHorizontalLayoutState(input);
  let frame = requiredPaintFrame(projectHorizontalLayout(state));
  let root: HorizontalLayoutFrameRoot | null = null;
  const listeners = new Set<() => void>();

  const publish = () => {
    frame = requiredPaintFrame(projectHorizontalLayout(state));
    if (root) {
      applyFrame(root, frame);
    }
    for (const listener of [...listeners]) {
      listener();
    }
  };

  return {
    attachRoot(nextRoot) {
      if (root && root !== nextRoot) {
        clearFrame(root);
      }
      root = nextRoot;
      if (root) {
        applyFrame(root, frame);
      }
    },
    dispatch(event) {
      const reduction = reduceHorizontalLayout(state, event);
      state = reduction.state;
      const checkpoint = state.policy === "legacy"
        ? reduction.effects.find(
            (effect) => effect.type === "window-layout-session",
          )
        : undefined;
      if (checkpoint?.type === "window-layout-session") {
        const acknowledgedSession = {
          ...state.acknowledgedSession,
          desiredOccupancy: checkpoint.command.desiredOccupancy,
          sessionRevision:
            state.acknowledgedSession.sessionRevision + 1,
        };
        const settled = reduceHorizontalLayout(state, {
          type: "WINDOW_LAYOUT_SESSION_APPLIED",
          rendererEpoch: state.rendererEpoch,
          transactionId: checkpoint.command.transactionId,
          acknowledgedSession,
        });
        if (settled.effects.length > 0) {
          throw new Error("legacy checkpoint acknowledgement emitted effects");
        }
        // Legacy mode has no native result that could ever refer back to a
        // transaction. Compact it after its ordered local checkpoint so the
        // adapter cannot retain an unbounded history of settled intents.
        state = {
          ...settled.state,
          transactions: {},
          headTransactionId: null,
        };
      }
      publish();
      return reduction.effects;
    },
    getSnapshot() {
      return frame;
    },
    getState() {
      return state;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

export function createLegacyHorizontalLayoutFrameStore(
  input: Omit<CreateHorizontalLayoutStateInput, "policy">,
): LegacyHorizontalLayoutFrameStore {
  return createHorizontalLayoutFrameStore({ ...input, policy: "legacy" });
}
