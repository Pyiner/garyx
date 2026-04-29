export type WebRoute =
  | {
      view: 'bot-console';
      botId: string | null;
      endpointKey: string | null;
      threadId: null;
    }
  | {
      view: 'threads';
      botId: string | null;
      endpointKey: string | null;
      threadId: null;
    }
  | {
      view: 'settings';
      botId: string | null;
      endpointKey: string | null;
      threadId: null;
    }
  | {
      view: 'status';
      botId: string | null;
      endpointKey: string | null;
      threadId: null;
    }
  | {
      view: 'logs';
      botId: string | null;
      endpointKey: string | null;
      threadId: null;
    }
  | {
      view: 'cron';
      botId: string | null;
      endpointKey: string | null;
      threadId: null;
    };

function decodeLoose(value: string | null): string | null {
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

function readSearchParam(url: URL, key: string): string | null {
  return decodeLoose(url.searchParams.get(key));
}

export function resolveWebRoute(href = window.location.href): WebRoute {
  const url = new URL(href);
  const view = readSearchParam(url, 'view');
  const botId = readSearchParam(url, 'bot');
  const endpointKey = readSearchParam(url, 'endpoint');

  if (view === 'threads') {
    return {
      view: 'threads',
      botId,
      endpointKey,
      threadId: null,
    };
  }

  if (view === 'settings') {
    return {
      view: 'settings',
      botId,
      endpointKey,
      threadId: null,
    };
  }

  if (view === 'status') {
    return {
      view: 'status',
      botId,
      endpointKey,
      threadId: null,
    };
  }

  if (view === 'logs') {
    return {
      view: 'logs',
      botId,
      endpointKey,
      threadId: null,
    };
  }

  if (view === 'cron') {
    return {
      view: 'cron',
      botId,
      endpointKey,
      threadId: null,
    };
  }

  return {
    view: 'bot-console',
    botId,
    endpointKey,
    threadId: null,
  };
}

export function buildWebRouteHref(
  next: {
    view: 'bot-console' | 'threads' | 'settings' | 'status' | 'logs' | 'cron';
    botId?: string | null;
    endpointKey?: string | null;
  },
  href = window.location.href,
): string {
  const url = new URL(href);
  url.searchParams.set('view', next.view);

  if (next.botId) {
    url.searchParams.set('bot', next.botId);
  } else {
    url.searchParams.delete('bot');
  }

  if (next.endpointKey) {
    url.searchParams.set('endpoint', next.endpointKey);
  } else {
    url.searchParams.delete('endpoint');
  }

  url.searchParams.delete('thread');
  url.searchParams.delete('thread_id');

  return url.toString();
}
