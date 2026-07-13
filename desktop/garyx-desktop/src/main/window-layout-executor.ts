import {
  CLOSED_WINDOW_LAYOUT_OCCUPANCY,
  WINDOW_LAYOUT_PANEL_ORDER,
  type AcknowledgedWindowLayoutSession,
  type ClaimInitialWindowLayoutCommand,
  type HorizontalLayoutPolicyName,
  type WindowBoundsCommand,
  type WindowLayoutBootstrap,
  type WindowLayoutCommand,
  type WindowLayoutCommandRejectionReason,
  type WindowLayoutCommandResult,
  type WindowLayoutConfirmedFunding,
  type WindowLayoutFundingByPanel,
  type WindowLayoutPanelId,
  type WindowLayoutPanelOccupancy,
  type WindowLayoutRectangle,
  type WindowLayoutSessionCommand,
  type WindowLayoutSnapshot,
  type WindowLayoutSnapshotOrigin,
  type WindowLayoutSnapshotUpdate,
} from "../shared/contracts.ts";

export type WindowLayoutPhysicalEnvironment = Omit<
  WindowLayoutSnapshot,
  "windowRevision" | "origin"
>;

export type WindowLayoutHost = Readonly<{
  windowId: number;
  readEnvironment(): WindowLayoutPhysicalEnvironment;
  setBounds(bounds: WindowLayoutRectangle): void;
}>;

export type WindowLayoutRequestContext = Readonly<{
  senderWindowId: number | null;
}>;

type LayoutCheckpoint = Readonly<{
  sequence: number;
  sessionRevision: number;
  desiredOccupancy: WindowLayoutPanelOccupancy;
}>;

type QueuedCommand = {
  command: WindowLayoutCommand;
  context: WindowLayoutRequestContext;
  resolve(result: WindowLayoutCommandResult): void;
};

export type WindowLayoutExecutorDebugState = Readonly<{
  activeRendererEpoch: string | null;
  freshSession: boolean;
  latestSequence: number;
  acknowledgedSession: AcknowledgedWindowLayoutSession;
  snapshot: WindowLayoutSnapshot;
  checkpoints: Readonly<Record<string, LayoutCheckpoint>>;
  setBoundsCount: number;
  queuedCommandCount: number;
}>;

function cloneRectangle(
  rectangle: WindowLayoutRectangle,
): WindowLayoutRectangle {
  return { ...rectangle };
}

function cloneOccupancy(
  occupancy: WindowLayoutPanelOccupancy,
): WindowLayoutPanelOccupancy {
  return {
    globalSidebar: occupancy.globalSidebar,
    conversationRail: occupancy.conversationRail,
    sideTools: occupancy.sideTools,
  };
}

function cloneFunding(
  funding: WindowLayoutFundingByPanel,
): WindowLayoutFundingByPanel {
  const clone: Partial<
    Record<WindowLayoutPanelId, WindowLayoutConfirmedFunding>
  > = {};
  for (const panel of WINDOW_LAYOUT_PANEL_ORDER) {
    const entry = funding[panel];
    if (entry) {
      clone[panel] = {
        ...entry,
        repayAuthority: { ...entry.repayAuthority },
      };
    }
  }
  return clone;
}

function cloneSession(
  session: AcknowledgedWindowLayoutSession,
): AcknowledgedWindowLayoutSession {
  return {
    ...session,
    normalBaseBounds: cloneRectangle(session.normalBaseBounds),
    fundingByPanel: cloneFunding(session.fundingByPanel),
    desiredOccupancy: cloneOccupancy(session.desiredOccupancy),
  };
}

function cloneSnapshot(snapshot: WindowLayoutSnapshot): WindowLayoutSnapshot {
  return {
    ...snapshot,
    bounds: cloneRectangle(snapshot.bounds),
    contentBounds: cloneRectangle(snapshot.contentBounds),
    normalBounds: cloneRectangle(snapshot.normalBounds),
    workArea: cloneRectangle(snapshot.workArea),
  };
}

function snapshotFromEnvironment(
  environment: WindowLayoutPhysicalEnvironment,
  windowRevision: number,
  origin: WindowLayoutSnapshotOrigin,
): WindowLayoutSnapshot {
  return {
    ...environment,
    bounds: cloneRectangle(environment.bounds),
    contentBounds: cloneRectangle(environment.contentBounds),
    normalBounds: cloneRectangle(environment.normalBounds),
    workArea: cloneRectangle(environment.workArea),
    windowRevision,
    origin,
  };
}

