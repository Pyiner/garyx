import ClaudeCodeColor from '@lobehub/icons/es/ClaudeCode/components/Color';
import CodexColor from '@lobehub/icons/es/Codex/components/Color';
import GeminiCliColor from '@lobehub/icons/es/GeminiCLI/components/Color';
import OpencodeMono from '@lobehub/icons/es/OpenCode/components/Mono';

import type { DesktopApiProviderType } from '@shared/contracts';

type BuiltInAgentIconKey = 'claude' | 'codex' | 'gemini' | 'opencode';

const BUILT_IN_AGENT_ICONS = {
  claude: ClaudeCodeColor,
  codex: CodexColor,
  gemini: GeminiCliColor,
  opencode: OpencodeMono,
};

function normalizeAgentIconKey(value?: string | null): BuiltInAgentIconKey | null {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) {
    return null;
  }
  if (normalized === 'claude' || normalized === 'claude_code') {
    return 'claude';
  }
  if (normalized === 'codex' || normalized === 'codex_app_server') {
    return 'codex';
  }
  if (normalized === 'gemini' || normalized === 'gemini_cli') {
    return 'gemini';
  }
  if (normalized === 'opencode') {
    return 'opencode';
  }
  return null;
}

export function getProviderAgentIconKey(
  agentId?: string | null,
  providerType?: DesktopApiProviderType | null,
): BuiltInAgentIconKey | null {
  return normalizeAgentIconKey(agentId) || normalizeAgentIconKey(providerType);
}

export function hasProviderAgentIcon(
  agentId?: string | null,
  providerType?: DesktopApiProviderType | null,
): boolean {
  return Boolean(getProviderAgentIconKey(agentId, providerType));
}

type ProviderAgentIconProps = {
  agentId?: string | null;
  className?: string;
  providerType?: DesktopApiProviderType | null;
  size?: number | string;
};

export function ProviderAgentIcon({
  agentId,
  className,
  providerType,
  size = '1em',
}: ProviderAgentIconProps) {
  const iconKey = getProviderAgentIconKey(agentId, providerType);
  if (!iconKey) {
    return null;
  }

  const Icon = BUILT_IN_AGENT_ICONS[iconKey];
  return <Icon aria-hidden className={className} size={size} />;
}
