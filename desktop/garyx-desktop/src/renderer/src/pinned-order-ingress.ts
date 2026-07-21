import type { DesktopState } from "@shared/contracts";

export type DesktopStateDeliveryEnvelope = {
  state: DesktopState;
  capturedEpoch: number;
  rendererSessionId: string;
  gatewayIdentity: string;
  /** The gateway DOMAIN GENERATION the request was issued under. Identity
   *  (the URL) is not enough: switching A -> B -> A re-matches the URL, so
   *  a delivery from the first A connection could resurrect and overwrite
   *  the new A generation's state. Every gateway switch (and rollback)
   *  advances the generation, and acceptance requires an exact match. */
  domainGeneration: number;
};

export type DesktopStateAction =
  | DesktopState
  | null
  | ((current: DesktopState | null) => DesktopState | null);

export type PinnedOrderGatewayDomainSnapshot = {
  initialized: boolean;
  gatewayIdentity: string;
  epoch: number;
  revisionFloor: number;
  committedOrder: string[];
  unsettledDesiredOrder: string[] | null;
  unsettledBaseRevision: number;
};

const deliveryEnvelopes = new WeakMap<DesktopState, DesktopStateDeliveryEnvelope>();
let installedIngress: PinnedOrderIngress | null = null;

function normalizeIds(values: readonly string[]): string[] {
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const raw of values) {
    const id = typeof raw === "string" ? raw.trim() : "";
    if (!id || seen.has(id)) {
      continue;
    }
    seen.add(id);
    ids.push(id);
  }
  return ids;
}

function normalizeRevision(value: unknown): number {
  return Number.isSafeInteger(value) && (value as number) >= 0
    ? value as number
    : 0;
}

export function normalizeGatewayIdentity(
  value: string | null | undefined,
): string {
  return (value || "").trim().replace(/\/+$/, "").toLowerCase();
}

function stateGatewayIdentity(state: DesktopState): string {
  return normalizeGatewayIdentity(
    state.entitiesGatewayUrl || state.settings.gatewayUrl,
  );
}

function ordersEqual(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((id, index) => id === right[index]);
}

function mergeLocalOrder(
  localOrder: readonly string[],
  membershipOrder: readonly string[],
): string[] {
  const membership = new Set(membershipOrder);
  const local = normalizeIds(localOrder).filter((id) => membership.has(id));
  const localSet = new Set(local);
  const newAtHead = normalizeIds(membershipOrder).filter((id) => !localSet.has(id));
  return normalizeIds([...newAtHead, ...local]);
}

/**
 * Renderer-owned last-hop authority for DesktopState pin fields.
 *
 * Main owns the durable reducer. This smaller renderer domain exists because
 * a DesktopState can already be resolved and queued in startTransition when a
 * local drop happens. Envelopes are renderer-only and are stamped before the
 * request awaits; commitState performs the rejection inside React's functional
 * setter, at the actual commit hop.
 */
export class PinnedOrderIngress {
  readonly rendererSessionId: string;

  private initialized = false;
  private gatewayIdentity = "";
  private domainGeneration = 0;
  private epoch = 0;
  private revisionFloor = 0;
  private committedOrder: string[] = [];
  private unsettledDesiredOrder: string[] | null = null;
  private unsettledBaseRevision = 0;
  private dragBaselineOrder: string[] | null = null;

  constructor(rendererSessionId: string) {
    this.rendererSessionId = rendererSessionId;
  }

  get currentEpoch(): number {
    return this.epoch;
  }

  /** The gateway domain generation (advances on every switch/rollback):
   *  async continuations capture it to fence their non-state side effects
   *  (navigation, local caches) the same way deliveries are fenced. */
  get currentDomainGeneration(): number {
    return this.domainGeneration;
  }

  isCurrentDomainGeneration(generation: number): boolean {
    return this.domainGeneration === generation;
  }

  get highestObservedRevision(): number {
    return this.revisionFloor;
  }

  get desiredOrder(): string[] | null {
    return this.unsettledDesiredOrder
      ? [...this.unsettledDesiredOrder]
      : null;
  }

  get presentedOrder(): string[] {
    return [...(this.unsettledDesiredOrder ?? this.committedOrder)];
  }

  get dragActive(): boolean {
    return this.dragBaselineOrder !== null;
  }

  initializeFromState(state: DesktopState): void {
    if (this.initialized) {
      return;
    }
    this.initialized = true;
    this.gatewayIdentity = stateGatewayIdentity(state);
    this.revisionFloor = normalizeRevision(state.pinsRevision);
    this.committedOrder = normalizeIds(state.pinnedThreadIds);
    this.unsettledDesiredOrder = null;
  }

