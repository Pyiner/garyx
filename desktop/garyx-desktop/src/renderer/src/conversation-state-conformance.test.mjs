// Conformance suite for the cross-platform conversation state contract.
// Runs the shared fixtures in spec/conversation-state against the desktop
// reference implementation. The iOS twin lives in
// mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxConversationStateConformanceTests.swift
// and must consume the same fixture files. See docs/agents/conversation-state.md.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  COMPOSER_PHASES,
  INTENT_DISPATCH_MODES,
  INTENT_SOURCES,
  INTENT_STATES,
  THREAD_RUNTIME_STATES,
  findPendingAckIntentIndex,
  initialMessageMachineState,
  isRuntimeBusy,
  messageMachineReducer,
  nextComposerPhase,
  shouldTrackProviderAckAfterStreamInputResponse,
} from './message-machine.ts';
import { LIVE_STREAM_STATUSES, TRANSCRIPT_ENTRY_STATES } from './app-shell/types.ts';
import { deriveThreadActivityModel } from './app-shell/thread-activity.ts';

const specDir = new URL('../../../../../spec/conversation-state/', import.meta.url);

function loadSpecJson(relativePath) {
  return JSON.parse(readFileSync(new URL(relativePath, specDir), 'utf8'));
}

const states = loadSpecJson('states.json');
const machineFixtures = loadSpecJson('scenarios/machine.json');
const activityFixtures = loadSpecJson('scenarios/activity.json');
const functionFixtures = loadSpecJson('scenarios/function-cases.json');

test('enum vocabularies match the shared schema', () => {
  assert.deepEqual([...INTENT_STATES], states.intentState);
  assert.deepEqual([...INTENT_SOURCES], states.intentSource);
  assert.deepEqual([...INTENT_DISPATCH_MODES], states.intentDispatchMode);
  assert.deepEqual([...THREAD_RUNTIME_STATES], states.threadRuntimeState);
  assert.deepEqual([...LIVE_STREAM_STATUSES], states.liveStreamStatus);
  assert.deepEqual([...TRANSCRIPT_ENTRY_STATES], states.transcriptEntryState);
  assert.deepEqual([...COMPOSER_PHASES], states.composerPhase);
});

function buildFixtureIntent(raw) {
  return {
    intentId: raw.intentId,
    threadId: raw.threadId,
    text: raw.text || '',
    images: [],
    files: [],
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    state: raw.state,
    source: raw.source,
    ...(raw.dispatchMode ? { dispatchMode: raw.dispatchMode } : {}),
    ...(raw.remoteRunId ? { remoteRunId: raw.remoteRunId } : {}),
    ...(raw.pendingInputId ? { pendingInputId: raw.pendingInputId } : {}),
    ...(raw.responseText ? { responseText: raw.responseText } : {}),
  };
}

function toAction(raw) {
  if (raw.type === 'intent/created') {
    return { type: raw.type, intent: buildFixtureIntent(raw.intent), enqueue: raw.enqueue };
  }
  return raw;
}

function assertNullableEqual(actual, expected, label) {
  if (expected === null) {
    assert.ok(
      actual === undefined || actual === null,
      `${label}: expected absent, got ${JSON.stringify(actual)}`,
    );
    return;
  }
  assert.equal(actual, expected, label);
}