function rectanglesEqual(
  left: WindowLayoutRectangle,
  right: WindowLayoutRectangle,
): boolean {
  return (
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height
  );
}

function snapshotsDescribeSameEnvironment(
  snapshot: WindowLayoutSnapshot,
  environment: WindowLayoutPhysicalEnvironment,
): boolean {
  return (
    rectanglesEqual(snapshot.bounds, environment.bounds) &&
    rectanglesEqual(snapshot.contentBounds, environment.contentBounds) &&
    rectanglesEqual(snapshot.normalBounds, environment.normalBounds) &&
    rectanglesEqual(snapshot.workArea, environment.workArea) &&
    snapshot.mode === environment.mode &&
    snapshot.displayId === environment.displayId &&
    snapshot.scaleFactor === environment.scaleFactor
  );
}

function isFiniteRectangle(rectangle: WindowLayoutRectangle): boolean {
  return (
    Number.isFinite(rectangle.x) &&
    Number.isFinite(rectangle.y) &&
    Number.isFinite(rectangle.width) &&
    Number.isFinite(rectangle.height) &&
    rectangle.width > 0 &&
    rectangle.height > 0
  );
}

function rectangleContainedBy(
  inner: WindowLayoutRectangle,
  outer: WindowLayoutRectangle,
): boolean {
  return (
    inner.x >= outer.x &&
    inner.y >= outer.y &&
    inner.x + inner.width <= outer.x + outer.width &&
    inner.y + inner.height <= outer.y + outer.height
  );
}

function occupanciesEqual(
  left: WindowLayoutPanelOccupancy,
  right: WindowLayoutPanelOccupancy,
): boolean {
  return WINDOW_LAYOUT_PANEL_ORDER.every(
    (panel) => left[panel] === right[panel],
  );
}

function fundingEntryEqual(
  left: WindowLayoutConfirmedFunding | undefined,
  right: WindowLayoutConfirmedFunding | undefined,
): boolean {
  if (!left || !right) {
    return left === right;
  }
  return (
    left.fundingId === right.fundingId &&
    left.panel === right.panel &&
    left.widthDelta === right.widthDelta &&
    left.xCompensation === right.xCompensation &&
    left.repayAuthority.fundingId === right.repayAuthority.fundingId
  );
}

function fundingMapsEqual(
  left: WindowLayoutFundingByPanel,
  right: WindowLayoutFundingByPanel,
): boolean {
  return WINDOW_LAYOUT_PANEL_ORDER.every((panel) =>
    fundingEntryEqual(left[panel], right[panel]),
  );
}

function fundingIsValid(funding: WindowLayoutFundingByPanel): boolean {
  const ids = new Set<string>();
  for (const panel of WINDOW_LAYOUT_PANEL_ORDER) {
    const entry = funding[panel];
    if (!entry) {
      continue;
    }
    if (
      entry.panel !== panel ||
      !entry.fundingId ||
      ids.has(entry.fundingId) ||
      !Number.isFinite(entry.widthDelta) ||
      entry.widthDelta <= 0 ||
      !Number.isFinite(entry.xCompensation) ||
      entry.repayAuthority.fundingId !== entry.fundingId
    ) {
      return false;
    }
    ids.add(entry.fundingId);
  }
  return true;
}

function totalFundingWidth(funding: WindowLayoutFundingByPanel): number {
  return WINDOW_LAYOUT_PANEL_ORDER.reduce(
    (total, panel) => total + (funding[panel]?.widthDelta ?? 0),
    0,
  );
}

function totalFundingXCompensation(
  funding: WindowLayoutFundingByPanel,
): number {
  return WINDOW_LAYOUT_PANEL_ORDER.reduce(
    (total, panel) => total + (funding[panel]?.xCompensation ?? 0),
    0,
  );
}

function boundsForSession(
  session: Pick<
    AcknowledgedWindowLayoutSession,
    "normalBaseBounds" | "fundingByPanel"
  >,
): WindowLayoutRectangle {
  return {
    x:
      session.normalBaseBounds.x +
      totalFundingXCompensation(session.fundingByPanel),
    y: session.normalBaseBounds.y,
    width:
      session.normalBaseBounds.width +
      totalFundingWidth(session.fundingByPanel),
    height: session.normalBaseBounds.height,
  };
}

