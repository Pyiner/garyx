import AntigravityColor from '@lobehub/icons/es/Antigravity/components/Color';
import ClaudeCodeColor from '@lobehub/icons/es/ClaudeCode/components/Color';
import CodexColor from '@lobehub/icons/es/Codex/components/Color';
import GeminiColor from '@lobehub/icons/es/Gemini/components/Color';

import type {
  DesktopApiProviderType,
  DesktopProviderIconDescriptor,
} from '@shared/contracts';

type BuiltInAgentIconKey = 'antigravity' | 'claude' | 'codex' | 'traex' | 'gemini';

const BUILT_IN_AGENT_ICONS = {
  antigravity: AntigravityColor,
  claude: ClaudeCodeColor,
  codex: CodexColor,
  // TRAE CLI is a Codex fork; reuse the Codex glyph until a dedicated icon exists.
  traex: CodexColor,
  gemini: GeminiColor,
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
  if (normalized === 'traex' || normalized === 'trae' || normalized === 'trae_cli' || normalized === 'traecli') {
    return 'traex';
  }
  if (normalized === 'antigravity' || normalized === 'agy') {
    return 'antigravity';
  }
  if (normalized === 'gemini') {
    return 'gemini';
  }
  return null;
}

export function getProviderAgentIconKey(
  agentId?: string | null,
  providerType?: DesktopApiProviderType | null,
  providerIcon?: DesktopProviderIconDescriptor | null,
): BuiltInAgentIconKey | null {
  if (providerIcon?.key && providerIcon.key in BUILT_IN_AGENT_ICONS) {
    return providerIcon.key;
  }
  return normalizeAgentIconKey(agentId) || normalizeAgentIconKey(providerType);
}

export function hasProviderAgentIcon(
  agentId?: string | null,
  providerType?: DesktopApiProviderType | null,
  providerIcon?: DesktopProviderIconDescriptor | null,
): boolean {
  return Boolean(getProviderAgentIconKey(agentId, providerType, providerIcon));
}

type ProviderAgentIconProps = {
  agentId?: string | null;
  className?: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  providerType?: DesktopApiProviderType | null;
  size?: number | string;
};

export function ProviderAgentIcon({
  agentId,
  className,
  providerIcon,
  providerType,
  size = '1em',
}: ProviderAgentIconProps) {
  const iconKey = getProviderAgentIconKey(agentId, providerType, providerIcon);
  if (!iconKey) {
    return null;
  }

  const Icon = BUILT_IN_AGENT_ICONS[iconKey];
  return <Icon aria-hidden className={className} size={size} />;
}
