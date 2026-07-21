import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

import { build } from 'esbuild';

const TEST_MAX_CACHE_RECORDS = 240;

function transcript(threadId, text = '') {
  return {
    threadId,
    messages: text
      ? [{ id: `${threadId}:1`, role: 'user', text }]
      : [],
    pendingInputs: [],
  };
}

async function cacheRecordNames(directory) {
  return (await readdir(directory)).filter((name) => name.endsWith('.json'));
}

async function importTranscriptCache(userDataDirectory, bundleDirectory) {
  const entryPoint = fileURLToPath(new URL('./transcript-cache.ts', import.meta.url));
  const outputPath = join(bundleDirectory, 'transcript-cache-under-test.mjs');
  const result = await build({
    bundle: true,
    entryPoints: [entryPoint],
    format: 'esm',
    logLevel: 'silent',
    platform: 'node',
    plugins: [
      {
        name: 'electron-user-data-test-double',
        setup(buildApi) {
          buildApi.onResolve({ filter: /^electron$/ }, () => ({
            namespace: 'electron-test-double',
            path: 'electron',
          }));
          buildApi.onLoad(
            { filter: /.*/, namespace: 'electron-test-double' },
            () => ({
              contents: `export const app = { getPath: () => ${JSON.stringify(userDataDirectory)} };`,
              loader: 'js',
            }),
          );
        },
      },
    ],
    write: false,
  });
  await writeFile(outputPath, result.outputFiles[0].contents);
  return import(pathToFileURL(outputPath).href);
}

test('bounds the on-disk transcript cache by record count', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'garyx-transcript-cache-test-'));
  try {
    const userDataDirectory = join(temporaryRoot, 'user-data');
    const cache = await importTranscriptCache(userDataDirectory, temporaryRoot);

    await Promise.all(
      Array.from({ length: TEST_MAX_CACHE_RECORDS + 1 }, (_, index) =>
        cache.saveThreadTranscriptCache(
          'http://gateway-a',
          transcript(`thread-${String(index).padStart(4, '0')}`),
        ),
      ),
    );

    const cacheDirectory = join(userDataDirectory, 'transcript-cache');
    const cacheFiles = await cacheRecordNames(cacheDirectory);
    assert.ok(
      cacheFiles.length <= TEST_MAX_CACHE_RECORDS,
      `expected at most ${TEST_MAX_CACHE_RECORDS} cache records, found ${cacheFiles.length}`,
    );
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});

test('uses load access time for record-count LRU eviction', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'garyx-transcript-cache-lru-'));
  try {
    const cache = await importTranscriptCache(join(temporaryRoot, 'unused-user-data'), temporaryRoot);
    const cacheDirectory = join(temporaryRoot, 'cache');
    let now = Date.parse('2026-07-01T00:00:00.000Z');
    const store = new cache.TranscriptCacheStore({
      directory: () => cacheDirectory,
      maxBytes: Number.MAX_SAFE_INTEGER,
      maxRecords: 2,
      now: () => new Date((now += 1_000)),
    });

    await store.save('http://gateway-a', transcript('thread-a'));
    await store.save('http://gateway-a', transcript('thread-b'));
    assert.ok(await store.load('http://gateway-a', 'thread-a'));
    await store.save('http://gateway-a', transcript('thread-c'));

    assert.equal(await store.load('http://gateway-a', 'thread-b'), null, 'least-recently used record is evicted');
    assert.ok(await store.load('http://gateway-a', 'thread-a'));
    assert.ok(await store.load('http://gateway-a', 'thread-c'));
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});

test('evicts records until the transcript-cache byte limit holds', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'garyx-transcript-cache-bytes-'));
  try {
    const cache = await importTranscriptCache(join(temporaryRoot, 'unused-user-data'), temporaryRoot);
    const cacheDirectory = join(temporaryRoot, 'cache');
    const store = new cache.TranscriptCacheStore({
      directory: () => cacheDirectory,
      maxBytes: 256,
      maxRecords: 10,
    });

    await store.save('http://gateway-a', transcript('thread-large', 'X'.repeat(2_048)));

    assert.deepEqual(await cacheRecordNames(cacheDirectory), []);
    assert.equal(await store.load('http://gateway-a', 'thread-large'), null);
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});

test('startup pruning bounds records written before the current process', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'garyx-transcript-cache-startup-'));
  try {
    const cache = await importTranscriptCache(join(temporaryRoot, 'unused-user-data'), temporaryRoot);
    const cacheDirectory = join(temporaryRoot, 'cache');
    const permissiveStore = new cache.TranscriptCacheStore({
      directory: () => cacheDirectory,
      maxBytes: Number.MAX_SAFE_INTEGER,
      maxRecords: 10,
    });
    await permissiveStore.save('http://gateway-a', transcript('thread-a'));
    await permissiveStore.save('http://gateway-a', transcript('thread-b'));
    await permissiveStore.save('http://gateway-a', transcript('thread-c'));
    assert.equal((await cacheRecordNames(cacheDirectory)).length, 3);

    const startupStore = new cache.TranscriptCacheStore({
      directory: () => cacheDirectory,
      maxBytes: Number.MAX_SAFE_INTEGER,
      maxRecords: 2,
    });
    await startupStore.prune();

    assert.equal((await cacheRecordNames(cacheDirectory)).length, 2);
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});

test('cache identity is the (gatewayScope, threadId) pair', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'garyx-transcript-cache-scope-'));
  try {
    const cache = await importTranscriptCache(join(temporaryRoot, 'unused-user-data'), temporaryRoot);
    const store = new cache.TranscriptCacheStore({
      directory: () => join(temporaryRoot, 'cache'),
      maxBytes: Number.MAX_SAFE_INTEGER,
      maxRecords: 10,
    });

    // Thread ids are only unique per gateway: the SAME id on another
    // gateway must be a cache miss, never gateway A's transcript.
    await store.save('http://gateway-a', transcript('thread-same-id'));
    assert.ok(await store.load('http://gateway-a', 'thread-same-id'));
    assert.equal(
      await store.load('http://gateway-b', 'thread-same-id'),
      null,
      'a colliding id on another gateway misses instead of leaking',
    );

    // Clear is partition-local too.
    await store.save('http://gateway-b', transcript('thread-same-id'));
    await store.clear('http://gateway-b', 'thread-same-id');
    assert.equal(await store.load('http://gateway-b', 'thread-same-id'), null);
    assert.ok(
      await store.load('http://gateway-a', 'thread-same-id'),
      'clearing one gateway partition leaves the other intact',
    );
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});

test('serializes save and clear mutations with the latest call winning', async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), 'garyx-transcript-cache-order-'));
  try {
    const cache = await importTranscriptCache(join(temporaryRoot, 'unused-user-data'), temporaryRoot);
    const store = new cache.TranscriptCacheStore({
      directory: () => join(temporaryRoot, 'cache'),
    });

    await Promise.all([
      store.save('http://gateway-a', transcript('thread-order', 'first')),
      store.clear('http://gateway-a', 'thread-order'),
      store.save('http://gateway-a', transcript('thread-order', 'latest')),
    ]);

    const loaded = await store.load('http://gateway-a', 'thread-order');
    assert.equal(loaded?.transcript.messages[0]?.text, 'latest');
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});
