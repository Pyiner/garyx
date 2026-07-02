import test from "node:test";
import assert from "node:assert/strict";

import {
  apiKeyEnvName,
  buildProviderEnvPayload,
  envRowsFromEnvMap,
  isNativeModelProvider,
} from "./agent-env-editor.ts";

test("envRowsFromEnvMap sorts keys and tolerates a missing map", () => {
  assert.deepEqual(envRowsFromEnvMap({ B: "2", A: "1" }), [
    { key: "A", value: "1" },
    { key: "B", value: "2" },
  ]);
  assert.deepEqual(envRowsFromEnvMap(null), []);
  assert.deepEqual(envRowsFromEnvMap(undefined), []);
});

test("buildProviderEnvPayload keeps all rows and drops empty keys", () => {
  const out = buildProviderEnvPayload(
    [
      { key: "KEEP", value: "yes" },
      { key: "  ", value: "ignored" },
      { key: "EMPTY_VALUE", value: "" },
    ],
    "claude_code",
    "",
  );
  assert.deepEqual(out, { KEEP: "yes", EMPTY_VALUE: "" });
});

test("buildProviderEnvPayload merges the native api key without dropping other keys", () => {
  const out = buildProviderEnvPayload(
    [{ key: "OTHER", value: "keep" }],
    "gpt",
    "test-openai-api-key",
  );
  assert.equal(out.OTHER, "keep");
  assert.equal(out.OPENAI_API_KEY, "test-openai-api-key");
});

test("buildProviderEnvPayload ignores the api key for non-native providers", () => {
  const out = buildProviderEnvPayload([], "claude_code", "should-be-ignored");
  assert.deepEqual(out, {});
});

test("buildProviderEnvPayload last row wins on duplicate keys", () => {
  const out = buildProviderEnvPayload(
    [
      { key: "DUP", value: "first" },
      { key: "DUP", value: "second" },
    ],
    "claude_code",
    "",
  );
  assert.equal(out.DUP, "second");
});

test("apiKeyEnvName and isNativeModelProvider cover the native providers", () => {
  assert.equal(apiKeyEnvName("gpt"), "OPENAI_API_KEY");
  assert.equal(apiKeyEnvName("anthropic"), "ANTHROPIC_API_KEY");
  assert.equal(apiKeyEnvName("google"), "GEMINI_API_KEY");
  assert.equal(apiKeyEnvName("claude_code"), null);
  assert.equal(isNativeModelProvider("gpt"), true);
  assert.equal(isNativeModelProvider("claude_code"), false);
});
