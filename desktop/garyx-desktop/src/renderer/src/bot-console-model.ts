import type {
  ConfiguredBot,
  DesktopBotConversationNode,
  DesktopBotConsoleStatus,
  DesktopBotConsoleSummary,
  DesktopChannelEndpoint,
} from '@shared/contracts';

export type BotConversationKind = 'private' | 'group' | 'topic' | 'unknown';

export function channelDisplayName(channel?: string | null): string {
  const normalized = channel?.trim();
  if (!normalized) {
    return 'Channel';
  }
  return normalized
    .split(/[-_]/g)
    .filter(Boolean)
    .map((segment) => segment.slice(0, 1).toUpperCase() + segment.slice(1))
    .join(' ');
}

export function botGroupIdForEndpoint(
  endpoint: Pick<DesktopChannelEndpoint, 'channel' | 'accountId'>,
): string {
  return `${endpoint.channel || 'unknown'}::${endpoint.accountId || 'default'}`;
}

export function latestEndpointActivity(
  endpoint: DesktopChannelEndpoint,
): string | null {
  return endpoint.lastInboundAt || endpoint.lastDeliveryAt || endpoint.threadUpdatedAt || null;
}

export function endpointConversationKind(
  endpoint: DesktopChannelEndpoint,
): BotConversationKind {
  if (endpoint.conversationKind) {
    return endpoint.conversationKind;
  }

  const scope = endpoint.threadScope?.trim();
  if (scope) {
    return scope === endpoint.chatId.trim() ? 'group' : 'topic';
  }
  if (
    endpoint.deliveryTargetType === 'open_id'
    || !endpoint.chatId.trim()
    || !endpoint.peerId.trim()
    || endpoint.chatId.trim() === endpoint.peerId.trim()
  ) {
    return 'private';
  }
  return 'group';
}

function endpointConversationBaseLabel(
  endpoint: DesktopChannelEndpoint,
): string {
  const label =
    endpoint.conversationLabel?.trim()
    || endpoint.displayLabel.trim()
    || endpoint.threadLabel?.trim()
    || endpoint.chatId.trim()
    || endpoint.peerId.trim();
  return label || 'Conversation';
}

function endpointConversationTitle(
  endpoint: DesktopChannelEndpoint,
  kind: BotConversationKind,
): string {
  const base = endpointConversationBaseLabel(endpoint);
  if (kind !== 'topic') {
    return base;
  }
  return endpoint.conversationLabel?.trim() ? base : `${base} · Topic`;
}

function endpointConversationId(
  endpoint: DesktopChannelEndpoint,
  kind: BotConversationKind,
): string {
  if (kind === 'topic' && endpoint.threadScope?.trim()) {
    return `${endpoint.channel}:${endpoint.accountId}:${endpoint.chatId}:${endpoint.threadScope.trim()}`;
  }
  if (kind === 'group') {
    return `${endpoint.channel}:${endpoint.accountId}:${endpoint.chatId}`;
  }
  return endpoint.endpointKey;
}

function endpointScopeHint(endpoint: DesktopChannelEndpoint): string | null {
  const scope = endpoint.threadScope?.trim();
  if (!scope) {
    return null;
  }
  if (scope.length <= 6) {
    return scope;
  }
  return scope.slice(-6);
}

export function buildBotSidebarThreadEntries(
  group: DesktopBotConsoleSummary,
): DesktopBotConversationNode[] {
  const deduped = new Map<string, DesktopBotConversationNode>();

  for (const endpoint of group.endpoints) {
    if (!endpoint.threadId || endpoint.threadId === group.mainThreadId) {
      continue;
    }
    const kind = endpointConversationKind(endpoint);
    if (kind !== 'group' && kind !== 'topic') {
      continue;
    }
    const id = endpointConversationId(endpoint, kind);
    if (deduped.has(id)) {
      continue;
    }
    deduped.set(id, {
      id,
      endpoint,
      kind,
      title: endpointConversationTitle(endpoint, kind),
      badge: kind === 'topic' ? 'Topic' : 'Group',
      latestActivity: latestEndpointActivity(endpoint),
      openable: Boolean(endpoint.threadId),
    });
  }

  const entries = [...deduped.values()].sort((left, right) => {
    return (right.latestActivity || '').localeCompare(left.latestActivity || '')
      || left.title.localeCompare(right.title);
  });

  const titleCounts = new Map<string, number>();
  for (const entry of entries) {
    titleCounts.set(entry.title, (titleCounts.get(entry.title) || 0) + 1);
  }

  return entries.map((entry) => {
    if ((titleCounts.get(entry.title) || 0) < 2) {
      return entry;
    }
    const hint = endpointScopeHint(entry.endpoint);
    if (!hint) {
      return entry;
    }
    return {
      ...entry,
      title: `${entry.title} · ${hint}`,
    };
  });
}

function statusForBot(
  endpoints: DesktopChannelEndpoint[],
  mainThreadId?: string | null,
): DesktopBotConsoleStatus {
  return mainThreadId || endpoints.some((endpoint) => Boolean(endpoint.threadId)) ? 'connected' : 'idle';
}

export function primaryBotEndpoint(
  group: DesktopBotConsoleSummary,
): DesktopChannelEndpoint | null {
  return group.defaultOpenEndpoint
    || group.mainEndpoint
    || group.endpoints.find((endpoint) => Boolean(endpoint.threadId))
    || group.endpoints[0]
    || null;
}

