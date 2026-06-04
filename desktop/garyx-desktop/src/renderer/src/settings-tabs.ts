export type SettingsTabId =
  | 'connection'
  | 'gateway'
  | 'provider'
  | 'channels'
  | 'labs'
  | 'performance'
  | 'commands'
  | 'mcp';

export const SETTINGS_TABS: Array<{
  id: SettingsTabId;
  label: string;
  eyebrow: string;
  description: string;
}> = [
  {
    id: 'labs',
    label: 'General',
    eyebrow: 'General',
    description: 'Desktop app behavior, updates, and experimental surfaces.',
  },
  {
    id: 'gateway',
    label: 'Gateway',
    eyebrow: 'Gateway',
    description: 'Gateway URL and storage.',
  },
  {
    id: 'provider',
    label: 'Provider',
    eyebrow: 'Providers',
    description: 'Desktop-side Claude env overrides and Codex auth.',
  },
  {
    id: 'performance',
    label: 'Performance',
    eyebrow: 'Diagnostics',
    description: 'Renderer health, slow calls, frame stalls, long tasks, and memory samples.',
  },
  {
    id: 'channels',
    label: 'Channels',
    eyebrow: 'Bots',
    description: 'Telegram and Feishu/Lark bot accounts.',
  },
  {
    id: 'commands',
    label: 'Commands',
    eyebrow: 'Slash Commands',
    description: 'Manage global prompt shortcuts.',
  },
  {
    id: 'mcp',
    label: 'MCP Servers',
    eyebrow: 'MCP',
    description: 'Manage external MCP server definitions and local tool config sync.',
  },
];
