import assert from 'node:assert/strict';
import test from 'node:test';

import { addChannelAccount } from './channel-setup.ts';
import { setGatewayFetch } from './gary-client.ts';

const settings = {
  gatewayUrl: 'https://gateway.example.test',
  gatewayAuthToken: '',
};

function response(payload) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

test('Add Bot follow-global input is saved without materializing Claude', async () => {
  let savedConfig = null;
  setGatewayFetch(async (url, init) => {
    const path = new URL(String(url)).pathname;
    if (path.endsWith('/validate_account')) {
      return response({ ok: true, validated: true });
    }
    if (path === '/api/settings' && init?.method === 'PUT') {
      savedConfig = JSON.parse(String(init.body));
      return response({ ok: true });
    }
    return response({ config: savedConfig || { channels: {} } });
  });
  try {
    await addChannelAccount(settings, {
      channel: 'test-channel',
      accountId: 'test-account',
      name: 'Test Bot',
      workspaceDir: null,
      workspaceMode: 'local',
      agentId: null,
      config: {},
    });
  } finally {
    setGatewayFetch(null);
  }

  const account = savedConfig.channels['test-channel'].accounts['test-account'];
  assert.equal(Object.hasOwn(account, 'agent_id'), false);
});
