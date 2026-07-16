import assert from 'node:assert/strict';
import test from 'node:test';

import {
  agentManagementActionState,
  defaultBadgeForAgent,
  isNewDraftBindingBlocked,
  suggestedAgentId,
} from './agent-availability-model.ts';

const agent = (agentId, enabled = true) => ({ agentId, enabled, standalone: true });

test('default badges cover configured, inactive, acting, auto, and all-disabled states', () => {
  assert.equal(
    defaultBadgeForAgent(
      { defaultAgentId: 'codex', effectiveDefaultAgentId: 'codex' },
      agent('codex'),
    ),
    'default',
  );
  assert.equal(
    defaultBadgeForAgent(
      { defaultAgentId: 'codex', effectiveDefaultAgentId: 'claude' },
      agent('codex', false),
    ),
    'default-inactive',
  );
  assert.equal(
    defaultBadgeForAgent(
      { defaultAgentId: 'codex', effectiveDefaultAgentId: 'claude' },
      agent('claude'),
    ),
    'acting-default',
  );
  assert.equal(
    defaultBadgeForAgent(
      { defaultAgentId: null, effectiveDefaultAgentId: 'traex' },
      agent('traex'),
    ),
    'default-auto',
  );
  assert.equal(
    defaultBadgeForAgent(
      { defaultAgentId: null, effectiveDefaultAgentId: null },
      agent('traex', false),
    ),
    null,
  );
});

test('disabled management rows cannot chat or set default', () => {
  assert.deepEqual(
    agentManagementActionState({ defaultAgentId: 'claude' }, agent('codex', false)),
    { chatEnabled: false, setDefaultVisible: false },
  );
  assert.deepEqual(
    agentManagementActionState({ defaultAgentId: 'claude' }, agent('codex', true)),
    { chatEnabled: true, setDefaultVisible: true },
  );
  assert.deepEqual(
    agentManagementActionState(
      { defaultAgentId: 'claude' },
      { ...agent('worker', true), standalone: false },
    ),
    { chatEnabled: false, setDefaultVisible: false },
  );
});

test('all-disabled empty state blocks only a new draft, not an existing thread', () => {
  assert.equal(isNewDraftBindingBlocked(true, null), true);
  assert.equal(isNewDraftBindingBlocked(false, null), false);
  assert.equal(isNewDraftBindingBlocked(true, agent('claude', true)), false);
  assert.equal(suggestedAgentId({ effectiveDefaultAgentId: null }), null);
  assert.equal(suggestedAgentId({ effectiveDefaultAgentId: 'codex' }), 'codex');
});