function normalBaseForBounds(
  bounds: WindowLayoutRectangle,
  funding: WindowLayoutFundingByPanel,
): WindowLayoutRectangle {
  return {
    x: bounds.x - totalFundingXCompensation(funding),
    y: bounds.y,
    width: bounds.width - totalFundingWidth(funding),
    height: bounds.height,
  };
}

function validateBoundsAuthority(
  command: WindowBoundsCommand,
  session: AcknowledgedWindowLayoutSession,
): boolean {
  if (
    command.expectedWindowRevision !== session.windowRevision ||
    command.expectedSessionRevision !== session.sessionRevision ||
    !rectanglesEqual(
      command.targetNormalBaseBounds,
      session.normalBaseBounds,
    ) ||
    !rectanglesEqual(command.targetBounds, boundsForSession({
      normalBaseBounds: command.targetNormalBaseBounds,
      fundingByPanel: command.targetFundingByPanel,
    }))
  ) {
    return false;
  }

  if (command.authority.kind === "user-cause") {
    return (
      (command.authority.cause === "user-panel" ||
        command.authority.cause === "user-route") &&
      command.authority.transactionId === command.transactionId &&
      command.authority.rendererEpoch === command.rendererEpoch &&
      command.authority.sequence === command.sequence
    );
  }

  const proof = command.authority;
  if (
    proof.expectedSessionRevision !== command.expectedSessionRevision ||
    proof.expectedSessionRevision !== session.sessionRevision ||
    proof.fundingIds.length === 0
  ) {
    return false;
  }
  const proofIds = new Set(proof.fundingIds);
  if (proofIds.size !== proof.fundingIds.length) {
    return false;
  }
  for (const panel of WINDOW_LAYOUT_PANEL_ORDER) {
    const current = session.fundingByPanel[panel];
    const target = command.targetFundingByPanel[panel];
    if (!current && target) {
      return false;
    }
    if (!current) {
      continue;
    }
    const changed = !fundingEntryEqual(current, target);
    if (
      (changed && Boolean(target)) ||
      (changed && !proofIds.has(current.fundingId)) ||
      (!changed && proofIds.has(current.fundingId))
    ) {
      return false;
    }
  }
  for (const fundingId of proofIds) {
    if (
      !WINDOW_LAYOUT_PANEL_ORDER.some(
        (panel) =>
          session.fundingByPanel[panel]?.fundingId === fundingId,
      )
    ) {
      return false;
    }
  }
  return (
    totalFundingWidth(command.targetFundingByPanel) <=
      totalFundingWidth(session.fundingByPanel) &&
    command.targetBounds.width <= boundsForSession(session).width
  );
}

function defaultSession(
  snapshot: WindowLayoutSnapshot,
): AcknowledgedWindowLayoutSession {
  return {
    normalBaseBounds: cloneRectangle(snapshot.normalBounds),
    fundingByPanel: {},
    desiredOccupancy: cloneOccupancy(CLOSED_WINDOW_LAYOUT_OCCUPANCY),
    windowRevision: snapshot.windowRevision,
    sessionRevision: 0,
  };
}

export class WindowLayoutExecutor {
  readonly #host: WindowLayoutHost;
  readonly #policy: HorizontalLayoutPolicyName;
  readonly #ackDelayMs: number;
  readonly #onSnapshot: (update: WindowLayoutSnapshotUpdate) => void;
  #activeRendererEpoch: string | null = null;
  #freshSession = true;
  #latestSequence = 0;
  #acknowledgedSession: AcknowledgedWindowLayoutSession;
  #snapshot: WindowLayoutSnapshot;
  #checkpoints: Record<string, LayoutCheckpoint> = {};
  #setBoundsCount = 0;
  #queue: QueuedCommand[] = [];
  #drainScheduled = false;

