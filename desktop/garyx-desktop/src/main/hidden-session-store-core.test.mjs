import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { createHiddenSessionStore } from './hidden-session-store-core.ts';

function tempStorePath() {
  const dir = mkdtempSync(join(tmpdir(), 'garyx-hidden-sessions-'));
  return { dir, file: join(dir, 'garyx-hidden-sessions.json') };
}

function summary(id) {
  return { id, title: `Side chat ${id}` };
}

/**
 * In-memory IO harness. Reads block on a caller-controlled gate so tests can
 * hold a load in flight; writes track overlap so tests can assert the
 * serialization invariant instead of only inspecting the final file.
 */
function createGatedIo({ initialContent }) {
  let releaseRead;
  const readGate = new Promise((resolve) => {
    releaseRead = resolve;
  });
  const state = {
    readCalls: 0,
    activeWrites: 0,
    maxActiveWrites: 0,
    files: new Map(),
    releaseRead: () => releaseRead(),
  };
  const io = {
    async readFile() {
      state.readCalls += 1;
      await readGate;
      if (initialContent === undefined) {
        throw Object.assign(new Error('ENOENT'), { code: 'ENOENT' });
      }
      return initialContent;
    },
    async writeFile(filePath, data) {
      state.activeWrites += 1;
      state.maxActiveWrites = Math.max(
        state.maxActiveWrites,
        state.activeWrites,
      );
      // Yield twice so an overlapping (non-serialized) writer would be
      // observed mid-flight rather than completing atomically in one tick.
      await new Promise((resolve) => setImmediate(resolve));
      await new Promise((resolve) => setImmediate(resolve));
      state.files.set(filePath, data);
      state.activeWrites -= 1;
    },
    async rename(fromPath, toPath) {
      state.files.set(toPath, state.files.get(fromPath));
      state.files.delete(fromPath);
    },
    async mkdir() {},
  };
  return { io, state };
}

test('a mutation queued behind an in-flight load lands on the shared cache', async () => {
  // The reviewer's race, reproduced for real: load-1 is HELD in flight,
  // load-2 starts while it is paused, and a mutation is queued behind them.
  // Correctness requires all of it to converge on one cache: exactly one
  // read, the persisted baseline preserved, the mutation applied on top.
  const { io, state } = createGatedIo({
    initialContent: JSON.stringify({
      'http://gateway-a': {
        'thread::pre-existing': summary('thread::pre-existing'),
      },
    }),
  });
  const store = createHiddenSessionStore(() => '/store/hidden.json', io);

  const loadOne = store.ensureLoaded();
  const loadTwo = store.ensureLoaded();
  // Queue the mutation while BOTH loads are still paused inside the read.
  const mutation = store.remember('http://gateway-a', summary('thread::child-1'));
  assert.equal(state.readCalls, 1, 'concurrent loads share one read');

  state.releaseRead();
  await Promise.all([loadOne, loadTwo, mutation]);
  // A later load must be a cache hit, not a second read that could install
  // a stale snapshot over the mutated cache.
  await store.ensureLoaded();
  assert.equal(state.readCalls, 1, 'no duplicate read after the cache exists');

  assert.deepEqual(
    store.list('http://gateway-a').map((entry) => entry.id).sort(),
    ['thread::child-1', 'thread::pre-existing'],
    'baseline and mutation both survive in memory',
  );
  const persisted = JSON.parse(state.files.get('/store/hidden.json'));
  assert.deepEqual(
    Object.keys(persisted['http://gateway-a']).sort(),
    ['thread::child-1', 'thread::pre-existing'],
    'baseline and mutation both survive on disk',
  );
});

test('concurrent mutations serialize: persists never overlap', async () => {
  // Kills the detached-promise mutant: if enqueue() ran on Promise.resolve()
  // instead of the shared chain, these persists would overlap and
  // maxActiveWrites would exceed 1 (writeFile yields mid-flight).
  const { io, state } = createGatedIo({ initialContent: undefined });
  const store = createHiddenSessionStore(() => '/store/hidden.json', io);
  state.releaseRead();

  const ids = ['a', 'b', 'c', 'd', 'e'].map((tag) => `thread::child-${tag}`);
  await Promise.all(
    ids.map((id) => store.remember('http://gateway-a', summary(id))),
  );

  assert.equal(state.maxActiveWrites, 1, 'persists are strictly serialized');
  const persisted = JSON.parse(state.files.get('/store/hidden.json'));
  assert.deepEqual(
    Object.keys(persisted['http://gateway-a']).sort(),
    [...ids].sort(),
    'every concurrent remember survives the merge',
  );
});

test('forget clears only the named partition entry', async (t) => {
  const { dir, file } = tempStorePath();
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = createHiddenSessionStore(() => file);

  // Equal thread ids on two gateways are independent rows.
  await store.remember('http://gateway-a', summary('thread::shared'));
  await store.remember('http://gateway-b', summary('thread::shared'));
  await store.forget('http://gateway-a', 'thread::shared');
  assert.deepEqual(store.list('http://gateway-a'), []);
  assert.deepEqual(
    store.list('http://gateway-b').map((entry) => entry.id),
    ['thread::shared'],
  );
});

test('a second store instance reads back the persisted partitions', async (t) => {
  const { dir, file } = tempStorePath();
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const first = createHiddenSessionStore(() => file);
  await first.remember('http://gateway-a', summary('thread::child-1'));

  // Restart shape: a fresh instance over the same file (real fs IO).
  const second = createHiddenSessionStore(() => file);
  await second.ensureLoaded();
  assert.deepEqual(
    second.list('http://gateway-a').map((entry) => entry.id),
    ['thread::child-1'],
  );
});