export function buildBotGroups(
  endpoints: DesktopChannelEndpoint[],
  configuredBots: ConfiguredBot[] = [],
  botMainThreads: Record<string, string> = {},
  botConsoles: DesktopBotConsoleSummary[] = [],
): DesktopBotConsoleSummary[] {
  const groups = new Map<string, DesktopBotConsoleSummary>();
  const configuredGroupIds = new Set<string>();
  const orderByGroupId = new Map<string, number>();
  let nextOrder = 0;
  for (const bot of configuredBots) {
    const id = `${bot.channel}::${bot.accountId}`;
    if (!orderByGroupId.has(id)) {
      orderByGroupId.set(id, nextOrder);
      nextOrder += 1;
    }
  }
  for (const group of botConsoles) {
    if (!orderByGroupId.has(group.id)) {
      orderByGroupId.set(group.id, nextOrder);
      nextOrder += 1;
    }
    configuredGroupIds.add(group.id);
    groups.set(group.id, {
      ...group,
      conversationNodes: [...group.conversationNodes],
      endpoints: [...group.endpoints],
    });
  }
  for (const bot of configuredBots) {
    const id = `${bot.channel}::${bot.accountId}`;
    configuredGroupIds.add(id);
    const existing = groups.get(id);
    groups.set(id, {
      id,
      channel: bot.channel,
      accountId: bot.accountId,
      title: existing?.title || bot.displayName,
      subtitle: existing?.subtitle || `${channelDisplayName(bot.channel)} Bot · ${bot.accountId}`,
      rootBehavior: existing?.rootBehavior || bot.rootBehavior,
      status: existing?.status || 'idle',
      latestActivity: existing?.latestActivity || null,
      endpointCount: existing?.endpointCount || 0,
      boundEndpointCount: existing?.boundEndpointCount || 0,
      mainEndpointStatus: existing?.mainEndpointStatus || bot.mainEndpointStatus,
      mainEndpoint: existing?.mainEndpoint ?? bot.mainEndpoint ?? null,
      mainThreadId:
        existing?.mainThreadId
        || botMainThreads[id]
        || bot.mainEndpointThreadId
        || bot.mainEndpoint?.threadId
        || null,
      defaultOpenEndpoint: existing?.defaultOpenEndpoint ?? bot.defaultOpenEndpoint ?? bot.mainEndpoint ?? null,
      defaultOpenThreadId:
        existing?.defaultOpenThreadId
        || bot.defaultOpenThreadId
        || bot.mainEndpointThreadId
        || bot.defaultOpenEndpoint?.threadId
        || bot.mainEndpoint?.threadId
        || null,
      conversationNodes: existing?.conversationNodes || [],
      endpoints: existing?.endpoints || [],
      workspaceDir: existing?.workspaceDir || bot.workspaceDir,
    });
  }

  for (const endpoint of endpoints) {
    const channel = endpoint.channel || 'unknown';
    const accountId = endpoint.accountId || 'default';
    const id = `${channel}::${accountId}`;
    if (configuredGroupIds.size > 0 && !configuredGroupIds.has(id)) {
      continue;
    }
    if (!orderByGroupId.has(id)) {
      orderByGroupId.set(id, nextOrder);
      nextOrder += 1;
    }
    const existing = groups.get(id) || {
      id,
      channel,
      accountId,
      title: `${channel}/${accountId}`,
      subtitle: `${channelDisplayName(channel)} Bot · ${accountId}`,
      rootBehavior: 'open_default' as const,
      status: 'idle' as DesktopBotConsoleStatus,
      latestActivity: null,
      endpointCount: 0,
      boundEndpointCount: 0,
      mainEndpointStatus: 'unresolved' as const,
      mainEndpoint: null,
      mainThreadId: botMainThreads[id] || null,
      defaultOpenEndpoint: null,
      defaultOpenThreadId: null,
      conversationNodes: [],
      endpoints: [],
      workspaceDir: null,
    };

    if (!existing.endpoints.some((entry) => entry.endpointKey === endpoint.endpointKey)) {
      existing.endpoints.push(endpoint);
      existing.endpointCount += 1;
      if (endpoint.threadId) {
        existing.boundEndpointCount += 1;
      }
    }
    const activity = latestEndpointActivity(endpoint);
    if (activity && (!existing.latestActivity || activity > existing.latestActivity)) {
      existing.latestActivity = activity;
    }
    existing.status = statusForBot(existing.endpoints, existing.mainThreadId);
    groups.set(id, existing);
  }

  return [...groups.values()]
    .map((group) => ({
      ...group,
      status: statusForBot(group.endpoints, group.mainThreadId),
      mainThreadId: group.mainThreadId || botMainThreads[group.id] || group.mainEndpoint?.threadId || null,
      defaultOpenEndpoint: group.defaultOpenEndpoint || group.mainEndpoint || null,
      defaultOpenThreadId:
        group.defaultOpenThreadId
        || group.mainThreadId
        || botMainThreads[group.id]
        || group.defaultOpenEndpoint?.threadId
        || group.mainEndpoint?.threadId
        || null,
      conversationNodes: group.conversationNodes.length
        ? group.conversationNodes
        : buildBotSidebarThreadEntries({
            ...group,
            status: statusForBot(group.endpoints, group.mainThreadId),
            mainThreadId: group.mainThreadId || botMainThreads[group.id] || group.mainEndpoint?.threadId || null,
          }),
      endpoints: [...group.endpoints].sort((left, right) => {
        const rightActivity = latestEndpointActivity(right) || '';
        const leftActivity = latestEndpointActivity(left) || '';
        return rightActivity.localeCompare(leftActivity) || left.displayLabel.localeCompare(right.displayLabel);
      }),
    }))
    .sort((left, right) => {
      const leftOrder = orderByGroupId.get(left.id) ?? Number.MAX_SAFE_INTEGER;
      const rightOrder = orderByGroupId.get(right.id) ?? Number.MAX_SAFE_INTEGER;
      return leftOrder - rightOrder
        || left.title.localeCompare(right.title)
        || left.id.localeCompare(right.id);
    });
}
