import { Label, Input, Checkbox } from 'garyx-desktop'
import { Bot } from 'lucide-react'

export function LabelBasic() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 320 }}>
      <Label htmlFor="agent-name">Agent name</Label>
      <Label htmlFor="agent-icon">
        <Bot size={15} />
        Default agent
      </Label>
    </div>
  )
}

export function LabelWithControl() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, maxWidth: 320 }}>
      <Label htmlFor="gateway-host">Gateway host</Label>
      <Input id="gateway-host" defaultValue="gateway.local" />
      <Label
        htmlFor="notify-thread"
        style={{ display: 'flex', alignItems: 'center', gap: 8 }}
      >
        <Checkbox id="notify-thread" defaultChecked />
        Notify current thread
      </Label>
    </div>
  )
}

export function LabelDisabled() {
  return (
    <div className="group" data-disabled="true" style={{ display: 'flex', flexDirection: 'column', gap: 8, maxWidth: 320 }}>
      <Label htmlFor="archived-workspace">Archived workspace</Label>
      <Input id="archived-workspace" disabled defaultValue="/Users/test/repos/legacy" className="peer" />
    </div>
  )
}
