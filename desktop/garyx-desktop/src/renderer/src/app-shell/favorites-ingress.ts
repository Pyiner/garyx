import type {
  DesktopThreadFavoritesPage,
  DesktopThreadFavoritesSnapshot,
  DesktopThreadSummary,
} from "@shared/contracts";

export type FavoriteIntentPhase =
  | { kind: "active" }
  | {
      kind: "retryScheduled";
      effectToken: number;
      cause: "notSent" | "rejected";
    }
  | { kind: "awaitVerify"; effectToken: number };

export interface FavoriteIntent {
  generation: number;
  desired: boolean;
  phase: FavoriteIntentPhase;
}

export interface FavoriteMutationTicket {
  gatewayScope: string;
  runtimeEpoch: number;
  requestToken: number;
  threadId: string;
  target: boolean;
  flightGeneration: number;
  expectedRevision: number;
  expectedStoreIncarnation: string;
  origin: "ordinary" | "verify";
}

export interface FavoriteBackoffStamp {
  gatewayScope: string;
  runtimeEpoch: number;
  threadId: string;
  generation: number;
  effectToken: number;
}

export interface FavoritesSnapshotTicket {
  gatewayScope: string;
  runtimeEpoch: number;
  requestToken: number;
}

export type FavoritesIngressEffect =
  | { kind: "mutate"; ticket: FavoriteMutationTicket }
  | {
      kind: "backoff";
      stamp: FavoriteBackoffStamp;
      delayMs: number;
    }
  | { kind: "snapshot"; ticket: FavoritesSnapshotTicket }
  | { kind: "surfaceError"; threadId: string; message: string };

export interface FavoritesIngressState {
  gatewayScope: string;
  runtimeEpoch: number;
  storeIncarnationId: string | null;
  rawRevision: number | null;
  rawThreadIds: string[];
  highestObservedRevision: number | null;
  intents: Record<string, FavoriteIntent>;
  inFlight: Record<string, FavoriteMutationTicket>;
  unresolvedFence: Record<string, number>;
  favoriteRows: DesktopThreadSummary[];
  favoritesServerBootId: string | null;
  favoritesSnapshotTruncated: boolean;
  activeSnapshotTicket: FavoritesSnapshotTicket | null;
  snapshotTrailingDirty: boolean;
  snapshotFailure: string | null;
  nextGeneration: number;
  nextRequestToken: number;
  nextEffectToken: number;
}

export interface FavoritesTransition {
  state: FavoritesIngressState;
  effects: FavoritesIngressEffect[];
}

export interface StoreResponseStamp {
  gatewayScope: string;
  runtimeEpoch: number;
  /** The caller has already proved the request/effect token is still owned. */
  owned: boolean;
}

export type StoreIdentityDecision = "accept" | "drop" | "scopeClear";

export interface StoreIdentityTransition extends FavoritesTransition {
  decision: StoreIdentityDecision;
}

export type FavoriteMutationSettlement =
  | { kind: "ok"; page: DesktopThreadFavoritesPage }
  | {
      kind: "definitiveEndpointResponse";
      status: number;
      code: string;
      message?: string;
      page: DesktopThreadFavoritesPage | null;
    }
  | { kind: "ambiguous"; message: string }
  | { kind: "notSent"; message: string };

const BACKOFF_MS = 750;

export function createFavoritesIngressState(
  gatewayScope = "",
): FavoritesIngressState {
  return {
    gatewayScope,
    runtimeEpoch: 0,
    storeIncarnationId: null,
    rawRevision: null,
    rawThreadIds: [],
    highestObservedRevision: null,
    intents: {},
    inFlight: {},
    unresolvedFence: {},
    favoriteRows: [],
    favoritesServerBootId: null,
    favoritesSnapshotTruncated: false,
    activeSnapshotTicket: null,
    snapshotTrailingDirty: false,
    snapshotFailure: null,
    nextGeneration: 1,
    nextRequestToken: 1,
    nextEffectToken: 1,
  };
}

