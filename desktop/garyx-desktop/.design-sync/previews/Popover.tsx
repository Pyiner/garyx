import {
  Button,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Switch,
} from 'garyx-desktop';

export const Settings = () => (
  <Popover defaultOpen>
    <PopoverTrigger asChild>
      <Button variant="outline">Thread settings</Button>
    </PopoverTrigger>
    <PopoverContent
      style={{
        width: 248,
        background: '#ffffff',
        border: '1px solid #e4e4e2',
        borderRadius: 12,
        boxShadow: '0 8px 24px rgba(0,0,0,0.08), 0 2px 6px rgba(0,0,0,0.04)',
        padding: 12,
      }}
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        <div style={{ fontSize: 13, fontWeight: 600, color: '#0d0d0d' }}>Streaming</div>
        <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 14 }}>
          <span>Tool calls</span>
          <Switch defaultChecked />
        </label>
        <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 14 }}>
          <span>Tail thinking</span>
          <Switch />
        </label>
      </div>
    </PopoverContent>
  </Popover>
);
