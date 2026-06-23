import { Input, Label } from 'garyx-desktop'

export function InputStates() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 360 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Label>Agent name</Label>
        <Input defaultValue="Release Manager" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Label>Workspace path</Label>
        <Input placeholder="/Users/test/repos/garyx" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Label>Gateway URL (disabled)</Label>
        <Input disabled defaultValue="https://gateway.local:8787" />
      </div>
    </div>
  )
}

export function InputTypes() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 360 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Label>Bot token</Label>
        <Input type="password" defaultValue="telegram-bot-secret" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Label>Channel admin email</Label>
        <Input type="email" placeholder="ops@example.com" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Label>Max concurrent runs</Label>
        <Input type="number" defaultValue={4} />
      </div>
    </div>
  )
}