  beginGatewaySwitch(gatewayIdentity: string): PinnedOrderGatewayDomainSnapshot {
    const previous = this.gatewayDomainSnapshot();
    const normalized = normalizeGatewayIdentity(gatewayIdentity);
    if (!normalized) {
      // Hardening: an empty target identity is treated as no-switch instead
      // of adopting "" as the domain key (which would freeze identity checks
      // against main-side DEFAULT fallback URLs).
      return previous;
    }
    if (this.initialized && normalized === this.gatewayIdentity) {
      return previous;
    }
    this.initialized = true;
    this.gatewayIdentity = normalized;
    this.domainGeneration += 1;
    this.epoch += 1;
    this.revisionFloor = 0;
    this.committedOrder = [];
    this.unsettledDesiredOrder = null;
    this.unsettledBaseRevision = 0;
    this.dragBaselineOrder = null;
    return previous;
  }

  restoreGatewayDomain(snapshot: PinnedOrderGatewayDomainSnapshot): void {
    const invalidatingEpoch = Math.max(this.epoch, snapshot.epoch) + 1;
    this.initialized = snapshot.initialized;
    this.gatewayIdentity = snapshot.gatewayIdentity;
    // A rollback is ALSO a new generation: deliveries issued inside the
    // aborted switch window (and before it) must not be acceptable.
    this.domainGeneration += 1;
    this.epoch = invalidatingEpoch;
    this.revisionFloor = snapshot.revisionFloor;
    this.committedOrder = [...snapshot.committedOrder];
    this.unsettledDesiredOrder = snapshot.unsettledDesiredOrder
      ? [...snapshot.unsettledDesiredOrder]
      : null;
    this.unsettledBaseRevision = snapshot.unsettledBaseRevision;
    this.dragBaselineOrder = null;
  }

  beginDrag(): string[] {
    if (!this.dragBaselineOrder) {
      this.dragBaselineOrder = this.presentedOrder;
    }
    return [...this.dragBaselineOrder];
  }

  cancelDrag(): string[] {
    this.dragBaselineOrder = null;
    return this.presentedOrder;
  }

  commitDragOrder(order: readonly string[]): string[] {
    const baseline = this.dragBaselineOrder ?? this.presentedOrder;
    const membershipOrder = this.presentedOrder;
    this.dragBaselineOrder = null;
    const reduced = mergeLocalOrder(order, membershipOrder);
    if (ordersEqual(reduced, baseline)) {
      return this.presentedOrder;
    }
    // Every accepted drop advances the epoch, even if a mid-drag accepted
    // page independently reached the same order.
    this.epoch += 1;
    this.unsettledBaseRevision = this.revisionFloor;
    this.unsettledDesiredOrder = reduced;
    return [...reduced];
  }

  commitLocalOrder(order: readonly string[]): string[] {
    const normalized = normalizeIds(order);
    if (ordersEqual(normalized, this.presentedOrder)) {
      return normalized;
    }
    this.epoch += 1;
    this.unsettledBaseRevision = this.revisionFloor;
    this.unsettledDesiredOrder = normalized;
    return [...normalized];
  }

  commitLocalMembership(threadId: string, pinned: boolean): string[] {
    const id = threadId.trim();
    if (!id) {
      return this.presentedOrder;
    }
    const current = this.presentedOrder.filter((candidate) => candidate !== id);
    if (pinned) {
      current.unshift(id);
    }
    if (ordersEqual(current, this.presentedOrder)) {
      return current;
    }
    this.epoch += 1;
    this.unsettledBaseRevision = this.revisionFloor;
    this.unsettledDesiredOrder = current;
    return [...current];
  }

  rollbackLocalMembership(order: readonly string[]): string[] {
    const restored = normalizeIds(order);
    this.epoch += 1;
    this.committedOrder = restored;
    this.unsettledDesiredOrder = null;
    this.unsettledBaseRevision = this.revisionFloor;
    return [...restored];
  }

  async requestState(
    request: () => Promise<DesktopState>,
    gatewayIdentityOverride?: string,
  ): Promise<DesktopState> {
    return this.requestStateResult(
      request,
      (state) => state,
      gatewayIdentityOverride,
    );
  }

