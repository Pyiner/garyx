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
