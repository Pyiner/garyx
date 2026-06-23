import { Badge } from 'garyx-desktop'
import { Check, CircleDot, Zap, GitBranch } from 'lucide-react'

export function Variants() {
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'center' }}>
      <Badge variant="default">Active</Badge>
      <Badge variant="secondary">Draft</Badge>
      <Badge variant="destructive">Failed</Badge>
      <Badge variant="outline">Archived</Badge>
      <Badge variant="ghost">Idle</Badge>
      <Badge variant="link">View run</Badge>
    </div>
  )
}

export function WithIcons() {
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'center' }}>
      <Badge variant="default">
        <Check />
        Completed
      </Badge>
      <Badge variant="secondary">
        <CircleDot />
        Running
      </Badge>
      <Badge variant="destructive">
        <Zap />
        Error
      </Badge>
      <Badge variant="outline">
        <GitBranch />
        main
      </Badge>
    </div>
  )
}

export function StatusInRows() {
  const tasks = [
    { name: 'Build local CLI', variant: 'default' as const, label: 'Done' },
    { name: 'Restart gateway', variant: 'secondary' as const, label: 'Queued' },
    { name: 'Sign macOS bundle', variant: 'destructive' as const, label: 'Failed' },
  ]
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10, minWidth: 240 }}>
      {tasks.map((t) => (
        <div
          key={t.name}
          style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}
        >
          <span style={{ fontSize: 13, color: '#1c1c1a' }}>{t.name}</span>
          <Badge variant={t.variant}>{t.label}</Badge>
        </div>
      ))}
    </div>
  )
}

export function CountBadges() {
  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'center' }}>
      <Badge variant="default">3</Badge>
      <Badge variant="secondary">12</Badge>
      <Badge variant="outline">128</Badge>
      <Badge variant="destructive">99+</Badge>
    </div>
  )
}
