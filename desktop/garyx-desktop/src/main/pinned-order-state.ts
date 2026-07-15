export type PinnedOrderPage = {
  threadIds: string[];
  revision: number;
};

export type PinnedOrderRequestStamp = {
  gatewayIdentity: string;
  epoch: number;
};

export type PinnedOrderOutbox = {
  gatewayIdentity: string;
  desiredOrder: string[];
  lastKnownRevision: number;
};

export type PinnedOrderReorderRequest = {
  token: number;
  stamp: PinnedOrderRequestStamp;
  threadIds: string[];
  expectedRevision: number;
};

export type PinnedOrderMembershipRequest = {
  token: number;
  stamp: PinnedOrderRequestStamp;
  threadId: string;
  pinned: boolean;
};

export type PinnedOrderAcceptanceOutcome =
  | "discardedBelowFloor"
  | "merged"
  | "authoritative";

export type PinnedOrderSyncState =
  | { kind: "settled" }
  | { kind: "ready" }
  | { kind: "inFlight" }
  | { kind: "waitingForMembership" }
  | { kind: "coalescedBehindFlight" }
  | { kind: "retryScheduled"; attempt: number; notBefore: number }
  | { kind: "pausedPermanent"; statusCode: number | null };

export type PinnedOrderReorderFailure =
  | { kind: "retryable"; delay: number }
  | { kind: "permanent"; statusCode: number | null }
  | { kind: "cancelled" };

export type PinnedOrderEffect =
  | { kind: "publish"; order: string[] }
  | {
      kind: "persist";
      outbox: PinnedOrderOutbox | null;
      gatewayIdentity: string;
    }
  | { kind: "sendReorder"; request: PinnedOrderReorderRequest }
  | { kind: "noteLocalMutation" };

export type PinnedOrderUpdate = {
  identityAccepted: boolean;
  acceptance?: PinnedOrderAcceptanceOutcome;
  membershipRequest?: PinnedOrderMembershipRequest;
  effects: PinnedOrderEffect[];
};

type MembershipPhase =
  | { kind: "live" }
  | { kind: "retiredPin"; completionRevision: number };

type MembershipIntent = {
  token: number;
  targetPinned: boolean;
  originallyPinned: boolean;
  rollbackOrder: string[];
  phase: MembershipPhase;
};

type DragSession = {
  baseline: string[];
  preview: string[];
  previewChanged: boolean;
  acceptedBuffer: string[] | null;
};

const EMPTY_UPDATE = (): PinnedOrderUpdate => ({
  identityAccepted: true,
  effects: [],
});

const WRONG_IDENTITY = (): PinnedOrderUpdate => ({
  identityAccepted: false,
  effects: [],
});

