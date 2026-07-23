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
import {
  DURABLE_CREATE_DELIVERY_PHASES,
  DURABLE_CREATE_USER_DISPOSITIONS,
  DURABLE_DELIVERY_EVIDENCE,
  DURABLE_DELIVERY_STATES,
  DURABLE_DELIVERY_USER_DISPOSITIONS,
  acknowledgeDurableCreate,
  acknowledgeDurableDelivery,
  acknowledgeDurableDeliveryEvidence,
  initialDurableCreateDelivery,
  initialDurableDeliveryRecord,
  markDurableCreateBindingCompleted,
  markDurableCreateChatStartAttempted,
  markDurableCreateResponseLost,
  markDurableCreateThreadCreated,
  markDurableDeliveryAmbiguous,
  markDurableTransportAttempted,
  resendDurableDeliveryAsDuplicate,
  restoreDurableDeliveryDraft,
  settleDurableCreateForScopeRevoke,
  settleDurableDeliveryForScopeRevoke,
} from './durable-delivery.ts';
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
const durableDeliveryFixtures = loadSpecJson('scenarios/durable-delivery.json');

test('enum vocabularies match the shared schema', () => {
  assert.deepEqual([...INTENT_STATES], states.intentState);
  assert.deepEqual([...INTENT_SOURCES], states.intentSource);
  assert.deepEqual([...INTENT_DISPATCH_MODES], states.intentDispatchMode);
  assert.deepEqual([...THREAD_RUNTIME_STATES], states.threadRuntimeState);
  assert.deepEqual([...LIVE_STREAM_STATUSES], states.liveStreamStatus);
  assert.deepEqual([...TRANSCRIPT_ENTRY_STATES], states.transcriptEntryState);
  assert.deepEqual([...COMPOSER_PHASES], states.composerPhase);
  assert.deepEqual([...DURABLE_DELIVERY_STATES], states.durableDeliveryState);
  assert.deepEqual([...DURABLE_DELIVERY_EVIDENCE], states.durableDeliveryEvidence);
  assert.deepEqual(
    [...DURABLE_DELIVERY_USER_DISPOSITIONS],
    states.durableDeliveryUserDisposition,
  );
  assert.deepEqual(
    [...DURABLE_CREATE_DELIVERY_PHASES],
    states.durableCreateDeliveryPhase,
  );
  assert.deepEqual(
    [...DURABLE_CREATE_USER_DISPOSITIONS],
    states.durableCreateUserDisposition,
  );
});

test('durable delivery fixtures name both canonical consumers', () => {
  assert.equal(durableDeliveryFixtures.platformConsumers.ios, 'implemented');
  assert.equal(durableDeliveryFixtures.platformConsumers.mac, 'implemented');
  assert.ok(durableDeliveryFixtures.scenarios.length > 0);
  assert.ok(durableDeliveryFixtures.createScenarios.length > 0);
});

const durableFixtureScope = { identity: 'fixture-gateway', epoch: 1 };

function makeDurableDeliveryRecord(
  id = 'delivery',
  correlationID = 'intent-original',
  envelope,
) {
  return initialDurableDeliveryRecord({
    id,
    scope: durableFixtureScope,
    entryID: 'fixture-entry',
    reservationID: 1,
    correlationID,
    envelope: envelope || {
      text: 'fixture message',
      attachmentIDs: [],
      generation: 1,
      clientIntentID: correlationID,
    },
  });
}