export function replaceFavoritesGatewayScope(
  state: FavoritesIngressState,
  gatewayScope: string,
  requestSnapshot = true,
): FavoritesTransition {
  if (state.gatewayScope === gatewayScope) {
    return requestSnapshot
      ? requestFavoritesSnapshot(state)
      : { state, effects: [] };
  }
  const cleared = clearFavoritesDomain(state, gatewayScope);
  return requestSnapshot && gatewayScope
    ? requestFavoritesSnapshot(cleared)
    : { state: cleared, effects: [] };
}

/**
 * Shared three-step response judgment from design v24 §7.1.
 *
 * The ownership tuple is checked before incarnation. In particular, an old
 * epoch response is dropped without looking at its store id, so it cannot
 * clear the newly bootstrapped domain back to the old store.
 */
export function observeStoreIdentity(
  state: FavoritesIngressState,
  stamp: StoreResponseStamp,
  responseStoreIncarnationId: string,
): StoreIdentityTransition {
  if (
    !stamp.owned ||
    stamp.gatewayScope !== state.gatewayScope ||
    stamp.runtimeEpoch !== state.runtimeEpoch
  ) {
    return { state, effects: [], decision: "drop" };
  }
  if (state.storeIncarnationId === null) {
    return {
      state: {
        ...state,
        storeIncarnationId: responseStoreIncarnationId,
      },
      effects: [],
      decision: "accept",
    };
  }
  if (state.storeIncarnationId === responseStoreIncarnationId) {
    return { state, effects: [], decision: "accept" };
  }

  const cleared = clearFavoritesDomain(state, state.gatewayScope);
  const replacement = requestFavoritesSnapshot(cleared);
  return {
    ...replacement,
    decision: "scopeClear",
  };
}

export function favoriteIsPresented(
  state: FavoritesIngressState,
  threadId: string,
): boolean {
  const id = threadId.trim();
  const intent = state.intents[id];
  return intent?.desired ?? state.rawThreadIds.includes(id);
}

export function presentedFavoriteRows(
  state: FavoritesIngressState,
  supplementalRows: DesktopThreadSummary[] = [],
): DesktopThreadSummary[] {
  const rowsById = new Map(state.favoriteRows.map((row) => [row.id, row]));
  // Snapshot membership/order remains authoritative, while fresher shared
  // entity summaries win field-by-field as whole rows until the next snapshot.
  for (const row of supplementalRows) {
    rowsById.set(row.id, row);
  }
  return presentedFavoriteThreadIds(state).flatMap((id) => {
    const row = rowsById.get(id);
    return row ? [row] : [];
  });
}

/**
 * Snapshot row order is authoritative. Optimistic additions lead until the
 * next replacement snapshot supplies their row, while raw membership is a
 * bounded fallback for summaries already cached by the host.
 */
export function presentedFavoriteThreadIds(
  state: FavoritesIngressState,
): string[] {
  const seen = new Set<string>();
  const ids: string[] = [];
  const append = (rawId: string) => {
    const id = rawId.trim();
    if (id && !seen.has(id) && favoriteIsPresented(state, id)) {
      seen.add(id);
      ids.push(id);
    }
  };

  Object.entries(state.intents)
    .filter(([, intent]) => intent.desired)
    .sort(([, left], [, right]) => right.generation - left.generation)
    .forEach(([id]) => append(id));
  state.favoriteRows.forEach((row) => append(row.id));
  state.rawThreadIds.forEach(append);
  return ids.slice(0, 500);
}

export function toggleFavoriteIntent(
  state: FavoritesIngressState,
  threadId: string,
  desired: boolean,
): FavoritesTransition {
  const id = threadId.trim();
  if (!id) {
    return { state, effects: [] };
  }
  const generation = state.nextGeneration;
  let next: FavoritesIngressState = {
    ...state,
    nextGeneration: generation + 1,
    intents: {
      ...state.intents,
      [id]: { generation, desired, phase: { kind: "active" } },
    },
  };
  return drainFavorite(next, id, "ordinary");
}

