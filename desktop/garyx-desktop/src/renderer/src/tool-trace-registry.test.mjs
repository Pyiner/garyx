import test from 'node:test';
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';
import fs from 'node:fs';
import path from 'node:path';

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

const claudeReadImageProjection = {
  tool_name: 'Read',
  kind: 'file_read',
  visibility: 'normal',
  call: {
    root: 'content',
    path: ['input', 'file_path'],
    format: 'path',
    label: 'file',
  },
  result: {
    root: 'content',
    path: ['result'],
    format: 'image',
    label: 'image',
  },
};

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
    claudeReadImageProjection,
  );

  assert.equal(merged.resultImages.length, 1);
  assert.equal(
    merged.resultImages[0].src,
    `data:image/png;base64,${FAKE_BASE64}`,
  );
  const detail = merged.resultDetail || '';
  assert.ok(!detail.includes(FAKE_BASE64), 'detail must not leak base64');
});

test('projected mixed image result extracts the image without serializing its payload', () => {
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
    {
      tool_name: 'browser_screenshot',
      kind: 'image',
      visibility: 'normal',
      result: {
        root: 'content',
        path: ['result'],
        format: 'image',
        label: 'image',
      },
    },
  );

  assert.equal(merged.resultImages.length, 1);
  assert.equal(merged.resultDetail, undefined);
  assert.ok(!(merged.resultDetail || '').includes(FAKE_BASE64));
});

test('projected untyped source.data image is extracted without base64 detail', () => {
  // Review #TASK-1677 blocker shape: no `type` field, image only via
  // source.data. Image selectors must never expose that payload as text.
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
    {
      tool_name: 'someTool',
      kind: 'image',
      visibility: 'normal',
      result: {
        root: 'content',
        path: ['result'],
        format: 'image',
        label: 'image',
      },
    },
  );

  assert.equal(merged.resultImages.length, 1);
  assert.ok(!(merged.resultDetail || '').includes(FAKE_BASE64), 'detail must not leak base64');
});

test('url-bearing non-image results are not thumbnailed', () => {
  // WebFetch-style result: has a url field but is not an image block. The
  // lenient message-bubble collector would treat it as an image; the generic
  // projection resolver must not.
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
    {
      tool_name: 'WebFetch',
      kind: 'web',
      visibility: 'normal',
      result: {
        root: 'content',
        path: ['result'],
        format: 'json',
        label: 'response',
      },
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
    {
      tool_name: 'Bash',
      kind: 'command',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['input', 'command'],
        format: 'code',
        label: 'command',
      },
      result: {
        root: 'content',
        path: ['result'],
        format: 'code',
        label: 'output',
      },
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
    {
      tool_name: 'imageView',
      kind: 'image',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['path'],
        format: 'image',
        label: 'image',
      },
      result: {
        root: 'content',
        path: ['path'],
        format: 'image',
        label: 'image',
      },
    },
  );

  assert.deepEqual(merged.pathImages, [
    {
      key: 'projected-image:/tmp/screens/thread-runtime-expanded.png',
      path: '/tmp/screens/thread-runtime-expanded.png',
      alt: 'thread-runtime-expanded.png',
    },
  ]);
});

