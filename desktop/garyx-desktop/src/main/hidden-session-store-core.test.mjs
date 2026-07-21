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

test('a late duplicate load can never clobber a cache mutations advanced', async (t) => {
  const { dir, file } = tempStorePath();
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = createHiddenSessionStore(() => file);

  // The reviewer's exact race: two loads start concurrently, a mutation
  // lands between them. Single-flight initialization means both loads share
  // one read and the cache is installed exactly once.
  const loadOne = store.ensureLoaded();
  const loadTwo = store.ensureLoaded();
  await Promise.all([loadOne, loadTwo]);
  await store.remember('http://gateway-a', summary('thread::child-1'));
  // A third load after the mutation must not reset the cache either.
  await store.ensureLoaded();
  await store.remember('http://gateway-a', summary('thread::child-2'));

  const ids = store
    .list('http://gateway-a')
    .map((entry) => entry.id)
    .sort();
  assert.deepEqual(ids, ['thread::child-1', 'thread::child-2']);
  const persisted = JSON.parse(await readFile(file, 'utf8'));
  assert.deepEqual(
    Object.keys(persisted['http://gateway-a']).sort(),
    ['thread::child-1', 'thread::child-2'],
    'both children survive on disk',
  );
});

test('concurrent remembers on one store merge instead of overwriting', async (t) => {
  const { dir, file } = tempStorePath();
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const store = createHiddenSessionStore(() => file);

  await Promise.all([
    store.remember('http://gateway-a', summary('thread::child-a')),
    store.remember('http://gateway-a', summary('thread::child-b')),
  ]);
  assert.deepEqual(
    store.list('http://gateway-a').map((entry) => entry.id).sort(),
    ['thread::child-a', 'thread::child-b'],
  );
  const persisted = JSON.parse(await readFile(file, 'utf8'));
  assert.equal(Object.keys(persisted['http://gateway-a']).length, 2);
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

  // Restart shape: a fresh instance over the same file.
  const second = createHiddenSessionStore(() => file);
  await second.ensureLoaded();
  assert.deepEqual(
    second.list('http://gateway-a').map((entry) => entry.id),
    ['thread::child-1'],
  );
});
