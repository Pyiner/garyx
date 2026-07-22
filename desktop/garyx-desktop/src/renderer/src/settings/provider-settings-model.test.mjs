import test from 'node:test';
import assert from 'node:assert/strict';

import {
  applyProviderCatalogDefaults,
  applyProviderConfigDraftToGatewayConfig,
  emptyModelProviderConfigDraft,
  fixedModelProviderRow,
  modelProviderDraftFromState,
  sanitizedServiceTier,
  serviceTierOptionsForModel,
} from './provider-settings-model.ts';

const FAST_TIER = { id: 'priority', label: 'Fast', description: null, recommended: false };

function codexCatalog(overrides = {}) {
  return {
    providerType: 'codex_app_server',
    supportsModelSelection: true,
    supportsReasoningEffortSelection: true,
    supportsServiceTierSelection: true,
    defaultModel: 'test-model-a',
    models: [
      {
        id: 'test-model-a',
        label: 'Test Model A',
        recommended: true,
        supportedReasoningEfforts: [
          { id: 'medium', label: 'Medium', recommended: true },
          { id: 'high', label: 'High', recommended: false },
        ],
        serviceTiers: [FAST_TIER],
      },
      {
        id: 'test-model-b',
        label: 'Test Model B',
        recommended: false,
        supportedReasoningEfforts: [
          { id: 'medium', label: 'Medium', recommended: true },
        ],
        serviceTiers: [],
      },
    ],
    reasoningEfforts: [
      { id: 'medium', label: 'Medium', recommended: true },
      { id: 'high', label: 'High', recommended: false },
    ],
    serviceTiers: [FAST_TIER],
    ...overrides,
  };
}

function codexDraft(overrides = {}) {
  return {
    ...emptyModelProviderConfigDraft('codex_app_server'),
    ...overrides,
  };
}

test('provider draft reads the configured service tier', () => {
  const draft = modelProviderDraftFromState('codex_app_server', {
    agents: {
      codex: {
        provider_type: 'codex_app_server',
        default_model: 'test-model-a',
        model_reasoning_effort: 'high',
        model_service_tier: 'priority',
      },
    },
  });
  assert.equal(draft.model, 'test-model-a');
  assert.equal(draft.modelReasoningEffort, 'high');
  assert.equal(draft.modelServiceTier, 'priority');
});

test('empty provider draft has no service tier', () => {
  assert.equal(emptyModelProviderConfigDraft('codex_app_server').modelServiceTier, '');
});

test('Grok is a first-class provider settings row', () => {
  const row = fixedModelProviderRow('grok_build');
  assert.deepEqual(
    { agentId: row.agentId, label: row.label, providerType: row.providerType },
    { agentId: 'grok', label: 'Grok', providerType: 'grok_build' },
  );
  const gatewayConfig = { agents: { grok: { grok_bin: '/usr/local/bin/grok' } } };
  applyProviderConfigDraftToGatewayConfig(
    gatewayConfig,
    row,
    { ...emptyModelProviderConfigDraft('grok_build'), model: 'grok-test', modelReasoningEffort: 'high' },
  );
  assert.equal(gatewayConfig.agents.grok.provider_type, 'grok_build');
  assert.equal(gatewayConfig.agents.grok.default_model, 'grok-test');
  assert.equal(gatewayConfig.agents.grok.grok_bin, '/usr/local/bin/grok');
});

test('saving the provider dialog persists the selected service tier', () => {
  const gatewayConfig = { agents: {} };
  applyProviderConfigDraftToGatewayConfig(
    gatewayConfig,
    fixedModelProviderRow('codex_app_server'),
    codexDraft({ model: 'test-model-a', modelReasoningEffort: 'high', modelServiceTier: 'priority' }),
  );
  assert.deepEqual(gatewayConfig.agents.codex, {
    provider_type: 'codex_app_server',
    default_model: 'test-model-a',
    model_reasoning_effort: 'high',
    model_service_tier: 'priority',
  });
});

test('saving the provider dialog round-trips an already configured service tier', () => {
  // Regression: the old save path deleted model_service_tier outright, so any
  // configured tier was wiped by an unrelated Save from the Configure dialog.
  const gatewayConfig = {
    agents: {
      codex: {
        provider_type: 'codex_app_server',
        default_model: 'test-model-a',
        model_reasoning_effort: 'high',
        model_service_tier: 'priority',
        extra_key: 'preserved',
      },
    },
  };
  const draft = modelProviderDraftFromState('codex_app_server', gatewayConfig);
  applyProviderConfigDraftToGatewayConfig(
    gatewayConfig,
    fixedModelProviderRow('codex_app_server'),
    draft,
  );
  assert.equal(gatewayConfig.agents.codex.model_service_tier, 'priority');
  assert.equal(gatewayConfig.agents.codex.extra_key, 'preserved');
});

