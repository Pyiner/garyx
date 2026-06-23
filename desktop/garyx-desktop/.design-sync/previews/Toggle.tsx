import { Toggle } from 'garyx-desktop'
import { Bell, BellOff, Pin, Eye, Wifi } from 'lucide-react'

export function PressedStates() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
      <Toggle aria-label="Pin thread">
        <Pin />
      </Toggle>
      <Toggle defaultPressed aria-label="Pin thread pressed">
        <Pin />
      </Toggle>
    </div>
  )
}

export function WithLabels() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
      <Toggle defaultPressed>
        <Bell />
        Notifications
      </Toggle>
      <Toggle>
        <Eye />
        Auto-follow
      </Toggle>
    </div>
  )
}

export function OutlineVariant() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
      <Toggle variant="outline" defaultPressed>
        <Wifi />
        Online
      </Toggle>
      <Toggle variant="outline">
        <BellOff />
        Muted
      </Toggle>
    </div>
  )
}

export function Sizes() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
      <Toggle size="sm" variant="outline" defaultPressed>
        <Pin />
      </Toggle>
      <Toggle size="default" variant="outline" defaultPressed>
        <Pin />
      </Toggle>
      <Toggle size="lg" variant="outline" defaultPressed>
        <Pin />
      </Toggle>
    </div>
  )
}

export function Disabled() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
      <Toggle disabled>
        <Bell />
        Notifications
      </Toggle>
      <Toggle disabled defaultPressed>
        <Pin />
        Pinned
      </Toggle>
    </div>
  )
}
