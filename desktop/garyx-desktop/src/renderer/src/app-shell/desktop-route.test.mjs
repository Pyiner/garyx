import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildDesktopRouteHash,
  contentViewForDesktopRoute,
  parseDesktopRoute,
} from './desktop-route.ts';

// Minimal base for currentDesktopRoute(); individual tests override fields.
const baseRouteInput = {
  contentView: 'capsules',
  newThreadDraftActive: false,
  pendingAgentId: null,
  pendingWorkspacePath: null,
  selectedAutomationId: null,
  selectedThreadId: null,
  settingsActiveTab: 'gateway',
  capsulePreviewId: null,
};

test('parses thread hash route', () => {
  const route = parseDesktopRoute('file:///Garyx.app/index.html#/thread/thread%3A%3Aabc123');
  assert.deepEqual(route, {
    kind: 'thread',
    threadId: 'thread::abc123',
  });
  assert.equal(contentViewForDesktopRoute(route), 'thread');
});

test('parses new thread hash route with workspace path', () => {
  assert.deepEqual(
    parseDesktopRoute('file:///Garyx.app/index.html#/new?workspace=%2FUsers%2Fgary%2Frepo&agent=codex'),
    {
      kind: 'new-thread',
      workspacePath: '/Users/gary/repo',
      agentId: 'codex',
    },
  );
});

test('builds stable hash routes', () => {
  assert.equal(
    buildDesktopRouteHash({ kind: 'thread', threadId: 'thread::abc123' }),
    '#/thread/thread%3A%3Aabc123',
  );
  assert.equal(
    buildDesktopRouteHash({
      kind: 'new-thread',
      workspacePath: '/Users/gary/repo',
      agentId: 'claude',
    }),
    '#/new?workspace=%2FUsers%2Fgary%2Frepo&agent=claude',
  );
  assert.equal(
    buildDesktopRouteHash({ kind: 'settings', tabId: 'gateway' }),
    '#/settings/gateway',
  );
});

test('parses utility views', () => {
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/automation/job-1'), {
    kind: 'automation',
    automationId: 'job-1',
  });
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/tasks'), {
    kind: 'view',
    view: 'tasks',
  });
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/capsules'), {
    kind: 'view',
    view: 'capsules',
  });
  assert.equal(buildDesktopRouteHash({ kind: 'view', view: 'capsules' }), '#/capsules');
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/settings/connection'), {
    kind: 'settings',
    tabId: 'gateway',
  });
});

test('falls back unknown hash routes to thread home', () => {
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/unknown/place'), {
    kind: 'thread-home',
  });
});

test('parses and builds capsule preview routes', () => {
  const route = parseDesktopRoute(
    'file:///Garyx.app/index.html#/capsules/01900000-0000-7000-8000-000000000001',
  );
  assert.deepEqual(route, {
    kind: 'capsule',
    capsuleId: '01900000-0000-7000-8000-000000000001',
  });
  assert.equal(contentViewForDesktopRoute(route), 'capsules');
  assert.equal(
    buildDesktopRouteHash(route),
    '#/capsules/01900000-0000-7000-8000-000000000001',
  );
  // The bare gallery route is unchanged (regression guard for the parse order).
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/capsules'), {
    kind: 'view',
    view: 'capsules',
  });
});
