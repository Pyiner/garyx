import test from "node:test";
import assert from "node:assert/strict";

import {
  apiKeyEnvName,
  apiKeyValueFromRows,
  buildProviderEnvPayload,
  envRowsFromEnvMap,
  envRowsHaveInvalidKey,
  formatEnvText,
  isNativeModelProvider,
  isValidEnvKey,
  parseEnvText,
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
  let rows = setApiKeyInRows([{ key: "OTHER", value: "keep" }], "gpt", "test-openai-api-key");
  assert.deepEqual(buildProviderEnvPayload(rows), {
    OTHER: "keep",
    OPENAI_API_KEY: "test-openai-api-key",
  });
  // update in place
  rows = setApiKeyInRows(rows, "gpt", "test-openai-api-key-2");
  assert.equal(apiKeyValueFromRows(rows, "gpt"), "test-openai-api-key-2");
  assert.deepEqual(buildProviderEnvPayload(rows), {
    OTHER: "keep",
    OPENAI_API_KEY: "test-openai-api-key-2",
  });
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

test('formatEnvText emits numeric values verbatim without quotes', () => {
  const text = formatEnvText([
    { key: 'CLAUDE_CODE_AUTO_COMPACT_WINDOW', value: '183500' },
    { key: 'CLAUDE_CODE_ATTRIBUTION_HEADER', value: '0' },
  ]);
  assert.equal(
    text,
    'CLAUDE_CODE_AUTO_COMPACT_WINDOW=183500\nCLAUDE_CODE_ATTRIBUTION_HEADER=0'
  );
});

test('parseEnvText keeps numeric values as-is and strips whole-value quotes', () => {
  const rows = parseEnvText('A=183500\nB="183500"\nC=plat_tok"en\nD=  spaced  ');
  assert.deepEqual(rows, [
    { key: 'A', value: '183500' },
    { key: 'B', value: '183500' },
    { key: 'C', value: 'plat_tok"en' },
    { key: 'D', value: '  spaced' },
  ]);
});

test('parseEnvText skips blanks and comments, keeps eq-less lines for the save gate', () => {
  const rows = parseEnvText('\n# comment\nGOOD=1\nBROKEN_LINE\n');
  assert.deepEqual(rows, [
    { key: 'GOOD', value: '1' },
    { key: 'BROKEN_LINE', value: '' },
  ]);
});

test('env text round-trips newline and quote-wrapped values losslessly', () => {
  const rows = [
    { key: 'MULTI', value: 'line1\nline2' },
    { key: 'QUOTED', value: '"already quoted"' },
    { key: 'EDGE_WS', value: '  padded  ' },
    { key: 'PLAIN', value: 'https://super-relay.example/v1' },
  ];
  const roundTripped = parseEnvText(formatEnvText(rows));
  assert.deepEqual(roundTripped, rows);
});

test('parseEnvText then formatEnvText is stable for hand-written text', () => {
  const text = 'A=1\nB=two words\nC="183500"';
  const once = formatEnvText(parseEnvText(text));
  const twice = formatEnvText(parseEnvText(once));
  assert.equal(twice, once);
});
