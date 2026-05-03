import test from 'node:test';
import assert from 'node:assert/strict';

import { shouldRenderToolTraceMessage } from './tool-trace-registry.ts';

test('suppresses Codex reasoning tool traces in desktop rendering', () => {
  assert.equal(
    shouldRenderToolTraceMessage({
      role: 'tool_use',
      text: '',
      content: {
        type: 'reasoning',
        summary: ['Checked current state before acting.'],
      },
      metadata: {
        source: 'codex_app_server',
      },
    }),
    false,
  );
});

test('keeps non-reasoning Codex tool traces visible', () => {
  assert.equal(
    shouldRenderToolTraceMessage({
      role: 'tool_use',
      text: '',
      content: {
        type: 'commandExecution',
        command: 'git status --short',
        status: 'in_progress',
      },
      metadata: {
        source: 'codex_app_server',
      },
    }),
    true,
  );
});

test('keeps non-Codex reasoning-like tool traces visible', () => {
  assert.equal(
    shouldRenderToolTraceMessage({
      role: 'tool_use',
      text: '',
      content: {
        tool: 'reasoning',
        input: {},
      },
      metadata: {
        source: 'claude_sdk',
      },
    }),
    true,
  );
});
