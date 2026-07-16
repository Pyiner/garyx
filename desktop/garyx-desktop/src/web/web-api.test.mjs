import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

test("web settings mutations use exactly one transport attempt", async () => {
  const originalWindow = globalThis.window;
  const originalFetch = globalThis.fetch;
  const attempts = [];
  globalThis.window = {
    location: {
      href: "https://desktop.example.test/?gateway=https%3A%2F%2Fgateway.example.test",
      origin: "https://desktop.example.test",
    },
  };
  globalThis.fetch = async (_url, init) => {
    attempts.push(init?.method || "GET");
    return new Response(JSON.stringify({ error: "unavailable" }), {
      status: 503,
      statusText: "Service Unavailable",
    });
  };
  try {
    const { saveGatewaySettings } = await import("./web-api.ts");
    await assert.rejects(() => saveGatewaySettings({}));
    assert.deepEqual(attempts, ["PUT"]);
  } finally {
    globalThis.fetch = originalFetch;
    globalThis.window = originalWindow;
  }
});

test("web request helper has no default semantics and every call supplies one", async () => {
  const source = await readFile(new URL("./web-api.ts", import.meta.url), "utf8");
  assert.match(
    source,
    /semantics:\s*'readRetryable'\s*\|\s*'mutationSingleAttempt'/,
  );
  assert.doesNotMatch(source, /semantics:[^,\n]+=[^,\n]+/);
  const calls = [...source.matchAll(/requestJson(?:<[^>]+>)?\(/g)];
  assert.ok(calls.length >= 10, "inventory must cover the complete web helper surface");
  assert.match(source, /'mutationSingleAttempt'/);
  assert.match(source, /method:\s*'PUT'/);
});