test('saving the provider dialog retires the legacy model key', () => {
  const gatewayConfig = { agents: { codex: { model: 'legacy-model' } } };
  applyProviderConfigDraftToGatewayConfig(
    gatewayConfig,
    fixedModelProviderRow('codex_app_server'),
    codexDraft({ model: 'test-model-a' }),
  );
  assert.equal('model' in gatewayConfig.agents.codex, false);
  assert.equal(gatewayConfig.agents.codex.default_model, 'test-model-a');
});

test('saving claude keeps CLI mode and drops an empty CLI path', () => {
  const gatewayConfig = { agents: { claude: { claude_cli_path: '/tmp/old-cli' } } };
  applyProviderConfigDraftToGatewayConfig(
    gatewayConfig,
    fixedModelProviderRow('claude_code'),
    { ...emptyModelProviderConfigDraft('claude_code'), claudeCliMode: 'cctty' },
  );
  assert.equal(gatewayConfig.agents.claude.claude_cli_mode, 'cctty');
  assert.equal('claude_cli_path' in gatewayConfig.agents.claude, false);
});

test('catalog defaults keep a service tier the selected model supports', () => {
  const draft = applyProviderCatalogDefaults(
    codexDraft({ model: 'test-model-a', modelReasoningEffort: 'high', modelServiceTier: 'priority' }),
    codexCatalog(),
  );
  assert.equal(draft.modelServiceTier, 'priority');
});

test('catalog defaults preserve a service tier the catalog does not know', () => {
  // The CLI accepts arbitrary --service-tier strings; opening the dialog
  // (catalog hydrate) must not rewrite them.
  const draft = applyProviderCatalogDefaults(
    codexDraft({ model: 'test-model-a', modelServiceTier: 'flex' }),
    codexCatalog(),
  );
  assert.equal(draft.modelServiceTier, 'flex');
});

test('a catalog-unknown configured tier round-trips through open-dialog and save', () => {
  // Regression (review #TASK-2410): catalog hydrate used to sanitize the
  // tier, so opening the Configure dialog and saving silently cleared a
  // CLI-configured tier the catalog does not enumerate.
  const gatewayConfig = {
    agents: {
      codex: {
        provider_type: 'codex_app_server',
        default_model: 'test-model-a',
        model_reasoning_effort: 'high',
        model_service_tier: 'flex',
      },
    },
  };
  const opened = applyProviderCatalogDefaults(
    modelProviderDraftFromState('codex_app_server', gatewayConfig),
    codexCatalog(),
  );
  assert.equal(opened.modelServiceTier, 'flex');
  applyProviderConfigDraftToGatewayConfig(
    gatewayConfig,
    fixedModelProviderRow('codex_app_server'),
    opened,
  );
  assert.equal(gatewayConfig.agents.codex.model_service_tier, 'flex');
});

test('catalog defaults keep the draft untouched while the catalog is missing', () => {
  const draft = codexDraft({ model: 'test-model-a', modelServiceTier: 'priority' });
  assert.equal(applyProviderCatalogDefaults(draft, null), draft);
});

test('provider default model keeps a provider-level service tier', () => {
  const draft = applyProviderCatalogDefaults(
    codexDraft({ model: '', modelServiceTier: 'priority' }),
    codexCatalog(),
  );
  assert.equal(draft.model, '');
  assert.equal(draft.modelServiceTier, 'priority');
});

test('sanitized service tier falls back to provider-level tiers for a model without tiers', () => {
  // Mirrors the composer/agent-dialog fallback: an empty per-model tier list
  // defers to the provider-level tier list.
  assert.equal(sanitizedServiceTier(codexCatalog(), 'test-model-b', 'priority'), 'priority');
});

test('sanitized service tier clears when the target model does not support it', () => {
  const catalog = codexCatalog({ serviceTiers: [] });
  assert.equal(sanitizedServiceTier(catalog, 'test-model-b', 'priority'), '');
});

test('service tier options include an unknown configured tier for display', () => {
  const options = serviceTierOptionsForModel(codexCatalog(), 'test-model-a', 'retired-tier');
  assert.deepEqual(options.map((option) => option.id), ['priority', 'retired-tier']);
});

test('service tier options come from the catalog without a configured tier', () => {
  const options = serviceTierOptionsForModel(codexCatalog(), 'test-model-a', '');
  assert.deepEqual(options.map((option) => option.id), ['priority']);
});