export function requestFavoritesSnapshot(
  state: FavoritesIngressState,
): FavoritesTransition {
  if (!state.gatewayScope) {
    return { state, effects: [] };
  }
  if (state.activeSnapshotTicket) {
    return {
      state: state.snapshotTrailingDirty
        ? state
        : { ...state, snapshotTrailingDirty: true },
      effects: [],
    };
  }
  const ticket: FavoritesSnapshotTicket = {
    gatewayScope: state.gatewayScope,
    runtimeEpoch: state.runtimeEpoch,
    requestToken: state.nextRequestToken,
  };
  return {
    state: {
      ...state,
      nextRequestToken: state.nextRequestToken + 1,
      activeSnapshotTicket: ticket,
      snapshotTrailingDirty: false,
      snapshotFailure: null,
    },
    effects: [{ kind: "snapshot", ticket }],
  };
}

export function completeFavoritesSnapshot(
  state: FavoritesIngressState,
  ticket: FavoritesSnapshotTicket,
  snapshot: DesktopThreadFavoritesSnapshot,
): FavoritesTransition {
  const owned = snapshotTicketIsOwned(state, ticket);
  const identity = observeStoreIdentity(
    state,
    { ...ticket, owned },
    snapshot.storeIncarnationId,
  );
  if (identity.decision !== "accept") {
    return identity;
  }

  let next = identity.state;
  const wasTrailingDirty = next.snapshotTrailingDirty;
  next = {
    ...next,
    activeSnapshotTicket: null,
    snapshotTrailingDirty: false,
    snapshotFailure: null,
  };
  if (
    next.highestObservedRevision !== null &&
    snapshot.revision < next.highestObservedRevision
  ) {
    // Membership and rows are one atomic acceptance unit. A low page may not
    // replace either half, and always schedules one fresh snapshot.
    return requestFavoritesSnapshot({
      ...next,
      snapshotTrailingDirty: true,
    });
  }

  const accepted = acceptRawPageWithoutReconcile(next, snapshot);
  next = {
    ...accepted.state,
    favoriteRows: snapshot.recent.threads,
    favoritesServerBootId: snapshot.serverBootId,
    favoritesSnapshotTruncated: snapshot.recent.truncated,
  };
  const reconciled = reconcileAllIdleIntents(next);
  if (wasTrailingDirty) {
    const followUp = requestFavoritesSnapshot(reconciled.state);
    return {
      state: followUp.state,
      effects: [...reconciled.effects, ...followUp.effects],
    };
  }
  return reconciled;
}

export function failFavoritesSnapshot(
  state: FavoritesIngressState,
  ticket: FavoritesSnapshotTicket,
  message = "Favorite threads are unavailable",
): FavoritesTransition {
  if (!snapshotTicketIsOwned(state, ticket)) {
    return { state, effects: [] };
  }
  const trailing = state.snapshotTrailingDirty;
  const next = {
    ...state,
    activeSnapshotTicket: null,
    snapshotTrailingDirty: false,
    snapshotFailure: message,
  };
  return trailing
    ? requestFavoritesSnapshot(next)
    : { state: next, effects: [] };
}

export function acceptFavoritesReadPage(
  state: FavoritesIngressState,
  stamp: StoreResponseStamp,
  page: DesktopThreadFavoritesPage,
): FavoritesTransition {
  const identity = observeStoreIdentity(
    state,
    stamp,
    page.storeIncarnationId,
  );
  if (identity.decision !== "accept") {
    return identity;
  }
  if (
    identity.state.highestObservedRevision !== null &&
    page.revision < identity.state.highestObservedRevision
  ) {
    return { state: identity.state, effects: [] };
  }
  const bootChanged =
    identity.state.favoritesServerBootId !== null &&
    identity.state.favoritesServerBootId !== page.serverBootId;
  const accepted = acceptRawPageWithoutReconcile(identity.state, page);
  return appendFavoritesBootReplacement(
    reconcileAllIdleIntents(accepted.state),
    bootChanged,
  );
}

