import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import test from 'node:test';

import {
  loadWorkspaceGitStatusCached,
  WorkspaceGitStatusCache,
} from './workspace-git-status-cache.ts';

const execFileAsync = promisify(execFile);

async function readGitStatus(workspacePath) {
  try {
    const { stdout } = await execFileAsync(
      'git',
      ['-C', workspacePath, 'rev-parse', '--is-inside-work-tree'],
      { encoding: 'utf8' },
    );
    return { isGitRepo: stdout.trim() === 'true' };
  } catch {
    return { isGitRepo: false };
  }
}

test('focus invalidation refreshes a cached negative after git init', async () => {
  const workspacePath = await mkdtemp(join(tmpdir(), 'garyx-git-status-cache-'));
  try {
    const cache = new WorkspaceGitStatusCache();
    let loads = 0;
    const load = async () => {
      loads += 1;
      return readGitStatus(workspacePath);
    };

    assert.deepEqual(
      await loadWorkspaceGitStatusCached({ cache, load, workspacePath }),
      { isGitRepo: false },
    );
    await execFileAsync('git', ['init', '--quiet', workspacePath]);
    assert.deepEqual(
      await loadWorkspaceGitStatusCached({ cache, load, workspacePath }),
      { isGitRepo: false },
      'the setup proves the negative was cached',
    );
    assert.equal(loads, 1);

    assert.equal(cache.invalidateNegative(workspacePath), true);
    assert.deepEqual(
      await loadWorkspaceGitStatusCached({ cache, load, workspacePath }),
      { isGitRepo: true },
    );
    assert.equal(loads, 2);
  } finally {
    await rm(workspacePath, { force: true, recursive: true });
  }
});

test('bounds entries by access-order LRU and expires them by TTL', () => {
  const cache = new WorkspaceGitStatusCache({ maxEntries: 2, ttlMs: 100 });
  cache.set('/workspace/a', { isGitRepo: true }, 0);
  cache.set('/workspace/b', { isGitRepo: false }, 0);
  assert.deepEqual(cache.get('/workspace/a', 50), { isGitRepo: true });

  cache.set('/workspace/c', { isGitRepo: true }, 50);
  assert.equal(cache.get('/workspace/b', 50), null, 'least-recent entry is evicted');
  assert.deepEqual(cache.get('/workspace/a', 50), { isGitRepo: true });
  assert.equal(cache.get('/workspace/a', 100), null, 'entry expires at its TTL');
  assert.equal(cache.invalidateNegative('/workspace/c'), false);
});
