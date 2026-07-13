import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

import { build } from 'esbuild';

const TEST_MAX_CACHE_RECORDS = 240;

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
        cache.saveThreadTranscriptCache({
          threadId: `thread-${String(index).padStart(4, '0')}`,
          messages: [],
          pendingInputs: [],
        }),
      ),
    );

    const cacheDirectory = join(userDataDirectory, 'transcript-cache');
    const cacheFiles = (await readdir(cacheDirectory)).filter((name) =>
      name.endsWith('.json'),
    );
    assert.ok(
      cacheFiles.length <= TEST_MAX_CACHE_RECORDS,
      `expected at most ${TEST_MAX_CACHE_RECORDS} cache records, found ${cacheFiles.length}`,
    );
  } finally {
    await rm(temporaryRoot, { force: true, recursive: true });
  }
});
