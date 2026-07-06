import test from 'node:test';
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';

import * as esbuild from 'esbuild';

const bundled = await esbuild.build({
  entryPoints: ['src/renderer/src/tool-trace-registry.ts'],
  bundle: true,
  format: 'esm',
  platform: 'node',
  write: false,
});
const registry = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`
);
const { resolveMergedToolTrace } = registry;

const FAKE_BASE64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk';

function imageBlock() {
  return {
    type: 'image',
    source: { type: 'base64', media_type: 'image/png', data: FAKE_BASE64 },
  };
}

// Real captured shape: a Claude Read of an image file commits a tool_result
// whose content is { result: [imageBlock], text: "" }.
test('image-only tool result renders thumbnails instead of base64 detail', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { tool: 'Read', input: { file_path: '/Users/test/shot.png' } },
      toolUseId: 'tool:1',
      toolName: 'Read',
    },
    {
      role: 'tool_result',
      content: { result: [imageBlock()], text: '' },
      toolUseId: 'tool:1',
      toolName: 'Read',
    },
  );

  assert.equal(merged.resultImages.length, 1);
  assert.equal(
    merged.resultImages[0].src,
    `data:image/png;base64,${FAKE_BASE64}`,
  );
  const detail = merged.resultDetail || '';
  assert.ok(!detail.includes(FAKE_BASE64), 'detail must not leak base64');
});

test('mixed text and image result keeps text detail and extracts the image', () => {
  const merged = resolveMergedToolTrace(
    undefined,
    {
      role: 'tool_result',
      content: {
        result: [{ type: 'text', text: 'wrote screenshot' }, imageBlock()],
        text: '',
      },
      toolUseId: 'tool:2',
      toolName: 'browser_screenshot',
    },
  );

  assert.equal(merged.resultImages.length, 1);
  assert.ok((merged.resultDetail || '').includes('wrote screenshot'));
  assert.ok(!(merged.resultDetail || '').includes(FAKE_BASE64));
});

test('imageless tool results keep their existing detail behavior', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { tool: 'Bash', input: { command: 'ls' } },
      toolUseId: 'tool:3',
      toolName: 'Bash',
    },
    {
      role: 'tool_result',
      content: { result: 'file-a\nfile-b', text: 'file-a\nfile-b' },
      toolUseId: 'tool:3',
      toolName: 'Bash',
    },
  );

  assert.equal(merged.resultImages.length, 0);
  assert.ok((merged.resultDetail || merged.summary || '').length > 0);
});
