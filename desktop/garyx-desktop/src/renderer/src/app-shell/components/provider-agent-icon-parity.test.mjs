import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import test from 'node:test';

const providers = [
  { appAsset: 'claude', modelAsset: 'claude', mobileAsset: 'ProviderClaude' },
  { appAsset: 'codex', modelAsset: 'codex', mobileAsset: 'ProviderCodex' },
  {
    appAsset: 'antigravity',
    modelAsset: 'gemini',
    mobileAsset: 'ProviderAntigravity',
  },
  { appAsset: 'trae', modelAsset: 'trae', mobileAsset: 'ProviderTrae' },
  { appAsset: 'grok', modelAsset: 'grok', mobileAsset: 'ProviderGrok' },
];

test('Mac and iOS Provider avatars match the built-in Agent artwork', async () => {
  for (const provider of providers) {
    const modelAsset = await readFile(
      resolve(
        process.cwd(),
        `../../garyx-models/assets/builtin_agent_avatars/${provider.modelAsset}.png`,
      ),
    );
    const desktopAsset = await readFile(
      resolve(
        process.cwd(),
        `src/renderer/src/assets/provider-avatars/${provider.appAsset}.png`,
      ),
    );
    const mobileAsset = await readFile(
      resolve(
        process.cwd(),
        `../../mobile/garyx-mobile/App/GaryxMobile/Assets.xcassets/${provider.mobileAsset}.imageset/provider-${provider.appAsset}.png`,
      ),
    );

    assert.deepEqual(
      desktopAsset,
      modelAsset,
      `${provider.appAsset} desktop avatar drifted`,
    );
    assert.deepEqual(
      mobileAsset,
      modelAsset,
      `${provider.appAsset} mobile avatar drifted`,
    );
  }
});
