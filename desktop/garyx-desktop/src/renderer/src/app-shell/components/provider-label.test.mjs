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
      'gemini',
    ].map((provider) => providerLabel(provider)),
    ['Claude', 'Codex', 'Antigravity', 'Traex', 'Gemini'],
  );
});
