import assert from "node:assert/strict";
import test from "node:test";

import {
  acceptFavoritesReadPage,
  completeFavoritesSnapshot,
  createFavoritesIngressState,
  failFavoritesSnapshot,
  favoriteIsPresented,
  fireFavoriteBackoff,
  observeStoreIdentity,
  presentedFavoriteRows,
  presentedFavoriteThreadIds,
  replaceFavoritesGatewayScope,
  requestFavoritesSnapshot,
  settleFavoriteMutation,
  toggleFavoriteIntent,
} from "./favorites-ingress.ts";

const scope = "https://gateway.test";

function summary(id) {
  return {
    id,
    title: id,
    createdAt: "2026-07-16T00:00:00Z",
    updatedAt: "2026-07-16T00:00:00Z",
    lastMessagePreview: "",
    activitySeq: 10,
  };
}

function page(revision, ids = [], options = {}) {
  return {
    storeIncarnationId: options.incarnation ?? "inc-a",
    serverBootId: options.boot ?? "boot-a",
    revision,
    threadIds: ids,
    favorites: ids.map((id) => ({
      threadId: id,
      favoritedAt: "2026-07-16T00:00:00Z",
    })),
  };
}

function snapshot(revision, ids = [], options = {}) {
  return {
    ...page(revision, ids, options),
    recent: {
      threads: (options.rows ?? ids).map(summary),
      total: options.total ?? (options.rows ?? ids).length,
      truncated: options.truncated ?? false,
    },
  };
}

function prime(revision = 1, ids = []) {
  let transition = requestFavoritesSnapshot(
    createFavoritesIngressState(scope),
  );
  const ticket = transition.state.activeSnapshotTicket;
  assert.ok(ticket);
  transition = completeFavoritesSnapshot(
    transition.state,
    ticket,
    snapshot(revision, ids),
  );
  return transition.state;
}

function effect(transition, kind) {
  return transition.effects.find((candidate) => candidate.kind === kind);
}

test("cold identity bootstrap accepts without clearing and bare Recent is not write-ready", () => {
  let state = createFavoritesIngressState(scope);
  const observed = observeStoreIdentity(
    state,
    { gatewayScope: scope, runtimeEpoch: 0, owned: true },
    "inc-a",
  );
  state = observed.state;
  assert.equal(observed.decision, "accept");
  assert.equal(state.runtimeEpoch, 0);
  assert.equal(state.storeIncarnationId, "inc-a");
  assert.equal(state.rawRevision, null);

  const toggled = toggleFavoriteIntent(state, "thread::queued", true);
  assert.equal(favoriteIsPresented(toggled.state, "thread::queued"), true);
  assert.equal(effect(toggled, "mutate"), undefined);
  assert.equal(toggled.state.intents["thread::queued"].phase.kind, "active");
});

test("old epoch pages drop before incarnation judgment in both arrival orders", () => {
  let state = prime();
  const oldEpoch = state.runtimeEpoch;
  state = replaceFavoritesGatewayScope(state, "https://gateway-b.test", false).state;
  const afterNewScope = state;

  const oldFirst = observeStoreIdentity(
    state,
    { gatewayScope: scope, runtimeEpoch: oldEpoch, owned: true },
    "inc-a",
  );
  assert.equal(oldFirst.decision, "drop");
  assert.deepEqual(oldFirst.state, afterNewScope);

  const bootstrapped = observeStoreIdentity(
    state,
    {
      gatewayScope: "https://gateway-b.test",
      runtimeEpoch: state.runtimeEpoch,
      owned: true,
    },
    "inc-b",
  );
  const oldLast = observeStoreIdentity(
    bootstrapped.state,
    { gatewayScope: scope, runtimeEpoch: oldEpoch, owned: true },
    "inc-a",
  );
  assert.equal(oldLast.decision, "drop");
  assert.equal(oldLast.state.storeIncarnationId, "inc-b");
  assert.equal(oldLast.state.runtimeEpoch, bootstrapped.state.runtimeEpoch);
});