function normalizedRevision(value: number): number {
  return Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

function clonedOutbox(outbox: PinnedOrderOutbox | null): PinnedOrderOutbox | null {
  return outbox
    ? {
        gatewayIdentity: outbox.gatewayIdentity,
        desiredOrder: [...outbox.desiredOrder],
        lastKnownRevision: outbox.lastKnownRevision,
      }
    : null;
}

/**
 * Pure, gateway-scoped authority for pinned membership and order.
 *
 * Transport owners feed complete response events into this value. Every
 * response enforces identity -> transport completion -> revision acceptance
 * -> publication -> one drain. This is intentionally isomorphic to the iOS
 * GaryxPinnedOrderState so the two clients share the same race semantics.
 */
export class PinnedOrderState {
  gatewayIdentity: string;
  desiredOrder: string[];
  epoch = 0;
  highestObservedRevision: number;
  outbox: PinnedOrderOutbox | null;
  pendingSync: PinnedOrderSyncState;
  activeReorderFlight: PinnedOrderReorderRequest | null = null;
  wakeRequested = false;

  private resolvedOrder: string[];
  private publishedOrder: string[];
  private latestAcceptedRawOrder: string[] | null = null;
  private membershipIntents = new Map<string, MembershipIntent>();
  private dragSession: DragSession | null = null;
  private nextToken = 0;
  private retryAttempt = 0;
  private retryNotBefore: number | null = null;
  private permanentPauseStatus: number | null | undefined;

  constructor(input: {
    gatewayIdentity: string;
    initialOrder?: string[];
    revision?: number;
    restoredOutbox?: PinnedOrderOutbox | null;
  }) {
    const initial = PinnedOrderState.normalized(input.initialOrder ?? []);
    const restored = input.restoredOutbox?.gatewayIdentity === input.gatewayIdentity
      ? input.restoredOutbox
      : null;
    const floor = Math.max(
      normalizedRevision(input.revision ?? 0),
      normalizedRevision(restored?.lastKnownRevision ?? 0),
    );
    const restoredDomainOutbox = restored
      ? {
          gatewayIdentity: input.gatewayIdentity,
          desiredOrder: PinnedOrderState.normalized(restored.desiredOrder),
          lastKnownRevision: floor,
        }
      : null;

    this.gatewayIdentity = input.gatewayIdentity;
    this.highestObservedRevision = floor;
    this.outbox = restoredDomainOutbox;
    this.desiredOrder = [...(restoredDomainOutbox?.desiredOrder ?? initial)];
    this.resolvedOrder = [...this.desiredOrder];
    this.publishedOrder = [...this.desiredOrder];
    this.pendingSync = restoredDomainOutbox ? { kind: "ready" } : { kind: "settled" };
  }

  get presentedOrder(): string[] {
    return [...(this.dragSession?.preview ?? this.resolvedOrder)];
  }

  get isDragging(): boolean {
    return this.dragSession !== null;
  }

  get isUnsettled(): boolean {
    return this.outbox !== null;
  }

  get hasPendingSync(): boolean {
    return this.outbox !== null;
  }

  get nextRetryAttempt(): number {
    return this.retryAttempt + 1;
  }

  get liveMembershipIntentCount(): number {
    let count = 0;
    for (const intent of this.membershipIntents.values()) {
      if (intent.phase.kind === "live") {
        count += 1;
      }
    }
    return count;
  }

  requestStamp(): PinnedOrderRequestStamp {
    return {
      gatewayIdentity: this.gatewayIdentity,
      epoch: this.epoch,
    };
  }

  beginDrag(): PinnedOrderUpdate {
    if (this.dragSession) {
      return EMPTY_UPDATE();
    }
    const baseline = this.presentedOrder;
    this.dragSession = {
      baseline,
      preview: [...baseline],
      previewChanged: false,
      acceptedBuffer: null,
    };
    return EMPTY_UPDATE();
  }

  previewDrag(order: string[]): PinnedOrderUpdate {
    if (!this.dragSession) {
      return EMPTY_UPDATE();
    }
    const preview = PinnedOrderState.overlay(order, this.resolvedOrder);
    this.dragSession.preview = preview;
    this.dragSession.previewChanged =
      this.dragSession.previewChanged ||
      !PinnedOrderState.ordersEqual(preview, this.dragSession.baseline);
    return EMPTY_UPDATE();
  }

  acceptDrop(now = 0): PinnedOrderUpdate {
    if (!this.dragSession) {
      return EMPTY_UPDATE();
    }
    if (!this.dragSession.previewChanged) {
      return this.cancelDrag();
    }

    const effects: PinnedOrderEffect[] = [];
    const committed = PinnedOrderState.overlay(
      this.dragSession.preview,
      this.resolvedOrder,
    );
    this.dragSession = null;
    this.resolvedOrder = committed;
    this.desiredOrder = [...committed];
    this.epoch += 1;
    effects.push({ kind: "noteLocalMutation" });
    this.outbox = this.makeOutbox();
    effects.push({
      kind: "persist",
      outbox: clonedOutbox(this.outbox),
      gatewayIdentity: this.gatewayIdentity,
    });
    this.wakeRequested = true;
    this.appendPublicationIfChanged(effects);
    this.drain(now, effects);
    return { identityAccepted: true, effects };
  }

  /** Main-process entry point for an already accepted dnd-kit drop. */
  commitOrder(order: string[], now = 0): PinnedOrderUpdate {
    const committed = PinnedOrderState.overlay(order, this.resolvedOrder);
    if (PinnedOrderState.ordersEqual(committed, this.presentedOrder)) {
      return EMPTY_UPDATE();
    }
    this.dragSession = {
      baseline: this.presentedOrder,
      preview: committed,
      previewChanged: true,
      acceptedBuffer: null,
    };
    return this.acceptDrop(now);
  }

  cancelDrag(): PinnedOrderUpdate {
    if (!this.dragSession) {
      return EMPTY_UPDATE();
    }
    this.dragSession = null;
    const effects: PinnedOrderEffect[] = [];
    this.appendPublicationIfChanged(effects);
    return { identityAccepted: true, effects };
  }

  beginMembershipChange(
    rawThreadId: string,
    pinned: boolean,
    now = 0,
  ): PinnedOrderUpdate {
    const threadId = PinnedOrderState.normalizedId(rawThreadId);
    if (
      !threadId ||
      this.membershipIntents.has(threadId) ||
      this.presentedOrder.includes(threadId) === pinned
    ) {
      return EMPTY_UPDATE();
    }

    let rollbackOrder = this.desiredOrder;
    for (const intent of this.membershipIntents.values()) {
      if (intent.phase.kind === "live") {
        rollbackOrder = intent.rollbackOrder;
        break;
      }
    }
    this.nextToken += 1;
    const request: PinnedOrderMembershipRequest = {
      token: this.nextToken,
      stamp: this.requestStamp(),
      threadId,
      pinned,
    };
    this.membershipIntents.set(threadId, {
      token: request.token,
      targetPinned: pinned,
      originallyPinned: this.desiredOrder.includes(threadId),
      rollbackOrder: [...rollbackOrder],
      phase: { kind: "live" },
    });

    const next = this.desiredOrder.filter((id) => id !== threadId);
    if (pinned) {
      next.unshift(threadId);
    }
    this.resolvedOrder = next;
    this.desiredOrder = [...next];
    this.epoch += 1;

    const effects: PinnedOrderEffect[] = [{ kind: "noteLocalMutation" }];
    if (this.outbox) {
      this.outbox = this.makeOutbox();
      effects.push({
        kind: "persist",
        outbox: clonedOutbox(this.outbox),
        gatewayIdentity: this.gatewayIdentity,
      });
      this.wakeRequested = true;
    }
    this.appendPublicationIfChanged(effects);
    this.drain(now, effects);
    return {
      identityAccepted: true,
      membershipRequest: request,
      effects,
    };
  }

  receivePage(
    rawPage: PinnedOrderPage,
    stamp: PinnedOrderRequestStamp,
    now = 0,
  ): PinnedOrderUpdate {
    if (stamp.gatewayIdentity !== this.gatewayIdentity) {
      return WRONG_IDENTITY();
    }
    const page = PinnedOrderState.page(rawPage);
    const effects: PinnedOrderEffect[] = [];
    const acceptance = this.acceptPage(page, stamp, effects);
    if (acceptance !== "discardedBelowFloor" && this.outbox) {
      this.wakeRequested = true;
    }
    this.drain(now, effects);
    return { identityAccepted: true, acceptance, effects };
  }

  completeMembership(
    request: PinnedOrderMembershipRequest,
    rawPage: PinnedOrderPage,
    now = 0,
  ): PinnedOrderUpdate {
    const intent = this.membershipIntents.get(request.threadId);
    if (
      request.stamp.gatewayIdentity !== this.gatewayIdentity ||
      !intent ||
      intent.token !== request.token ||
      intent.phase.kind !== "live"
    ) {
      return WRONG_IDENTITY();
    }
    const page = PinnedOrderState.page(rawPage);

    // Pipeline step 2 resolves transport only. Dispatch waits until this
    // response has completed revision acceptance below.
    if (intent.targetPinned) {
      intent.phase = {
        kind: "retiredPin",
        completionRevision: page.revision,
      };
      this.membershipIntents.set(request.threadId, intent);
    } else {
      this.membershipIntents.delete(request.threadId);
    }
    this.epoch += 1;
    if (this.outbox) {
      this.wakeRequested = true;
    }

    const effects: PinnedOrderEffect[] = [];
    const acceptance = this.acceptPage(page, request.stamp, effects);
    const retiredIntentRemoved = this.cleanupRetiredPinIntents();
    if (
      acceptance === "discardedBelowFloor" &&
      (!intent.targetPinned || retiredIntentRemoved) &&
      this.latestAcceptedRawOrder
    ) {
      const merged = PinnedOrderState.mergeLocalOrder(
        this.desiredOrder,
        this.membershipOrder(this.latestAcceptedRawOrder),
      );
      this.desiredOrder = merged;
      this.resolvedOrder = [...merged];
      this.appendPublicationIfChanged(effects);
    }
    if (this.outbox) {
      this.outbox = this.makeOutbox();
      effects.push({
        kind: "persist",
        outbox: clonedOutbox(this.outbox),
        gatewayIdentity: this.gatewayIdentity,
      });
      this.wakeRequested = true;
    }
    this.drain(now, effects);
    return { identityAccepted: true, acceptance, effects };
  }

  failMembership(
    request: PinnedOrderMembershipRequest,
    now = 0,
  ): PinnedOrderUpdate {
    const intent = this.membershipIntents.get(request.threadId);
    if (
      request.stamp.gatewayIdentity !== this.gatewayIdentity ||
      !intent ||
      intent.token !== request.token ||
      intent.phase.kind !== "live"
    ) {
      return WRONG_IDENTITY();
    }
    this.membershipIntents.delete(request.threadId);

    if (intent.originallyPinned) {
      this.desiredOrder = PinnedOrderState.restoring(
        request.threadId,
        intent.rollbackOrder,
        this.desiredOrder,
      );
    } else {
      this.desiredOrder = this.desiredOrder.filter((id) => id !== request.threadId);
    }
    this.resolvedOrder = [...this.desiredOrder];
    this.epoch += 1;

    const effects: PinnedOrderEffect[] = [{ kind: "noteLocalMutation" }];
    if (this.outbox) {
      this.outbox = this.makeOutbox();
      effects.push({
        kind: "persist",
        outbox: clonedOutbox(this.outbox),
        gatewayIdentity: this.gatewayIdentity,
      });
      this.wakeRequested = true;
    }
    this.appendPublicationIfChanged(effects);
    this.drain(now, effects);
    return { identityAccepted: true, effects };
  }

  completeReorder(
    request: PinnedOrderReorderRequest,
    rawPage: PinnedOrderPage,
    now = 0,
  ): PinnedOrderUpdate {
    if (request.stamp.gatewayIdentity !== this.gatewayIdentity) {
      return WRONG_IDENTITY();
    }
    if (this.activeReorderFlight?.token !== request.token) {
      return EMPTY_UPDATE();
    }
    const page = PinnedOrderState.page(rawPage);

    // Pipeline step 2 only closes this transport token. If another accepted
    // response already settled/cleared the outbox, this completion cannot
    // revive it.
    this.activeReorderFlight = null;
    if (this.outbox) {
      this.wakeRequested = true;
    }
    const effects: PinnedOrderEffect[] = [];
    const acceptance = this.acceptPage(page, request.stamp, effects);
    if (this.outbox) {
      this.wakeRequested = true;
    }
    this.drain(now, effects);
    return { identityAccepted: true, acceptance, effects };
  }

  failReorder(
    request: PinnedOrderReorderRequest,
    failure: PinnedOrderReorderFailure,
    now = 0,
  ): PinnedOrderUpdate {
    if (request.stamp.gatewayIdentity !== this.gatewayIdentity) {
      return WRONG_IDENTITY();
    }
    if (this.activeReorderFlight?.token !== request.token) {
      return EMPTY_UPDATE();
    }
    this.activeReorderFlight = null;
    if (!this.outbox) {
      this.pendingSync = { kind: "settled" };
      return EMPTY_UPDATE();
    }

    if (failure.kind === "retryable") {
      this.retryAttempt += 1;
      this.retryNotBefore = now + Math.max(0, failure.delay);
      this.wakeRequested = false;
      this.pendingSync = {
        kind: "retryScheduled",
        attempt: this.retryAttempt,
        notBefore: this.retryNotBefore,
      };
      return EMPTY_UPDATE();
    }
    if (failure.kind === "permanent") {
      this.permanentPauseStatus = failure.statusCode;
      this.wakeRequested = false;
      this.pendingSync = {
        kind: "pausedPermanent",
        statusCode: failure.statusCode,
      };
      return EMPTY_UPDATE();
    }

    this.wakeRequested = true;
    const effects: PinnedOrderEffect[] = [];
    this.drain(now, effects);
    return { identityAccepted: true, effects };
  }

  retryTick(now: number): PinnedOrderUpdate {
    if (!this.outbox || this.permanentPauseStatus !== undefined) {
      return EMPTY_UPDATE();
    }
    if (this.retryNotBefore !== null && now < this.retryNotBefore) {
      return EMPTY_UPDATE();
    }
    this.retryNotBefore = null;
    this.wakeRequested = true;
    const effects: PinnedOrderEffect[] = [];
    this.drain(now, effects);
    return { identityAccepted: true, effects };
  }

  resumePausedSync(now = 0): PinnedOrderUpdate {
    if (!this.outbox) {
      return EMPTY_UPDATE();
    }
    this.permanentPauseStatus = undefined;
    this.retryNotBefore = null;
    this.retryAttempt = 0;
    this.wakeRequested = true;
    const effects: PinnedOrderEffect[] = [];
    this.drain(now, effects);
    return { identityAccepted: true, effects };
  }

  switchGateway(
    newIdentity: string,
    restoredOutbox: PinnedOrderOutbox | null = null,
  ): PinnedOrderUpdate {
    const oldIdentity = this.gatewayIdentity;
    const replacement = new PinnedOrderState({
      gatewayIdentity: newIdentity,
      restoredOutbox,
    });
    this.replaceWith(replacement);
    return {
      identityAccepted: true,
      effects: [
        {
          kind: "persist",
          outbox: null,
          gatewayIdentity: oldIdentity,
        },
      ],
    };
  }

  reloadCurrentGateway(input?: {
    initialOrder?: string[];
    revision?: number;
    restoredOutbox?: PinnedOrderOutbox | null;
  }): PinnedOrderUpdate {
    this.replaceWith(
      new PinnedOrderState({
        gatewayIdentity: this.gatewayIdentity,
        initialOrder: input?.initialOrder,
        revision: input?.revision,
        restoredOutbox: input?.restoredOutbox,
      }),
    );
    return EMPTY_UPDATE();
  }

  private acceptPage(
    page: PinnedOrderPage,
    stamp: PinnedOrderRequestStamp,
    effects: PinnedOrderEffect[],
  ): PinnedOrderAcceptanceOutcome {
    if (page.revision < this.highestObservedRevision) {
      return "discardedBelowFloor";
    }

    this.highestObservedRevision = page.revision;
    this.latestAcceptedRawOrder = [...page.threadIds];
    this.cleanupRetiredPinIntents();

    if (this.outbox && this.desiredOrder.length === 0 && this.liveMembershipIntentCount === 0) {
      // A projected-empty outbox may survive a process death while its
      // membership requests do not. With no live intent after restore, it is
      // safe to clear before fetched membership can revive the order debt.
      this.settleOutbox(effects);
    } else if (
      this.outbox &&
      PinnedOrderState.ordersEqual(this.outbox.desiredOrder, page.threadIds)
    ) {
      this.resolvedOrder = [...this.outbox.desiredOrder];
      this.desiredOrder = [...this.outbox.desiredOrder];
      this.settleOutbox(effects);
    }

    const needsMerge =
      stamp.epoch < this.epoch ||
      this.outbox !== null ||
      this.membershipIntents.size > 0;
    let outcome: PinnedOrderAcceptanceOutcome;
    if (needsMerge) {
      const merged = PinnedOrderState.mergeLocalOrder(
        this.desiredOrder,
        this.membershipOrder(page.threadIds),
      );
      this.resolvedOrder = merged;
      this.desiredOrder = [...merged];
      if (this.outbox) {
        this.outbox = this.makeOutbox();
        effects.push({
          kind: "persist",
          outbox: clonedOutbox(this.outbox),
          gatewayIdentity: this.gatewayIdentity,
        });
      }
      outcome = "merged";
    } else {
      this.resolvedOrder = [...page.threadIds];
      this.desiredOrder = [...page.threadIds];
      outcome = "authoritative";
    }

    if (this.dragSession) {
      this.dragSession.acceptedBuffer = [...this.resolvedOrder];
    } else {
      this.appendPublicationIfChanged(effects);
    }
    return outcome;
  }

  private drain(now: number, effects: PinnedOrderEffect[]): void {
    if (!this.outbox) {
      this.wakeRequested = false;
      this.pendingSync = { kind: "settled" };
      return;
    }
    if (!this.wakeRequested) {
      this.refreshPendingState(now);
      return;
    }
    if (this.activeReorderFlight) {
      this.pendingSync = { kind: "coalescedBehindFlight" };
      return;
    }
    if (this.liveMembershipIntentCount > 0) {
      if (
        this.desiredOrder.length === 0 &&
        this.latestAcceptedRawOrder?.length === 0
      ) {
        this.clearOutbox(effects);
        return;
      }
      this.pendingSync = { kind: "waitingForMembership" };
      this.wakeRequested = false;
      return;
    }
    if (this.desiredOrder.length === 0) {
      this.clearOutbox(effects);
      return;
    }
    if (
      this.latestAcceptedRawOrder &&
      PinnedOrderState.ordersEqual(this.latestAcceptedRawOrder, this.desiredOrder)
    ) {
      this.settleOutbox(effects);
      return;
    }
    if (this.permanentPauseStatus !== undefined) {
      this.pendingSync = {
        kind: "pausedPermanent",
        statusCode: this.permanentPauseStatus,
      };
      this.wakeRequested = false;
      return;
    }
    if (this.retryNotBefore !== null && now < this.retryNotBefore) {
      this.pendingSync = {
        kind: "retryScheduled",
        attempt: this.retryAttempt,
        notBefore: this.retryNotBefore,
      };
      this.wakeRequested = false;
      return;
    }

    this.nextToken += 1;
    const request: PinnedOrderReorderRequest = {
      token: this.nextToken,
      stamp: this.requestStamp(),
      threadIds: [...this.desiredOrder],
      expectedRevision: this.highestObservedRevision,
    };
    this.activeReorderFlight = request;
    this.wakeRequested = false;
    this.pendingSync = { kind: "inFlight" };
    effects.push({ kind: "sendReorder", request });
  }

  private refreshPendingState(now: number): void {
    if (this.permanentPauseStatus !== undefined) {
      this.pendingSync = {
        kind: "pausedPermanent",
        statusCode: this.permanentPauseStatus,
      };
    } else if (this.retryNotBefore !== null && now < this.retryNotBefore) {
      this.pendingSync = {
        kind: "retryScheduled",
        attempt: this.retryAttempt,
        notBefore: this.retryNotBefore,
      };
    } else if (this.activeReorderFlight) {
      this.pendingSync = { kind: "inFlight" };
    } else if (this.liveMembershipIntentCount > 0) {
      this.pendingSync = { kind: "waitingForMembership" };
    } else {
      this.pendingSync = { kind: "ready" };
    }
  }

  private settleOutbox(effects: PinnedOrderEffect[]): void {
    if (!this.outbox) {
      return;
    }
    this.outbox = null;
    this.retryAttempt = 0;
    this.retryNotBefore = null;
    this.permanentPauseStatus = undefined;
    this.wakeRequested = false;
    this.epoch += 1;
    this.pendingSync = { kind: "settled" };
    effects.push({
      kind: "persist",
      outbox: null,
      gatewayIdentity: this.gatewayIdentity,
    });
  }

  private clearOutbox(effects: PinnedOrderEffect[]): void {
    this.settleOutbox(effects);
  }

  private makeOutbox(): PinnedOrderOutbox {
    return {
      gatewayIdentity: this.gatewayIdentity,
      desiredOrder: [...this.desiredOrder],
      lastKnownRevision: this.highestObservedRevision,
    };
  }

  private appendPublicationIfChanged(effects: PinnedOrderEffect[]): void {
    const order = this.presentedOrder;
    if (PinnedOrderState.ordersEqual(this.publishedOrder, order)) {
      return;
    }
    this.publishedOrder = [...order];
    effects.push({ kind: "publish", order });
  }

  private cleanupRetiredPinIntents(): boolean {
    const before = this.membershipIntents.size;
    for (const [threadId, intent] of this.membershipIntents) {
      if (
        intent.phase.kind === "retiredPin" &&
        this.highestObservedRevision >= intent.phase.completionRevision
      ) {
        this.membershipIntents.delete(threadId);
      }
    }
    return this.membershipIntents.size !== before;
  }

  private membershipOrder(rawOrder: string[]): string[] {
    const membership = [...rawOrder];
    const intents = [...this.membershipIntents.entries()].sort(
      (left, right) => left[1].token - right[1].token,
    );
    for (const [threadId, intent] of intents) {
      if (
        (intent.phase.kind === "live" && intent.targetPinned) ||
        intent.phase.kind === "retiredPin"
      ) {
        if (!membership.includes(threadId)) {
          membership.push(threadId);
        }
      } else {
        const index = membership.indexOf(threadId);
        if (index >= 0) {
          membership.splice(index, 1);
        }
      }
    }
    return PinnedOrderState.normalized(membership);
  }

  private replaceWith(next: PinnedOrderState): void {
    this.gatewayIdentity = next.gatewayIdentity;
    this.desiredOrder = next.desiredOrder;
    this.epoch = next.epoch;
    this.highestObservedRevision = next.highestObservedRevision;
    this.outbox = next.outbox;
    this.pendingSync = next.pendingSync;
    this.activeReorderFlight = next.activeReorderFlight;
    this.wakeRequested = next.wakeRequested;
    this.resolvedOrder = next.resolvedOrder;
    this.publishedOrder = next.publishedOrder;
    this.latestAcceptedRawOrder = next.latestAcceptedRawOrder;
    this.membershipIntents = next.membershipIntents;
    this.dragSession = next.dragSession;
    this.nextToken = next.nextToken;
    this.retryAttempt = next.retryAttempt;
    this.retryNotBefore = next.retryNotBefore;
    this.permanentPauseStatus = next.permanentPauseStatus;
  }

  static normalized(values: string[]): string[] {
    const seen = new Set<string>();
    const result: string[] = [];
    for (const raw of values) {
      const id = PinnedOrderState.normalizedId(raw);
      if (!id || seen.has(id)) {
        continue;
      }
      seen.add(id);
      result.push(id);
    }
    return result;
  }

  private static normalizedId(raw: string): string | null {
    const id = typeof raw === "string" ? raw.trim() : "";
    return id || null;
  }

  private static page(raw: PinnedOrderPage): PinnedOrderPage {
    return {
      threadIds: PinnedOrderState.normalized(raw.threadIds),
      revision: normalizedRevision(raw.revision),
    };
  }

  private static ordersEqual(left: string[], right: string[]): boolean {
    return left.length === right.length && left.every((id, index) => id === right[index]);
  }

  private static mergeLocalOrder(
    localOrder: string[],
    membershipOrder: string[],
  ): string[] {
    const membership = new Set(membershipOrder);
    const local = PinnedOrderState.normalized(localOrder).filter((id) => membership.has(id));
    const localSet = new Set(local);
    const newAtHead = membershipOrder.filter((id) => !localSet.has(id));
    return PinnedOrderState.normalized([...newAtHead, ...local]);
  }

  private static overlay(order: string[], membershipOrder: string[]): string[] {
    return PinnedOrderState.mergeLocalOrder(order, membershipOrder);
  }

  private static restoring(
    threadId: string,
    baselineOrder: string[],
    currentOrder: string[],
  ): string[] {
    const result = PinnedOrderState.normalized(currentOrder).filter((id) => id !== threadId);
    const baseline = PinnedOrderState.normalized(baselineOrder);
    const originalIndex = baseline.indexOf(threadId);
    if (originalIndex < 0) {
      result.unshift(threadId);
      return result;
    }
    for (let index = originalIndex - 1; index >= 0; index -= 1) {
      const predecessorIndex = result.indexOf(baseline[index]);
      if (predecessorIndex >= 0) {
        result.splice(predecessorIndex + 1, 0, threadId);
        return result;
      }
    }
    for (let index = originalIndex + 1; index < baseline.length; index += 1) {
      const successorIndex = result.indexOf(baseline[index]);
      if (successorIndex >= 0) {
        result.splice(successorIndex, 0, threadId);
        return result;
      }
    }
    result.splice(Math.min(originalIndex, result.length), 0, threadId);
    return result;
  }
}
