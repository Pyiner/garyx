import { Checkbox, Label } from 'garyx-desktop'

export function CheckboxStates() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Checkbox id="cb-unchecked" />
        <Label htmlFor="cb-unchecked">Pause automation</Label>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Checkbox id="cb-checked" defaultChecked />
        <Label htmlFor="cb-checked">Notify current thread</Label>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Checkbox id="cb-indeterminate" checked="indeterminate" />
        <Label htmlFor="cb-indeterminate">Some channels selected</Label>
      </div>
    </div>
  )
}

export function CheckboxDisabled() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Checkbox id="cb-dis-off" disabled />
        <Label htmlFor="cb-dis-off">Restart gateway (locked)</Label>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Checkbox id="cb-dis-on" disabled defaultChecked />
        <Label htmlFor="cb-dis-on">Managed by gateway</Label>
      </div>
    </div>
  )
}

export function CheckboxList() {
  const channels = ['Telegram', 'Discord', 'Feishu', 'API']
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, maxWidth: 280 }}>
      {channels.map((name, i) => (
        <div key={name} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <Checkbox id={`ch-${name}`} defaultChecked={i < 2} />
          <Label htmlFor={`ch-${name}`}>{name}</Label>
        </div>
      ))}
    </div>
  )
}