test('native image generation resolves its result-owned prompt and saved image path', () => {
  const prompt = 'A synthetic lighthouse beneath a violet evening sky.';
  const imagePath = '/Users/test/.codex/generated_images/synthetic/exec-native.png';
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: {
        id: 'exec-native',
        result: '',
        revisedPrompt: null,
        status: 'in_progress',
        type: 'imageGeneration',
      },
      toolUseId: 'tool:image-generation',
      toolName: 'imageGeneration',
    },
    {
      role: 'tool_result',
      content: {
        id: 'exec-native',
        result: FAKE_BASE64,
        revisedPrompt: prompt,
        savedPath: imagePath,
        status: 'completed',
        type: 'imageGeneration',
      },
      toolUseId: 'tool:image-generation',
      toolName: 'imageGeneration',
    },
    {
      tool_name: 'imageGeneration',
      kind: 'image',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['revisedPrompt'],
        format: 'text',
        label: 'prompt',
      },
      result: {
        root: 'content',
        path: ['savedPath'],
        format: 'image',
        label: 'image',
      },
      status: 'completed',
    },
  );

  assert.equal(merged.summary, prompt);
  assert.equal(merged.inputDetail, prompt);
  assert.equal(merged.inputLabel, 'Prompt');
  assert.equal(merged.resultDetail, undefined);
  assert.equal(merged.resultLabel, 'Image');
  assert.deepEqual(merged.pathImages, [
    {
      key: `projected-image:${imagePath}`,
      path: imagePath,
      alt: 'exec-native.png',
    },
  ]);
  assert.ok(!JSON.stringify(merged).includes(FAKE_BASE64), 'tool row must not leak base64');
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
    {
      tool_name: 'Read',
      kind: 'file_read',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['input', 'file_path'],
        format: 'path',
        label: 'file',
      },
    },
  );

  assert.deepEqual(merged.pathImages, []);
});

test('missing projection renders a provider-neutral empty fallback', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { tool: 'Bash', input: { command: 'cat private.txt' } },
      toolUseId: 'tool:unprojectable',
      toolName: 'Bash',
      metadata: { source: 'claude_sdk' },
    },
    {
      role: 'tool_result',
      content: { result: 'private output' },
      toolUseId: 'tool:unprojectable',
      toolName: 'Bash',
      metadata: { source: 'claude_sdk' },
      isError: true,
    },
  );

  assert.deepEqual(merged, {
    title: 'Tool',
    badges: [],
    resultImages: [],
    pathImages: [],
    icon: '·',
    isError: true,
  });
});

test('sanitized captured server snapshot maps Claude and Codex rows through selectors only', () => {
  // Sanitized from current local Claude SDK and Codex app-server transcript
  // shapes. The entry objects mirror `RenderToolEntry` from a render frame;
  // message bodies remain in the seq-keyed client cache.
  const captured = {
    entries: [
      {
        tool_use: { seq: 41 },
        tool_result: { seq: 42 },
        projection: {
          tool_name: 'Read',
          kind: 'file_read',
          visibility: 'normal',
          call: {
            root: 'content',
            path: ['input', 'file_path'],
            format: 'path',
            label: 'file',
          },
          result: {
            root: 'content',
            path: ['result'],
            format: 'text',
            label: 'result',
          },
        },
      },
      {
        tool_use: { seq: 73 },
        tool_result: { seq: 74 },
        projection: {
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
          duration_ms: 7,
        },
      },
    ],
    messagesBySeq: {
      41: {
        role: 'tool_use',
        content: { tool: 'Read', input: { file_path: '/Users/test/repo/README.md' } },
        toolUseId: 'tool:claude-read',
        toolName: 'Read',
        metadata: { source: 'claude_sdk' },
      },
      42: {
        role: 'tool_result',
        content: { result: 'captured read output', text: 'captured read output' },
        toolUseId: 'tool:claude-read',
        metadata: { source: 'claude_sdk' },
      },
      73: {
        role: 'tool_use',
        content: {
          type: 'commandExecution',
          command: '/bin/zsh -lc "git status --short"',
          status: 'inProgress',
        },
        toolUseId: 'tool:codex-command',
        toolName: 'commandExecution',
        metadata: { source: 'codex_app_server', item_type: 'commandExecution' },
      },
      74: {
        role: 'tool_result',
        content: {
          type: 'commandExecution',
          aggregatedOutput: ' M README.md\n',
          status: 'completed',
          exitCode: 0,
          durationMs: 7,
        },
        toolUseId: 'tool:codex-command',
        toolName: 'commandExecution',
        metadata: { source: 'codex_app_server', item_type: 'commandExecution' },
      },
    },
  };

  const rows = captured.entries.map((entry) => resolveMergedToolTrace(
    captured.messagesBySeq[entry.tool_use.seq],
    captured.messagesBySeq[entry.tool_result.seq],
    entry.projection,
  ));

  assert.deepEqual(
    rows.map((row) => ({
      title: row.title,
      summary: row.summary,
      input: row.inputDetail,
      result: row.resultDetail,
      badges: row.badges,
    })),
    [
      {
        title: 'Read',
        summary: '/Users/test/repo/README.md',
        input: '/Users/test/repo/README.md',
        result: 'captured read output',
        badges: ['repo/README.md'],
      },
      {
        title: 'Command',
        summary: '/bin/zsh -lc "git status --short"',
        input: '/bin/zsh -lc "git status --short"',
        result: ' M README.md\n',
        badges: ['exit 0', '7 ms'],
      },
    ],
  );
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

test('Claude command keeps its description in the row and its command in CALL', () => {
  const description = 'Read schema definition';
  const command = "sed -n '5,60p' src/schema.rs";
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: {
        tool: 'Bash',
        input: { description, command },
      },
      toolUseId: 'tool:claude-command',
      toolName: 'Bash',
    },
    undefined,
    {
      tool_name: 'Bash',
      kind: 'command',
      visibility: 'normal',
      summary: {
        root: 'content',
        path: ['input', 'description'],
        format: 'text',
        label: 'call',
      },
      call: {
        root: 'content',
        path: ['input', 'command'],
        format: 'code',
        label: 'command',
      },
    },
  );

  assert.equal(merged.summary, description);
  assert.equal(merged.inputDetail, command);
  assert.equal(merged.inputLabel, 'Command');
});

