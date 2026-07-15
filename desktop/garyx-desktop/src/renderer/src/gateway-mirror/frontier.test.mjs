import assert from "node:assert/strict";
import { test } from "node:test";

import { ThreadFrontier } from "./frontier.ts";

test("render frontier is an ordering gate: stale rejects, equal and forward accept", () => {
  const frontier = new ThreadFrontier();

  assert.equal(frontier.acceptRender(12), true);
  const atTwelve = frontier.snapshot();
  assert.equal(atTwelve.renderBasedOnSeq, 12);

  assert.equal(frontier.acceptRender(11), false, "stale cursor rejected");
  assert.equal(frontier.snapshot(), atTwelve, "rejection keeps snapshot identity");

  assert.equal(frontier.acceptRender(12), true, "same-seq overwrite accepted");
  assert.equal(
    frontier.snapshot(),
    atTwelve,
    "ordering acceptance alone does not mutate the frontier",
  );

  assert.equal(frontier.acceptRender(13), true, "forward cursor accepted");
  assert.equal(frontier.snapshot().renderBasedOnSeq, 13);
  assert.equal(frontier.acceptRender(Number.NaN), false);
});

test("empty-ledger based_on_seq zero is accepted once without a sentinel collision", () => {
  const frontier = new ThreadFrontier();
  const before = frontier.snapshot();
  assert.equal(frontier.acceptRender(0), true);
  assert.notEqual(frontier.snapshot(), before);
  assert.equal(frontier.acceptRender(0), true);
});
