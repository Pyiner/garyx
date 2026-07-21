import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";

import * as esbuild from "esbuild";

const bundled = await esbuild.build({
  entryPoints: ["src/renderer/src/platform/desktop-api.ts"],
  bundle: true,
  format: "esm",
  platform: "node",
  write: false,
});
const desktopApiModule = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString("base64")}`
);
const { createDesktopApiFacade, getDesktopApi } = desktopApiModule;

test("frozen Electron bridge is materialized as a callable facade", async () => {
  const calls = [];
  const rawApi = Object.freeze({
    horizontalLayoutPolicy: "expand-v1",
    async listTasks(input) {
      calls.push({ input, receiver: this });
      return { tasks: [{ taskId: "task-1" }], nextCursor: null };
    },
  });
  globalThis.window = { garyxDesktop: rawApi };

  const api = getDesktopApi();
  const listTasks = api.listTasks;
  const page = await api.listTasks({ limit: 1 });

  assert.notStrictEqual(api, rawApi);
  assert.strictEqual(api, getDesktopApi());
  assert.strictEqual(api.listTasks, listTasks);
  assert.equal(Object.isFrozen(api), true);
  assert.equal(api.horizontalLayoutPolicy, "expand-v1");
  assert.deepEqual(page, {
    tasks: [{ taskId: "task-1" }],
    nextCursor: null,
  });
  assert.deepEqual(calls, [{ input: { limit: 1 }, receiver: rawApi }]);
});

test("facade routes only DesktopState methods through pinned-order ingress", async () => {
  const state = { pinnedThreadIds: ["thread-1"], pinsRevision: 4 };
  const calls = [];
  const ingressCalls = [];
  const rawApi = Object.freeze({
    async getState() {
      calls.push({ method: "getState", receiver: this });
      return state;
    },
    async createThread() {
      calls.push({ method: "createThread", receiver: this });
      return { state, threadId: "thread-1" };
    },
    async listTasks() {
      calls.push({ method: "listTasks", receiver: this });
      return { tasks: [], nextCursor: null };
    },
  });
  const facade = createDesktopApiFacade(rawApi, {
    async requestState(request) {
      ingressCalls.push("state");
      return request();
    },
    async requestStateResult(request, selectState) {
      ingressCalls.push("state-result");
      const result = await request();
      assert.strictEqual(selectState(result), state);
      return result;
    },
  });

  assert.strictEqual(await facade.getState(), state);
  assert.deepEqual(await facade.createThread(), {
    state,
    threadId: "thread-1",
  });
  assert.deepEqual(await facade.listTasks(), {
    tasks: [],
    nextCursor: null,
  });
  assert.deepEqual(ingressCalls, ["state", "state-result"]);
  assert.deepEqual(calls, [
    { method: "getState", receiver: rawApi },
    { method: "createThread", receiver: rawApi },
    { method: "listTasks", receiver: rawApi },
  ]);
});

test("lifecycle mutations stamp their NESTED authoritative state", async () => {
  const appliedState = { pinnedThreadIds: [], pinsRevision: 7 };
  const rawApi = Object.freeze({
    async archiveThread() {
      return { kind: "ok", value: appliedState, status: 200 };
    },
    async deleteThread() {
      return { kind: "ambiguous", message: "gateway restarted mid-flight" };
    },
    async archiveDefinitive() {
      return {
        kind: "definitiveEndpointResponse",
        status: 409,
        error: { tag: "conflict" },
        value: appliedState,
        body: "{}",
      };
    },
    async archiveNotSent() {
      return { kind: "notSent", message: "no connection" };
    },
  });
  const selections = [];
  const facade = createDesktopApiFacade(rawApi, {
    async requestState() {
      throw new Error(
        "a mutation RESULT must never ride the plain-state lane: stamping " +
          "the wrapper crashes identity reads and skips the nested state",
      );
    },
    async requestStateResult(request, selectState) {
      const result = await request();
      selections.push(selectState(result));
      return result;
    },
  });

  const archived = await facade.archiveThread({ threadId: "thread-1" });
  assert.equal(archived.kind, "ok");
  assert.strictEqual(archived.value, appliedState);
  const deleted = await facade.deleteThread({ threadId: "thread-1" });
  assert.equal(deleted.kind, "ambiguous");
  // All four REAL wire discriminants: "ok" and a value-carrying
  // "definitiveEndpointResponse" select their nested state; "ambiguous"
  // and "notSent" select null (nothing to stamp).
  assert.deepEqual(selections, [appliedState, null]);
});
