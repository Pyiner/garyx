// Behavior tests for ensureSideChatThread's scope-generation ownership:
// the WHOLE operation (existing-binding adoption and creation alike) is
// owned by the connection generation it started on. A continuation that
// resumes after a gateway switch must not adopt, create, error, or clean
// up anything in the new scope. These run the real SideChatSessions store,
// so weakening the generation gate (e.g. `sameGeneration = () => true`, or
// capturing the generation after the first await) goes red here.

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

const storageBacking = new Map();
let createThreadImpl = async () => {
  throw new Error("createThread not stubbed");
};
globalThis.window = {
  sessionStorage: {
    getItem: (key) => (storageBacking.has(key) ? storageBacking.get(key) : null),
    setItem: (key, value) => {
      storageBacking.set(key, String(value));
    },
    removeItem: (key) => {
      storageBacking.delete(key);
    },
  },
  garyxDesktop: {
    createThread: (body) => createThreadImpl(body),
  },
};

const { SideChatSessions } = await import("./side-chat-sessions.ts");
const { ensureSideChatThread, sideChatForkAgentId } = await import(
  "./side-chat-ops.ts"
);
const { installPinnedOrderIngress } = await import("../pinned-order-ingress.ts");

// Pass-through ingress: the generation gates under test live in the ops
// layer, not in delivery ordering.
installPinnedOrderIngress({
  requestState: (request) => request(),
  requestStateResult: (request) => request(),
  beginGatewaySwitch: () => ({}),
  restoreGatewayDomain: () => {},
});

const SOURCE = "thread::source-1";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeContext(sessions, mirrorOverrides = {}) {
  const calls = {
    setDesktopState: [],
    setError: [],
    updateMessagesByThread: 0,
  };
  const ctx = {
    sessions,
    mirror: {
      ensureThreadOpenable: async () => true,
      updateMessagesByThread: () => {
        calls.updateMessagesByThread += 1;
      },
      ...mirrorOverrides,
    },
    sourceThreadId: SOURCE,
    activeThread: null,
    threadSummaryById: new Map(),
    setDesktopState: (state) => {
      calls.setDesktopState.push(state);
    },
    setError: (error) => {
      calls.setError.push(error);
    },
  };
  return { ctx, calls };
}

beforeEach(() => {
  storageBacking.clear();
  createThreadImpl = async () => {
    throw new Error("createThread not stubbed");
  };
});

test("side-chat fork preserves a canonical source binding", () => {
  assert.equal(sideChatForkAgentId({ agentId: " codex " }), "codex");
});

test("side-chat legacy fork leaves a missing source agent unspecified", () => {
  assert.equal(sideChatForkAgentId({ agentId: null }), null);
  assert.equal(sideChatForkAgentId({ agentId: "   " }), null);
  assert.equal(sideChatForkAgentId(null), null);
});

test("a stale openability answer cannot adopt a binding into the new scope", async () => {
  const sessions = new SideChatSessions();
  sessions.setGatewayScope("http://gateway-a");
  sessions.rememberThread(SOURCE, "thread::same-child");

  const openable = deferred();
  const { ctx, calls } = makeContext(sessions, {
    ensureThreadOpenable: () => openable.promise,
  });
  const pending = ensureSideChatThread(ctx);

  // Gateway switch while the openability check is in flight; B restores its
  // own binding for the SAME source (thread ids may even collide across
  // gateways — the reviewer's exact probe).
  sessions.setGatewayScope("http://gateway-b");
  sessions.rememberThread(SOURCE, "thread::b-child");

  openable.resolve(true);
  assert.equal(await pending, null, "stale adoption resolves to null");
  assert.equal(
    sessions.threadFor(SOURCE),
    "thread::b-child",
    "B's binding survives untouched",
  );
  assert.deepEqual(
    [...sessions.sideChatThreadIdsRef.current],
    ["thread::b-child"],
    "no stale id enters the routing shadow",
  );
  assert.equal(
    storageBacking.get(
      `garyx.side-tools.side-chat-thread.http://gateway-b.${SOURCE}`,
    ),
    "thread::b-child",
    "B's persisted partition is not overwritten",
  );
  assert.equal(calls.setError.length, 0);
});

