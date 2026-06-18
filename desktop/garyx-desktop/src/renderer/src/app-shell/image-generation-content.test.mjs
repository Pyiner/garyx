import test from 'node:test';
import assert from 'node:assert/strict';

import { extractImageGenerationImageContent } from './image-generation-content.ts';

function generatedImageMessage(content) {
  return {
    content,
    metadata: {
      source: 'codex_app_server',
      item_type: 'imageGeneration',
    },
    toolName: 'imageGeneration',
    toolUseId: content.id || 'ig-test',
  };
}

test('image generation display prefers savedPath over result payload', () => {
  assert.deepEqual(
    extractImageGenerationImageContent(
      generatedImageMessage({
        id: 'ig-test',
        type: 'imageGeneration',
        savedPath: '/tmp/generated image.png',
        result: 'data:image/png;base64,should-not-be-used',
      }),
    ),
    [
      {
        type: 'image',
        name: 'generated image.png',
        path: '/tmp/generated image.png',
        media_type: 'image/png',
        url: 'file:///tmp/generated%20image.png',
      },
    ],
  );
});

test('image generation display uses hydrated source from savedPath history', () => {
  assert.deepEqual(
    extractImageGenerationImageContent(
      generatedImageMessage({
        id: 'ig-test',
        type: 'imageGeneration',
        savedPath: '/tmp/generated.png',
        result: 'truncated-result',
        source: {
          type: 'base64',
          media_type: 'image/png',
          data: 'full-image-data',
        },
      }),
    ),
    [
      {
        type: 'image',
        name: 'generated.png',
        path: '/tmp/generated.png',
        media_type: 'image/png',
        source: {
          type: 'base64',
          media_type: 'image/png',
          data: 'full-image-data',
        },
      },
    ],
  );
});

test('image generation display keeps result fallback for older payloads', () => {
  assert.deepEqual(
    extractImageGenerationImageContent(
      generatedImageMessage({
        id: 'ig-test',
        type: 'imageGeneration',
        result: 'data:image/webp;base64,legacy-data',
      }),
    ),
    [
      {
        type: 'image',
        name: 'ig-test.png',
        source: {
          type: 'base64',
          media_type: 'image/webp',
          data: 'legacy-data',
        },
      },
    ],
  );
});