  async requestStateResult<Result>(
    request: () => Promise<Result>,
    selectState: (result: Result) => DesktopState | null,
    gatewayIdentityOverride?: string,
  ): Promise<Result> {
    // The complete stamp is captured before invoking/awaiting the request.
    const capturedEpoch = this.epoch;
    const rendererSessionId = this.rendererSessionId;
    const domainGeneration = this.domainGeneration;
    const gatewayIdentity = normalizeGatewayIdentity(
      gatewayIdentityOverride ?? this.gatewayIdentity,
    );
    const result = await request();
    const state = selectState(result);
    if (!state || typeof state !== "object") {
      // Result variants without an authoritative state (e.g. rejected
      // lifecycle mutations) carry nothing to stamp or settle.
      return result;
    }
    const envelope: DesktopStateDeliveryEnvelope = {
      state,
      capturedEpoch,
      rendererSessionId,
      gatewayIdentity,
      domainGeneration,
    };
    // Bookkeeping advances NOW, in the delivery continuation — the single
    // place ingress state may mutate. The later commitState call re-runs
    // only the PURE checks (session/identity against the instance at commit
    // time) and rebases read-only.
    this.settleDelivery(envelope);
    deliveryEnvelopes.set(state, envelope);
    return result;
  }

  /** Pure acceptance checks shared by delivery settlement (mutation gate)
   *  and the commit decision. */
  private envelopeAcceptable(
    envelope: DesktopStateDeliveryEnvelope,
  ): boolean {
    if (envelope.rendererSessionId !== this.rendererSessionId) {
      return false;
    }
    if (envelope.domainGeneration !== this.domainGeneration) {
      // Cross-generation delivery: the URL may match again (A -> B -> A),
      // but the connection it answered for is gone.
      return false;
    }
    const responseIdentity = stateGatewayIdentity(envelope.state);
    const bootstrapIdentity = !envelope.gatewayIdentity && this.epoch === 0;
    if (
      !bootstrapIdentity &&
      (envelope.gatewayIdentity !== this.gatewayIdentity ||
        responseIdentity !== this.gatewayIdentity)
    ) {
      return false;
    }
    if (bootstrapIdentity && responseIdentity !== this.gatewayIdentity) {
      return false;
    }
    return true;
  }

  /** Advance the pinned-order bookkeeping for one delivered state. Runs
   *  exactly once per delivery, in plain async code — never inside a React
   *  updater (which can be replayed or abandoned without committing). */
  private settleDelivery(envelope: DesktopStateDeliveryEnvelope): void {
    if (!this.initialized) {
      this.initializeFromState(envelope.state);
    }
    if (!this.envelopeAcceptable(envelope)) {
      return;
    }
    const revision = normalizeRevision(envelope.state.pinsRevision);
    if (
      envelope.capturedEpoch < this.epoch ||
      revision < this.revisionFloor
    ) {
      // Stale stamp: partial acceptance at commit time, no floor advance.
      return;
    }

    this.revisionFloor = Math.max(this.revisionFloor, revision);
    const incomingOrder = normalizeIds(envelope.state.pinnedThreadIds);
    if (this.unsettledDesiredOrder) {
      const merged = mergeLocalOrder(this.unsettledDesiredOrder, incomingOrder);
      this.unsettledDesiredOrder = merged;
      const acceptedAtFloor = revision >= this.unsettledBaseRevision;
      if (acceptedAtFloor && ordersEqual(incomingOrder, merged)) {
        this.committedOrder = merged;
        this.unsettledDesiredOrder = null;
        this.epoch += 1;
      }
    } else {
      this.committedOrder = incomingOrder;
    }
  }

  deliveryEnvelope(state: DesktopState): DesktopStateDeliveryEnvelope | null {
    return deliveryEnvelopes.get(state) ?? null;
  }

  /**
   * PURE commit decision for a React functional updater. React may execute,
   * replay, or abandon this without committing, so nothing here may write:
   * a stamped delivery was already settled (and its bookkeeping advanced)
   * in the delivery continuation; here its recorded disposition is applied
   * against the bookkeeping as of THIS invocation, and unstamped candidates
   * rebase read-only.
   */
  commitState(
    current: DesktopState | null,
    action: DesktopStateAction,
  ): DesktopState | null {
    const functional = typeof action === "function";
    const candidate = functional ? action(current) : action;
    if (!candidate) {
      return candidate;
    }
    const envelope = deliveryEnvelopes.get(candidate);
    if (envelope) {
      if (!this.envelopeAcceptable(envelope)) {
        return current;
      }
      return this.rebasePinnedFields(envelope.state);
    }
    if (functional) {
      // A functional updater derives from the LIVE committed state React
      // hands it, so an unstamped result is current-generation by
      // construction.
      return this.rebaseUnstampedCandidate(current, candidate);
    }
    // An unstamped DIRECT object is indistinguishable from a rebuilt stale
    // delivery (a spread strips the WeakMap identity, and after A -> B -> A
    // the URL matches again). Reject it: deliveries commit as-is, derived
    // versions go through deriveStampedDesktopState, and local mutations
    // use functional updaters.
    return current;
  }