export function settleFavoriteMutation(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
  settlement: FavoriteMutationSettlement,
): FavoritesTransition {
  if (!mutationTicketIsOwned(state, ticket)) {
    return { state, effects: [] };
  }

  const page =
    settlement.kind === "ok"
      ? settlement.page
      : settlement.kind === "definitiveEndpointResponse"
        ? settlement.page
        : null;
  if (page) {
    const identity = observeStoreIdentity(
      state,
      { ...ticket, owned: true },
      page.storeIncarnationId,
    );
    if (identity.decision !== "accept") {
      return identity;
    }
    state = identity.state;
  }

  const bootChanged = Boolean(
    page &&
      state.favoritesServerBootId !== null &&
      state.favoritesServerBootId !== page.serverBootId,
  );
  let transition: FavoritesTransition;
  if (settlement.kind === "ok") {
    transition = settleAppliedPage(state, ticket, settlement.page);
  } else if (settlement.kind === "ambiguous") {
    transition = settleDeferred(state, ticket, "ambiguous");
  } else if (settlement.kind === "notSent") {
    transition = settleDeferred(state, ticket, "notSent");
  } else if (settlement.code === "wrong_incarnation") {
    // A valid page with a different id was handled by the three-step judgment
    // above. A same-id/malformed wrong-incarnation response is definitive that
    // this write did not apply, but cannot establish a fresh baseline.
    transition = settleWrongIncarnation(state, ticket);
  } else if (settlement.status === 409 && settlement.page) {
    transition = settleConflictPage(state, ticket, settlement.page);
  } else if (settlement.status === 404) {
    transition = settleNotFound(state, ticket, settlement.page);
  } else if (
    settlement.status === 429 ||
    settlement.code === "unavailable" ||
    settlement.code === "temporarily_unavailable"
  ) {
    transition = settleDeferred(state, ticket, "rejected");
  } else {
    transition = settleTerminalRejection(
      state,
      ticket,
      settlement.message || settlement.code || "Failed to update favorite.",
    );
  }
  return appendFavoritesBootReplacement(transition, bootChanged);
}

function settleWrongIncarnation(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
): FavoritesTransition {
  let next = withoutFlight(state, ticket.threadId);
  if (next.intents[ticket.threadId]) {
    next = withIntentPhase(next, ticket.threadId, { kind: "active" });
  }
  return requestFavoritesSnapshot(next);
}

function appendFavoritesBootReplacement(
  transition: FavoritesTransition,
  required: boolean,
): FavoritesTransition {
  if (!required) {
    return transition;
  }
  const replacement = requestFavoritesSnapshot(transition.state);
  return {
    state: replacement.state,
    effects: [...transition.effects, ...replacement.effects],
  };
}

export function fireFavoriteBackoff(
  state: FavoritesIngressState,
  stamp: FavoriteBackoffStamp,
): FavoritesTransition {
  if (
    state.gatewayScope !== stamp.gatewayScope ||
    state.runtimeEpoch !== stamp.runtimeEpoch ||
    state.inFlight[stamp.threadId]
  ) {
    return { state, effects: [] };
  }
  const intent = state.intents[stamp.threadId];
  if (
    !intent ||
    intent.generation !== stamp.generation ||
    intent.phase.kind === "active" ||
    intent.phase.effectToken !== stamp.effectToken
  ) {
    return { state, effects: [] };
  }
  return drainFavorite(
    {
      ...state,
      intents: {
        ...state.intents,
        [stamp.threadId]: {
          ...intent,
          phase: { kind: "active" },
        },
      },
    },
    stamp.threadId,
    intent.phase.kind === "awaitVerify" ? "verify" : "ordinary",
  );
}

function clearFavoritesDomain(
  state: FavoritesIngressState,
  gatewayScope: string,
): FavoritesIngressState {
  return {
    ...createFavoritesIngressState(gatewayScope),
    runtimeEpoch: state.runtimeEpoch + 1,
    // Allocators are monotonic for the lifetime of the renderer controller.
    nextGeneration: state.nextGeneration,
    nextRequestToken: state.nextRequestToken,
    nextEffectToken: state.nextEffectToken,
  };
}