function assertSnapshot(state, expectation, label) {
  for (const [intentId, expected] of Object.entries(expectation.intents || {})) {
    const intent = state.intentsById[intentId];
    if (expected === null) {
      assert.equal(intent, undefined, `${label}: intent ${intentId} should be absent`);
      continue;
    }
    assert.ok(intent, `${label}: intent ${intentId} should exist`);
    for (const [field, value] of Object.entries(expected)) {
      assertNullableEqual(intent[field], value, `${label}: intent ${intentId}.${field}`);
    }
  }
  for (const [threadId, expectedQueue] of Object.entries(expectation.queues || {})) {
    assert.deepEqual(
      state.queueByThread[threadId] || [],
      expectedQueue,
      `${label}: queue for ${threadId}`,
    );
  }
  for (const [threadId, expected] of Object.entries(expectation.runtimes || {})) {
    const runtime = state.threadRuntimeByThread[threadId];
    if (expected.exists === false) {
      assert.equal(runtime, undefined, `${label}: runtime ${threadId} should be absent`);
      continue;
    }
    assert.ok(runtime, `${label}: runtime ${threadId} should exist`);
    if ('state' in expected) {
      assert.equal(runtime.state, expected.state, `${label}: runtime ${threadId}.state`);
    }
    if ('busy' in expected) {
      assert.equal(isRuntimeBusy(runtime.state), expected.busy, `${label}: runtime ${threadId} busy`);
    }
    if ('activeIntentId' in expected) {
      assertNullableEqual(runtime.activeIntentId, expected.activeIntentId, `${label}: runtime ${threadId}.activeIntentId`);
    }
    if ('remoteRunId' in expected) {
      assertNullableEqual(runtime.remoteRunId, expected.remoteRunId, `${label}: runtime ${threadId}.remoteRunId`);
    }
  }
  if ('composerPhase' in expectation) {
    assert.equal(state.composerPhase, expectation.composerPhase, `${label}: composerPhase`);
  }
}

for (const scenario of machineFixtures.scenarios) {
  test(`machine fixture: ${scenario.name}`, () => {
    let state = initialMessageMachineState;
    scenario.steps.forEach((step, index) => {
      if (step.action) {
        state = messageMachineReducer(state, toAction(step.action));
      }
      if (step.expect) {
        assertSnapshot(state, step.expect, `${scenario.name} step ${index}`);
      }
    });
  });
}

for (const fixture of activityFixtures.cases) {
  test(`activity fixture: ${fixture.name}`, () => {
    const input = fixture.input;
    const messages = input.messages.map((message, index) => ({
      id: `m${index}`,
      role: message.role,
      text: '',
      timestamp: '',
      pending: Boolean(message.pending),
      internal: Boolean(message.internal),
      internalKind: message.internalKind,
    }));
    const model = deriveThreadActivityModel({
      messages,
      threadInfo: input.activeRunId ? { activeRun: { runId: input.activeRunId } } : null,
      liveStream: input.liveStreamStatus
        ? { threadId: 't', pendingAckIntentIds: [], streamStatus: input.liveStreamStatus }
        : null,
      runtimeBusy: input.runtimeBusy,
      pendingAckIntentCount: input.pendingAckIntentCount,
      remoteAwaitingAckInputCount: input.remoteAwaitingAckInputCount,
      pendingHistoryIntent: input.pendingHistoryIntent,
    });
    assert.deepEqual(model, fixture.expect);
  });
}

test('function fixtures: findPendingAckIntentIndex', () => {
  for (const fixtureCase of functionFixtures.pendingAckIndex) {
    const index = findPendingAckIntentIndex(
      fixtureCase.pendingAckIntentIds,
      fixtureCase.acknowledgedPendingInputId,
      fixtureCase.intents,
    );
    assert.equal(index, fixtureCase.expect, fixtureCase.name);
  }
});

test('function fixtures: shouldTrackProviderAckAfterStreamInputResponse', () => {
  for (const fixtureCase of functionFixtures.providerAckTracking) {
    const intent = fixtureCase.intentState === null ? null : { state: fixtureCase.intentState };
    assert.equal(
      shouldTrackProviderAckAfterStreamInputResponse(intent),
      fixtureCase.expect,
      `intentState=${fixtureCase.intentState}`,
    );
  }
});

test('function fixtures: nextComposerPhase', () => {
  for (const fixtureCase of functionFixtures.composerPhase) {
    assert.equal(
      nextComposerPhase(fixtureCase),
      fixtureCase.expect,
      `hasText=${fixtureCase.hasText} isComposing=${fixtureCase.isComposing} locked=${fixtureCase.locked}`,
    );
  }
});
