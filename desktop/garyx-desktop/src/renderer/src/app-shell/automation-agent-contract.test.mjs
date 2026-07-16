import assert from 'node:assert/strict';
import test from 'node:test';

import {
  automationAgentIdForMutation,
  generatedAutomationAgentError,
  initialAutomationAgentId,
} from './automation-agent-contract.ts';

function draft(overrides = {}) {
  return {
    agentId: 'codex',
    agentChanged: false,
    initialTargetMode: 'new_thread',
    targetMode: 'new_thread',
    ...overrides,
  };
}

test('generated create sends its effective-default selection', () => {
  const selected = initialAutomationAgentId({
    targetMode: 'new_thread',
    effectiveDefaultAgentId: 'codex',
  });
  const value = draft({ agentId: selected });
  assert.equal(generatedAutomationAgentError('create', value, new Set(['codex'])), null);
  assert.equal(automationAgentIdForMutation('create', value), 'codex');
});

test('target-existing draft displays the wire-derived thread agent, not the global default', () => {
  assert.equal(initialAutomationAgentId({
    targetMode: 'existing_thread',
    targetEffectiveAgentId: 'traex',
    effectiveDefaultAgentId: 'codex',
  }), 'traex');
});

test('editing a generated automation preserves an unavailable current agent when unchanged', () => {
  const value = draft({ agentId: 'disabled-agent' });
  assert.equal(generatedAutomationAgentError('edit', value, new Set()), null);
  assert.equal(automationAgentIdForMutation('edit', value), undefined);
});

test('explicit agent reselection is the only edit path that sends agentId', () => {
  const value = draft({ agentChanged: true });
  assert.equal(generatedAutomationAgentError('edit', value, new Set(['codex'])), null);
  assert.equal(automationAgentIdForMutation('edit', value), 'codex');
});

test('target-existing remains valid with all agents disabled while unsafe conversion is blocked', () => {
  const target = draft({
    agentId: '',
    initialTargetMode: 'existing_thread',
    targetMode: 'existing_thread',
  });
  assert.equal(generatedAutomationAgentError('create', target, new Set()), null);
  assert.equal(automationAgentIdForMutation('create', target), undefined);

  const converted = { ...target, targetMode: 'new_thread', agentId: 'disabled-agent' };
  assert.equal(
    generatedAutomationAgentError('edit', converted, new Set()),
    'Choose an agent for this automation.',
  );
});
