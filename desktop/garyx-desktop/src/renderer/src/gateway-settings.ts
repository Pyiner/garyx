const DEFAULT_GATEWAY_HOST = '0.0.0.0';
const DEFAULT_GATEWAY_PORT = 31337;
const DEFAULT_IMAGE_MODEL = 'gemini-3.1-flash-image-preview';
const DEFAULT_CHANNEL_AGENT_ID = 'claude';

export type GatewaySettingsMode = 'form' | 'json';

export function cloneJson<T>(value: T): T {
  return JSON.parse(JSON.stringify(value ?? {})) as T;
}

export function parseList(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function stringifyList(items: unknown): string {
  if (!Array.isArray(items)) {
    return '';
  }

  return items
    .map((item) => String(item).trim())
    .filter(Boolean)
    .join(', ');
}

export function stringifyJsonBlock(value: unknown): string {
  const config = cloneJson(ensureRecord(value));
  stripLegacyChannelAccountFields(config);
  stripLegacyAccountAgentBindings(config);

  const agentDefaults = ensureRecord(config.agent_defaults);
  delete agentDefaults.workspace_dir;
  config.agent_defaults = agentDefaults;

  const sessions = ensureRecord(config.sessions);
  delete sessions.redis;
  delete sessions.store_type;
  delete sessions.thread_history_backend;
  const dataDir = coerceOptionalString(sessions.data_dir);
  if (dataDir) {
    sessions.data_dir = dataDir;
  } else {
    delete sessions.data_dir;
  }
  if (Object.keys(sessions).length === 0) {
    delete config.sessions;
  } else {
    config.sessions = sessions;
  }

  const cron = ensureRecord(config.cron);
  delete cron.enabled;
  cron.jobs = Array.isArray(cron.jobs) ? cron.jobs : [];
  if (cron.jobs.length === 0) {
    delete config.cron;
  } else {
    config.cron = cron;
  }

  return JSON.stringify(config, null, 2);
}

function ensureRecord(value: unknown): Record<string, any> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, any>)
    : {};
}

function coerceInteger(value: unknown, fallback: number): number {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return Math.trunc(value);
  }

  const parsed = Number.parseInt(String(value ?? ''), 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function coerceNumber(value: unknown, fallback: number): number {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value;
  }

  const parsed = Number.parseFloat(String(value ?? ''));
  return Number.isFinite(parsed) ? parsed : fallback;
}

function coerceOptionalString(value: unknown): string | null {
  const text = typeof value === 'string' ? value.trim() : '';
  return text ? text : null;
}

function defaultTelegramPluginConfig() {
  return {
    token: '',
    owner_target: null as null | { target_type: string; target_id: string },
    groups: {},
  };
}

function defaultFeishuPluginConfig() {
  return {
    app_id: '',
    app_secret: '',
    domain: 'feishu',
    require_mention: true,
    topic_session_mode: 'disabled',
  };
}

function defaultWeixinPluginConfig() {
  return {
    token: '',
    uin: '',
    base_url: 'https://ilinkai.weixin.qq.com',
  };
}

function ensureFlatChannelBuckets(channels: Record<string, any>): Record<string, any> {
  const legacyPlugins = ensureRecord(channels.plugins);
  for (const [channelId, channelValue] of Object.entries(legacyPlugins)) {
    if (!(channelId in channels)) {
      channels[channelId] = channelValue;
    }
  }
  delete channels.plugins;
  return channels;
}