test("current-epoch incarnation mismatch performs one scope clear and refetch", () => {
  let state = prime(4, ["thread::a"]);
  state = toggleFavoriteIntent(state, "thread::b", true).state;
  const epoch = state.runtimeEpoch;
  const changed = observeStoreIdentity(
    state,
    { gatewayScope: scope, runtimeEpoch: epoch, owned: true },
    "inc-b",
  );
  assert.equal(changed.decision, "scopeClear");
  assert.equal(changed.state.runtimeEpoch, epoch + 1);
  assert.equal(changed.state.storeIncarnationId, null);
  assert.deepEqual(changed.state.intents, {});
  assert.deepEqual(changed.state.inFlight, {});
  assert.deepEqual(changed.state.unresolvedFence, {});
  assert.deepEqual(changed.state.rawThreadIds, []);
  assert.ok(effect(changed, "snapshot"));

  const staleAgain = observeStoreIdentity(
    changed.state,
    { gatewayScope: scope, runtimeEpoch: epoch, owned: true },
    "inc-a",
  );
  assert.equal(staleAgain.decision, "drop");
  assert.equal(staleAgain.state.runtimeEpoch, epoch + 1);
});

test("first snapshot failure retains queued intent and next success drains it", () => {
  let transition = requestFavoritesSnapshot(
    createFavoritesIngressState(scope),
  );
  const firstTicket = transition.state.activeSnapshotTicket;
  assert.ok(firstTicket);
  transition = toggleFavoriteIntent(
    transition.state,
    "thread::queued",
    true,
  );
  assert.equal(effect(transition, "mutate"), undefined);
  transition = failFavoritesSnapshot(transition.state, firstTicket);
  assert.ok(transition.state.intents["thread::queued"]);
  assert.equal(transition.state.snapshotFailure, "Favorite threads are unavailable");

  transition = requestFavoritesSnapshot(transition.state);
  assert.equal(transition.state.snapshotFailure, null);
  const secondTicket = transition.state.activeSnapshotTicket;
  assert.ok(secondTicket);
  transition = completeFavoritesSnapshot(
    transition.state,
    secondTicket,
    snapshot(7),
  );
  const mutation = effect(transition, "mutate");
  assert.ok(mutation);
  assert.equal(mutation.ticket.target, true);
  assert.equal(mutation.ticket.expectedRevision, 7);
  assert.equal(mutation.ticket.expectedStoreIncarnation, "inc-a");
  assert.equal(transition.state.snapshotFailure, null);
});

test("ok settlement retires its generation while a newer reverse intent drains", () => {
  let state = prime(2);
  let transition = toggleFavoriteIntent(state, "thread::a", true);
  const first = effect(transition, "mutate").ticket;
  transition = toggleFavoriteIntent(transition.state, "thread::a", false);
  assert.equal(Object.keys(transition.state.inFlight).length, 1);

  transition = settleFavoriteMutation(transition.state, first, {
    kind: "ok",
    page: page(3, ["thread::a"]),
  });
  const reverse = effect(transition, "mutate");
  assert.ok(reverse);
  assert.equal(reverse.ticket.target, false);
  assert.equal(reverse.ticket.expectedRevision, 3);
  assert.equal(favoriteIsPresented(transition.state, "thread::a"), false);

  transition = settleFavoriteMutation(transition.state, reverse.ticket, {
    kind: "ok",
    page: page(4),
  });
  assert.equal(transition.state.intents["thread::a"], undefined);
  assert.equal(transition.state.inFlight["thread::a"], undefined);
});

