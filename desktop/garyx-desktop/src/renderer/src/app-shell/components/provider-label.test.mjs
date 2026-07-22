import assert from 'node:assert/strict';
import test from 'node:test';

import { providerLabel } from './agents-hub-helpers.ts';

test('provider labels share one explicit presentation mapping', () => {
  assert.deepEqual(
    [
      'claude_code',
      'codex_app_server',
      'antigravity',
      'traex',
      'grok_build',
      'gemini',
    ].map((provider) => providerLabel(provider)),
    ['Claude Code', 'Codex', 'Antigravity', 'Traex', 'Grok', 'Gemini'],
  );
});
