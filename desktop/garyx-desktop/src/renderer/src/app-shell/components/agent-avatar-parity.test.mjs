import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import test from 'node:test';

import {
  AVATAR_STYLE_OPTIONS,
  buildAgentAvatarPrompt,
} from '../../../../shared/agent-avatar-prompt.ts';
import { deriveId } from './agents-hub-helpers.ts';

async function fixture() {
  const path = resolve(
    process.cwd(),
    '../../mobile/garyx-mobile/Tests/GaryxMobileCoreTests/Fixtures/agent-avatar-parity.json',
  );
  return JSON.parse(await readFile(path, 'utf8'));
}

test('TypeScript deriveId matches shared golden cases', async () => {
  const value = await fixture();
  for (const item of value.deriveIdCases) {
    assert.equal(deriveId(item.name), item.expected, JSON.stringify(item.name));
  }
});

test('TypeScript style catalog matches the Swift parity fixture', async () => {
  const value = await fixture();
  assert.deepEqual(AVATAR_STYLE_OPTIONS, value.styles);
});

test('TypeScript prompt composition matches the Swift parity fixture', async () => {
  const value = await fixture();
  for (const item of value.promptCases) {
    assert.equal(buildAgentAvatarPrompt({
      agentId: item.identifier,
      displayName: item.displayName,
      stylePrompt: item.stylePrompt,
    }), item.expected);
  }
});