function stripLegacyChannelAccountFields(config: Record<string, any>): void {
  const channelAccountGroups = [config.channels?.api?.accounts];

  for (const accounts of channelAccountGroups) {
    if (!accounts || typeof accounts !== 'object' || Array.isArray(accounts)) {
      continue;
    }

    for (const account of Object.values(accounts)) {
      if (!account || typeof account !== 'object' || Array.isArray(account)) {
        continue;
      }
      delete (account as Record<string, unknown>).agent_provider;
      const name = coerceOptionalString((account as Record<string, unknown>).name);
      if (name) {
        (account as Record<string, unknown>).name = name;
      } else {
        delete (account as Record<string, unknown>).name;
      }
      const workspaceDir = coerceOptionalString((account as Record<string, unknown>).workspace_dir);
      if (workspaceDir) {
        (account as Record<string, unknown>).workspace_dir = workspaceDir;
      } else {
        delete (account as Record<string, unknown>).workspace_dir;
      }
      const agentId = coerceOptionalString((account as Record<string, unknown>).agent_id);
      if (agentId && agentId !== DEFAULT_CHANNEL_AGENT_ID) {
        (account as Record<string, unknown>).agent_id = agentId;
      } else {
        delete (account as Record<string, unknown>).agent_id;
      }
    }
  }

  const channels = ensureFlatChannelBuckets(ensureRecord(config.channels));
  for (const [channelId, channelConfig] of Object.entries(channels)) {
    if (channelId === 'api') {
      continue;
    }
    const pluginConfig = ensureRecord(channelConfig);
    const accounts = ensureRecord(ensureRecord(pluginConfig).accounts);
    for (const account of Object.values(accounts)) {
      if (!account || typeof account !== 'object' || Array.isArray(account)) {
        continue;
      }
      delete (account as Record<string, unknown>).agent_provider;
      const name = coerceOptionalString((account as Record<string, unknown>).name);
      if (name) {
        (account as Record<string, unknown>).name = name;
      } else {
        delete (account as Record<string, unknown>).name;
      }
      const workspaceDir = coerceOptionalString((account as Record<string, unknown>).workspace_dir);
      if (workspaceDir) {
        (account as Record<string, unknown>).workspace_dir = workspaceDir;
      } else {
        delete (account as Record<string, unknown>).workspace_dir;
      }
      const agentId = coerceOptionalString((account as Record<string, unknown>).agent_id);
      if (agentId && agentId !== DEFAULT_CHANNEL_AGENT_ID) {
        (account as Record<string, unknown>).agent_id = agentId;
      } else {
        delete (account as Record<string, unknown>).agent_id;
      }
    }
  }
}

function stripLegacyAccountAgentBindings(config: Record<string, any>): void {
  const agents = ensureRecord(config.agents);
  const bindings = Array.isArray(agents.bindings) ? agents.bindings : null;
  if (!bindings) {
    return;
  }

  agents.bindings = bindings.filter((entry) => {
    const binding = ensureRecord(entry);
    const match = ensureRecord(binding.match);
    const hasAccountMatch =
      typeof match.channel === 'string'
      && (
        (typeof match.accountId === 'string' && match.accountId.trim())
        || (typeof match.account_id === 'string' && match.account_id.trim())
      );
    const hasEndpointScope = Boolean(
      match.peer
      || match.peer_id
      || match.guildId
      || match.guild_id
      || match.teamId
      || match.team_id,
    );

    return !hasAccountMatch || hasEndpointScope;
  });

  config.agents = agents;
}

export function defaultChannelAgentId(): string {
  return DEFAULT_CHANNEL_AGENT_ID;
}

export function defaultApiAccount() {
  return {
    enabled: true,
    name: null as string | null,
    agent_id: DEFAULT_CHANNEL_AGENT_ID,
    workspace_dir: null as string | null,
  };
}

export function defaultTelegramAccount() {
  return {
    token: '',
    enabled: true,
    name: null as string | null,
    agent_id: DEFAULT_CHANNEL_AGENT_ID,
    workspace_dir: null as string | null,
    owner_target: null as null | { target_type: string; target_id: string },
    groups: {},
  };
}

export function defaultFeishuAccount() {
  return {
    app_id: '',
    app_secret: '',
    enabled: true,
    domain: 'feishu',
    name: null as string | null,
    agent_id: DEFAULT_CHANNEL_AGENT_ID,
    workspace_dir: null as string | null,
    require_mention: true,
    topic_session_mode: 'disabled',
  };
}

function ensureChannelAgentId(value: unknown): string {
  const normalized = coerceOptionalString(value);
  return normalized || DEFAULT_CHANNEL_AGENT_ID;
}

