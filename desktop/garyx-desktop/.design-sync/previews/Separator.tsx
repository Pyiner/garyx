import { Separator } from 'garyx-desktop'

export function HorizontalBetweenMenuItems() {
  const items = ['Threads', 'Workspaces', 'Automations', 'Agents']
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        minWidth: 220,
        border: '1px solid #eee',
        borderRadius: 8,
        overflow: 'hidden',
      }}
    >
      {items.map((label, i) => (
        <div key={label}>
          <div style={{ padding: '10px 14px', fontSize: 13, color: '#1c1c1a' }}>{label}</div>
          {i < items.length - 1 && <Separator />}
        </div>
      ))}
    </div>
  )
}

export function VerticalBetweenStats() {
  const stats = [
    { value: '24', label: 'Threads' },
    { value: '6', label: 'Agents' },
    { value: '3', label: 'Teams' },
  ]
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 16,
        border: '1px solid #eee',
        borderRadius: 8,
        padding: '12px 18px',
      }}
    >
      {stats.map((s, i) => (
        <div key={s.label} style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
            <span style={{ fontSize: 18, fontWeight: 600, color: '#1c1c1a' }}>{s.value}</span>
            <span style={{ fontSize: 11, color: '#8a8a85' }}>{s.label}</span>
          </div>
          {i < stats.length - 1 && (
            <Separator orientation="vertical" style={{ height: 32 }} />
          )}
        </div>
      ))}
    </div>
  )
}

export function InlineToolbar() {
  const itemStyle = { fontSize: 13, color: '#1c1c1a' } as const
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        border: '1px solid #eee',
        borderRadius: 8,
        padding: '8px 14px',
      }}
    >
      <span style={itemStyle}>Run</span>
      <Separator orientation="vertical" style={{ height: 16 }} />
      <span style={itemStyle}>Pause</span>
      <Separator orientation="vertical" style={{ height: 16 }} />
      <span style={itemStyle}>Restart</span>
    </div>
  )
}

export function SectionWithLabeledRule() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, minWidth: 240 }}>
      <span style={{ fontSize: 13, color: '#1c1c1a' }}>Claude Code finished its run.</span>
      <Separator />
      <span style={{ fontSize: 11, fontWeight: 500, color: '#8a8a85', letterSpacing: 0.4 }}>
        TODAY
      </span>
      <span style={{ fontSize: 13, color: '#1c1c1a' }}>Codex started a review task.</span>
    </div>
  )
}