function snapshotTicketIsOwned(
  state: FavoritesIngressState,
  ticket: FavoritesSnapshotTicket,
): boolean {
  const active = state.activeSnapshotTicket;
  return Boolean(
    active &&
      active.gatewayScope === ticket.gatewayScope &&
      active.runtimeEpoch === ticket.runtimeEpoch &&
      active.requestToken === ticket.requestToken,
  );
}

function mutationTicketIsOwned(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
): boolean {
  const active = state.inFlight[ticket.threadId];
  return Boolean(
    active &&
      active.gatewayScope === ticket.gatewayScope &&
      active.runtimeEpoch === ticket.runtimeEpoch &&
      active.requestToken === ticket.requestToken,
  );
}

function acceptRawPageWithoutReconcile(
  state: FavoritesIngressState,
  page: DesktopThreadFavoritesPage,
): { state: FavoritesIngressState; accepted: boolean } {
  if (
    state.highestObservedRevision !== null &&
    page.revision < state.highestObservedRevision
  ) {
    return { state, accepted: false };
  }
  const unresolvedFence = { ...state.unresolvedFence };
  for (const [id, fence] of Object.entries(unresolvedFence)) {
    if (page.revision > fence) {
      delete unresolvedFence[id];
    }
  }
  return {
    state: {
      ...state,
      rawRevision: page.revision,
      rawThreadIds: uniqueIds(page.threadIds),
      highestObservedRevision: Math.max(
        state.highestObservedRevision ?? page.revision,
        page.revision,
      ),
      unresolvedFence,
    },
    accepted: true,
  };
}

function settleAppliedPage(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
  page: DesktopThreadFavoritesPage,
): FavoritesTransition {
  const accepted = acceptRawPageWithoutReconcile(state, page);
  if (!accepted.accepted || page.revision <= ticket.expectedRevision) {
    return settleDeferred(state, ticket, "ambiguous");
  }
  let next = withoutFlight(accepted.state, ticket.threadId);
  const otherReconciled = reconcileAllIdleIntents(next, ticket.threadId);
  next = otherReconciled.state;
  const intent = next.intents[ticket.threadId];
  if (!intent || intent.generation <= ticket.flightGeneration) {
    return {
      state: withoutIntent(next, ticket.threadId),
      effects: otherReconciled.effects,
    };
  }
  const resolved = resolveCurrentIntentAfterRaw(next, ticket.threadId, true);
  return {
    state: resolved.state,
    effects: [...otherReconciled.effects, ...resolved.effects],
  };
}

function settleConflictPage(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
  page: DesktopThreadFavoritesPage,
): FavoritesTransition {
  const accepted = acceptRawPageWithoutReconcile(state, page);
  if (!accepted.accepted) {
    return settleDeferred(state, ticket, "ambiguous");
  }
  let next = withoutFlight(accepted.state, ticket.threadId);
  const otherReconciled = reconcileAllIdleIntents(next, ticket.threadId);
  next = otherReconciled.state;
  const intent = next.intents[ticket.threadId];
  if (!intent) {
    return { state: next, effects: otherReconciled.effects };
  }
  const raw = rawContains(next, ticket.threadId);
  if (intent.desired !== raw) {
    const active = withIntentPhase(next, ticket.threadId, { kind: "active" });
    const drained = drainFavorite(active, ticket.threadId, "ordinary");
    return {
      state: drained.state,
      effects: [...otherReconciled.effects, ...drained.effects],
    };
  }
  if (retirementGatePasses(next, ticket.threadId)) {
    return {
      state: withoutIntent(next, ticket.threadId),
      effects: otherReconciled.effects,
    };
  }
  const scheduled = scheduleIntent(next, ticket.threadId, "awaitVerify");
  return {
    state: scheduled.state,
    effects: [...otherReconciled.effects, ...scheduled.effects],
  };
}

