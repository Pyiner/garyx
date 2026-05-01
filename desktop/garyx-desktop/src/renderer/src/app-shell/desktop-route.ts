import type { SettingsTabId } from '../GatewaySettingsPanel';

import type { ContentView } from './types';

export type DesktopRoute =
  | { kind: 'thread-home' }
  | { kind: 'thread'; threadId: string }
  | { kind: 'new-thread'; workspacePath?: string | null; agentId?: string | null }
  | { kind: 'automation'; automationId?: string | null }
  | { kind: 'settings'; tabId?: SettingsTabId | null }
  | { kind: 'view'; view: Exclude<ContentView, 'thread' | 'automation' | 'settings'> };

const SIMPLE_VIEW_SEGMENTS: Record<string, Exclude<ContentView, 'thread' | 'automation' | 'settings'>> = {
  browser: 'browser',
  bots: 'bots',
  'auto-research': 'auto_research',
  auto_research: 'auto_research',
  agents: 'agents',
  teams: 'teams',
  skills: 'skills',
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
    };
  }

  if (first === 'automation') {
    return {
      kind: 'automation',
      automationId: second || decodeLoose(routeUrl.searchParams.get('id')),
    };
  }

  if (first === 'settings') {
    return {
      kind: 'settings',
      tabId: normalizeSettingsTab(second || decodeLoose(routeUrl.searchParams.get('tab'))),
    };
  }

  const simpleView = SIMPLE_VIEW_SEGMENTS[first];
  if (simpleView) {
    return { kind: 'view', view: simpleView };
  }

  return { kind: 'thread-home' };
}

export function contentViewForDesktopRoute(route: DesktopRoute): ContentView | null {
  switch (route.kind) {
    case 'thread-home':
    case 'thread':
    case 'new-thread':
      return 'thread';
    case 'automation':
      return 'automation';
    case 'settings':
      return 'settings';
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
      appendParam(params, 'agent', route.agentId && route.agentId !== 'claude' ? route.agentId : null);
      const query = params.toString();
      return query ? `#/new?${query}` : '#/new';
    }
    case 'automation':
      return route.automationId
        ? `#/automation/${encodeSegment(route.automationId)}`
        : '#/automation';
    case 'settings':
      return route.tabId
        ? `#/settings/${encodeSegment(route.tabId)}`
        : '#/settings';
    case 'view': {
      const segment = route.view === 'auto_research' ? 'auto-research' : route.view;
      return `#/${segment}`;
    }
  }
}

export function replaceDesktopRoute(route: DesktopRoute): void {
  const nextHash = buildDesktopRouteHash(route);
  if (globalThis.window?.location.hash === nextHash) {
    return;
  }
  globalThis.window?.history.replaceState(null, '', nextHash);
}
