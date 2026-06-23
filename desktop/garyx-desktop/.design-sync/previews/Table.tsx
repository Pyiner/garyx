import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from 'garyx-desktop'

export function ThreadsTable() {
  const rows = [
    { agent: 'Claude Code', status: 'Running', active: '2 min ago' },
    { agent: 'Codex', status: 'In review', active: '14 min ago' },
    { agent: 'Gemini', status: 'Idle', active: '1 hr ago' },
    { agent: 'Backend Team', status: 'Done', active: 'Yesterday' },
  ]
  return (
    <div style={{ width: 480 }}>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Agent</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Last active</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => (
            <TableRow key={r.agent}>
              <TableCell>{r.agent}</TableCell>
              <TableCell>{r.status}</TableCell>
              <TableCell>{r.active}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

export function UsageTable() {
  const rows = [
    { model: 'Opus 4.8', input: '128.4K', output: '42.1K', cost: '$3.42' },
    { model: 'Sonnet 4.6', input: '512.0K', output: '88.7K', cost: '$1.96' },
    { model: 'Haiku 4.5', input: '1.2M', output: '210.3K', cost: '$0.74' },
  ]
  return (
    <div style={{ width: 520 }}>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Model</TableHead>
            <TableHead style={{ textAlign: 'right' }}>Input</TableHead>
            <TableHead style={{ textAlign: 'right' }}>Output</TableHead>
            <TableHead style={{ textAlign: 'right' }}>Cost</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => (
            <TableRow key={r.model}>
              <TableCell>{r.model}</TableCell>
              <TableCell style={{ textAlign: 'right' }}>{r.input}</TableCell>
              <TableCell style={{ textAlign: 'right' }}>{r.output}</TableCell>
              <TableCell style={{ textAlign: 'right', fontWeight: 500 }}>{r.cost}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

export function BotsTable() {
  const rows = [
    { name: 'support-bot', channel: 'Telegram', workspace: 'garyx' },
    { name: 'reviewer', channel: 'Discord', workspace: 'garyx-desktop' },
    { name: 'standup', channel: 'Feishu', workspace: 'garyx-mobile' },
  ]
  return (
    <div style={{ width: 480 }}>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Bot</TableHead>
            <TableHead>Channel</TableHead>
            <TableHead>Workspace</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => (
            <TableRow key={r.name}>
              <TableCell>{r.name}</TableCell>
              <TableCell>{r.channel}</TableCell>
              <TableCell>{r.workspace}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}