function applyDurableDeliveryAction(action, records, label) {
  const delivery = records.delivery;
  assert.ok(delivery, `${label}: original delivery missing`);
  if (action === 'attempt') {
    const next = markDurableTransportAttempted(delivery);
    assert.ok(next, `${label}: transport attempt was rejected`);
    return { ...records, delivery: next };
  }
  if (action === 'ambiguous') {
    const next = markDurableDeliveryAmbiguous(delivery);
    assert.ok(next, `${label}: ambiguous transition was rejected`);
    return { ...records, delivery: next };
  }
  if (action === 'acknowledge') {
    return { ...records, delivery: acknowledgeDurableDelivery(delivery) };
  }
  if (action === 'evidence' || action === 'evidenceOtherScope') {
    const result = acknowledgeDurableDeliveryEvidence({
      correlationID: 'intent-original',
      authenticatedScope: action === 'evidence'
        ? durableFixtureScope
        : { identity: 'other-gateway', epoch: 1 },
      records,
    });
    if (action === 'evidenceOtherScope') {
      assert.equal(result.disposition, 'rejectedAuthenticationSource', label);
      assert.deepEqual(result.records, records, `${label}: rejected evidence mutated records`);
    } else {
      assert.equal(result.disposition, 'updated', label);
    }
    return result.records;
  }
  if (action === 'restoreDraft' || action === 'recoverUndispatchedDraft') {
    const result = restoreDurableDeliveryDraft({
      record: delivery,
      conflictSet: {
        id: action === 'restoreDraft'
          ? 'fixture-conflict'
          : 'fixture-undispatched-conflict',
        scope: durableFixtureScope,
        candidates: [],
        pendingDecision: false,
      },
      candidate: {
        entryID: action === 'restoreDraft'
          ? 'fixture-recovered-entry'
          : 'fixture-undispatched-entry',
        label: action === 'restoreDraft'
          ? 'Recovered send'
          : 'Recovered unsent message',
      },
      membershipDurabilityAvailable: true,
      allowingUndispatched: action === 'recoverUndispatchedDraft',
    });
    assert.equal(result.disposition, 'restored', `${label}: recovery disposition`);
    assert.equal(result.conflictSet.pendingDecision, true, `${label}: conflict pending`);
    assert.equal(result.conflictSet.candidates.length, 1, `${label}: conflict membership`);
    return { ...records, delivery: result.record };
  }
  if (action === 'resendCopy') {
    const result = resendDurableDeliveryAsDuplicate({
      record: delivery,
      newRecordID: 'delivery-copy',
      newClientIntentID: 'intent-copy',
    });
    assert.ok(result, `${label}: duplicate resend was rejected`);
    return {
      ...records,
      delivery: result.original,
      'delivery-copy': result.duplicate,
    };
  }
  if (action === 'scopeRevoke') {
    return Object.fromEntries(
      Object.entries(records).map(([id, record]) => [
        id,
        settleDurableDeliveryForScopeRevoke(record),
      ]),
    );
  }
  assert.fail(`${label}: unsupported durable delivery action ${action}`);
}

function assertDurableDelivery(record, expected, label) {
  assert.equal(record.state, expected.state, `${label}: state`);
  assert.equal(record.evidence, expected.evidence, `${label}: evidence`);
  assert.equal(
    record.userDisposition,
    expected.userDisposition,
    `${label}: userDisposition`,
  );
  assert.equal(Boolean(record.envelope), expected.envelopePresent, `${label}: envelope`);
  if ('duplicateRecordID' in expected) {
    assertNullableEqual(
      record.duplicateRecordID,
      expected.duplicateRecordID,
      `${label}: duplicateRecordID`,
    );
  }
  if ('clientIntentID' in expected) {
    assertNullableEqual(
      record.envelope?.clientIntentID,
      expected.clientIntentID,
      `${label}: clientIntentID`,
    );
  }
}

for (const scenario of durableDeliveryFixtures.scenarios) {
  test(`durable delivery fixture: ${scenario.name}`, () => {
    let records = { delivery: makeDurableDeliveryRecord() };
    for (const action of scenario.actions || []) {
      records = applyDurableDeliveryAction(action, records, scenario.name);
    }
    for (const [recordID, expected] of Object.entries(scenario.expect)) {
      const record = records[recordID];
      assert.ok(record, `${scenario.name}: ${recordID} should exist`);
      assertDurableDelivery(record, expected, `${scenario.name}: ${recordID}`);
    }
  });
}

for (const scenario of durableDeliveryFixtures.createScenarios) {
  test(`durable create fixture: ${scenario.name}`, () => {
    let state = initialDurableCreateDelivery({
      scope: durableFixtureScope,
      createIntentID: 'create-intent',
      entryID: 'fixture-entry',
    });
    for (const action of scenario.actions || []) {
      if (action === 'created') {
        state = markDurableCreateThreadCreated(state, 'thread-1');
      } else if (action === 'bound') {
        state = markDurableCreateBindingCompleted(state);
      } else if (action === 'chatAttempted') {
        state = markDurableCreateChatStartAttempted(state);
      } else if (action === 'responseLost') {
        state = markDurableCreateResponseLost(state);
      } else if (action === 'acknowledged') {
        state = acknowledgeDurableCreate(state);
      } else if (action === 'scopeRevoke') {
        state = settleDurableCreateForScopeRevoke(state);
      } else {
        assert.fail(`${scenario.name}: unsupported durable create action ${action}`);
      }
    }
    assert.equal(state.phase, scenario.expect.phase, `${scenario.name}: phase`);
    assertNullableEqual(
      state.ambiguousAfter,
      scenario.expect.ambiguousAfter,
      `${scenario.name}: ambiguousAfter`,
    );
    assert.equal(
      state.userDisposition,
      scenario.expect.userDisposition,
      `${scenario.name}: userDisposition`,
    );
    assertNullableEqual(state.threadID, scenario.expect.threadID, `${scenario.name}: threadID`);
  });
}

function buildFixtureIntent(raw) {
  return {
    intentId: raw.intentId,
    threadId: raw.threadId,
    text: raw.text || '',
    images: [],
    files: [],
    createdAt: '2026-01-01T00:00:00.000Z',
    clientTimestampLocal: '2026-01-01 00:00:00',
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