function settleNotFound(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
  page: DesktopThreadFavoritesPage | null,
): FavoritesTransition {
  let next = state;
  const effects: FavoritesIngressEffect[] = [];
  if (page) {
    const accepted = acceptRawPageWithoutReconcile(next, page);
    next = accepted.state;
    const reconciled = reconcileAllIdleIntents(next, ticket.threadId);
    next = reconciled.state;
    effects.push(...reconciled.effects);
  }
  next = withoutFlight(next, ticket.threadId);
  next = withoutIntent(next, ticket.threadId);
  const fences = { ...next.unresolvedFence };
  delete fences[ticket.threadId];
  return {
    state: { ...next, unresolvedFence: fences },
    effects,
  };
}

function settleTerminalRejection(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
  message: string,
): FavoritesTransition {
  let next = withoutFlight(state, ticket.threadId);
  const intent = next.intents[ticket.threadId];
  if (!intent) {
    return { state: next, effects: [] };
  }
  if (intent.generation === ticket.flightGeneration) {
    next = withoutIntent(next, ticket.threadId);
    return {
      state: next,
      effects: [
        { kind: "surfaceError", threadId: ticket.threadId, message },
      ],
    };
  }
  next = withIntentPhase(next, ticket.threadId, { kind: "active" });
  return resolveCurrentIntentAfterRaw(next, ticket.threadId, true);
}

function settleDeferred(
  state: FavoritesIngressState,
  ticket: FavoriteMutationTicket,
  cause: "ambiguous" | "notSent" | "rejected",
): FavoritesTransition {
  let next = withoutFlight(state, ticket.threadId);
  if (cause === "ambiguous") {
    const existingFence = next.unresolvedFence[ticket.threadId];
    next = {
      ...next,
      unresolvedFence: {
        ...next.unresolvedFence,
        [ticket.threadId]:
          existingFence === undefined
            ? ticket.expectedRevision
            : Math.min(existingFence, ticket.expectedRevision),
      },
    };
  }
  const intent = next.intents[ticket.threadId];
  if (!intent) {
    return { state: next, effects: [] };
  }
  if (intent.generation !== ticket.flightGeneration) {
    next = withIntentPhase(next, ticket.threadId, { kind: "active" });
    return resolveCurrentIntentAfterRaw(next, ticket.threadId, true);
  }
  return scheduleIntent(
    next,
    ticket.threadId,
    cause === "ambiguous" ? "awaitVerify" : "retryScheduled",
    cause === "notSent" ? "notSent" : "rejected",
  );
}

function scheduleIntent(
  state: FavoritesIngressState,
  threadId: string,
  kind: "awaitVerify" | "retryScheduled",
  cause: "notSent" | "rejected" = "rejected",
): FavoritesTransition {
  const intent = state.intents[threadId];
  if (!intent) {
    return { state, effects: [] };
  }
  const effectToken = state.nextEffectToken;
  const phase: FavoriteIntentPhase =
    kind === "awaitVerify"
      ? { kind, effectToken }
      : { kind, effectToken, cause };
  const next = {
    ...state,
    nextEffectToken: effectToken + 1,
    intents: {
      ...state.intents,
      [threadId]: { ...intent, phase },
    },
  };
  return {
    state: next,
    effects: [
      {
        kind: "backoff",
        delayMs: BACKOFF_MS,
        stamp: {
          gatewayScope: next.gatewayScope,
          runtimeEpoch: next.runtimeEpoch,
          threadId,
          generation: intent.generation,
          effectToken,
        },
      },
    ],
  };
}

function reconcileAllIdleIntents(
  state: FavoritesIngressState,
  excludingThreadId?: string,
): FavoritesTransition {
  let next = state;
  const effects: FavoritesIngressEffect[] = [];
  for (const id of Object.keys(next.intents)) {
    if (id === excludingThreadId || next.inFlight[id]) {
      continue;
    }
    const resolved = resolveCurrentIntentAfterRaw(next, id, false);
    next = resolved.state;
    effects.push(...resolved.effects);
  }
  return { state: next, effects };
}

