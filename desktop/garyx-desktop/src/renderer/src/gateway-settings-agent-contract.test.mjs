import assert from 'node:assert/strict';
import test from 'node:test';

import {
  defaultApiAccount,
  defaultChannelAgentId,
  defaultFeishuAccount,
  defaultTelegramAccount,
  ensureGatewayConfig,
  stringifyJsonBlock,
} from './gateway-settings.ts';

test('channel account defaults represent follow-global without materializing Claude', () => {
  assert.equal(defaultChannelAgentId(), null);
  assert.equal(defaultApiAccount().agent_id, null);
  assert.equal(defaultTelegramAccount().agent_id, null);
  assert.equal(defaultFeishuAccount().agent_id, null);

  const config = ensureGatewayConfig({
    channels: {
      api: { accounts: { inherited: {} } },
      telegram: {
        accounts: {
          inherited: { config: {} },
          explicit: { agent_id: 'claude', config: {} },
          blank: { agent_id: '  ', config: {} },
        },
      },
    },
  });
  assert.equal(config.channels.api.accounts.inherited.agent_id, null);
  assert.equal(config.channels.telegram.accounts.inherited.agent_id, null);
  assert.equal(config.channels.telegram.accounts.explicit.agent_id, 'claude');
  assert.equal(config.channels.telegram.accounts.blank.agent_id, null);
});

test('gateway settings JSON preserves explicit Claude and omits inheritance', () => {
  const serialized = JSON.parse(stringifyJsonBlock({
    channels: {
      api: {
        accounts: {
          explicit: { agent_id: 'claude' },
          inherited: { agent_id: null },
        },
      },
      telegram: {
        accounts: {
          explicit: { agent_id: 'claude' },
          inherited: { agent_id: null },
        },
      },
    },
  }));

  for (const channel of ['api', 'telegram']) {
    const accounts = serialized.channels[channel].accounts;
    assert.equal(accounts.explicit.agent_id, 'claude');
    assert.equal(Object.hasOwn(accounts.inherited, 'agent_id'), false);
  }
});
