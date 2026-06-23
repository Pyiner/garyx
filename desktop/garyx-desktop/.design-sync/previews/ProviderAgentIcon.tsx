import { ProviderAgentIcon } from 'garyx-desktop';

const cell: React.CSSProperties = { display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6, fontSize: 12, color: '#555' };

export const Providers = () => (
  <div style={{ display: 'flex', gap: 28, padding: 20, alignItems: 'flex-start' }}>
    <div style={cell}><ProviderAgentIcon agentId="claude" size={32} /><span>Claude</span></div>
    <div style={cell}><ProviderAgentIcon agentId="codex" size={32} /><span>Codex</span></div>
    <div style={cell}><ProviderAgentIcon agentId="gemini" size={32} /><span>Gemini</span></div>
    <div style={cell}><ProviderAgentIcon agentId="traex" size={32} /><span>TRAE</span></div>
  </div>
);

export const Sizes = () => (
  <div style={{ display: 'flex', gap: 20, padding: 20, alignItems: 'center' }}>
    <ProviderAgentIcon agentId="claude" size={16} />
    <ProviderAgentIcon agentId="claude" size={24} />
    <ProviderAgentIcon agentId="claude" size={40} />
  </div>
);
