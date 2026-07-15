import type {
  DesktopThreadPinOrderSnapshot,
  DesktopThreadPinOrderSyncState,
} from "@shared/contracts";

import {
  PinnedOrderState,
  type PinnedOrderEffect,
  type PinnedOrderMembershipRequest,
  type PinnedOrderOutbox,
  type PinnedOrderPage,
  type PinnedOrderReorderFailure,
  type PinnedOrderReorderRequest,
  type PinnedOrderRequestStamp,
  type PinnedOrderUpdate,
} from "./pinned-order-state.ts";

export type PinnedOrderControllerDeps = {
  now: () => number;
  persist: (
    outbox: PinnedOrderOutbox | null,
    gatewayIdentity: string,
  ) => Promise<void>;
  sendReorder: (request: PinnedOrderReorderRequest) => Promise<PinnedOrderPage>;
  classifyFailure: (
    error: unknown,
    attempt: number,
  ) => PinnedOrderReorderFailure;
  onPublish?: (order: string[]) => void;
  onLocalMutation?: () => void;
  isCurrent?: () => boolean;
};

/**
 * Main-process effect owner for {@link PinnedOrderState}.
 *
 * Reducer mutations remain synchronous. Durable persistence completes before
 * a newly emitted PUT starts, and every transport completion is folded back
 * through the reducer's single post-acceptance drain.
 */
export class PinnedOrderController {
  readonly state: PinnedOrderState;

  private readonly deps: PinnedOrderControllerDeps;
  private persistenceTail: Promise<void> = Promise.resolve();
  private activeTransport: Promise<void> | null = null;

  constructor(state: PinnedOrderState, deps: PinnedOrderControllerDeps) {
    this.state = state;
    this.deps = deps;
  }

  snapshot(): DesktopThreadPinOrderSnapshot {
    return {
      gatewayIdentity: this.state.gatewayIdentity,
      desiredOrder: [...this.state.desiredOrder],
      highestObservedRevision: this.state.highestObservedRevision,
      unsettled: this.state.isUnsettled,
      syncState: this.syncStateLabel(),
    };
  }

  requestStamp(): PinnedOrderRequestStamp {
    return this.state.requestStamp();
  }

  async commitOrder(order: string[]): Promise<void> {
    await this.applyUpdate(this.state.commitOrder(order, this.deps.now()));
  }

  async receivePage(
    page: PinnedOrderPage,
    stamp: PinnedOrderRequestStamp,
  ): Promise<PinnedOrderUpdate> {
    const update = this.state.receivePage(page, stamp, this.deps.now());
    await this.applyUpdate(update);
    return update;
  }

  async beginMembershipChange(
    threadId: string,
    pinned: boolean,
  ): Promise<PinnedOrderMembershipRequest | null> {
    const update = this.state.beginMembershipChange(
      threadId,
      pinned,
      this.deps.now(),
    );
    await this.applyUpdate(update);
    return update.membershipRequest ?? null;
  }

  async completeMembership(
    request: PinnedOrderMembershipRequest,
    page: PinnedOrderPage,
  ): Promise<PinnedOrderUpdate> {
    const update = this.state.completeMembership(
      request,
      page,
      this.deps.now(),
    );
    await this.applyUpdate(update);
    return update;
  }

  async failMembership(
    request: PinnedOrderMembershipRequest,
  ): Promise<PinnedOrderUpdate> {
    const update = this.state.failMembership(request, this.deps.now());
    await this.applyUpdate(update);
    return update;
  }

  async retryTick(): Promise<void> {
    await this.applyUpdate(this.state.retryTick(this.deps.now()));
  }

  async resumePausedSync(): Promise<void> {
    await this.applyUpdate(this.state.resumePausedSync(this.deps.now()));
  }

  /** Waits through a CAS follow-up chain, but not through scheduled backoff. */
  async waitForTransportIdle(): Promise<void> {
    while (this.activeTransport) {
      const observed = this.activeTransport;
      await observed;
      if (this.activeTransport === observed) {
        return;
      }
    }
  }

  private async applyUpdate(update: PinnedOrderUpdate): Promise<void> {
    const sends: PinnedOrderReorderRequest[] = [];
    for (const effect of update.effects) {
      switch (effect.kind) {
        case "persist":
          await this.enqueuePersistence(effect);
          break;
        case "publish":
          this.deps.onPublish?.([...effect.order]);
          break;
        case "noteLocalMutation":
          this.deps.onLocalMutation?.();
          break;
        case "sendReorder":
          sends.push(effect.request);
          break;
      }
    }
    for (const request of sends) {
      this.startReorder(request);
    }
  }

  private enqueuePersistence(
    effect: Extract<PinnedOrderEffect, { kind: "persist" }>,
  ): Promise<void> {
    const persist = this.persistenceTail.then(() =>
      this.deps.persist(effect.outbox, effect.gatewayIdentity),
    );
    this.persistenceTail = persist.catch(() => undefined);
    return persist;
  }

  private startReorder(request: PinnedOrderReorderRequest): void {
    if (
      this.deps.isCurrent?.() === false ||
      request.stamp.gatewayIdentity !== this.state.gatewayIdentity ||
      this.state.activeReorderFlight?.token !== request.token
    ) {
      return;
    }

    const transport = (async () => {
      try {
        const page = await this.deps.sendReorder(request);
        if (this.deps.isCurrent?.() === false) {
          return;
        }
        const update = this.state.completeReorder(
          request,
          page,
          this.deps.now(),
        );
        await this.applyUpdate(update);
      } catch (error) {
        if (this.deps.isCurrent?.() === false) {
          return;
        }
        const failure = this.deps.classifyFailure(
          error,
          this.state.nextRetryAttempt,
        );
        const update = this.state.failReorder(
          request,
          failure,
          this.deps.now(),
        );
        await this.applyUpdate(update);
      }
    })();
    this.activeTransport = transport;
    void transport
      .finally(() => {
        if (this.activeTransport === transport) {
          this.activeTransport = null;
        }
      })
      .catch(() => undefined);
  }

  private syncStateLabel(): DesktopThreadPinOrderSyncState {
    switch (this.state.pendingSync.kind) {
      case "settled":
        return "settled";
      case "ready":
        return "ready";
      case "inFlight":
        return "in_flight";
      case "waitingForMembership":
        return "waiting_for_membership";
      case "coalescedBehindFlight":
        return "coalesced_behind_flight";
      case "retryScheduled":
        return "retry_scheduled";
      case "pausedPermanent":
        return "paused_permanent";
    }
  }
}

export type RemotePinsSliceResult = {
  ok: boolean;
  value: { threadIds: string[]; revision: number };
};

/**
 * The pins step of a remote-state merge (behavior seam for
 * `mergeRemoteDesktopState`): a successful fetch feeds the raw page — with
 * the stamp captured before the request was issued — through the reducer's
 * acceptance pipeline; a failed fetch only advances the retry policy. Either
 * way the reducer's presented order, never the raw remote page, is what the
 * merged DesktopState carries.
 */
export async function applyRemotePinsMergeStep(
  controller: PinnedOrderController,
  pins: RemotePinsSliceResult,
  stamp: PinnedOrderRequestStamp,
): Promise<string[]> {
  if (pins.ok) {
    await controller.receivePage(
      { threadIds: pins.value.threadIds, revision: pins.value.revision },
      stamp,
    );
  } else {
    await controller.retryTick();
  }
  return controller.state.presentedOrder;
}
