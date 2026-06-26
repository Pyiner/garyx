import test from 'node:test';
import assert from 'node:assert/strict';

import { resolveComposerModelControlState } from './composer-model-control.ts';

const providerModels = {
  providerType: 'claude_code',
  supportsModelSelection: true,
  models: [
    {
      id: 'claude-opus-4-8',
      label: 'Claude Opus 4.8',
      recommended: false,
      defaultReasoningEffort: 'high',
      supportedReasoningEfforts: [
        { id: 'low', label: 'Low', recommended: false },
        { id: 'medium', label: 'Medium', recommended: false },
        { id: 'high', label: 'High', recommended: true },
        { id: 'xhigh', label: 'Extra High', recommended: false },
        { id: 'max', label: 'Max', recommended: false },
      ],
      serviceTiers: [],
    },
    {
      id: 'claude-haiku-4-5',
      label: 'Claude Haiku 4.5',
      recommended: false,
      defaultReasoningEffort: 'high',
      supportedReasoningEfforts: [
        { id: 'low', label: 'Low', recommended: false },
        { id: 'medium', label: 'Medium', recommended: false },
        { id: 'high', label: 'High', recommended: true },
      ],
      serviceTiers: [],
    },
  ],
  supportsReasoningEffortSelection: true,
  reasoningEfforts: [
    { id: 'low', label: 'Low', recommended: false },
    { id: 'medium', label: 'Medium', recommended: false },
    { id: 'high', label: 'High', recommended: true },
  ],
  supportsServiceTierSelection: false,
  serviceTiers: [],
  defaultModel: null,
  source: 'claude_code_builtin',
};

function resolve(overrides = {}) {
  return resolveComposerModelControlState({
    providerModels,
    modelFallbackLabel: 'Model',
    thinkingLevelFallbackLabel: 'Thinking level',
    standardServiceTierLabel: 'Standard',
    ...overrides,
  });
}

test('explicit model override is not treated as the default reset row', () => {
  const state = resolve({ selectedModel: 'claude-opus-4-8' });

  assert.equal(state.effectiveModelId, 'claude-opus-4-8');
  assert.equal(state.defaultModelId, '');
  assert.equal(state.defaultModelLabel, 'Model');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8');
  assert.deepEqual(
    state.models.map((option) => option.id),
    ['claude-opus-4-8', 'claude-haiku-4-5'],
  );
  assert.deepEqual(
    state.reasoningEfforts.map((option) => option.id),
    ['low', 'medium', 'high', 'xhigh', 'max'],
  );
});

test('effective model can still act as default when no override is selected', () => {
  const state = resolve({ effectiveModel: 'claude-opus-4-8' });

  assert.equal(state.effectiveModelId, 'claude-opus-4-8');
  assert.equal(state.defaultModelId, 'claude-opus-4-8');
  assert.equal(state.defaultModelLabel, 'Claude Opus 4.8');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8');
});

test('default catalog model labels the trigger before the user selects an override', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
  });

  assert.equal(state.effectiveModelId, '');
  assert.equal(state.defaultModelId, 'claude-opus-4-8');
  assert.equal(state.defaultModelLabel, 'Claude Opus 4.8');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8');
});

test('default catalog model keeps the selected reasoning effort suffix', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    selectedReasoningEffort: 'high',
  });

  assert.equal(state.effectiveModelId, '');
  assert.equal(state.effectiveReasoningEffortId, 'high');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8 · High');
});

test('model-less Claude Code menu keeps provider-level reasoning intersection', () => {
  const state = resolve();

  assert.equal(state.effectiveModelId, '');
  assert.equal(state.defaultModelId, '');
  assert.equal(state.triggerLabel, 'Model');
  assert.deepEqual(
    state.reasoningEfforts.map((option) => option.id),
    ['low', 'medium', 'high'],
  );
});
