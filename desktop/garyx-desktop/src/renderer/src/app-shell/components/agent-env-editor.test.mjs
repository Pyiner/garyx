import test from "node:test";
import assert from "node:assert/strict";

import {
  buildProviderEnvPayload,
  envRowsFromEnvMap,
  envRowsHaveInvalidKey,
  formatEnvText,
  isValidEnvKey,
  parseEnvText,
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

test("isValidEnvKey and envRowsHaveInvalidKey", () => {
  assert.ok(isValidEnvKey("OPENAI_API_KEY"));
  assert.ok(isValidEnvKey("_X1"));
  assert.ok(!isValidEnvKey("1BAD"));
  assert.ok(!isValidEnvKey("HAS SPACE"));
  assert.ok(!isValidEnvKey("HAS=EQ"));
  assert.ok(!envRowsHaveInvalidKey([{ key: "OK_KEY", value: "1" }, { key: "  ", value: "" }]));
  assert.ok(envRowsHaveInvalidKey([{ key: "1bad", value: "1" }]));
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
