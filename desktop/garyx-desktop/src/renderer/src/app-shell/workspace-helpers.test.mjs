import test from 'node:test';
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';

import * as esbuild from 'esbuild';

const bundled = await esbuild.build({
  entryPoints: ['src/renderer/src/app-shell/workspace-helpers.ts'],
  bundle: true,
  format: 'esm',
  platform: 'node',
  write: false,
});
const helpers = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`
);
const { resolveThreadFilePreviewTarget } = helpers;

test('absolute transcript image paths preview through their gateway-side parent', () => {
  assert.deepEqual(
    resolveThreadFilePreviewTarget(
      '/Users/test/project',
      '/tmp/garyx-screenshots/thread-runtime-expanded.png',
    ),
    {
      workspacePath: '/tmp/garyx-screenshots',
      filePath: 'thread-runtime-expanded.png',
    },
  );
});

test('relative transcript image paths preview through the thread workspace', () => {
  assert.deepEqual(
    resolveThreadFilePreviewTarget('/Users/test/project/', './shots/result.png'),
    {
      workspacePath: '/Users/test/project',
      filePath: 'shots/result.png',
    },
  );
});

test('relative transcript image paths need a thread workspace', () => {
  assert.equal(resolveThreadFilePreviewTarget(null, 'shots/result.png'), null);
});