test("R11-7 keeps the orthogonal fence across toggle, notSent, and GET@E", () => {
  let state = prime(5);
  let transition = toggleFavoriteIntent(state, "thread::a", true);
  const put = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, put, {
    kind: "ambiguous",
    message: "connection lost",
  });
  assert.equal(transition.state.unresolvedFence["thread::a"], 5);

  transition = toggleFavoriteIntent(transition.state, "thread::a", false);
  const remove = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, remove, {
    kind: "notSent",
    message: "aborted before dispatch",
  });
  const retry = effect(transition, "backoff");
  assert.ok(retry);
  assert.equal(transition.state.intents["thread::a"].phase.kind, "retryScheduled");

  transition = acceptFavoritesReadPage(
    transition.state,
    { gatewayScope: scope, runtimeEpoch: transition.state.runtimeEpoch, owned: true },
    page(5),
  );
  assert.ok(transition.state.intents["thread::a"]);
  assert.equal(transition.state.unresolvedFence["thread::a"], 5);
  assert.equal(effect(transition, "mutate"), undefined);

  transition = fireFavoriteBackoff(transition.state, retry.stamp);
  const retriedDelete = effect(transition, "mutate");
  assert.ok(retriedDelete);
  assert.equal(retriedDelete.ticket.target, false);
  assert.equal(retriedDelete.ticket.expectedRevision, 5);
});

test("R11-8 raw mismatch never bypasses a scheduled retry", () => {
  let transition = toggleFavoriteIntent(prime(5), "thread::a", true);
  const first = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, first, {
    kind: "notSent",
    message: "not sent",
  });
  const timer = effect(transition, "backoff");

  transition = acceptFavoritesReadPage(
    transition.state,
    { gatewayScope: scope, runtimeEpoch: transition.state.runtimeEpoch, owned: true },
    page(6),
  );
  assert.equal(effect(transition, "mutate"), undefined);
  assert.equal(transition.state.intents["thread::a"].phase.kind, "retryScheduled");

  transition = fireFavoriteBackoff(transition.state, timer.stamp);
  assert.equal(effect(transition, "mutate").ticket.expectedRevision, 6);
});

test("a newer intent equal to raw retires after notSent but compensates ambiguity", () => {
  let transition = toggleFavoriteIntent(prime(5), "thread::a", true);
  const provablyUnsent = effect(transition, "mutate").ticket;
  transition = toggleFavoriteIntent(transition.state, "thread::a", false);
  transition = settleFavoriteMutation(transition.state, provablyUnsent, {
    kind: "notSent",
    message: "not sent",
  });
  assert.equal(transition.state.intents["thread::a"], undefined);
  assert.equal(effect(transition, "mutate"), undefined);

  transition = toggleFavoriteIntent(prime(5), "thread::a", true);
  const unknowable = effect(transition, "mutate").ticket;
  transition = toggleFavoriteIntent(transition.state, "thread::a", false);
  transition = settleFavoriteMutation(transition.state, unknowable, {
    kind: "ambiguous",
    message: "connection lost",
  });
  const compensation = effect(transition, "mutate");
  assert.ok(compensation);
  assert.equal(compensation.ticket.target, false);
  assert.equal(compensation.ticket.expectedRevision, 5);
  assert.equal(transition.state.unresolvedFence["thread::a"], 5);
});

test("conflict and 404 converge while terminal rejection resolves a newer intent against raw", () => {
  let transition = toggleFavoriteIntent(prime(1), "thread::a", true);
  const put = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, put, {
    kind: "definitiveEndpointResponse",
    status: 409,
    code: "revision_conflict",
    page: page(2, ["thread::a"]),
  });
  assert.equal(transition.state.intents["thread::a"], undefined);

  transition = toggleFavoriteIntent(transition.state, "thread::a", false);
  const remove = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, remove, {
    kind: "definitiveEndpointResponse",
    status: 404,
    code: "thread_not_found",
    page: page(3),
  });
  assert.equal(favoriteIsPresented(transition.state, "thread::a"), false);
  assert.equal(transition.state.intents["thread::a"], undefined);

  transition = toggleFavoriteIntent(transition.state, "thread::b", true);
  const rejected = effect(transition, "mutate").ticket;
  transition = toggleFavoriteIntent(transition.state, "thread::b", false);
  transition = settleFavoriteMutation(transition.state, rejected, {
    kind: "definitiveEndpointResponse",
    status: 403,
    code: "forbidden",
    message: "forbidden",
    page: null,
  });
  assert.equal(effect(transition, "surfaceError"), undefined);
  assert.equal(effect(transition, "mutate"), undefined);
  assert.equal(transition.state.intents["thread::b"], undefined);
});

