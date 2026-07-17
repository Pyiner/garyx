import assert from "node:assert/strict";
import test from "node:test";

import {
  LIFECYCLE_JOIN_WINDOW_MS,
  LIFECYCLE_RETRY_DELAYS_MS,
  LIFECYCLE_TRANSPORT_TIMEOUT_MS,
  classifyLifecycleAttempt,
  lifecycleUiSettlement,
  resolveLifecycleStoreIncarnation,
  runLifecycleMutation,
} from "./lifecycle-ingress.ts";

const request = {
  gatewayScope: "https://gateway.example.test",
  runtimeEpoch: 7,
  operationId: "10000000-0000-4000-8000-000000000001",
  expectedStoreIncarnation: "20000000-0000-4000-8000-000000000002",
  threadId: "thread::lifecycle",
};

function tagged(code, message = code) {
  return {
    kind: "definitiveEndpointResponse",
    status: code === "unavailable" ? 503 : 409,
    error: {
      kind: "garyx_api_error",
      operation: "thread_archive",
      code,
      message,
    },
    value: null,
    body: "{}",
  };
}

test("lifecycle limits keep the six-second join inside each eight-second attempt", () => {
  assert.equal(LIFECYCLE_JOIN_WINDOW_MS, 6_000);
  assert.equal(LIFECYCLE_TRANSPORT_TIMEOUT_MS, 8_000);
  assert.ok(LIFECYCLE_JOIN_WINDOW_MS < LIFECYCLE_TRANSPORT_TIMEOUT_MS);
  assert.deepEqual(LIFECYCLE_RETRY_DELAYS_MS, [1_000, 2_000, 4_000, 8_000, 8_000]);
});

test("lifecycle identity accepts partial feeds but rejects a torn incarnation", () => {
  const incarnation = "20000000-0000-4000-8000-000000000002";
  assert.equal(
    resolveLifecycleStoreIncarnation([null, incarnation, incarnation]),
    incarnation,
  );
  assert.equal(resolveLifecycleStoreIncarnation([null, undefined, ""]), null);
  assert.equal(
    resolveLifecycleStoreIncarnation([
      incarnation,
      "20000000-0000-4000-8000-000000000003",
    ]),
    null,
  );
});

test("operation_in_progress and ambiguous transport reuse one operation identity", async () => {
  const attempts = [];
  const sleeps = [];
  const responses = [
    tagged("operation_in_progress"),
    { kind: "ambiguous", message: "response lost" },
    { kind: "ok", status: 200, value: { archived: true } },
  ];
  const completion = await runLifecycleMutation(
    request,
    async (attempt) => {
      attempts.push(attempt);
      return responses.shift();
    },
    {
      isCurrent: () => true,
      sleep: async (delayMs) => sleeps.push(delayMs),
    },
  );

  assert.equal(completion.kind, "applied");
  assert.equal(completion.attempts, 3);
  assert.deepEqual(sleeps, [1_000, 2_000]);
  assert.deepEqual(attempts.map((attempt) => attempt.attemptNumber), [1, 2, 3]);
  assert.deepEqual(
    new Set(attempts.map((attempt) => attempt.operationId)),
    new Set([request.operationId]),
  );
  assert.deepEqual(
    new Set(attempts.map((attempt) => attempt.expectedStoreIncarnation)),
    new Set([request.expectedStoreIncarnation]),
  );
});

test("five backoff resends exhaust after six single transport attempts", async () => {
  let attempts = 0;
  const sleeps = [];
  const completion = await runLifecycleMutation(
    request,
    async () => {
      attempts += 1;
      return { kind: "ambiguous", message: `lost-${attempts}` };
    },
    {
      isCurrent: () => true,
      sleep: async (delayMs) => sleeps.push(delayMs),
    },
  );

  assert.deepEqual(sleeps, [...LIFECYCLE_RETRY_DELAYS_MS]);
  assert.equal(attempts, 6);
  assert.deepEqual(completion, {
    kind: "exhausted",
    message: "lost-6",
    attempts: 6,
  });
});

test("deterministic rejected outcomes terminate without retry", async () => {
  for (const code of ["rejected_conflict", "rejected_not_found", "wrong_incarnation"]) {
    let attempts = 0;
    const completion = await runLifecycleMutation(
      request,
      async () => {
        attempts += 1;
        return tagged(code, `terminal ${code}`);
      },
      { isCurrent: () => true, sleep: async () => assert.fail("must not retry") },
    );
    assert.equal(attempts, 1);
    assert.equal(completion.kind, "rejected");
    assert.equal(completion.code, code);
  }
});

test("operation_id_conflict is a client-bug terminal outcome", () => {
  assert.deepEqual(classifyLifecycleAttempt(tagged("operation_id_conflict", "bad reuse")), {
    kind: "operationIdConflict",
    message: "bad reuse",
  });
});

test("UI settlement covers applied, rejected, and exhausted cleanup paths", () => {
  assert.deepEqual(
    lifecycleUiSettlement({ kind: "applied", value: {}, attempts: 1 }),
    {
      rollbackOptimistic: false,
      requireFullReplacement: true,
      errorMessage: null,
      operationIdConflict: false,
    },
  );
  assert.deepEqual(
    lifecycleUiSettlement({
      kind: "rejected",
      code: "rejected_conflict",
      message: "busy",
      attempts: 1,
    }),
    {
      rollbackOptimistic: true,
      requireFullReplacement: false,
      errorMessage: "busy",
      operationIdConflict: false,
    },
  );
  assert.deepEqual(
    lifecycleUiSettlement({ kind: "exhausted", message: "lost", attempts: 6 }),
    {
      rollbackOptimistic: true,
      requireFullReplacement: true,
      errorMessage: "lost",
      operationIdConflict: false,
    },
  );
  assert.equal(
    lifecycleUiSettlement({
      kind: "operationIdConflict",
      message: "bad reuse",
      attempts: 1,
    }).operationIdConflict,
    true,
  );
});

test("scope or epoch replacement retires an in-flight retry loop", async () => {
  let current = true;
  let attempts = 0;
  const completion = await runLifecycleMutation(
    request,
    async () => {
      attempts += 1;
      return tagged("operation_in_progress");
    },
    {
      isCurrent: () => current,
      sleep: async () => {
        current = false;
      },
    },
  );
  assert.deepEqual(completion, { kind: "cancelled", attempts: 1 });
  assert.equal(attempts, 1);
});

test("unavailable and provably not-sent attempts share the bounded retry lane", () => {
  assert.equal(classifyLifecycleAttempt(tagged("unavailable")).kind, "retry");
  assert.equal(
    classifyLifecycleAttempt({ kind: "notSent", message: "settings unavailable" }).kind,
    "retry",
  );
});