test('structured Web parameters stay out of the collapsed row summary', () => {
  const parameters = {
    action: null,
    id: 'exec-00000000-0000-0000-0000-000000000001',
    query: '',
    type: 'webSearch',
  };
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: parameters,
      toolUseId: parameters.id,
      toolName: 'webSearch',
    },
    undefined,
    {
      tool_name: 'webSearch',
      kind: 'web',
      visibility: 'normal',
      call: {
        root: 'content',
        format: 'json',
        label: 'parameters',
      },
    },
  );

  assert.equal(merged.summary, undefined);
  assert.ok(merged.inputDetail?.includes('"type": "webSearch"'));
  assert.equal(merged.inputLabel, 'Parameters');
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

test('diff recipes compose Unified and Pair segments in wire order against their declared source', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: {
        unified: '--- a/file\n+++ b/file\n-old\n+new',
        old: 'first\n ',
        inserted: '\nsecond',
        ignored: '+wrong-source',
      },
      toolUseId: 'tool:diff-composition',
      toolName: 'custom_tool',
    },
    {
      role: 'tool_result',
      content: { unified: '+wrong-result' },
      toolUseId: 'tool:diff-composition',
      toolName: 'custom_tool',
    },
    {
      tool_name: 'custom_tool',
      kind: 'generic',
      visibility: 'normal',
      diff: {
        source: 'tool_use',
        segments: [
          { unified: { text: { root: 'content', path: ['unified'] } } },
          {
            pair: {
              old: { root: 'content', path: ['old'] },
              new: { root: 'content', path: ['inserted'] },
            },
          },
          { pair: { old: null, new: { root: 'content', path: ['ignored'] } } },
        ],
      },
    },
  );

  assert.deepEqual(merged.diffLines, [
    { kind: 'context', text: '--- a/file' },
    { kind: 'context', text: '+++ b/file' },
    { kind: 'removed', text: 'old' },
    { kind: 'added', text: 'new' },
    { kind: 'removed', text: 'first' },
    { kind: 'removed', text: ' ' },
    { kind: 'added', text: '' },
    { kind: 'added', text: 'second' },
    { kind: 'added', text: '+wrong-source' },
  ]);
  assert.deepEqual(merged.diffStats, { added: 4, removed: 3 });
});

test('empty operands contribute zero lines while whitespace-only and empty split lines survive', () => {
  const projection = {
    tool_name: 'Edit',
    kind: 'file_edit',
    visibility: 'normal',
    diff: {
      source: 'tool_use',
      segments: [
        {
          pair: {
            old: { root: 'content', path: ['input', 'empty'] },
            new: { root: 'content', path: ['input', 'whitespace'] },
          },
        },
      ],
    },
  };
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { input: { empty: '', whitespace: ' \n' } },
      toolUseId: 'tool:raw-lines',
      toolName: 'Edit',
    },
    undefined,
    projection,
  );
  assert.deepEqual(merged.diffLines, [
    { kind: 'added', text: ' ' },
    { kind: 'added', text: '' },
  ]);
  assert.deepEqual(merged.diffStats, { added: 2, removed: 0 });

  assert.equal(
    resolveMergedToolTrace(undefined, undefined, projection).diffLines,
    undefined,
    'a recipe whose source body is not loaded omits the diff section',
  );
});