test("stale backoff four-tuples and old mutation responses are inert", () => {
  let transition = toggleFavoriteIntent(prime(1), "thread::a", true);
  const first = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, first, {
    kind: "ambiguous",
    message: "lost",
  });
  const oldTimer = effect(transition, "backoff").stamp;
  transition = toggleFavoriteIntent(transition.state, "thread::a", false);
  const second = effect(transition, "mutate").ticket;

  const fired = fireFavoriteBackoff(transition.state, oldTimer);
  assert.deepEqual(fired.effects, []);
  assert.equal(fired.state.inFlight["thread::a"].requestToken, second.requestToken);

  const staleSettle = settleFavoriteMutation(fired.state, first, {
    kind: "ok",
    page: page(2, ["thread::a"]),
  });
  assert.deepEqual(staleSettle.effects, []);
  assert.deepEqual(staleSettle.state, fired.state);
});

test("wrong_incarnation settlement uses the shared three-step judgment", () => {
  let transition = toggleFavoriteIntent(prime(1), "thread::a", true);
  const ticket = effect(transition, "mutate").ticket;
  const oldEpoch = transition.state.runtimeEpoch;
  transition = settleFavoriteMutation(transition.state, ticket, {
    kind: "definitiveEndpointResponse",
    status: 409,
    code: "wrong_incarnation",
    page: page(0, [], { incarnation: "inc-b", boot: "boot-b" }),
  });
  assert.equal(transition.state.runtimeEpoch, oldEpoch + 1);
  assert.equal(transition.state.storeIncarnationId, null);
  assert.deepEqual(transition.state.intents, {});
  assert.ok(effect(transition, "snapshot"));
});

test("undecodable wrong_incarnation queues a read without retrying stale CAS", () => {
  let transition = toggleFavoriteIntent(prime(1), "thread::a", true);
  const ticket = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, ticket, {
    kind: "definitiveEndpointResponse",
    status: 409,
    code: "wrong_incarnation",
    page: null,
  });

  assert.equal(transition.state.inFlight["thread::a"], undefined);
  assert.equal(transition.state.intents["thread::a"].phase.kind, "active");
  assert.ok(effect(transition, "snapshot"));
  assert.equal(effect(transition, "backoff"), undefined);
  assert.equal(effect(transition, "mutate"), undefined);
});

test("favorites GET establishes write readiness and drains queued intent", () => {
  let transition = toggleFavoriteIntent(
    createFavoritesIngressState(scope),
    "thread::queued",
    true,
  );
  assert.equal(effect(transition, "mutate"), undefined);

  transition = acceptFavoritesReadPage(
    transition.state,
    { gatewayScope: scope, runtimeEpoch: 0, owned: true },
    page(9),
  );
  const mutation = effect(transition, "mutate");
  assert.ok(mutation);
  assert.equal(mutation.ticket.expectedRevision, 9);
  assert.equal(mutation.ticket.expectedStoreIncarnation, "inc-a");
  assert.equal(mutation.ticket.target, true);
});

test("snapshot membership and rows are atomic under the revision high-water", () => {
  let state = prime(2, ["thread::a"]);
  let transition = toggleFavoriteIntent(state, "thread::b", true);
  const mutation = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, mutation, {
    kind: "ok",
    page: page(4, ["thread::a", "thread::b"]),
  });
  state = transition.state;

  transition = requestFavoritesSnapshot(state);
  const ticket = transition.state.activeSnapshotTicket;
  transition = completeFavoritesSnapshot(
    transition.state,
    ticket,
    snapshot(3, ["thread::a"], { rows: ["thread::a"] }),
  );
  assert.deepEqual(transition.state.rawThreadIds, ["thread::a", "thread::b"]);
  assert.deepEqual(transition.state.favoriteRows.map((row) => row.id), ["thread::a"]);
  assert.ok(transition.state.activeSnapshotTicket, "low snapshot queues one replacement");
  assert.ok(effect(transition, "snapshot"));
});

