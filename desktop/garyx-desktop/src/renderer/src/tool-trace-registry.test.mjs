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

test('untyped source.data record is extracted AND stripped symmetrically', () => {
  // Review #TASK-1677 blocker shape: no `type` field, image only via
  // source.data. Extraction and stripping must agree, or base64 leaks.
  const merged = resolveMergedToolTrace(
    undefined,
    {
      role: 'tool_result',
      content: {
        result: [{ source: { type: 'base64', media_type: 'image/png', data: FAKE_BASE64 } }],
        text: '',
      },
      toolUseId: 'tool:4',
      toolName: 'someTool',
    },
  );

  assert.equal(merged.resultImages.length, 1);
  assert.ok(!(merged.resultDetail || '').includes(FAKE_BASE64), 'detail must not leak base64');
});

test('url-bearing non-image results are neither thumbnailed nor stripped', () => {
  // WebFetch-style result: has a url field but is not an image block. The
  // lenient message-bubble collector would treat it as an image; the tool
  // strip must not.
  const merged = resolveMergedToolTrace(
    undefined,
    {
      role: 'tool_result',
      content: {
        result: { url: 'https://example.com/page', content: 'fetched body text' },
        text: '',
      },
      toolUseId: 'tool:5',
      toolName: 'WebFetch',
    },
  );

  assert.equal(merged.resultImages.length, 0);
  assert.ok((merged.resultDetail || '').includes('fetched body text'));
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

test('Image view exposes one gateway path preview when use and result repeat the path', () => {
  const imageView = {
    id: 'exec-image-1',
    path: '/tmp/screens/thread-runtime-expanded.png',
    type: 'ImageView',
  };
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: imageView,
      toolUseId: 'tool:image-view',
      toolName: 'imageView',
    },
    {
      role: 'tool_result',
      content: imageView,
      toolUseId: 'tool:image-view',
      toolName: 'imageView',
    },
  );

  assert.deepEqual(merged.pathImages, [
    {
      key: 'image-view:/tmp/screens/thread-runtime-expanded.png',
      path: '/tmp/screens/thread-runtime-expanded.png',
      alt: 'thread-runtime-expanded.png',
    },
  ]);
});

test('ordinary path-bearing tools do not request gateway image previews', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { tool: 'Read', input: { file_path: '/Users/test/notes.txt' } },
      toolUseId: 'tool:read',
      toolName: 'Read',
    },
    undefined,
  );

  assert.deepEqual(merged.pathImages, []);
});

test('server projection shows only the selected Codex command fields', () => {
  const command = '/bin/zsh -lc "git status --short"';
  const output = ' M README.md\n M package.json\n';
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: {
        command,
        cwd: '/Users/test/repo',
        id: 'exec-test',
        status: 'inProgress',
        type: 'commandExecution',
      },
      toolUseId: 'tool:command',
      toolName: 'commandExecution',
    },
    {
      role: 'tool_result',
      content: {
        aggregatedOutput: output,
        command,
        cwd: '/Users/test/repo',
        durationMs: 12,
        exitCode: 0,
        id: 'exec-test',
        status: 'completed',
        type: 'commandExecution',
      },
      toolUseId: 'tool:command',
      toolName: 'commandExecution',
    },
    {
      tool_name: 'commandExecution',
      kind: 'command',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['command'],
        format: 'code',
        label: 'command',
      },
      result: {
        root: 'content',
        path: ['aggregatedOutput'],
        format: 'code',
        label: 'output',
      },
      status: 'completed',
      exit_code: 0,
      duration_ms: 12,
    },
  );

  assert.equal(merged.inputDetail, command);
  assert.equal(merged.inputLabel, 'Command');
  assert.equal(merged.resultDetail, output);
  assert.equal(merged.resultLabel, 'Output');
  assert.ok(!merged.resultDetail.includes('/Users/test/repo'));
  assert.ok(!merged.resultDetail.includes('exec-test'));
  assert.deepEqual(merged.badges.slice(-2), ['exit 0', '12 ms']);
});

test('projection with no result selector never falls back to the JSON envelope', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { command: 'true', type: 'commandExecution' },
      toolUseId: 'tool:no-output',
      toolName: 'commandExecution',
    },
    {
      role: 'tool_result',
      content: {
        aggregatedOutput: null,
        command: 'true',
        cwd: '/Users/test/repo',
        exitCode: 0,
        status: 'completed',
        type: 'commandExecution',
      },
      toolUseId: 'tool:no-output',
      toolName: 'commandExecution',
    },
    {
      tool_name: 'commandExecution',
      kind: 'command',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['command'],
        format: 'code',
        label: 'command',
      },
      status: 'completed',
      exit_code: 0,
    },
  );

  assert.equal(merged.inputDetail, 'true');
  assert.equal(merged.resultDetail, undefined);
});

test('server projection unwraps Antigravity JSON-encoded scalar labels', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: {
        name: 'run_command',
        args: { toolSummary: '"Check status"' },
      },
      toolUseId: 'tool:antigravity',
      toolName: 'run_command',
    },
    undefined,
    {
      tool_name: 'run_command',
      kind: 'command',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['args', 'toolSummary'],
        format: 'text',
        label: 'call',
      },
    },
  );

  assert.equal(merged.summary, 'Check status');
  assert.equal(merged.inputDetail, 'Check status');
});
