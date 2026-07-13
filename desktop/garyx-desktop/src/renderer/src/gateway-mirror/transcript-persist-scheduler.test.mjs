import assert from "node:assert/strict";
import { test } from "node:test";

import { TranscriptPersistScheduler } from "./transcript-persist-scheduler.ts";

class FakeClock {
  now = 0;
  nextId = 1;
  timers = new Map();

  setTimeout(callback, delayMs) {
    const id = this.nextId;
    this.nextId += 1;
    this.timers.set(id, { callback, dueAt: this.now + delayMs });
    return id;
  }

  clearTimeout(id) {
    this.timers.delete(id);
  }

  advance(delayMs) {
    const target = this.now + delayMs;
    while (true) {
      const due = [...this.timers.entries()]
        .filter(([, timer]) => timer.dueAt <= target)
        .sort((left, right) =>
          left[1].dueAt - right[1].dueAt || left[0] - right[0]
        )[0];
      if (!due) {
        break;
      }
      const [id, timer] = due;
      this.timers.delete(id);
      this.now = timer.dueAt;
      timer.callback();
    }
    this.now = target;
  }
}

test("transcript persistence burst writes only the latest state at the trailing edge", () => {
  const clock = new FakeClock();
  const latest = new Map();
  const writes = [];
  const scheduler = new TranscriptPersistScheduler(
    (threadId) => writes.push([threadId, latest.get(threadId)]),
    clock,
    1_000,
    5_000,
  );

  latest.set("thread::a", 1);
  scheduler.schedule("thread::a");
  clock.advance(600);
  latest.set("thread::a", 2);
  scheduler.schedule("thread::a");
  clock.advance(999);
  assert.deepEqual(writes, []);
  clock.advance(1);
  assert.deepEqual(writes, [["thread::a", 2]]);
  assert.equal(scheduler.pendingThreadCount(), 0);
});

test("continuous transcript traffic is forced by the non-resetting max wait", () => {
  const clock = new FakeClock();
  const writes = [];
  const scheduler = new TranscriptPersistScheduler(
    (threadId) => writes.push([threadId, clock.now]),
    clock,
    1_000,
    5_000,
  );

  scheduler.schedule("thread::a");
  for (let index = 0; index < 5; index += 1) {
    clock.advance(900);
    scheduler.schedule("thread::a");
  }
  assert.deepEqual(writes, []);
  clock.advance(500);
  assert.deepEqual(writes, [["thread::a", 5_000]]);
});

test("flush, cancel, flushAll, and per-thread timers are independent", () => {
  const clock = new FakeClock();
  const writes = [];
  const scheduler = new TranscriptPersistScheduler(
    (threadId) => writes.push(threadId),
    clock,
    1_000,
    5_000,
  );

  scheduler.schedule("thread::a");
  clock.advance(250);
  scheduler.schedule("thread::b");
  assert.equal(scheduler.flush("thread::a"), true);
  assert.equal(scheduler.cancel("thread::b"), true);
  clock.advance(10_000);
  assert.deepEqual(writes, ["thread::a"], "cancelled timers never revive");

  scheduler.schedule("thread::a");
  scheduler.schedule("thread::b");
  assert.equal(scheduler.flushAll(), 2);
  assert.deepEqual(writes, ["thread::a", "thread::a", "thread::b"]);
});
