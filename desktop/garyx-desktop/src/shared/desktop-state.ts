import type {
  ConfiguredBot,
  DesktopBotConsoleStatus,
  DesktopBotConsoleSummary,
  DesktopChannelEndpoint,
  DesktopState,
} from './contracts';

function normalizedThreadId(threadId: string): string {
  return threadId.trim();
}

function endpointReferencesThread(
  endpoint: DesktopChannelEndpoint | null | undefined,
  threadId: string,
): boolean {
  return Boolean(endpoint?.threadId && endpoint.threadId === threadId);
}

function endpointWithoutThread(
  endpoint: DesktopChannelEndpoint | null | undefined,
  threadId: string,
): DesktopChannelEndpoint | null {
  return endpointReferencesThread(endpoint, threadId) ? null : endpoint ?? null;
}

function botConsoleStatus(
  endpoints: DesktopChannelEndpoint[],
  mainThreadId: string | null,
  mainEndpoint: DesktopChannelEndpoint | null,
): DesktopBotConsoleStatus {
  return mainThreadId || mainEndpoint?.threadId || endpoints.some((endpoint) => Boolean(endpoint.threadId))
    ? 'connected'
    : 'idle';
}

function configuredBotWithoutThread(
  bot: ConfiguredBot,
  threadId: string,
): ConfiguredBot {
  const removedMainEndpoint = endpointReferencesThread(bot.mainEndpoint, threadId);
  const mainEndpoint = endpointWithoutThread(bot.mainEndpoint, threadId);
  const defaultOpenEndpoint = endpointWithoutThread(bot.defaultOpenEndpoint, threadId);
  return {
    ...bot,
    mainEndpoint,
    mainEndpointThreadId: bot.mainEndpointThreadId === threadId ? null : bot.mainEndpointThreadId ?? null,
    defaultOpenEndpoint,
    defaultOpenThreadId: bot.defaultOpenThreadId === threadId ? null : bot.defaultOpenThreadId ?? null,
    mainEndpointStatus: removedMainEndpoint ? 'unresolved' : bot.mainEndpointStatus,
  };
}

function botConsoleWithoutThread(
  group: DesktopBotConsoleSummary,
  threadId: string,
): DesktopBotConsoleSummary {
  const endpoints = group.endpoints.filter((endpoint) => !endpointReferencesThread(endpoint, threadId));
  const conversationNodes = group.conversationNodes.filter((node) => (
    !endpointReferencesThread(node.endpoint, threadId)
  ));
  const removedMainEndpoint = endpointReferencesThread(group.mainEndpoint, threadId);
  const mainEndpoint = endpointWithoutThread(group.mainEndpoint, threadId);
  const defaultOpenEndpoint = endpointWithoutThread(group.defaultOpenEndpoint, threadId);
  const mainThreadId = group.mainThreadId === threadId ? null : group.mainThreadId;
  const defaultOpenThreadId = group.defaultOpenThreadId === threadId ? null : group.defaultOpenThreadId;
  return {
    ...group,
    status: botConsoleStatus(endpoints, mainThreadId, mainEndpoint),
    endpointCount: endpoints.length,
    boundEndpointCount: endpoints.filter((endpoint) => Boolean(endpoint.threadId)).length,
    mainEndpointStatus: removedMainEndpoint ? 'unresolved' : group.mainEndpointStatus,
    mainEndpoint,
    mainThreadId,
    defaultOpenEndpoint,
    defaultOpenThreadId,
    conversationNodes,
    endpoints,
  };
}

export function desktopStateWithoutThread(
  state: DesktopState,
  threadId: string,
): DesktopState {
  const id = normalizedThreadId(threadId);
  if (!id) {
    return state;
  }

  return {
    ...state,
    threads: state.threads.filter((thread) => thread.id !== id),
    sessions: state.sessions.filter((thread) => thread.id !== id),
    pinnedThreadIds: state.pinnedThreadIds.filter((pinnedThreadId) => pinnedThreadId !== id),
    endpoints: state.endpoints.filter((endpoint) => !endpointReferencesThread(endpoint, id)),
    configuredBots: state.configuredBots.map((bot) => configuredBotWithoutThread(bot, id)),
    botConsoles: state.botConsoles.map((group) => botConsoleWithoutThread(group, id)),
    botMainThreads: Object.fromEntries(
      Object.entries(state.botMainThreads || {}).filter(([, mappedThreadId]) => mappedThreadId !== id),
    ),
  };
}

/**
 * `sessions` is the summary cache: the regular thread list PLUS hidden
 * session threads (side-chat children) that never appear in `threads`.
 * A full-state refresh rebuilds `threads`, so the merge must retain the
 * hidden entries — they are seeded once at creation from the authoritative
 * create response and have no other owner. Gateway-scope switches drop the
 * whole slice, which bounds retention.
 */
export function mergeRetainedHiddenSessions<T extends { id: string }>(
  threads: T[],
  previousSessions: T[] | undefined,
): T[] {
  const known = new Set(threads.map((thread) => thread.id));
  const retained = (previousSessions || []).filter(
    (session) => !known.has(session.id),
  );
  return retained.length ? [...threads, ...retained] : threads;
}

/**
 * Fold a just-created thread summary into a state snapshot: if the fetched
 * thread list does not carry it (hidden session threads never appear
 * there), it joins the sessions cache. This runs in the MAIN process at
 * creation, making main the durable cross-process owner — later full-state
 * refreshes flow through mergeRetainedHiddenSessions and keep it.
 */
export function stateWithCreatedThread<
  T extends { id: string },
  S extends { threads: T[]; sessions: T[] },
>(state: S, thread: T): S {
  if (
    state.threads.some((entry) => entry.id === thread.id) ||
    state.sessions.some((entry) => entry.id === thread.id)
  ) {
    return state;
  }
  return { ...state, sessions: [...state.sessions, thread] };
}
