import test from 'node:test';
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';

import * as esbuild from 'esbuild';

const bundled = await esbuild.build({
  entryPoints: ['src/renderer/src/message-rich-content-core.ts'],
  bundle: true,
  format: 'esm',
  platform: 'node',
  write: false,
});
const core = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`
);
const { collectTranscriptSegments } = core;

test('path-only image blocks remain image references instead of file cards', () => {
  assert.deepEqual(
    collectTranscriptSegments(
      {
        type: 'image',
        name: 'image.png',
        path: '/tmp/gateway/image.png',
        media_type: 'image/png',
      },
      'user',
    ),
    [
      {
        kind: 'image_reference',
        key: 'root:image-ref',
        path: '/tmp/gateway/image.png',
        label: 'image.png',
        mediaType: 'image/png',
      },
    ],
  );
});

test('inline image blocks still render immediately from their data source', () => {
  assert.deepEqual(
    collectTranscriptSegments(
      {
        type: 'image',
        source: {
          type: 'base64',
          media_type: 'image/png',
          data: 'aGVsbG8=',
        },
      },
      'user',
    ),
    [
      {
        kind: 'image',
        key: 'root:image',
        src: 'data:image/png;base64,aGVsbG8=',
        alt: 'user image',
      },
    ],
  );
});
