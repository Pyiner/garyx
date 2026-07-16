import assert from 'node:assert/strict';
import test from 'node:test';

import { sideChatForkAgentId } from './side-chat-ops.ts';

test('side-chat fork preserves a canonical source binding', () => {
  assert.equal(sideChatForkAgentId({ agentId: ' codex ' }), 'codex');
});

test('side-chat legacy fork leaves a missing source agent unspecified', () => {
  assert.equal(sideChatForkAgentId({ agentId: null }), null);
  assert.equal(sideChatForkAgentId({ agentId: '   ' }), null);
  assert.equal(sideChatForkAgentId(null), null);
});
