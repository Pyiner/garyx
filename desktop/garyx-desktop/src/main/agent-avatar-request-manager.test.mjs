import assert from 'node:assert/strict';
import test from 'node:test';

import { AgentAvatarRequestManager } from './agent-avatar-request-manager.ts';

function pendingUntilAbort(signal, onAbort) {
  return new Promise((_resolve, reject) => {
    signal.addEventListener('abort', () => {
      onAbort();
      reject(signal.reason);
    }, { once: true });
  });
}

test('scoped cancel aborts only the matching avatar request', async () => {
  const manager = new AgentAvatarRequestManager();
  let firstAborted = false;
  let secondAborted = false;
  const first = manager.run('first', 30_000, ({ signal }) => (
    pendingUntilAbort(signal, () => { firstAborted = true; })
  ));
  const second = manager.run('second', 30_000, ({ signal }) => (
    pendingUntilAbort(signal, () => { secondAborted = true; })
  ));

  assert.equal(manager.cancel('first'), true);
  await assert.rejects(first, { name: 'AbortError' });
  assert.equal(firstAborted, true);
  assert.equal(secondAborted, false);
  assert.equal(manager.has('first'), false);
  assert.equal(manager.has('second'), true);

  manager.cancel('second');
  await assert.rejects(second, { name: 'AbortError' });
});

test('normal completion cleans up without aborting the operation', async () => {
  const manager = new AgentAvatarRequestManager();
  let userAborted = false;
  let timeoutAborted = false;
  const value = await manager.run('complete', 30_000, async ({ userSignal, timeoutSignal }) => {
    userSignal.addEventListener('abort', () => { userAborted = true; });
    timeoutSignal.addEventListener('abort', () => { timeoutAborted = true; });
    return 'done';
  });

  assert.equal(value, 'done');
  assert.equal(userAborted, false);
  assert.equal(timeoutAborted, false);
  assert.equal(manager.has('complete'), false);
  assert.equal(manager.cancel('complete'), false);
});

test('timeout aborts the combined signal without marking user cancellation', async () => {
  const manager = new AgentAvatarRequestManager();
  let userAborted = false;
  let timeoutAborted = false;
  await assert.rejects(
    manager.run('timeout', 5, ({ signal, userSignal, timeoutSignal }) => (
      pendingUntilAbort(signal, () => {
        userAborted = userSignal.aborted;
        timeoutAborted = timeoutSignal.aborted;
      })
    )),
    { name: 'TimeoutError' },
  );

  assert.equal(userAborted, false);
  assert.equal(timeoutAborted, true);
  assert.equal(manager.has('timeout'), false);
});

test('duplicate active request IDs are rejected without replacing ownership', async () => {
  const manager = new AgentAvatarRequestManager();
  const first = manager.run('same', 30_000, ({ signal }) => (
    pendingUntilAbort(signal, () => {})
  ));

  await assert.rejects(
    manager.run('same', 30_000, async () => 'unexpected'),
    /already active/,
  );
  assert.equal(manager.cancel('same'), true);
  await assert.rejects(first, { name: 'AbortError' });
});
