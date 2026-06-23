import { Switch } from 'garyx-desktop';

const Line =({ label, children }: { label: string; children: React.ReactNode }) => (
  <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 24, width: 280, fontSize: 14, color: '#0d0d0d' }}>
    <span>{label}</span>
    {children}
  </label>
);

export const States = () => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 16, padding: 16 }}>
    <Line label="Stream tool calls"><Switch defaultChecked /></Line>
    <Line label="Notify on completion"><Switch /></Line>
    <Line label="Auto-restart gateway"><Switch defaultChecked disabled /></Line>
    <Line label="Experimental routing"><Switch disabled /></Line>
  </div>
);
