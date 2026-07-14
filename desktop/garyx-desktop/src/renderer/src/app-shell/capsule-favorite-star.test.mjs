import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const appShellDir = path.dirname(fileURLToPath(import.meta.url));
const rendererDir = path.resolve(appShellDir, '..');
const read = (relativePath) => readFileSync(path.join(rendererDir, relativePath), 'utf8');

test('Capsule favorite star wraps lucide with semantic gradient tokens', () => {
  const component = read('app-shell/components/CapsuleFavoriteStar.tsx');
  const panel = read('app-shell/components/CapsulesPanel.tsx');
  const baseCss = read('styles/base.css');
  const capsulesCss = read('styles/capsules.css');

  assert.match(component, /import \{ Star \} from 'lucide-react'/);
  assert.match(component, /<linearGradient/);
  assert.match(component, /var\(--color-capsule-favorite-stroke\)/);
  assert.match(panel, /<CapsuleFavoriteStar favorited=\{favorited\}/);
  for (const token of [
    '--color-capsule-favorite-gold-top: #ffe082;',
    '--color-capsule-favorite-gold-bottom: #f5a623;',
    '--color-capsule-favorite-stroke: #d68910;',
    '--color-capsule-favorite-glow: rgba(245, 166, 35, 0.45);',
  ]) {
    assert.ok(baseCss.includes(token), `missing ${token}`);
  }
  assert.match(capsulesCss, /drop-shadow\(0 0 5px var\(--color-capsule-favorite-glow\)\)/);
  assert.match(capsulesCss, /@media \(prefers-reduced-motion: reduce\)/);
});
