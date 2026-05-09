import type { ReactNode } from 'react';

import type { DesktopApiProviderType } from '@shared/contracts';

import {
  Avatar,
  AvatarFallback,
  AvatarImage,
} from '@/components/ui/avatar';
import { cn } from '@/lib/utils';

import { AgentsIcon } from '../icons';
import {
  ProviderAgentIcon,
  hasProviderAgentIcon,
} from './ProviderAgentIcon';

type AgentOptionKind = 'builtin' | 'agent' | 'team';

type AgentOptionAvatarProps = {
  agentId?: string | null;
  avatarDataUrl?: string | null;
  className?: string;
  kind?: AgentOptionKind;
  label?: string | null;
  providerType?: DesktopApiProviderType | null;
  size?: 'sm' | 'default' | 'lg';
};

function agentInitials(label?: string | null, agentId?: string | null): string {
  const source = (label || agentId || '').trim();
  if (!source) {
    return 'A';
  }
  const words = source
    .replace(/[()]/g, ' ')
    .split(/[\s/_-]+/)
    .map((word) => word.trim())
    .filter(Boolean);
  if (words.length >= 2) {
    return `${words[0][0]}${words[1][0]}`.toUpperCase();
  }
  return source.slice(0, 2).toUpperCase();
}

export function AgentOptionAvatar({
  agentId,
  avatarDataUrl,
  className,
  kind = 'agent',
  label,
  providerType,
  size = 'sm',
}: AgentOptionAvatarProps) {
  const showProviderIcon =
    !avatarDataUrl && hasProviderAgentIcon(agentId, providerType);

  return (
    <Avatar
      className={cn(
        'agent-option-avatar',
        kind === 'team' && 'agent-option-avatar--team',
        showProviderIcon && 'agent-option-avatar--provider',
        className,
      )}
      size={size}
    >
      {avatarDataUrl ? <AvatarImage alt="" src={avatarDataUrl} /> : null}
      <AvatarFallback>
        {showProviderIcon ? (
          <ProviderAgentIcon
            agentId={agentId}
            providerType={providerType}
            size="1em"
          />
        ) : kind === 'team' ? (
          <AgentsIcon />
        ) : (
          agentInitials(label, agentId)
        )}
      </AvatarFallback>
    </Avatar>
  );
}

export function AgentOptionRow({
  agentId,
  avatarDataUrl,
  children,
  detail,
  kind = 'agent',
  label,
  providerType,
}: AgentOptionAvatarProps & {
  children?: ReactNode;
  detail?: string | null;
}) {
  const text = label || agentId || '';

  return (
    <span className="agent-option-row">
      <AgentOptionAvatar
        agentId={agentId}
        avatarDataUrl={avatarDataUrl}
        kind={kind}
        label={text}
        providerType={providerType}
      />
      <span className="agent-option-copy">
        <span className="agent-option-label">{children || text}</span>
        {detail ? <span className="agent-option-detail">{detail}</span> : null}
      </span>
    </span>
  );
}