function ensureOwnerTarget(value: unknown) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  const next = ensureRecord(value);
  const targetType = String(next.target_type || '').trim();
  const targetId = String(next.target_id || '').trim();
  if (!targetType && !targetId) {
    return null;
  }
  return {
    target_type: targetType,
    target_id: targetId,
  };
}

export function ensureGatewayConfig(raw: unknown): any {
  const config = cloneJson(ensureRecord(raw));

  config.agents = ensureRecord(config.agents);

  config.gateway = ensureRecord(config.gateway);
  config.gateway.host = String(config.gateway.host || DEFAULT_GATEWAY_HOST);
  config.gateway.port = coerceInteger(config.gateway.port, DEFAULT_GATEWAY_PORT);
  config.gateway.public_url = String(config.gateway.public_url || '');
  config.gateway.image_gen = ensureRecord(config.gateway.image_gen);
  config.gateway.image_gen.api_key = String(config.gateway.image_gen.api_key || '');
  config.gateway.image_gen.model = String(
    config.gateway.image_gen.model || DEFAULT_IMAGE_MODEL,
  );

  config.agent_defaults = ensureRecord(config.agent_defaults);
  delete config.agent_defaults.workspace_dir;
  config.agent_defaults.heartbeat = ensureRecord(config.agent_defaults.heartbeat);
  config.agent_defaults.heartbeat.enabled =
    typeof config.agent_defaults.heartbeat.enabled === 'boolean'
      ? config.agent_defaults.heartbeat.enabled
      : true;
  config.agent_defaults.heartbeat.every = String(
    config.agent_defaults.heartbeat.every || '3h',
  );
  config.agent_defaults.heartbeat.target = String(
    config.agent_defaults.heartbeat.target || 'last',
  );
  config.agent_defaults.heartbeat.ack_max_chars = coerceInteger(
    config.agent_defaults.heartbeat.ack_max_chars,
    500,
  );
  config.agent_defaults.heartbeat.active_hours = {
    start: String(config.agent_defaults.heartbeat.active_hours?.start || '09:00'),
    end: String(config.agent_defaults.heartbeat.active_hours?.end || '23:00'),
    timezone: String(config.agent_defaults.heartbeat.active_hours?.timezone || 'user'),
  };

  config.sessions = ensureRecord(config.sessions);
  delete config.sessions.redis;
  delete config.sessions.store_type;
  delete config.sessions.thread_history_backend;
  config.sessions.data_dir = coerceOptionalString(config.sessions.data_dir);

  config.desktop = ensureRecord(config.desktop);
  config.desktop.labs = ensureRecord(config.desktop.labs);
  config.desktop.labs.auto_research =
    typeof config.desktop.labs.auto_research === 'boolean'
      ? config.desktop.labs.auto_research
      : true;

  config.cron = ensureRecord(config.cron);
  delete config.cron.enabled;
  config.cron.jobs = Array.isArray(config.cron.jobs) ? config.cron.jobs : [];

  config.channels = ensureRecord(config.channels);
  ensureFlatChannelBuckets(config.channels);

  config.channels.api = ensureRecord(config.channels.api);
  config.channels.api.accounts = ensureRecord(config.channels.api.accounts);
  for (const [accountId, accountValue] of Object.entries(config.channels.api.accounts)) {
    const account: Record<string, any> = {
      ...defaultApiAccount(),
      ...ensureRecord(accountValue),
    };
    account.enabled = Boolean(account.enabled);
    account.name = coerceOptionalString(account.name);
    account.agent_id = ensureChannelAgentId(account.agent_id);
    account.workspace_dir = coerceOptionalString(account.workspace_dir);
    Reflect.deleteProperty(account, 'agent_provider');
    config.channels.api.accounts[accountId] = account;
  }

  for (const [pluginId, pluginValue] of Object.entries(config.channels)) {
    if (pluginId === 'api') {
      continue;
    }
    const plugin = ensureRecord(pluginValue);
    plugin.accounts = ensureRecord(plugin.accounts);
    for (const [accountId, accountValue] of Object.entries(plugin.accounts)) {
      const account = ensureRecord(accountValue);
      account.enabled = typeof account.enabled === 'boolean' ? account.enabled : true;
      account.name = coerceOptionalString(account.name);
      account.agent_id = ensureChannelAgentId(account.agent_id);
      account.workspace_dir = coerceOptionalString(account.workspace_dir);
      delete account.agent_provider;

      let pluginConfig = ensureRecord(account.config);
      if (pluginId === 'telegram') {
        pluginConfig = { ...defaultTelegramPluginConfig(), ...pluginConfig };
        pluginConfig.token = String(pluginConfig.token || '');
        pluginConfig.owner_target = ensureOwnerTarget(pluginConfig.owner_target);
        delete pluginConfig.capabilities;
        pluginConfig.groups = ensureRecord(pluginConfig.groups);
        Reflect.deleteProperty(pluginConfig, 'allow_from');
        Reflect.deleteProperty(pluginConfig, 'reply_to_mode');
        delete pluginConfig.webhook_url;
        delete pluginConfig.webhook_path;
        delete pluginConfig.webhook_secret;
      } else if (pluginId === 'feishu') {
        pluginConfig = { ...defaultFeishuPluginConfig(), ...pluginConfig };
        pluginConfig.app_id = String(pluginConfig.app_id || '');
        pluginConfig.app_secret = String(pluginConfig.app_secret || '');
        Reflect.deleteProperty(pluginConfig, 'verification_token');
        Reflect.deleteProperty(pluginConfig, 'encrypt_key');
        pluginConfig.domain = String(pluginConfig.domain || 'feishu');
        pluginConfig.require_mention = typeof pluginConfig.require_mention === 'boolean'
          ? pluginConfig.require_mention
          : true;
        pluginConfig.topic_session_mode = pluginConfig.topic_session_mode === 'enabled'
          ? 'enabled'
          : 'disabled';
        if ('owner_target' in pluginConfig) {
          const ownerTarget = ensureOwnerTarget(pluginConfig.owner_target);
          if (ownerTarget) {
            pluginConfig.owner_target = ownerTarget;
          } else {
            Reflect.deleteProperty(pluginConfig, 'owner_target');
          }
        }
        Reflect.deleteProperty(pluginConfig, 'dm_policy');
        Reflect.deleteProperty(pluginConfig, 'allow_from');
        Reflect.deleteProperty(pluginConfig, 'group_policy');
        Reflect.deleteProperty(pluginConfig, 'group_allow_from');
        Reflect.deleteProperty(pluginConfig, 'groups');
        Reflect.deleteProperty(pluginConfig, 'history_limit');
      } else if (pluginId === 'weixin') {
        pluginConfig = { ...defaultWeixinPluginConfig(), ...pluginConfig };
        pluginConfig.token = String(pluginConfig.token || '');
        pluginConfig.uin = String(pluginConfig.uin || '');
        pluginConfig.base_url = String(pluginConfig.base_url || 'https://ilinkai.weixin.qq.com');
      }

      account.config = pluginConfig;
      plugin.accounts[accountId] = account;
    }
    config.channels[pluginId] = plugin;
  }

  stripLegacyAccountAgentBindings(config);

  config.commands = Array.isArray(config.commands)
    ? config.commands.map((entry) => {
      const command = ensureRecord(entry);
      return {
        name: String(command.name || '').trim(),
        description: String(command.description || '').trim(),
        prompt: coerceOptionalString(command.prompt),
      };
    }).filter((entry) => entry.name && entry.description)
    : [];

  config.mcp_servers = ensureRecord(config.mcp_servers);
  for (const [serverName, serverValue] of Object.entries(config.mcp_servers)) {
    const server = ensureRecord(serverValue);
    config.mcp_servers[serverName] = {
      command: String(server.command || '').trim(),
      args: Array.isArray(server.args) ? server.args.map((value) => String(value)) : [],
      env: ensureRecord(server.env),
      enabled: typeof server.enabled === 'boolean' ? server.enabled : true,
      working_dir: coerceOptionalString(server.working_dir ?? server.workingDir),
    };
  }

  return config;
}
