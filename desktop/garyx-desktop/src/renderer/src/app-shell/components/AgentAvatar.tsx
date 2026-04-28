/**
 * Circular avatar for a team agent member.
 * Shows the first letter of the display name with a role-based background color.
 * Color is injected via --agent-bg CSS custom property; all layout is in styles.css.
 */

export const MEMBER_COLORS = ['#10B981', '#F59E0B', '#8B5CF6', '#EF4444', '#06B6D4', '#EC4899'];
export const LEADER_COLOR = '#3B82F6';

function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) - hash + str.charCodeAt(i)) | 0;
  }
  return Math.abs(hash);
}

/** Returns the color associated with a given agent. */
export function getAgentColor(agentId: string, role: 'leader' | 'member'): string {
  return role === 'leader' ? LEADER_COLOR : MEMBER_COLORS[hashCode(agentId) % MEMBER_COLORS.length];
}

export interface AgentAvatarProps {
  agentId: string;
  displayName: string;
  role: 'leader' | 'member';
  size?: number;
  active?: boolean;
  onClick?: () => void;
}

export function AgentAvatar({ agentId, displayName, role, size = 36, active, onClick }: AgentAvatarProps) {
  const bgColor = getAgentColor(agentId, role);
  const initial = displayName.charAt(0).toUpperCase() || '?';
  const label = `${displayName} (${role})`;
  const className = `agent-avatar${active ? ' agent-avatar--active' : ''}${onClick ? ' agent-avatar--interactive' : ''}`;
  const style = {
    '--agent-bg': bgColor,
    '--agent-size': `${size}px`,
    '--agent-font': `${size * 0.42}px`,
  } as React.CSSProperties;

  if (onClick) {
    return (
      <button
        aria-label={label}
        className={className}
        onClick={onClick}
        style={style}
        title={label}
        type="button"
      >
        {initial}
      </button>
    );
  }

  return (
    <span
      aria-label={label}
      className={className}
      role="img"
      style={style}
      title={label}
    >
      {initial}
    </span>
  );
}