  constructor({
    host,
    policy,
    ackDelayMs = 0,
    onSnapshot = () => {},
  }: {
    host: WindowLayoutHost;
    policy: HorizontalLayoutPolicyName;
    ackDelayMs?: number;
    onSnapshot?: (update: WindowLayoutSnapshotUpdate) => void;
  }) {
    this.#host = host;
    this.#policy = policy;
    this.#ackDelayMs = Math.max(0, Math.min(10_000, ackDelayMs));
    this.#onSnapshot = onSnapshot;
    this.#snapshot = snapshotFromEnvironment(
      host.readEnvironment(),
      1,
      "hydrate",
    );
    this.#acknowledgedSession = defaultSession(this.#snapshot);
  }

  bootstrap(
    rendererEpoch: string,
    context: WindowLayoutRequestContext,
  ): WindowLayoutBootstrap {
    if (!this.#authorized(context) || !rendererEpoch.trim()) {
      throw new Error("invalid window layout bootstrap sender or epoch");
    }
    if (rendererEpoch !== this.#activeRendererEpoch) {
      this.#activeRendererEpoch = rendererEpoch;
      this.#latestSequence = 0;
      this.#checkpoints = {};
      this.#rejectQueuedForEpochTakeover();
    }
    this.syncExternalEnvironment("hydrate", false);
    return {
      policy: this.#policy,
      freshSession: this.#freshSession,
      snapshot: cloneSnapshot(this.#snapshot),
      acknowledgedSession: cloneSession(this.#acknowledgedSession),
    };
  }

  execute(
    command: WindowLayoutCommand,
    context: WindowLayoutRequestContext,
  ): Promise<WindowLayoutCommandResult> {
    return new Promise((resolve) => {
      const queued: QueuedCommand = { command, context, resolve };
      if (!this.#authorized(context)) {
        resolve(this.#rejected(command, "invalid"));
        return;
      }
      if (command.rendererEpoch !== this.#activeRendererEpoch) {
        resolve(this.#rejected(command, "superseded"));
        return;
      }

      const survivors: QueuedCommand[] = [];
      for (const pending of this.#queue) {
        if (
          pending.command.rendererEpoch === command.rendererEpoch &&
          pending.command.sequence < command.sequence
        ) {
          pending.resolve(this.#rejected(pending.command, "superseded"));
        } else {
          survivors.push(pending);
        }
      }
      this.#queue = [...survivors, queued];
      if (!this.#drainScheduled) {
        this.#drainScheduled = true;
        queueMicrotask(() => this.#drain());
      }
    });
  }

  syncExternalEnvironment(
    origin: WindowLayoutSnapshotOrigin,
    notify = true,
  ): WindowLayoutSnapshotUpdate | null {
    const environment = this.#host.readEnvironment();
    if (snapshotsDescribeSameEnvironment(this.#snapshot, environment)) {
      return null;
    }
    const snapshot = snapshotFromEnvironment(
      environment,
      this.#snapshot.windowRevision + 1,
      origin,
    );
    // A physical width drag establishes a new user-owned horizontal base.
    // Keeping old panel funding after that point makes a later explicit expand
    // look already paid for, so the renderer can only overlay/reflow instead of
    // growing the window. Height-only resizes and moves keep the horizontal
    // funding ledger intact.
    const userResized =
      origin === "user" &&
      snapshot.normalBounds.width !== this.#snapshot.normalBounds.width;
    const fundingByPanel = userResized
      ? {}
      : this.#acknowledgedSession.fundingByPanel;
    const normalBaseBounds = normalBaseForBounds(
      snapshot.normalBounds,
      fundingByPanel,
    );
    const baseChanged = !rectanglesEqual(
      normalBaseBounds,
      this.#acknowledgedSession.normalBaseBounds,
    );
    const fundingChanged = !fundingMapsEqual(
      fundingByPanel,
      this.#acknowledgedSession.fundingByPanel,
    );
    this.#snapshot = snapshot;
    this.#acknowledgedSession = {
      ...this.#acknowledgedSession,
      normalBaseBounds,
      fundingByPanel: cloneFunding(fundingByPanel),
      windowRevision: snapshot.windowRevision,
      sessionRevision:
        this.#acknowledgedSession.sessionRevision +
        (baseChanged || fundingChanged ? 1 : 0),
    };
    const update = this.#currentUpdate();
    if (notify) {
      this.#onSnapshot(update);
    }
    return update;
  }

  debugState(): WindowLayoutExecutorDebugState {
    return {
      activeRendererEpoch: this.#activeRendererEpoch,
      freshSession: this.#freshSession,
      latestSequence: this.#latestSequence,
      acknowledgedSession: cloneSession(this.#acknowledgedSession),
      snapshot: cloneSnapshot(this.#snapshot),
      checkpoints: { ...this.#checkpoints },
      setBoundsCount: this.#setBoundsCount,
      queuedCommandCount: this.#queue.length,
    };
  }

  #authorized(context: WindowLayoutRequestContext): boolean {
    return context.senderWindowId === this.#host.windowId;
  }

  #currentUpdate(): WindowLayoutSnapshotUpdate {
    return {
      snapshot: cloneSnapshot(this.#snapshot),
      acknowledgedSession: cloneSession(this.#acknowledgedSession),
    };
  }

  #rejectQueuedForEpochTakeover(): void {
    const queued = this.#queue;
    this.#queue = [];
    for (const pending of queued) {
      pending.resolve(this.#rejected(pending.command, "superseded"));
    }
  }

  #drain(): void {
    this.#drainScheduled = false;
    const queued = this.#queue;
    this.#queue = [];
    for (const pending of queued) {
      const result = this.#executeNow(pending.command, pending.context);
      const delayed =
        pending.command.type === "APPLY_WINDOW_BOUNDS" &&
        this.#ackDelayMs > 0;
      if (delayed) {
        setTimeout(() => pending.resolve(result), this.#ackDelayMs);
      } else {
        pending.resolve(result);
      }
    }
    if (this.#queue.length > 0 && !this.#drainScheduled) {
      this.#drainScheduled = true;
      queueMicrotask(() => this.#drain());
    }
  }

  #executeNow(
    command: WindowLayoutCommand,
    context: WindowLayoutRequestContext,
  ): WindowLayoutCommandResult {
    if (!this.#authorized(context)) {
      return this.#rejected(command, "invalid");
    }
    if (command.rendererEpoch !== this.#activeRendererEpoch) {
      return this.#rejected(command, "superseded");
    }
    if (command.sequence < this.#latestSequence) {
      return this.#rejected(command, "superseded");
    }
    switch (command.type) {
      case "CHECKPOINT_DESIRED_OCCUPANCY":
        return this.#checkpoint(command);
      case "CLAIM_INITIAL_LAYOUT":
        return this.#claim(command);
      case "APPLY_WINDOW_BOUNDS":
        return this.#applyBounds(command);
    }
  }

  #checkpoint(
    command: WindowLayoutSessionCommand,
  ): WindowLayoutCommandResult {
    if (this.#freshSession) {
      return this.#rejected(command, "invalid");
    }
    if (
      command.expectedSessionRevision !==
      this.#acknowledgedSession.sessionRevision
    ) {
      return this.#rejected(command, "stale");
    }
    const sessionRevision = this.#acknowledgedSession.sessionRevision + 1;
    this.#acknowledgedSession = {
      ...this.#acknowledgedSession,
      desiredOccupancy: cloneOccupancy(command.desiredOccupancy),
      windowRevision: this.#snapshot.windowRevision,
      sessionRevision,
    };
    this.#latestSequence = Math.max(this.#latestSequence, command.sequence);
    this.#checkpoints = {
      [command.transactionId]: {
        sequence: command.sequence,
        sessionRevision,
        desiredOccupancy: cloneOccupancy(command.desiredOccupancy),
      },
    };
    return this.#accepted(command, false);
  }

  #claim(
    command: ClaimInitialWindowLayoutCommand,
  ): WindowLayoutCommandResult {
    if (
      command.expectedWindowRevision !== this.#snapshot.windowRevision ||
      command.expectedSessionRevision !==
        this.#acknowledgedSession.sessionRevision
    ) {
      return this.#rejected(command, "stale");
    }
    if (!this.#freshSession) {
      return this.#rejected(command, "invalid");
    }
    if (
      !isFiniteRectangle(command.targetNormalBaseBounds) ||
      !fundingIsValid(command.targetFundingByPanel) ||
      !rectanglesEqual(
        boundsForSession({
          normalBaseBounds: command.targetNormalBaseBounds,
          fundingByPanel: command.targetFundingByPanel,
        }),
        this.#snapshot.normalBounds,
      )
    ) {
      return this.#rejected(command, "invalid");
    }
    this.#acknowledgedSession = {
      normalBaseBounds: cloneRectangle(command.targetNormalBaseBounds),
      fundingByPanel: cloneFunding(command.targetFundingByPanel),
      desiredOccupancy: cloneOccupancy(command.targetDesiredOccupancy),
      windowRevision: this.#snapshot.windowRevision,
      sessionRevision: this.#acknowledgedSession.sessionRevision + 1,
    };
    this.#freshSession = false;
    return this.#accepted(command, false);
  }

  #hasCheckpointAuthority(command: WindowBoundsCommand): boolean {
    if (
      command.authority.kind === "repay-proof" &&
      command.transactionId === "hydrate-orphaned-funding"
    ) {
      return occupanciesEqual(
        command.targetDesiredOccupancy,
        this.#acknowledgedSession.desiredOccupancy,
      );
    }
    const checkpoint = this.#checkpoints[command.transactionId];
    return Boolean(
      checkpoint &&
        checkpoint.sequence === command.sequence &&
        checkpoint.sessionRevision <=
          this.#acknowledgedSession.sessionRevision &&
        occupanciesEqual(
          checkpoint.desiredOccupancy,
          command.targetDesiredOccupancy,
        ) &&
        occupanciesEqual(
          this.#acknowledgedSession.desiredOccupancy,
          command.targetDesiredOccupancy,
        ),
    );
  }

  #applyBounds(command: WindowBoundsCommand): WindowLayoutCommandResult {
    if (
      command.expectedWindowRevision !== this.#snapshot.windowRevision ||
      command.expectedSessionRevision !==
        this.#acknowledgedSession.sessionRevision
    ) {
      return this.#rejected(command, "stale");
    }
    if (!this.#hasCheckpointAuthority(command)) {
      return this.#rejected(command, "invalid");
    }
    if (
      !isFiniteRectangle(command.targetBounds) ||
      !isFiniteRectangle(command.targetNormalBaseBounds) ||
      !fundingIsValid(command.targetFundingByPanel) ||
      !validateBoundsAuthority(command, this.#acknowledgedSession)
    ) {
      return this.#rejected(command, "invalid");
    }

    const currentEnvironment = this.#host.readEnvironment();
    if (currentEnvironment.mode !== "normal") {
      this.syncExternalEnvironment("mode", false);
      return this.#rejected(command, "fixed-mode");
    }
    if (!rectangleContainedBy(command.targetBounds, currentEnvironment.workArea)) {
      this.syncExternalEnvironment("display", false);
      return this.#rejected(command, "outside-work-area");
    }
    if (
      !snapshotsDescribeSameEnvironment(this.#snapshot, currentEnvironment)
    ) {
      this.syncExternalEnvironment("panel-machine", false);
      return this.#rejected(command, "stale");
    }
    this.#host.setBounds(command.targetBounds);
    this.#setBoundsCount += 1;
    const actualEnvironment = this.#host.readEnvironment();
    if (!isFiniteRectangle(actualEnvironment.bounds)) {
      return this.#rejected(command, "invalid");
    }
    const windowRevision = this.#snapshot.windowRevision + 1;
    this.#snapshot = snapshotFromEnvironment(
      actualEnvironment,
      windowRevision,
      "panel-machine",
    );
    this.#acknowledgedSession = {
      normalBaseBounds: normalBaseForBounds(
        actualEnvironment.normalBounds,
        command.targetFundingByPanel,
      ),
      fundingByPanel: cloneFunding(command.targetFundingByPanel),
      desiredOccupancy: cloneOccupancy(command.targetDesiredOccupancy),
      windowRevision,
      sessionRevision: this.#acknowledgedSession.sessionRevision + 1,
    };
    this.#latestSequence = Math.max(this.#latestSequence, command.sequence);
    const checkpoints = { ...this.#checkpoints };
    delete checkpoints[command.transactionId];
    this.#checkpoints = checkpoints;
    const result = this.#accepted(command, true);
    this.#onSnapshot(this.#currentUpdate());
    return result;
  }

  #accepted(
    command: WindowLayoutCommand,
    setBoundsApplied: boolean,
  ): WindowLayoutCommandResult {
    return {
      accepted: true,
      commandType: command.type,
      setBoundsApplied,
      acknowledgedSession: cloneSession(this.#acknowledgedSession),
      snapshot: cloneSnapshot(this.#snapshot),
    };
  }

  #rejected(
    command: WindowLayoutCommand,
    reason: WindowLayoutCommandRejectionReason,
  ): WindowLayoutCommandResult {
    return {
      accepted: false,
      commandType: command.type,
      reason,
      acknowledgedSession: cloneSession(this.#acknowledgedSession),
      snapshot: cloneSnapshot(this.#snapshot),
    };
  }
}