test("snapshot triggers coalesce and settle produces exactly one trailing fetch", () => {
  let transition = requestFavoritesSnapshot(createFavoritesIngressState(scope));
  const ticket = transition.state.activeSnapshotTicket;
  transition = requestFavoritesSnapshot(transition.state);
  transition = requestFavoritesSnapshot(transition.state);
  assert.equal(transition.state.snapshotTrailingDirty, true);
  assert.deepEqual(transition.effects, []);

  transition = completeFavoritesSnapshot(
    transition.state,
    ticket,
    snapshot(1, ["thread::a"]),
  );
  assert.equal(transition.effects.filter((item) => item.kind === "snapshot").length, 1);
  assert.ok(transition.state.activeSnapshotTicket);
});

test("presented filtering is applied after cached snapshot rows", () => {
  let state = prime(1, ["thread::a", "thread::b"]);
  let transition = toggleFavoriteIntent(state, "thread::a", false);
  assert.deepEqual(
    presentedFavoriteRows(transition.state).map((row) => row.id),
    ["thread::b"],
  );

  const remove = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, remove, {
    kind: "definitiveEndpointResponse",
    status: 400,
    code: "invalid_request",
    page: null,
  });
  assert.deepEqual(
    presentedFavoriteRows(transition.state).map((row) => row.id),
    ["thread::a", "thread::b"],
  );
});

test("presented favorite ids lead with optimistic additions and stay capped", () => {
  let transition = toggleFavoriteIntent(
    prime(1, ["thread::a", "thread::b"]),
    "thread::new",
    true,
  );
  assert.deepEqual(presentedFavoriteThreadIds(transition.state), [
    "thread::new",
    "thread::a",
    "thread::b",
  ]);

  transition = toggleFavoriteIntent(
    transition.state,
    "thread::a",
    false,
  );
  assert.deepEqual(presentedFavoriteThreadIds(transition.state), [
    "thread::new",
    "thread::b",
  ]);

  const ids = Array.from({ length: 500 }, (_, index) => `thread::${index}`);
  transition = toggleFavoriteIntent(prime(2, ids), "thread::new", true);
  const presented = presentedFavoriteThreadIds(transition.state);
  assert.equal(presented.length, 500);
  assert.equal(presented[0], "thread::new");
  assert.equal(presented.at(-1), "thread::498");
  assert.equal(presented.includes("thread::499"), false);
});

test("presented rows keep snapshot order while shared live summaries win", () => {
  const state = prime(1, ["thread::a", "thread::snapshot-only"]);
  const rows = presentedFavoriteRows(state, [
    { ...summary("thread::a"), title: "Live title" },
    { ...summary("thread::not-favorite"), title: "Ignored" },
  ]);

  assert.deepEqual(rows.map((row) => row.id), [
    "thread::a",
    "thread::snapshot-only",
  ]);
  assert.equal(rows[0].title, "Live title");
  assert.equal(rows[1].title, "thread::snapshot-only");
});

test("retryable rejection schedules while terminal same-generation rejection surfaces", () => {
  let transition = toggleFavoriteIntent(prime(1), "thread::a", true);
  const first = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, first, {
    kind: "definitiveEndpointResponse",
    status: 429,
    code: "rate_limited",
    message: "slow down",
    page: null,
  });
  const timer = effect(transition, "backoff");
  assert.ok(timer);
  assert.equal(transition.state.intents["thread::a"].phase.kind, "retryScheduled");
  assert.equal(transition.state.intents["thread::a"].phase.cause, "rejected");

  transition = fireFavoriteBackoff(transition.state, timer.stamp);
  const retried = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, retried, {
    kind: "definitiveEndpointResponse",
    status: 403,
    code: "forbidden",
    message: "Favorite denied",
    page: null,
  });
  assert.equal(effect(transition, "surfaceError").message, "Favorite denied");
  assert.equal(transition.state.intents["thread::a"], undefined);
  assert.equal(favoriteIsPresented(transition.state, "thread::a"), false);
});

