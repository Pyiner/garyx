import {
  Avatar,
  AvatarImage,
  AvatarFallback,
  AvatarBadge,
  AvatarGroup,
  AvatarGroupCount,
} from 'garyx-desktop'
import { Check, Bot } from 'lucide-react'

export function Sizes() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
      <Avatar size="sm">
        <AvatarFallback>CC</AvatarFallback>
      </Avatar>
      <Avatar size="default">
        <AvatarFallback>GA</AvatarFallback>
      </Avatar>
      <Avatar size="lg">
        <AvatarFallback>OX</AvatarFallback>
      </Avatar>
    </div>
  )
}

export function WithStatusBadge() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
      <Avatar size="default">
        <AvatarFallback>CC</AvatarFallback>
        <AvatarBadge style={{ backgroundColor: '#00a240' }} />
      </Avatar>
      <Avatar size="lg">
        <AvatarFallback>GA</AvatarFallback>
        <AvatarBadge>
          <Check />
        </AvatarBadge>
      </Avatar>
      <Avatar size="lg">
        <AvatarFallback>
          <Bot style={{ width: 18, height: 18 }} />
        </AvatarFallback>
        <AvatarBadge style={{ backgroundColor: '#9ca3af' }} />
      </Avatar>
    </div>
  )
}

export function StackedGroup() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <AvatarGroup>
        <Avatar>
          <AvatarFallback>CC</AvatarFallback>
        </Avatar>
        <Avatar>
          <AvatarFallback>GE</AvatarFallback>
        </Avatar>
        <Avatar>
          <AvatarFallback>CX</AvatarFallback>
        </Avatar>
        <AvatarGroupCount>+4</AvatarGroupCount>
      </AvatarGroup>
      <AvatarGroup>
        <Avatar size="lg">
          <AvatarFallback>RE</AvatarFallback>
        </Avatar>
        <Avatar size="lg">
          <AvatarFallback>PL</AvatarFallback>
        </Avatar>
        <AvatarGroupCount>+9</AvatarGroupCount>
      </AvatarGroup>
    </div>
  )
}

export function NamedRows() {
  const members = [
    { initials: 'CC', name: 'Claude Code', role: 'Coding agent' },
    { initials: 'GE', name: 'Gemini', role: 'Research agent' },
    { initials: 'CX', name: 'Codex', role: 'Review agent' },
  ]
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minWidth: 220 }}>
      {members.map((m) => (
        <div key={m.initials} style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <Avatar>
            <AvatarFallback>{m.initials}</AvatarFallback>
            <AvatarBadge style={{ backgroundColor: '#00a240' }} />
          </Avatar>
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            <span style={{ fontSize: 13, fontWeight: 500, color: '#1c1c1a' }}>{m.name}</span>
            <span style={{ fontSize: 11, color: '#8a8a85' }}>{m.role}</span>
          </div>
        </div>
      ))}
    </div>
  )
}
