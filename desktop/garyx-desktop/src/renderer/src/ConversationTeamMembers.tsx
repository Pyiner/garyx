import { useEffect, useRef, useState } from 'react';
import { IconUsersGroup } from '@tabler/icons-react';

import { AgentAvatar } from './app-shell/components/AgentAvatar';

export type ConversationTeamMember = {
  agentId: string;
  displayName: string;
  role: 'leader' | 'member';
  isCurrentAgent?: boolean;
  threadId?: string | null;
};

export type ConversationTeamSummary = {
  teamId: string;
  teamName: string;
  members: ConversationTeamMember[];
};

type ConversationTeamMembersProps = {
  teamSummary: ConversationTeamSummary;
  onOpenThread: (threadId: string) => void;
};

function memberCountLabel(count: number): string {
  return `${count} member${count === 1 ? '' : 's'}`;
}

export function ConversationTeamMembers({
  teamSummary,
  onOpenThread,
}: ConversationTeamMembersProps) {
  const [open, setOpen] = useState(false);
  const shellRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setOpen(false);
  }, [teamSummary.teamId]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function handlePointerDown(event: PointerEvent) {
      if (!shellRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    }

    window.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  const visibleMembers = teamSummary.members.slice(0, 3);
  const countLabel = memberCountLabel(teamSummary.members.length);

  return (
    <div className={`conversation-team-members${open ? ' is-open' : ''}`} ref={shellRef}>
      <button
        aria-expanded={open}
        aria-haspopup="dialog"
        className="conversation-team-members-trigger"
        onClick={() => {
          setOpen((current) => !current);
        }}
        title={`${teamSummary.teamName} • ${countLabel}`}
        type="button"
      >
        <span aria-hidden="true" className="conversation-team-members-stack">
          {visibleMembers.map((member) => (
            <span className="conversation-team-members-stack-item" key={member.agentId}>
              <AgentAvatar
                agentId={member.agentId}
                displayName={member.displayName}
                role={member.role}
                size={24}
              />
            </span>
          ))}
        </span>
        <span className="conversation-team-members-count">{countLabel}</span>
      </button>

      {open ? (
        <div
          aria-label={`${teamSummary.teamName} team members`}
          className="conversation-team-members-panel"
          role="dialog"
        >
          <div className="conversation-team-members-panel-header">
            <div className="conversation-team-members-panel-title">
              <div className="conversation-team-members-panel-eyebrow">
                <IconUsersGroup aria-hidden size={14} stroke={1.8} />
                <span>Team</span>
                <span aria-hidden="true">•</span>
                <span>{countLabel}</span>
              </div>
              <div className="conversation-team-members-panel-name">{teamSummary.teamName}</div>
              <div className="conversation-team-members-panel-subtitle">{teamSummary.teamId}</div>
            </div>
          </div>

          <div className="conversation-team-members-list">
            {teamSummary.members.map((member) => (
              <button
                className={`conversation-team-member-row${member.isCurrentAgent ? ' is-current' : ''}${member.threadId ? ' is-clickable' : ''}`}
                disabled={!member.threadId}
                key={member.agentId}
                onClick={() => {
                  if (!member.threadId) {
                    return;
                  }
                  setOpen(false);
                  onOpenThread(member.threadId);
                }}
                type="button"
              >
                <AgentAvatar
                  agentId={member.agentId}
                  displayName={member.displayName}
                  role={member.role}
                  size={32}
                />
                <div className="conversation-team-member-copy">
                  <div className="conversation-team-member-name-row">
                    <span className="conversation-team-member-name">{member.displayName}</span>
                    {member.role === 'leader' ? (
                      <span className="conversation-team-member-pill">Leader</span>
                    ) : null}
                    {member.isCurrentAgent ? (
                      <span className="conversation-team-member-pill is-current">Current</span>
                    ) : null}
                  </div>
                  <div className="conversation-team-member-meta">{member.agentId}</div>
                </div>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}
