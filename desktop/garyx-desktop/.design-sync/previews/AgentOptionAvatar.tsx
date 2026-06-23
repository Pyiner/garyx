import { AgentOptionAvatar } from 'garyx-desktop';

const row: React.CSSProperties = { display: 'flex', alignItems: 'center', gap: 14, padding: 16 };

export const Sizes = () => (
  <div style={row}>
    <AgentOptionAvatar agentId="claude" label="Claude Code" size="sm" />
    <AgentOptionAvatar agentId="claude" label="Claude Code" size="default" />
    <AgentOptionAvatar agentId="claude" label="Claude Code" size="lg" />
  </div>
);

export const Kinds = () => (
  <div style={row}>
    <AgentOptionAvatar agentId="codex" label="Codex" kind="agent" size="default" />
    <AgentOptionAvatar agentId="review-team" label="Review Team" kind="team" size="default" />
    <AgentOptionAvatar agentId="gemini" label="Gemini" kind="agent" size="default" />
  </div>
);

export const Initials = () => (
  <div style={row}>
    <AgentOptionAvatar label="Release Manager" size="default" />
    <AgentOptionAvatar label="Backend Team" kind="team" size="default" />
    <AgentOptionAvatar label="QA Bot" size="default" />
  </div>
);
