import type { SettingsTabId } from '../settings-tabs';

import type { ContentView } from './types';

export type DesktopRoute =
  | { kind: 'thread-home' }
  | { kind: 'thread'; threadId: string }
  | {
      kind: 'new-thread';
      workspacePath?: string | null;
      agentId?: string | null;
      workflowId?: string | null;
    }
  | { kind: 'workflow-task'; taskId: string }
  | { kind: 'automation'; automationId?: string | null }
  | { kind: 'settings'; tabId?: SettingsTabId | null }
  | { kind: 'capsule'; capsuleId: string }
  | { kind: 'view'; view: Exclude<ContentView, 'thread' | 'workflow' | 'automation' | 'settings'> };

const SIMPLE_VIEW_SEGMENTS: Record<string, Exclude<ContentView, 'thread' | 'workflow' | 'automation' | 'settings'>> = {
  browser: 'browser',
  bots: 'bots',
  capsules: 'capsules',
  agents: 'agents',
  teams: 'teams',
  skills: 'skills',
  tasks: 'tasks',
};

const SETTINGS_TAB_IDS = new Set<string>([
  'labs',
  'gateway',
  'provider',
  'channels',
  'commands',
  'mcp',
]);

function decodeLoose(value: string | null | undefined): string | null {
  if (!value) {
    return null;
  }
  let current = value.trim();
  if (!current) {
    return null;
  }
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      const decoded = decodeURIComponent(current);
      if (decoded === current) {
        break;
      }
      current = decoded.trim();
    } catch {
      break;
    }
  }
  return current || null;
}

function encodeSegment(value: string): string {
  return encodeURIComponent(value.trim());
}

function appendParam(
  params: URLSearchParams,
  key: string,
  value?: string | null,
): void {
  const trimmed = value?.trim() || '';
  if (trimmed) {
    params.set(key, trimmed);
  }
}

function normalizeSettingsTab(value: string | null): SettingsTabId | null {
  if (value === 'connection') {
    return 'gateway';
  }
  return value && SETTINGS_TAB_IDS.has(value)
    ? (value as SettingsTabId)
    : null;
}

