import type {
  DesktopApiProviderType,
  DesktopProviderIconDescriptor,
} from '@shared/contracts';

import antigravityAvatarUrl from '../../assets/provider-avatars/antigravity.png';
import claudeAvatarUrl from '../../assets/provider-avatars/claude.png';
import codexAvatarUrl from '../../assets/provider-avatars/codex.png';
import grokAvatarUrl from '../../assets/provider-avatars/grok.png';
import traeAvatarUrl from '../../assets/provider-avatars/trae.png';

type BuiltInAgentIconKey = 'antigravity' | 'claude' | 'codex' | 'traex' | 'gemini' | 'grok';

const BUILT_IN_AGENT_ICONS = {
  antigravity: antigravityAvatarUrl,
  claude: claudeAvatarUrl,
  codex: codexAvatarUrl,
  traex: traeAvatarUrl,
  gemini: antigravityAvatarUrl,
  grok: grokAvatarUrl,
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
  if (
    normalized === 'traex' ||
    normalized === 'trae' ||
    normalized === 'trae_cli' ||
    normalized === 'traecli'
  ) {
    return 'traex';
  }
  if (normalized === 'antigravity' || normalized === 'agy') {
    return 'antigravity';
  }
  if (normalized === 'gemini') {
    return 'gemini';
  }
  if (normalized === 'grok' || normalized === 'grok_build' || normalized === 'grok-build') {
    return 'grok';
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

  const avatarUrl = BUILT_IN_AGENT_ICONS[iconKey];
  return (
    <img
      alt=""
      aria-hidden
      className={className}
      draggable={false}
      src={avatarUrl}
      style={{ height: size, width: size }}
    />
  );
}
