import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildAgentOptions,
  buildAgentTargetOptions,
} from './agent-options.ts';

function agent(overrides) {
  return {
    agentId: 'planner',
    displayName: 'Planner',
    providerType: 'claude_code',
    builtIn: false,
    standalone: true,
    model: '',
    systemPrompt: '',
    defaultWorkspaceDir: '',
    avatarDataUrl: null,
    ...overrides,
  };
}

test('agent target labels do not append provider names', () => {
  const options = buildAgentTargetOptions([
    agent({
      agentId: 'test-planner',
      displayName: 'Test Planner',
      providerType: 'claude_code',
    }),
    agent({
      agentId: 'test-writer',
      displayName: 'Test Writer',
      providerType: 'codex_app_server',
    }),
  ]);

  assert.deepEqual(
    options.map((option) => ({
      label: option.label,
      detail: option.detail,
    })),
    [
      { label: 'Test Planner (test-planner)', detail: undefined },
      { label: 'Test Writer (test-writer)', detail: undefined },
    ],
  );
  assert(!options.some((option) => option.label.includes(' · ')));
});

test('composer agent options omit provider detail', () => {
  const agents = [
    agent({
      agentId: 'claude',
      displayName: 'Claude',
      providerType: 'claude_code',
      builtIn: true,
    }),
    agent({
      agentId: 'test-reviewer',
      displayName: 'Test Reviewer',
      providerType: 'codex_app_server',
    }),
  ];
  const composerOptions = buildAgentOptions(agents);

  assert(composerOptions.every((option) => option.detail === undefined));
});
