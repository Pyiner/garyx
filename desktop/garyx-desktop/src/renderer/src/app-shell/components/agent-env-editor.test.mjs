import test from "node:test";
import assert from "node:assert/strict";

import {
  apiKeyEnvName,
  apiKeyValueFromRows,
  buildProviderEnvPayload,
  envRowsFromEnvMap,
  envRowsHaveInvalidKey,
  isNativeModelProvider,
  isValidEnvKey,
  setApiKeyInRows,
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
  const out = buildProviderEnvPayload([
    { key: "KEEP", value: "yes" },
    { key: "  ", value: "ignored" },
    { key: "EMPTY_VALUE", value: "" },
  ]);
  assert.deepEqual(out, { KEEP: "yes", EMPTY_VALUE: "" });
});

test("buildProviderEnvPayload last row wins on duplicate keys", () => {
  const out = buildProviderEnvPayload([
    { key: "DUP", value: "first" },
    { key: "DUP", value: "second" },
  ]);
  assert.equal(out.DUP, "second");
});

test("api-key shortcut is derived from the row, never serialized over it", () => {
  // Editing the OPENAI_API_KEY row must win; there is no separate shortcut state
  // that could overwrite it.
  const rows = [{ key: "OPENAI_API_KEY", value: "new-value" }];
  assert.equal(apiKeyValueFromRows(rows, "gpt"), "new-value");
  assert.deepEqual(buildProviderEnvPayload(rows), { OPENAI_API_KEY: "new-value" });
});

test("setApiKeyInRows upserts and removes the well-known row", () => {
  // create
  let rows = setApiKeyInRows([{ key: "OTHER", value: "keep" }], "gpt", "sk-1");
  assert.deepEqual(buildProviderEnvPayload(rows), { OTHER: "keep", OPENAI_API_KEY: "sk-1" });
  // update in place
  rows = setApiKeyInRows(rows, "gpt", "sk-2");
  assert.equal(apiKeyValueFromRows(rows, "gpt"), "sk-2");
  assert.deepEqual(buildProviderEnvPayload(rows), { OTHER: "keep", OPENAI_API_KEY: "sk-2" });
  // clearing removes the row (does not resurrect it)
  rows = setApiKeyInRows(rows, "gpt", "");
  assert.equal(apiKeyValueFromRows(rows, "gpt"), "");
  assert.deepEqual(buildProviderEnvPayload(rows), { OTHER: "keep" });
});

test("deleting the well-known row is not undone by the shortcut", () => {
  // With no separate shortcut state, a row deletion serializes as removed.
  const rows = [];
  assert.deepEqual(buildProviderEnvPayload(rows), {});
  assert.equal(apiKeyValueFromRows(rows, "gpt"), "");
});

test("isValidEnvKey and envRowsHaveInvalidKey", () => {
  assert.ok(isValidEnvKey("OPENAI_API_KEY"));
  assert.ok(isValidEnvKey("_X1"));
  assert.ok(!isValidEnvKey("1BAD"));
  assert.ok(!isValidEnvKey("HAS SPACE"));
  assert.ok(!isValidEnvKey("HAS=EQ"));
  assert.ok(!envRowsHaveInvalidKey([{ key: "OK_KEY", value: "1" }, { key: "  ", value: "" }]));
  assert.ok(envRowsHaveInvalidKey([{ key: "1bad", value: "1" }]));
});

test("apiKeyEnvName and isNativeModelProvider cover the native providers", () => {
  assert.equal(apiKeyEnvName("gpt"), "OPENAI_API_KEY");
  assert.equal(apiKeyEnvName("anthropic"), "ANTHROPIC_API_KEY");
  assert.equal(apiKeyEnvName("google"), "GEMINI_API_KEY");
  assert.equal(apiKeyEnvName("claude_code"), null);
  assert.equal(isNativeModelProvider("gpt"), true);
  assert.equal(isNativeModelProvider("claude_code"), false);
});
