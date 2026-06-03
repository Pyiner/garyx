import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildDesktopRouteHash,
  contentViewForDesktopRoute,
  parseDesktopRoute,
} from './desktop-route.ts';

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
      workflowId: null,
    },
  );
  assert.deepEqual(
    parseDesktopRoute('file:///Garyx.app/index.html#/new/%2FUsers%2Fgary%2Frepo?workflow=ship-flow'),
    {
      kind: 'new-thread',
      workspacePath: '/Users/gary/repo',
      agentId: null,
      workflowId: 'ship-flow',
    },
  );
});

test('builds stable hash routes', () => {
  assert.equal(
    buildDesktopRouteHash({ kind: 'thread', threadId: 'thread::abc123' }),
    '#/thread/thread%3A%3Aabc123',
  );
  assert.equal(
    buildDesktopRouteHash({ kind: 'workflow-task', taskId: '#TASK-258' }),
    '#/workflow/%23TASK-258',
  );
  assert.equal(
    buildDesktopRouteHash({
      kind: 'new-thread',
      workspacePath: '/Users/gary/repo',
      agentId: 'claude',
    }),
    '#/new?workspace=%2FUsers%2Fgary%2Frepo',
  );
  assert.equal(
    buildDesktopRouteHash({
      kind: 'new-thread',
      workspacePath: '/Users/gary/repo',
      agentId: 'codex',
      workflowId: 'ship-flow',
    }),
    '#/new?workspace=%2FUsers%2Fgary%2Frepo&workflow=ship-flow',
  );
  assert.equal(
    buildDesktopRouteHash({ kind: 'settings', tabId: 'gateway' }),
    '#/settings/gateway',
  );
});

test('parses utility views', () => {
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/auto-research'), {
    kind: 'view',
    view: 'auto_research',
  });
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/automation/job-1'), {
    kind: 'automation',
    automationId: 'job-1',
  });
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/tasks'), {
    kind: 'view',
    view: 'tasks',
  });
  assert.deepEqual(parseDesktopRoute('file:///Garyx.app/index.html#/workflow/%23TASK-258'), {
    kind: 'workflow-task',
    taskId: '#TASK-258',
  });
  assert.equal(
    contentViewForDesktopRoute(
      parseDesktopRoute('file:///Garyx.app/index.html#/workflows?task=%23TASK-259'),
    ),
    'workflow',
  );
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
