import assert from 'node:assert/strict';
import test from 'node:test';

import {
  acceptAvatarCandidate,
  avatarGenerationFailure,
  beginAvatarGeneration,
  cancelAvatarGeneration,
  changeAvatarStyle,
  createAvatarGenerationFlow,
  ownsAvatarGenerationOperation,
  resolveAvatarGeneration,
} from './agent-avatar-flow.ts';

test('avatar flow keeps Current unchanged until Use avatar', () => {
  let flow = createAvatarGenerationFlow('current');
  flow = beginAvatarGeneration(flow, 'request-1');
  assert.equal(flow.phase, 'generating');

  flow = resolveAvatarGeneration(flow, 'request-1', {
    status: 'success',
    avatarDataUrl: 'new',
  });
  assert.equal(flow.phase, 'candidate');
  assert.equal(flow.currentAvatarDataUrl, 'current');
  assert.equal(flow.candidateAvatarDataUrl, 'new');

  const accepted = acceptAvatarCandidate(flow);
  assert.equal(accepted.avatarDataUrl, 'new');
  assert.equal(accepted.flow.currentAvatarDataUrl, 'new');
});

test('failure and retry preserve a usable prior candidate', () => {
  let flow = {
    ...createAvatarGenerationFlow('current'),
    phase: 'candidate',
    candidateAvatarDataUrl: 'prior',
  };
  flow = beginAvatarGeneration(flow, 'request-1');
  flow = resolveAvatarGeneration(flow, 'request-1', {
    status: 'failure',
    failure: avatarGenerationFailure('provider'),
  });
  assert.equal(flow.phase, 'failed');
  assert.equal(flow.candidateAvatarDataUrl, 'prior');

  flow = beginAvatarGeneration(flow, 'request-2');
  assert.equal(flow.phase, 'generating');
  assert.equal(flow.requestId, 'request-2');
  assert.equal(flow.candidateAvatarDataUrl, 'prior');
});

test('failed Change style returns to choosing', () => {
  const failed = {
    ...createAvatarGenerationFlow('current'),
    phase: 'failed',
    candidateAvatarDataUrl: 'prior',
    failure: avatarGenerationFailure('timeout'),
  };
  const choosing = changeAvatarStyle(failed);
  assert.equal(choosing.phase, 'choosing');
  assert.equal(choosing.candidateAvatarDataUrl, 'prior');
});

test('cancel and retry reject late success, failure, and finally ownership', () => {
  let flow = beginAvatarGeneration(createAvatarGenerationFlow('current'), 'request-1');
  flow = cancelAvatarGeneration(flow, 'request-1');
  flow = beginAvatarGeneration(flow, 'request-2');

  const lateSuccess = resolveAvatarGeneration(flow, 'request-1', {
    status: 'success',
    avatarDataUrl: 'late',
  });
  const lateFailure = resolveAvatarGeneration(flow, 'request-1', {
    status: 'failure',
    failure: avatarGenerationFailure('unknown'),
  });
  assert.strictEqual(lateSuccess, flow);
  assert.strictEqual(lateFailure, flow);
  assert.strictEqual(cancelAvatarGeneration(flow, 'request-1'), flow);

  assert.equal(
    ownsAvatarGenerationOperation(2, 'request-2', { epoch: 1, requestId: 'request-1' }),
    false,
  );
  assert.equal(
    ownsAvatarGenerationOperation(2, 'request-2', { epoch: 2, requestId: 'request-2' }),
    true,
  );
});

test('cancelled and superseded outcomes are silent choosing transitions', () => {
  for (const status of ['cancelled', 'superseded']) {
    const generating = beginAvatarGeneration(createAvatarGenerationFlow('current'), 'request');
    const resolved = resolveAvatarGeneration(generating, 'request', { status });
    assert.equal(resolved.phase, 'choosing');
    assert.equal(resolved.failure, null);
  }
});
