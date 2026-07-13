import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const testDir = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.join(
  testDir,
  'fixtures/legacy-horizontal-layout-oracle.json',
);
const rawFixture = readFileSync(fixturePath, 'utf8');
const oracle = JSON.parse(rawFixture);

const expectedOccupancy = new Map([
  ['baseline', [true, false, false]],
  ['sidebar-collapsed', [false, false, false]],
  ['side-tools', [true, false, true]],
  ['recent-rail', [true, true, false]],
  ['recent-rail-side-tools', [true, true, true]],
]);

function pixelTracks(value) {
  return [...value.matchAll(/(-?\d+(?:\.\d+)?)px/g)].map((match) =>
    Number(match[1]),
  );
}

test('packaged legacy oracle contains the complete normalized scenario matrix', () => {
  assert.equal(oracle.schemaVersion, 1);
  assert.equal(oracle.policy, 'legacy');
  assert.equal(oracle.capture, 'packaged-cdp-normalized-structure');
  assert.deepEqual(
    oracle.scenarios.map((scenario) => scenario.name),
    [...expectedOccupancy.keys()],
  );

  for (const scenario of oracle.scenarios) {
    const occupancy = scenario.desiredOccupancy;
    assert.deepEqual(
      [
        occupancy.globalSidebar,
        occupancy.conversationRail,
        occupancy.sideTools,
      ],
      expectedOccupancy.get(scenario.name),
      scenario.name,
    );
    assert.deepEqual(scenario.viewport, {
      width: 1480,
      height: 940,
      devicePixelRatio: 2,
    });
    assert.deepEqual(scenario.elements.appShell.rect, {
      x: 0,
      y: 0,
      width: 1480,
      height: 940,
    });
    const shellTracks = pixelTracks(
      scenario.elements.appShell.computed.gridTemplateColumns,
    );
    assert.ok(shellTracks.length >= 2, scenario.name);
    assert.equal(
      shellTracks.reduce((sum, track) => sum + track, 0),
      scenario.viewport.width,
      `${scenario.name}: shell tracks fill the packaged viewport`,
    );
  }
});

test('oracle pins classes, semantic attributes, computed tracks, and drag carveout', () => {
  for (const scenario of oracle.scenarios) {
    const { elements, presentation } = scenario;
    assert.ok(elements.appShell.classTokens.includes('app-shell'));
    assert.equal(elements.appShell.computed.display, 'grid');
    assert.equal(elements.conversation.computed.display, 'grid');
    assert.equal(elements.threadLayout.computed.display, 'grid');
    assert.deepEqual(elements.sidebarCarveout.classTokens, [
      'sidebar-collapse-toggle',
      'sidebar-collapse-toggle-carveout',
    ]);
    assert.equal(elements.sidebarCarveout.computed.appRegion, 'no-drag');
    assert.equal(elements.sidebarToggle.computed.appRegion, 'no-drag');
    assert.equal(
      elements.sidebarToggle.attributes['aria-pressed'],
      presentation.globalSidebar === 'collapsed' ? 'true' : 'false',
    );

    assert.equal(Boolean(elements.conversationRail), presentation.conversationRail !== 'closed');
    assert.equal(Boolean(elements.sideToolsPanel), presentation.sideTools === 'docked');
  }
});

test('oracle normalizes dynamic task state', () => {
  const scenario = (name) =>
    oracle.scenarios.find((candidate) => candidate.name === name);
  const baseline = scenario('baseline');
  const recentRail = scenario('recent-rail');

  assert.equal(baseline.elements.taskTree.rect.y, 58);
  assert.equal(baseline.elements.taskTree.rect.height, 'dynamic');
  assert.equal(recentRail.elements.taskTree.rect.y, 'dynamic');
  assert.equal(recentRail.elements.taskTree.rect.height, 'dynamic');
  assert.doesNotMatch(rawFixture, /has-active/);
  assert.doesNotMatch(rawFixture, /threadLogs|threadLogPanel|threadLogResizer/);
});

test('oracle fixture contains no user data or dynamic thread identity', () => {
  const forbidden = [
    /\/Users\//,
    /thread::/,
    /#TASK-/,
    /session id/i,
    /api[_-]?key/i,
    /auth[_-]?token/i,
    /access[_-]?token/i,
  ];
  for (const pattern of forbidden) {
    assert.doesNotMatch(rawFixture, pattern);
  }
});