  private rebaseUnstampedCandidate(
    current: DesktopState | null,
    candidate: DesktopState,
  ): DesktopState | null {
    if (stateGatewayIdentity(candidate) !== this.gatewayIdentity) {
      return current;
    }
    return this.rebasePinnedFields(candidate);
  }

  private rebasePinnedFields(candidate: DesktopState): DesktopState {
    const targetOrder =
      this.dragBaselineOrder ?? this.unsettledDesiredOrder ?? this.committedOrder;
    if (
      candidate.pinsRevision === this.revisionFloor &&
      ordersEqual(candidate.pinnedThreadIds ?? [], targetOrder)
    ) {
      // Preserve the reference when nothing would change so upstream
      // `current === nextState` short-circuits keep working (no extra React
      // commit per refresh).
      return candidate;
    }
    return {
      ...candidate,
      pinnedThreadIds: [...targetOrder],
      pinsRevision: this.revisionFloor,
    };
  }

  private gatewayDomainSnapshot(): PinnedOrderGatewayDomainSnapshot {
    return {
      initialized: this.initialized,
      gatewayIdentity: this.gatewayIdentity,
      epoch: this.epoch,
      revisionFloor: this.revisionFloor,
      committedOrder: [...this.committedOrder],
      unsettledDesiredOrder: this.unsettledDesiredOrder
        ? [...this.unsettledDesiredOrder]
        : null,
      unsettledBaseRevision: this.unsettledBaseRevision,
    };
  }
}

/**
 * Preserve delivery identity across a transformation. Production code that
 * needs to commit a DERIVED version of a delivered state (e.g. folding an
 * activity result into `state.automations`) must go through this helper:
 * the envelope — session, gateway identity, and domain generation — carries
 * over to the derived object, so the ingress can still judge which
 * connection the underlying answer came from. A bare spread would strip the
 * identity, and an unstamped direct commit is rejected outright.
 */
export function deriveStampedDesktopState(
  base: DesktopState,
  next: DesktopState,
): DesktopState {
  const envelope = deliveryEnvelopes.get(base);
  if (envelope) {
    deliveryEnvelopes.set(next, { ...envelope, state: next });
  }
  return next;
}

/** Module-level access to the installed ingress's domain generation, for
 *  async continuations that fence non-state side effects. */
export function currentPinnedOrderDomainGeneration(): number {
  return installedIngress?.currentDomainGeneration ?? 0;
}

export function isCurrentPinnedOrderDomainGeneration(
  generation: number,
): boolean {
  return currentPinnedOrderDomainGeneration() === generation;
}

export function installPinnedOrderIngress(ingress: PinnedOrderIngress): void {
  installedIngress = ingress;
}

/** The single renderer entry point for every async DesktopState request. */
export function requestDesktopState(
  request: () => Promise<DesktopState>,
  gatewayIdentityOverride?: string,
): Promise<DesktopState> {
  if (!installedIngress) {
    throw new Error("Pinned-order ingress is not installed");
  }
  return installedIngress.requestState(request, gatewayIdentityOverride);
}

/** Stamps a DesktopState nested inside an async IPC result before awaiting it. */
export function requestDesktopStateResult<Result>(
  request: () => Promise<Result>,
  selectState: (result: Result) => DesktopState | null,
  gatewayIdentityOverride?: string,
): Promise<Result> {
  if (!installedIngress) {
    throw new Error("Pinned-order ingress is not installed");
  }
  return installedIngress.requestStateResult(
    request,
    selectState,
    gatewayIdentityOverride,
  );
}

export function beginPinnedOrderGatewaySwitch(
  gatewayIdentity: string,
): PinnedOrderGatewayDomainSnapshot {
  if (!installedIngress) {
    throw new Error("Pinned-order ingress is not installed");
  }
  return installedIngress.beginGatewaySwitch(gatewayIdentity);
}

export function restorePinnedOrderGatewayDomain(
  snapshot: PinnedOrderGatewayDomainSnapshot,
): void {
  if (!installedIngress) {
    throw new Error("Pinned-order ingress is not installed");
  }
  installedIngress.restoreGatewayDomain(snapshot);
}
