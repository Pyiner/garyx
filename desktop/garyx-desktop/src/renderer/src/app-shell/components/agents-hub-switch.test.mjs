import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const panelSource = readFileSync(
  new URL('./AgentsHubPanel.tsx', import.meta.url),
  'utf8',
);

function firstSelfClosingTag(source, tagName) {
  const start = source.indexOf(`<${tagName}`);
  assert.notEqual(start, -1, `expected <${tagName}> in source`);
  const end = source.indexOf('/>', start);
  assert.notEqual(end, -1, `expected <${tagName}> to be self-closing`);
  return source.slice(start, end + 2);
}

test('agent availability switch stops row propagation without cancelling the Radix click', () => {
  const switchSource = firstSelfClosingTag(panelSource, 'Switch');

  assert.match(
    switchSource,
    /onClick=\{\(event\)\s*=>\s*\{\s*event\.stopPropagation\(\);?\s*\}\}/,
  );
  assert.doesNotMatch(switchSource, /preventDefault|stopEvent/);
});