function resolveCurrentIntentAfterRaw(
  state: FavoritesIngressState,
  threadId: string,
  forceActiveDrain: boolean,
): FavoritesTransition {
  const intent = state.intents[threadId];
  if (!intent || state.rawRevision === null || state.inFlight[threadId]) {
    return { state, effects: [] };
  }
  const equal = rawContains(state, threadId) === intent.desired;
  const gatePasses = retirementGatePasses(state, threadId);
  if (equal && gatePasses) {
    return { state: withoutIntent(state, threadId), effects: [] };
  }

  if (forceActiveDrain || intent.phase.kind === "active") {
    // Equality alone is not enough when an ambiguous older flight left a
    // revision fence. In that case a compensating CAS is the observation
    // that advances the raw baseline past the unresolved write.
    return drainFavorite(
      withIntentPhase(state, threadId, { kind: "active" }),
      threadId,
      "ordinary",
    );
  }
  if (intent.phase.kind === "awaitVerify") {
    if (state.unresolvedFence[threadId] !== undefined) {
      return { state, effects: [] };
    }
    if (equal) {
      return { state: withoutIntent(state, threadId), effects: [] };
    }
    return drainFavorite(
      withIntentPhase(state, threadId, { kind: "active" }),
      threadId,
      "ordinary",
    );
  }
  // retryScheduled deliberately preserves its original timer when raw still
  // differs (R11-8); a read response may not bypass transport backoff.
  return { state, effects: [] };
}

function drainFavorite(
  state: FavoritesIngressState,
  threadId: string,
  origin: "ordinary" | "verify",
): FavoritesTransition {
  const intent = state.intents[threadId];
  if (
    !intent ||
    state.inFlight[threadId] ||
    state.rawRevision === null ||
    state.storeIncarnationId === null
  ) {
    return { state, effects: [] };
  }
  const ticket: FavoriteMutationTicket = {
    gatewayScope: state.gatewayScope,
    runtimeEpoch: state.runtimeEpoch,
    requestToken: state.nextRequestToken,
    threadId,
    target: intent.desired,
    flightGeneration: intent.generation,
    expectedRevision: state.rawRevision,
    expectedStoreIncarnation: state.storeIncarnationId,
    origin,
  };
  return {
    state: {
      ...state,
      nextRequestToken: state.nextRequestToken + 1,
      inFlight: { ...state.inFlight, [threadId]: ticket },
    },
    effects: [{ kind: "mutate", ticket }],
  };
}

function withIntentPhase(
  state: FavoritesIngressState,
  threadId: string,
  phase: FavoriteIntentPhase,
): FavoritesIngressState {
  const intent = state.intents[threadId];
  return intent
    ? {
        ...state,
        intents: {
          ...state.intents,
          [threadId]: { ...intent, phase },
        },
      }
    : state;
}

function withoutFlight(
  state: FavoritesIngressState,
  threadId: string,
): FavoritesIngressState {
  if (!state.inFlight[threadId]) {
    return state;
  }
  const inFlight = { ...state.inFlight };
  delete inFlight[threadId];
  return { ...state, inFlight };
}

function withoutIntent(
  state: FavoritesIngressState,
  threadId: string,
): FavoritesIngressState {
  if (!state.intents[threadId]) {
    return state;
  }
  const intents = { ...state.intents };
  delete intents[threadId];
  return { ...state, intents };
}

function rawContains(state: FavoritesIngressState, threadId: string): boolean {
  return state.rawThreadIds.includes(threadId);
}

function retirementGatePasses(
  state: FavoritesIngressState,
  threadId: string,
): boolean {
  const fence = state.unresolvedFence[threadId];
  return (
    fence === undefined ||
    (state.rawRevision !== null && state.rawRevision > fence)
  );
}

function uniqueIds(ids: string[]): string[] {
  const seen = new Set<string>();
  return ids.flatMap((rawId) => {
    const id = rawId.trim();
    return id && !seen.has(id) && seen.add(id) ? [id] : [];
  });
}
