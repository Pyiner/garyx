import { join } from 'node:path';

import { app } from 'electron';

import type { DesktopThreadSummary } from '@shared/contracts';

import { createHiddenSessionStore } from './hidden-session-store-core';

const HIDDEN_SESSIONS_FILE_NAME = 'garyx-hidden-sessions.json';

const productionStore = createHiddenSessionStore(() =>
  join(app.getPath('userData'), HIDDEN_SESSIONS_FILE_NAME),
);

export function ensureHiddenSessionsLoaded(): Promise<void> {
  return productionStore.ensureLoaded();
}

export function listHiddenSessions(
  scope: string | null | undefined,
): DesktopThreadSummary[] {
  return productionStore.list(scope);
}

export function rememberHiddenSession(
  scope: string | null | undefined,
  thread: DesktopThreadSummary,
): Promise<void> {
  return productionStore.remember(scope, thread);
}

export function forgetHiddenSession(
  scope: string | null | undefined,
  threadId: string,
): Promise<void> {
  return productionStore.forget(scope, threadId);
}
