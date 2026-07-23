import test from 'node:test';
import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';

import * as esbuild from 'esbuild';

const bundled = await esbuild.build({
  entryPoints: ['src/renderer/src/bot-console-controller.ts'],
  bundle: true,
  format: 'esm',
  platform: 'node',
  write: false,
});
const controllerModule = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`
);
const { activateBotDraftThread } = controllerModule;

function makeEndpoint(overrides = {}) {
  return {
    endpointKey: 'weixin::test-bot::default',
    channel: 'weixin',
    accountId: 'test-bot',
    peerId: 'peer-1',
    chatId: 'chat-1',
    deliveryTargetType: 'chat_id',
    deliveryTargetId: 'chat-1',
    displayLabel: 'Test endpoint',
    threadId: 'thread::stale',
    ...overrides,
  };
}

function makeBotGroup(overrides = {}) {
  const endpoint = makeEndpoint();
  return {
    id: 'weixin::test-bot',
    channel: 'weixin',
    accountId: 'test-bot',
    title: 'Test Bot',
    subtitle: 'Weixin Bot',
    rootBehavior: 'open_default',
    status: 'connected',
    latestActivity: null,
    endpointCount: 1,
    boundEndpointCount: 1,
    workspaceDir: '/workspace/test-project',
    mainEndpointStatus: 'unresolved',
    mainEndpoint: null,
    mainThreadId: endpoint.threadId,
    defaultOpenEndpoint: endpoint,
    defaultOpenThreadId: endpoint.threadId,
    conversationNodes: [],
    endpoints: [endpoint],
    ...overrides,
  };
}

function makeDesktopState(group) {
  return {
    threads: [],
    workspaces: [{ path: '/workspace/test-project', exists: true }],
    endpoints: group.endpoints,
    configuredBots: [{
      channel: group.channel,
      accountId: group.accountId,
      displayName: group.title,
      rootBehavior: group.rootBehavior,
      mainEndpointStatus: group.mainEndpointStatus,
      mainEndpointThreadId: 'thread::stale',
      workspaceDir: group.workspaceDir,
    }],
    botMainThreads: { [group.id]: 'thread::stale' },
    botConsoles: [],
  };
}

test('falls back to a bound bot draft when a remembered bot thread cannot open', async () => {
  const group = makeBotGroup();
  const desktopState = makeDesktopState(group);
  const calls = [];
  const values = {
    error: 'previous error',
    draftNavigations: [],
    newThreadDraftActive: false,
    selectedThreadId: 'thread::previous',
    pendingWorkspacePath: null,
    pendingBotId: null,
    composerPhase: 'busy',
  };

  await activateBotDraftThread({
    platform: {
      getState: async () => desktopState,
      addWorkspaceByPath: async () => {
        throw new Error('unexpected workspace mutation');
      },
    },
    desktopState,
    group,
    onState: () => {},
    onOpenExistingThread: async () => false,
    onOpenThreadById: async (threadId) => {
      calls.push(threadId);
      return false;
    },
    shouldKeepNewDraft: () => true,
    shouldOpenResolvedThread: () => false,
    setError: (value) => {
      values.error = value;
    },
    enterBotDraft: (workspacePath, botId) => {
      values.draftNavigations.push({ workspacePath, botId });
      // Simulate the new-thread route application the bridge runs for the
      // committed route (draft entry + mailbox bot binding).
      values.newThreadDraftActive = true;
      values.selectedThreadId = null;
      values.pendingWorkspacePath = workspacePath;
      values.pendingBotId = botId;
    },
    setPendingWorkspacePath: (value) => {
      values.pendingWorkspacePath = value;
    },
    syncComposerPhase: (value) => {
      values.composerPhase = value;
    },
  });

  assert.deepEqual(calls, ['thread::stale']);
  assert.equal(values.error, null);
  assert.deepEqual(values.draftNavigations, [
    { workspacePath: '/workspace/test-project', botId: 'weixin::test-bot' },
  ]);
  assert.equal(values.newThreadDraftActive, true);
  assert.equal(values.selectedThreadId, null);
  assert.equal(values.pendingWorkspacePath, '/workspace/test-project');
  assert.equal(values.pendingBotId, 'weixin::test-bot');
  assert.equal(values.composerPhase, '');
});

test('keeps the bot draft when gateway reconciliation returns the same stale thread', async () => {
  const group = makeBotGroup();
  const desktopState = makeDesktopState(group);
  const calls = [];
  const values = {
    error: null,
    draftNavigations: [],
    newThreadDraftActive: false,
    selectedThreadId: 'thread::previous',
    pendingWorkspacePath: null,
    pendingBotId: null,
    composerPhase: 'busy',
  };

  await activateBotDraftThread({
    platform: {
      getState: async () => desktopState,
      addWorkspaceByPath: async () => {
        throw new Error('unexpected workspace mutation');
      },
    },
    desktopState,
    group,
    onState: () => {},
    onOpenExistingThread: async () => false,
    onOpenThreadById: async (threadId) => {
      calls.push(threadId);
      values.error = `Thread not found: ${threadId}`;
      values.newThreadDraftActive = false;
      return false;
    },
    shouldKeepNewDraft: (groupId, initialWorkspacePath) =>
      values.newThreadDraftActive &&
      values.selectedThreadId === null &&
      values.pendingBotId === groupId &&
      values.pendingWorkspacePath === initialWorkspacePath,
    shouldOpenResolvedThread: (groupId, initialWorkspacePath) =>
      values.newThreadDraftActive &&
      values.selectedThreadId === null &&
      values.pendingBotId === groupId &&
      values.pendingWorkspacePath === initialWorkspacePath,
    setError: (value) => {
      values.error = value;
    },
    enterBotDraft: (workspacePath, botId) => {
      values.draftNavigations.push({ workspacePath, botId });
      values.newThreadDraftActive = true;
      values.selectedThreadId = null;
      values.pendingWorkspacePath = workspacePath;
      values.pendingBotId = botId;
    },
    setPendingWorkspacePath: (value) => {
      values.pendingWorkspacePath = value;
    },
    syncComposerPhase: (value) => {
      values.composerPhase = value;
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(calls, ['thread::stale', 'thread::stale']);
  assert.equal(values.error, null);
  assert.deepEqual(values.draftNavigations, [
    { workspacePath: '/workspace/test-project', botId: 'weixin::test-bot' },
    { workspacePath: '/workspace/test-project', botId: 'weixin::test-bot' },
  ]);
  assert.equal(values.newThreadDraftActive, true);
  assert.equal(values.selectedThreadId, null);
  assert.equal(values.pendingWorkspacePath, '/workspace/test-project');
  assert.equal(values.pendingBotId, 'weixin::test-bot');
  assert.equal(values.composerPhase, '');
});

test('opens a refreshed bot-console main thread when configured-bot resolution is stale', async () => {
  const unresolvedGroup = makeBotGroup({
    status: 'idle',
    endpointCount: 0,
    boundEndpointCount: 0,
    mainEndpoint: null,
    mainThreadId: null,
    defaultOpenEndpoint: null,
    defaultOpenThreadId: null,
    endpoints: [],
  });
  const initialState = {
    ...makeDesktopState(unresolvedGroup),
    endpoints: [],
    configuredBots: [{
      channel: unresolvedGroup.channel,
      accountId: unresolvedGroup.accountId,
      displayName: unresolvedGroup.title,
      rootBehavior: unresolvedGroup.rootBehavior,
      mainEndpointStatus: 'unresolved',
      mainEndpoint: null,
      mainEndpointThreadId: null,
      workspaceDir: unresolvedGroup.workspaceDir,
    }],
    botMainThreads: {},
    botConsoles: [],
  };
  const boundEndpoint = makeEndpoint({ threadId: 'thread::bound-main' });
  const refreshedState = {
    ...initialState,
    botConsoles: [makeBotGroup({
      mainEndpointStatus: 'resolved',
      mainEndpoint: boundEndpoint,
      mainThreadId: boundEndpoint.threadId,
      defaultOpenEndpoint: boundEndpoint,
      defaultOpenThreadId: boundEndpoint.threadId,
      endpoints: [boundEndpoint],
    })],
  };
  const openedThreadIds = [];
  const values = {
    draftActive: false,
    selectedThreadId: 'thread::previous',
    pendingWorkspacePath: null,
    pendingBotId: null,
  };

  await activateBotDraftThread({
    platform: {
      getState: async () => refreshedState,
      addWorkspaceByPath: async () => {
        throw new Error('unexpected workspace mutation');
      },
    },
    desktopState: initialState,
    group: unresolvedGroup,
    onState: () => {},
    onOpenExistingThread: async () => {
      throw new Error('main thread id should be preferred');
    },
    onOpenThreadById: async (threadId) => {
      openedThreadIds.push(threadId);
      values.draftActive = false;
      values.selectedThreadId = threadId;
      return true;
    },
    shouldKeepNewDraft: (groupId, initialWorkspacePath) =>
      values.draftActive &&
      values.selectedThreadId === null &&
      values.pendingBotId === groupId &&
      values.pendingWorkspacePath === initialWorkspacePath,
    shouldOpenResolvedThread: (groupId, initialWorkspacePath) =>
      values.draftActive &&
      values.selectedThreadId === null &&
      values.pendingBotId === groupId &&
      values.pendingWorkspacePath === initialWorkspacePath,
    setError: () => {},
    enterBotDraft: (workspacePath, botId) => {
      values.draftActive = true;
      values.selectedThreadId = null;
      values.pendingWorkspacePath = workspacePath;
      values.pendingBotId = botId;
    },
    setPendingWorkspacePath: (value) => {
      values.pendingWorkspacePath = value;
    },
    syncComposerPhase: () => {},
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(openedThreadIds, ['thread::bound-main']);
  assert.equal(values.draftActive, false);
  assert.equal(values.selectedThreadId, 'thread::bound-main');
});
