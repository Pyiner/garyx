import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildAgentAndTeamOptions,
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
  ], []);

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

test('composer and automation agent options omit provider detail', () => {
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
  const teams = [{
    teamId: 'test-team',
    displayName: 'Test Team',
    avatarDataUrl: null,
    leaderAgentId: 'test-reviewer',
    memberAgentIds: ['test-reviewer'],
    workflowText: '',
    createdAt: '',
    updatedAt: '',
  }];

  const composerOptions = buildAgentOptions(agents, teams);
  const automationOptions = buildAgentAndTeamOptions(agents, teams, {
    agentLabelStyle: 'target',
    teamDetail: 'Team',
    teamLabelStyle: 'target',
    teamsFirst: true,
  });

  const composerAgentRows = composerOptions.filter((option) => option.kind !== 'team');
  const automationAgentRows = automationOptions.filter((option) => option.kind !== 'team');

  assert(composerAgentRows.every((option) => option.detail === undefined));
  assert(automationAgentRows.every((option) => option.detail === undefined));
  assert.equal(
    composerOptions.find((option) => option.kind === 'team')?.detail,
    'Lead: Test Reviewer',
  );
  assert.equal(
    automationOptions.find((option) => option.kind === 'team')?.detail,
    'Team',
  );
});