test("raw acceptance resolves awaitVerify and equal retryScheduled states", () => {
  let transition = toggleFavoriteIntent(prime(5), "thread::a", true);
  const ambiguous = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, ambiguous, {
    kind: "ambiguous",
    message: "lost",
  });
  transition = acceptFavoritesReadPage(
    transition.state,
    { gatewayScope: scope, runtimeEpoch: transition.state.runtimeEpoch, owned: true },
    page(6, ["thread::a"]),
  );
  assert.equal(transition.state.intents["thread::a"], undefined);
  assert.equal(transition.state.unresolvedFence["thread::a"], undefined);

  transition = toggleFavoriteIntent(prime(5), "thread::a", true);
  const notSent = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, notSent, {
    kind: "notSent",
    message: "not sent",
  });
  const timer = effect(transition, "backoff").stamp;
  transition = acceptFavoritesReadPage(
    transition.state,
    { gatewayScope: scope, runtimeEpoch: transition.state.runtimeEpoch, owned: true },
    page(6, ["thread::a"]),
  );
  assert.equal(transition.state.intents["thread::a"], undefined);
  assert.deepEqual(fireFavoriteBackoff(transition.state, timer).effects, []);
});

test("different ids stay isolated and every backoff stamp field fences effects", () => {
  let transition = toggleFavoriteIntent(prime(1), "thread::a", true);
  const first = effect(transition, "mutate").ticket;
  transition = toggleFavoriteIntent(transition.state, "thread::b", true);
  const second = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, first, {
    kind: "ok",
    page: page(2, ["thread::a"]),
  });
  assert.equal(
    transition.state.inFlight["thread::b"].requestToken,
    second.requestToken,
  );
  transition = settleFavoriteMutation(transition.state, second, {
    kind: "notSent",
    message: "not sent",
  });
  const timer = effect(transition, "backoff").stamp;
  const baseline = transition.state;
  for (const stamp of [
    { ...timer, gatewayScope: "https://other.test" },
    { ...timer, runtimeEpoch: timer.runtimeEpoch + 1 },
    { ...timer, generation: timer.generation + 1 },
    { ...timer, effectToken: timer.effectToken + 1 },
  ]) {
    const rejected = fireFavoriteBackoff(baseline, stamp);
    assert.deepEqual(rejected.effects, []);
    assert.deepEqual(rejected.state, baseline);
  }
  assert.ok(effect(fireFavoriteBackoff(baseline, timer), "mutate"));
});

test("same-store boot change requests replacement and external delete stays hidden", () => {
  let transition = toggleFavoriteIntent(
    prime(1, ["thread::a"]),
    "thread::b",
    true,
  );
  const put = effect(transition, "mutate").ticket;
  transition = settleFavoriteMutation(transition.state, put, {
    kind: "ok",
    page: page(2, ["thread::a", "thread::b"], { boot: "boot-b" }),
  });
  assert.ok(effect(transition, "snapshot"));
  assert.deepEqual(
    presentedFavoriteRows(transition.state).map((row) => row.id),
    ["thread::a"],
  );

  transition = acceptFavoritesReadPage(
    transition.state,
    { gatewayScope: scope, runtimeEpoch: transition.state.runtimeEpoch, owned: true },
    page(3, ["thread::b"], { boot: "boot-b" }),
  );
  assert.deepEqual(presentedFavoriteRows(transition.state), []);
  assert.equal(favoriteIsPresented(transition.state, "thread::a"), false);
});