test('path-formatted summary feeds File detail and badge but not collapsed summary text', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: {
        input: {
          file_path: '/Users/test/repo/Sample.txt',
          content: 'synthetic body',
        },
      },
      toolUseId: 'tool:path-summary',
      toolName: 'Write',
    },
    undefined,
    {
      tool_name: 'Write',
      kind: 'file_write',
      visibility: 'normal',
      summary: {
        root: 'content',
        path: ['input', 'file_path'],
        format: 'path',
        label: 'file',
      },
      diff: {
        source: 'tool_use',
        segments: [{ pair: { new: { root: 'content', path: ['input', 'content'] } } }],
      },
    },
  );

  assert.equal(merged.summary, undefined);
  assert.deepEqual(merged.badges, ['repo/Sample.txt']);
  assert.equal(merged.pathDetail, '/Users/test/repo/Sample.txt');
  assert.equal(merged.pathLabel, 'File');
  assert.deepEqual(merged.diffStats, { added: 1, removed: 0 });
});

test('unknown scalar format degrades only that field and keeps the row', () => {
  const merged = resolveMergedToolTrace(
    {
      role: 'tool_use',
      content: { value: 'must stay hidden' },
      toolUseId: 'tool:future-format',
      toolName: 'future_tool',
    },
    undefined,
    {
      tool_name: 'future_tool',
      kind: 'generic',
      visibility: 'normal',
      call: {
        root: 'content',
        path: ['value'],
        format: 'future_format',
        label: 'call',
      },
    },
  );

  assert.equal(merged.title, 'Future tool');
  assert.equal(merged.inputDetail, undefined);
  assert.equal(merged.summary, undefined);
});

test('shared captured transcript render state maps Write and Edit bodies end to end', () => {
  const fixtureRoot = path.resolve(process.cwd(), '../../test-fixtures/render-layer');
  const records = fs.readFileSync(
    path.join(fixtureRoot, 'file-change-body-transcript.jsonl'),
    'utf8',
  )
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line));
  const snapshot = JSON.parse(fs.readFileSync(
    path.join(fixtureRoot, 'file-change-body-render-state.json'),
    'utf8',
  ));
  const messagesBySeq = new Map(records.map((record) => [record.seq, record.message]));
  const entries = snapshot.rows[0].activity[0].steps[0].entries;
  const rows = entries.map((entry) => resolveMergedToolTrace(
    messagesBySeq.get(entry.tool_use.seq),
    messagesBySeq.get(entry.tool_result.seq),
    entry.projection,
  ));

  assert.deepEqual(rows.map((row) => ({
    title: row.title,
    path: row.pathDetail,
    diff: row.diffLines,
    stats: row.diffStats,
    result: row.resultDetail,
  })), [
    {
      title: 'Write',
      path: '/Users/test/repo/Sample.txt',
      diff: [
        { kind: 'added', text: 'alpha' },
        { kind: 'added', text: 'beta' },
        { kind: 'added', text: '' },
      ],
      stats: { added: 3, removed: 0 },
      result: 'File created successfully at: /Users/test/repo/Sample.txt',
    },
    {
      title: 'Edit',
      path: '/Users/test/repo/Sample.txt',
      diff: [
        { kind: 'removed', text: 'alpha' },
        { kind: 'removed', text: 'beta' },
        { kind: 'removed', text: '' },
        { kind: 'added', text: 'alpha' },
        { kind: 'added', text: 'gamma' },
        { kind: 'added', text: '' },
      ],
      stats: { added: 3, removed: 3 },
      result: 'The file /Users/test/repo/Sample.txt has been updated successfully.',
    },
  ]);
});