test("a stale not-openable answer falls into neither forget nor creation", async () => {
  const sessions = new SideChatSessions();
  sessions.setGatewayScope("http://gateway-a");
  sessions.rememberThread(SOURCE, "thread::a-child");

  let created = 0;
  createThreadImpl = async () => {
    created += 1;
    throw new Error("must not be reached");
  };
  const openable = deferred();
  const { ctx } = makeContext(sessions, {
    ensureThreadOpenable: () => openable.promise,
  });
  const pending = ensureSideChatThread(ctx);

  sessions.setGatewayScope("http://gateway-b");
  openable.resolve(false);
  assert.equal(await pending, null);
  assert.equal(created, 0, "the stale call context never reaches creation");
});

test("a stale openability failure cannot forget or create in the new scope", async () => {
  const sessions = new SideChatSessions();
  sessions.setGatewayScope("http://gateway-a");
  sessions.rememberThread(SOURCE, "thread::a-child");

  let created = 0;
  createThreadImpl = async () => {
    created += 1;
    throw new Error("must not be reached");
  };
  const openable = deferred();
  const { ctx } = makeContext(sessions, {
    ensureThreadOpenable: () => openable.promise,
  });
  const pending = ensureSideChatThread(ctx);

  sessions.setGatewayScope("http://gateway-b");
  sessions.rememberThread(SOURCE, "thread::b-child");
  openable.reject(new Error("gateway a went away"));
  assert.equal(await pending, null);
  assert.equal(created, 0);
  assert.equal(
    sessions.threadFor(SOURCE),
    "thread::b-child",
    "the stale catch must not forget the new scope's binding",
  );
});

test("a stale creation completion leaks nothing into the new scope", async () => {
  const sessions = new SideChatSessions();
  sessions.setGatewayScope("http://gateway-a");

  const creation = deferred();
  createThreadImpl = () => creation.promise;
  const { ctx, calls } = makeContext(sessions);
  const pending = ensureSideChatThread(ctx);

  sessions.setGatewayScope("http://gateway-b");
  creation.resolve({
    state: { marker: "stale-gateway-state" },
    thread: { id: "thread::stale-created" },
  });

  assert.equal(await pending, null, "stale creation resolves to null");
  assert.equal(calls.setDesktopState.length, 0, "no stale state commit");
  assert.equal(calls.updateMessagesByThread, 0, "no stale mirror seed");
  assert.equal(sessions.threadFor(SOURCE), null, "no stale binding");
  assert.equal(
    sessions.getSnapshot().creatingBySource[SOURCE],
    undefined,
    "no stale creating flag survives in the new scope",
  );
});

test("a stale creation failure surfaces no error in the new scope", async () => {
  const sessions = new SideChatSessions();
  sessions.setGatewayScope("http://gateway-a");

  const creation = deferred();
  createThreadImpl = () => creation.promise;
  const { ctx, calls } = makeContext(sessions);
  const pending = ensureSideChatThread(ctx);

  sessions.setGatewayScope("http://gateway-b");
  creation.reject(new Error("gateway a rejected"));

  assert.equal(await pending, null);
  assert.equal(calls.setError.length, 0, "no shell error from a stale scope");
  assert.equal(
    sessions.getSnapshot().errorBySource[SOURCE],
    undefined,
    "no per-source error from a stale scope",
  );
});

test("same-scope adoption and creation still work end to end", async () => {
  const sessions = new SideChatSessions();
  sessions.setGatewayScope("http://gateway-a");

  // Creation path: binds, commits the snapshot AS-IS, returns the id.
  const createdState = { marker: "fresh-state" };
  createThreadImpl = async (body) => {
    assert.equal(body.forkFromThreadId, SOURCE);
    return { state: createdState, thread: { id: "thread::created" } };
  };
  const { ctx, calls } = makeContext(sessions);
  assert.equal(await ensureSideChatThread(ctx), "thread::created");
  assert.equal(sessions.threadFor(SOURCE), "thread::created");
  assert.equal(calls.setDesktopState[0], createdState, "state committed as-is");
  assert.equal(
    sessions.creationPromiseFor(SOURCE),
    undefined,
    "same-scope cleanup releases the in-flight de-dupe slot",
  );

  // Adoption path: an openable existing binding is re-adopted.
  const { ctx: ctx2 } = makeContext(sessions);
  assert.equal(await ensureSideChatThread(ctx2), "thread::created");
});
