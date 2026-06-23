import { AgentAvatar } from 'garyx-desktop';

const row: React.CSSProperties = { display: 'flex', alignItems: 'center', gap: 14, padding: 16 };

export const TeamRoster = () => (
  <div style={row}>
    <AgentAvatar agentId="claude" displayName="Claude Code" role="leader" size={40} />
    <AgentAvatar agentId="codex" displayName="Codex" role="member" size={40} />
    <AgentAvatar agentId="gemini" displayName="Gemini" role="member" size={40} />
    <AgentAvatar agentId="research-bot" displayName="Research Bot" role="member" size={40} />
    <AgentAvatar agentId="ops-team" displayName="Ops Team" role="member" size={40} />
  </div>
);

export const Active = () => (
  <div style={row}>
    <AgentAvatar agentId="claude" displayName="Claude Code" role="leader" size={40} active />
    <AgentAvatar agentId="codex" displayName="Codex" role="member" size={40} />
  </div>
);

export const Sizes = () => (
  <div style={row}>
    <AgentAvatar agentId="claude" displayName="Claude Code" role="leader" size={24} />
    <AgentAvatar agentId="claude" displayName="Claude Code" role="leader" size={36} />
    <AgentAvatar agentId="claude" displayName="Claude Code" role="leader" size={48} />
  </div>
);
