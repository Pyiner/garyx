import assert from 'node:assert/strict';
import test from 'node:test';

import {
  channelAgentIdFromSelectValue,
  channelAgentSelectValue,
  explicitChannelAgentUnavailable,
  FOLLOW_GLOBAL_AGENT_SELECT_VALUE,
  suggestedChannelAgentId,
} from './channel-agent-selection.ts';

const target = (value) => ({ id: value, value, label: value, kind: 'builtin' });

test('AddBotDialog suggests the server effective default and supports follow-global', () => {
  const targets = [target('claude'), target('codex')];
  assert.equal(suggestedChannelAgentId(targets, 'codex'), 'codex');
  assert.equal(channelAgentSelectValue(null), FOLLOW_GLOBAL_AGENT_SELECT_VALUE);
  assert.equal(channelAgentIdFromSelectValue(FOLLOW_GLOBAL_AGENT_SELECT_VALUE), null);
});

test('AddBotDialog keeps explicit Claude distinct from follow-global', () => {
  assert.equal(channelAgentSelectValue('claude'), 'claude');
  assert.equal(channelAgentIdFromSelectValue('claude'), 'claude');
});

test('AddBotDialog all-disabled and disabled-current states stay repairable', () => {
  assert.equal(suggestedChannelAgentId([], null), null);
  assert.equal(explicitChannelAgentUnavailable([], null), false);
  assert.equal(explicitChannelAgentUnavailable([], 'disabled-agent'), true);
});
