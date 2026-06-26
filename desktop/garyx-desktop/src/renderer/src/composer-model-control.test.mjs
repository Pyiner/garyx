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
  assert.equal(state.triggerLabel, 'Claude Opus 4.8 · High');
});

test('default catalog model supplies its full reasoning effort menu before override', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: null,
  });

  assert.equal(state.effectiveModelId, '');
  assert.deepEqual(
    state.reasoningEfforts.map((option) => option.id),
    ['low', 'medium', 'high', 'xhigh', 'max'],
  );
});

test('default catalog model labels the trigger with its default reasoning effort', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: null,
  });

  assert.equal(state.effectiveModelId, '');
  assert.equal(state.defaultReasoningEffortId, 'high');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8 · High');
});

test('default catalog model prefers supported provider default reasoning effort', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
      defaultReasoningEffort: 'max',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: null,
  });

  assert.equal(state.effectiveModelId, '');
  assert.equal(state.defaultReasoningEffortId, 'max');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8 · Max');
});

test('empty provider default reasoning effort falls back to model default', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
      defaultReasoningEffort: '  ',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: null,
  });

  assert.equal(state.defaultReasoningEffortId, 'high');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8 · High');
});

test('unsupported provider default reasoning effort falls back to model default', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-haiku-4-5',
      defaultReasoningEffort: 'max',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: null,
  });

  assert.equal(state.defaultReasoningEffortId, 'high');
  assert.equal(state.triggerLabel, 'Claude Haiku 4.5 · High');
});

test('selected reasoning effort labels trigger before provider default', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      defaultModel: 'claude-opus-4-8',
      defaultReasoningEffort: 'max',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: 'high',
  });

  assert.equal(state.defaultReasoningEffortId, 'max');
  assert.equal(state.effectiveReasoningEffortId, 'high');
  assert.equal(state.triggerLabel, 'Claude Opus 4.8 · High');
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

test('selected haiku model keeps its three reasoning efforts without default suffix', () => {
  const state = resolve({ selectedModel: 'claude-haiku-4-5' });

  assert.equal(state.effectiveModelId, 'claude-haiku-4-5');
  assert.equal(state.triggerLabel, 'Claude Haiku 4.5');
  assert.deepEqual(
    state.reasoningEfforts.map((option) => option.id),
    ['low', 'medium', 'high'],
  );
});

test('selected sonnet model keeps its four reasoning efforts without default suffix', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      models: [
        ...providerModels.models,
        {
          id: 'claude-sonnet-4-6',
          label: 'Claude Sonnet 4.6',
          recommended: false,
          defaultReasoningEffort: 'high',
          supportedReasoningEfforts: [
            { id: 'low', label: 'Low', recommended: false },
            { id: 'medium', label: 'Medium', recommended: false },
            { id: 'high', label: 'High', recommended: true },
            { id: 'max', label: 'Max', recommended: false },
          ],
          serviceTiers: [],
        },
      ],
    },
    selectedModel: 'claude-sonnet-4-6',
  });

  assert.equal(state.effectiveModelId, 'claude-sonnet-4-6');
  assert.equal(state.triggerLabel, 'Claude Sonnet 4.6');
  assert.deepEqual(
    state.reasoningEfforts.map((option) => option.id),
    ['low', 'medium', 'high', 'max'],
  );
});

test('default catalog model without reasoning efforts does not add a trigger suffix', () => {
  const state = resolve({
    providerModels: {
      ...providerModels,
      models: [
        {
          id: 'model-without-effort',
          label: 'Model Without Effort',
          recommended: false,
          supportedReasoningEfforts: [],
          serviceTiers: [],
        },
      ],
      reasoningEfforts: [],
      defaultModel: 'model-without-effort',
    },
    agentConfiguredModel: null,
    effectiveModel: null,
    selectedModel: null,
    effectiveReasoningEffort: null,
    selectedReasoningEffort: null,
  });

  assert.equal(state.effectiveModelId, '');
  assert.equal(state.defaultReasoningEffortId, '');
  assert.equal(state.triggerLabel, 'Model Without Effort');
  assert.deepEqual(
    state.reasoningEfforts.map((option) => option.id),
    [],
  );
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
