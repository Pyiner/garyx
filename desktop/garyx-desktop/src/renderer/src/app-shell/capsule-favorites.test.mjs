import assert from 'node:assert/strict';
import test from 'node:test';

import {
  capsuleIsFavorited,
  createCapsuleFavoritesState,
  filterCapsulesForGallery,
  mergeCapsuleFavoriteRefresh,
  reduceCapsuleFavoriteFailure,
  reduceCapsuleFavoriteSuccess,
  reduceCapsuleFavoriteToggle,
} from './capsule-favorites.ts';

function capsule(id, favoritedAt = null, overrides = {}) {
  return {
    id,
    title: `Capsule ${id}`,
    description: '',
    threadId: null,
    runId: null,
    agentId: null,
    providerType: null,
    htmlSha256: 'a'.repeat(64),
    byteSize: 42,
    revision: 1,
    createdAt: '2026-07-14T00:00:00Z',
    updatedAt: '2026-07-14T00:00:00Z',
    favoritedAt,
    ...overrides,
  };
}

function success(capsuleValue) {
  return { favorited: capsuleValue.favoritedAt !== null, capsule: capsuleValue };
}

test('gallery filter keeps server order for all/favorites and handles empty', () => {
  const capsules = [capsule('a'), capsule('b', '2026-07-14T01:00:00Z'), capsule('c')];
  const state = createCapsuleFavoritesState();
  assert.deepEqual(filterCapsulesForGallery(capsules, 'all', state).map(({ id }) => id), [
    'a',
    'b',
    'c',
  ]);
  assert.deepEqual(
    filterCapsulesForGallery(capsules, 'favorites', state).map(({ id }) => id),
    ['b'],
  );
  assert.deepEqual(filterCapsulesForGallery([], 'favorites', state), []);
});

test('double tap serializes PUT then DELETE and settles on final intent', () => {
  let capsules = [capsule('a')];
  let state = createCapsuleFavoritesState();

  let transition = reduceCapsuleFavoriteToggle(capsules, state, 'a', true);
  ({ capsules, state } = transition);
  assert.deepEqual(transition.effect, { capsuleId: 'a', favorited: true });
  assert.equal(state.favoritesGeneration, 1);
  assert.equal(capsuleIsFavorited(capsules[0], state), true);

  transition = reduceCapsuleFavoriteToggle(capsules, state, 'a', false);
  ({ capsules, state } = transition);
  assert.equal(transition.effect, null, 'second tap waits for the first request');
  assert.equal(state.favoritesGeneration, 1);
  assert.equal(capsuleIsFavorited(capsules[0], state), false);

  transition = reduceCapsuleFavoriteSuccess(
    capsules,
    state,
    'a',
    success(capsule('a', '2026-07-14T01:00:00Z')),
  );
  ({ capsules, state } = transition);
  assert.deepEqual(transition.effect, { capsuleId: 'a', favorited: false });
  assert.equal(state.favoritesGeneration, 3, 'first settle and follow-up start both bump');
  assert.equal(capsuleIsFavorited(capsules[0], state), false);

  transition = reduceCapsuleFavoriteSuccess(capsules, state, 'a', success(capsule('a')));
  ({ capsules, state } = transition);
  assert.equal(transition.effect, null);
  assert.equal(state.favoritesGeneration, 4);
  assert.equal(state.mutations.a.inFlight, false);
  assert.equal(capsuleIsFavorited(capsules[0], state), false);
  assert.equal(capsules[0].favoritedAt, null);
});

test('favorite failure reverts optimistic desired state to server state', () => {
  let capsules = [capsule('a')];
  let state = createCapsuleFavoritesState();
  ({ capsules, state } = reduceCapsuleFavoriteToggle(capsules, state, 'a', true));
  assert.equal(capsuleIsFavorited(capsules[0], state), true);

  const failed = reduceCapsuleFavoriteFailure(capsules, state, 'a');
  assert.equal(failed.state.favoritesGeneration, 2);
  assert.equal(failed.state.mutations.a.inFlight, false);
  assert.equal(capsuleIsFavorited(failed.capsules[0], failed.state), false);
});

test('refresh merge keeps pending intent and adopts an ordinary settled refresh', () => {
  let capsules = [capsule('a')];
  let state = createCapsuleFavoritesState();
  ({ capsules, state } = reduceCapsuleFavoriteToggle(capsules, state, 'a', true));

  let merged = mergeCapsuleFavoriteRefresh(
    capsules,
    [capsule('a', null, { title: 'Refreshed title' })],
    state,
    state.favoritesGeneration,
  );
  assert.equal(merged.capsules[0].title, 'Refreshed title');
  assert.equal(merged.capsules[0].favoritedAt, null);
  assert.equal(capsuleIsFavorited(merged.capsules[0], merged.state), true);

  ({ capsules, state } = reduceCapsuleFavoriteFailure(merged.capsules, merged.state, 'a'));
  merged = mergeCapsuleFavoriteRefresh(
    capsules,
    [capsule('a', '2026-07-14T02:00:00Z')],
    state,
    state.favoritesGeneration,
  );
  assert.equal(merged.capsules[0].favoritedAt, '2026-07-14T02:00:00Z');
  assert.equal(capsuleIsFavorited(merged.capsules[0], merged.state), true);
});

test('favorites generation matrix rejects stale cross-operation list responses', async (t) => {
  await t.test('refresh sent before mutation and landing after settle cannot clobber', () => {
    let capsules = [capsule('a')];
    let state = createCapsuleFavoritesState();
    const captured = state.favoritesGeneration;
    ({ capsules, state } = reduceCapsuleFavoriteToggle(capsules, state, 'a', true));
    ({ capsules, state } = reduceCapsuleFavoriteSuccess(
      capsules,
      state,
      'a',
      success(capsule('a', '2026-07-14T03:00:00Z')),
    ));

    const merged = mergeCapsuleFavoriteRefresh(capsules, [capsule('a')], state, captured);
    assert.equal(merged.capsules[0].favoritedAt, '2026-07-14T03:00:00Z');
    assert.equal(capsuleIsFavorited(merged.capsules[0], merged.state), true);
  });

  await t.test('refresh sent during pending and landing after settle cannot clobber', () => {
    let capsules = [capsule('a')];
    let state = createCapsuleFavoritesState();
    ({ capsules, state } = reduceCapsuleFavoriteToggle(capsules, state, 'a', true));
    const captured = state.favoritesGeneration;
    ({ capsules, state } = reduceCapsuleFavoriteSuccess(
      capsules,
      state,
      'a',
      success(capsule('a', '2026-07-14T04:00:00Z')),
    ));

    const merged = mergeCapsuleFavoriteRefresh(capsules, [capsule('a')], state, captured);
    assert.equal(merged.capsules[0].favoritedAt, '2026-07-14T04:00:00Z');
  });

  await t.test('refresh sent after settle is adopted normally', () => {
    let capsules = [capsule('a')];
    let state = createCapsuleFavoritesState();
    ({ capsules, state } = reduceCapsuleFavoriteToggle(capsules, state, 'a', true));
    ({ capsules, state } = reduceCapsuleFavoriteSuccess(
      capsules,
      state,
      'a',
      success(capsule('a', '2026-07-14T05:00:00Z')),
    ));
    const captured = state.favoritesGeneration;

    const merged = mergeCapsuleFavoriteRefresh(capsules, [capsule('a')], state, captured);
    assert.equal(merged.capsules[0].favoritedAt, null);
    assert.equal(capsuleIsFavorited(merged.capsules[0], merged.state), false);
  });
});