function parseHashUrl(hash: string): URL | null {
  const raw = hash.trim().replace(/^#/, '');
  if (!raw) {
    return null;
  }
  const path = raw.startsWith('/') ? raw : `/${raw}`;
  try {
    return new URL(`garyx-desktop://route${path}`);
  } catch {
    return null;
  }
}

function routeSegments(url: URL): string[] {
  return url.pathname
    .split('/')
    .map((segment) => decodeLoose(segment))
    .filter((segment): segment is string => Boolean(segment));
}

export function parseDesktopRoute(href?: string): DesktopRoute {
  const currentHref = href || globalThis.window?.location.href || 'file:///index.html';
  let url: URL;
  try {
    url = new URL(currentHref);
  } catch {
    return { kind: 'thread-home' };
  }

  const routeUrl = parseHashUrl(url.hash);
  if (!routeUrl) {
    return { kind: 'thread-home' };
  }

  const segments = routeSegments(routeUrl);
  const first = segments[0]?.toLowerCase() || '';
  const second = segments[1] || null;

  if (first === 'thread' || first === 'threads') {
    return second
      ? { kind: 'thread', threadId: second }
      : { kind: 'thread-home' };
  }

  if (first === 'new' || first === 'workspace') {
    return {
      kind: 'new-thread',
      workspacePath:
        decodeLoose(routeUrl.searchParams.get('workspace')) ||
        decodeLoose(routeUrl.searchParams.get('workspacePath')) ||
        second ||
        null,
      agentId: decodeLoose(routeUrl.searchParams.get('agent')) || null,
      workflowId: decodeLoose(routeUrl.searchParams.get('workflow')) || null,
    };
  }

  if (first === 'automation') {
    return {
      kind: 'automation',
      automationId: second || decodeLoose(routeUrl.searchParams.get('id')),
    };
  }

  if (first === 'workflow' || first === 'workflows') {
    const taskId =
      second ||
      decodeLoose(routeUrl.searchParams.get('task')) ||
      decodeLoose(routeUrl.searchParams.get('taskId'));
    return taskId ? { kind: 'workflow-task', taskId } : { kind: 'view', view: 'tasks' };
  }

  if (first === 'settings') {
    return {
      kind: 'settings',
      tabId: normalizeSettingsTab(second || decodeLoose(routeUrl.searchParams.get('tab'))),
    };
  }

  // `#/capsules/<id>` opens the in-app Capsule preview; `#/capsules` stays the
  // gallery view below.
  if (first === 'capsules' && second) {
    return { kind: 'capsule', capsuleId: second };
  }

  const simpleView = SIMPLE_VIEW_SEGMENTS[first];
  if (simpleView) {
    return { kind: 'view', view: simpleView };
  }

  return { kind: 'thread-home' };
}

// The switch is exhaustive: every route kind maps to a view (the 6c-2b
// contentView selector relies on this being total).
export function contentViewForDesktopRoute(route: DesktopRoute): ContentView {
  switch (route.kind) {
    case 'thread-home':
    case 'thread':
    case 'new-thread':
      return 'thread';
    case 'automation':
      return 'automation';
    case 'workflow-task':
      return 'workflow';
    case 'settings':
      return 'settings';
    case 'capsule':
      return 'capsules';
    case 'view':
      return route.view;
  }
}

export function buildDesktopRouteHash(route: DesktopRoute): string {
  switch (route.kind) {
    case 'thread-home':
      return '#/thread';
    case 'thread':
      return `#/thread/${encodeSegment(route.threadId)}`;
    case 'new-thread': {
      const params = new URLSearchParams();
      appendParam(params, 'workspace', route.workspacePath);
      appendParam(
        params,
        'agent',
        route.workflowId ? null : route.agentId && route.agentId !== 'claude' ? route.agentId : null,
      );
      appendParam(params, 'workflow', route.workflowId);
      const query = params.toString();
      return query ? `#/new?${query}` : '#/new';
    }
    case 'workflow-task':
      return `#/workflow/${encodeSegment(route.taskId)}`;
    case 'automation':
      return route.automationId
        ? `#/automation/${encodeSegment(route.automationId)}`
        : '#/automation';
    case 'settings':
      return route.tabId
        ? `#/settings/${encodeSegment(route.tabId)}`
        : '#/settings';
    case 'capsule':
      return `#/capsules/${encodeSegment(route.capsuleId)}`;
    case 'view': {
      return `#/${route.view}`;
    }
  }
}

/**
 * Normalize a route to its hash round-trip form (batch 4a):
 * `parseDesktopRoute(buildDesktopRouteHash(route))` without the string
 * detour. buildDesktopRouteHash drops empty/default params — notably the
 * `new-thread` agent param when it is the 'claude' default or a workflow
 * is set — so a state-derived route and its parsed echo only compare
 * equal in canonical form. The route store commits canonical routes and
 * desktopRoutesEqual canonicalizes both sides.
 */
export function canonicalDesktopRoute(route: DesktopRoute): DesktopRoute {
  const trimmed = (value: string | null | undefined) => value?.trim() || null;
  switch (route.kind) {
    case 'thread':
      return { kind: 'thread', threadId: route.threadId.trim() };
    case 'new-thread': {
      const workspacePath = trimmed(route.workspacePath);
      const workflowId = trimmed(route.workflowId);
      const rawAgentId = trimmed(route.agentId);
      const agentId =
        workflowId || !rawAgentId || rawAgentId === 'claude'
          ? null
          : rawAgentId;
      return { kind: 'new-thread', workspacePath, agentId, workflowId };
    }
    case 'workflow-task':
      return { kind: 'workflow-task', taskId: route.taskId.trim() };
    case 'automation':
      return { kind: 'automation', automationId: trimmed(route.automationId) };
    case 'settings':
      return { kind: 'settings', tabId: route.tabId ?? null };
    case 'capsule':
      return { kind: 'capsule', capsuleId: route.capsuleId.trim() };
    case 'thread-home':
    case 'view':
      return route;
  }
}

/**
 * Route equality in canonical (hash round-trip) form (batch 4a): two
 * routes are equal exactly when they address the same hash.
 */
export function desktopRoutesEqual(a: DesktopRoute, b: DesktopRoute): boolean {
  const ca = canonicalDesktopRoute(a);
  const cb = canonicalDesktopRoute(b);
  if (ca.kind !== cb.kind) {
    return false;
  }
  switch (ca.kind) {
    case 'thread-home':
      return true;
    case 'thread':
      return ca.threadId === (cb as typeof ca).threadId;
    case 'new-thread': {
      const other = cb as typeof ca;
      return (
        ca.workspacePath === other.workspacePath &&
        ca.agentId === other.agentId &&
        ca.workflowId === other.workflowId
      );
    }
    case 'workflow-task':
      return ca.taskId === (cb as typeof ca).taskId;
    case 'automation':
      return ca.automationId === (cb as typeof ca).automationId;
    case 'settings':
      return ca.tabId === (cb as typeof ca).tabId;
    case 'capsule':
      return ca.capsuleId === (cb as typeof ca).capsuleId;
    case 'view':
      return ca.view === (cb as typeof ca).view;
  }
}
