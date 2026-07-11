import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
  SelectGroup,
  SelectLabel,
  SelectSeparator,
} from 'garyx-desktop'

export function ProviderSelect() {
  return (
    <Select defaultValue="claude">
      <SelectTrigger style={{ width: 240 }}>
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="claude">Claude Code</SelectItem>
        <SelectItem value="codex">Codex</SelectItem>
        <SelectItem value="google">Google</SelectItem>
      </SelectContent>
    </Select>
  )
}

export function ModelSelectGrouped() {
  return (
    <Select defaultValue="opus-4-8">
      <SelectTrigger style={{ width: 260 }}>
        <SelectValue placeholder="Select a model" />
      </SelectTrigger>
      <SelectContent>
        <SelectGroup>
          <SelectLabel>Claude</SelectLabel>
          <SelectItem value="opus-4-8">Opus 4.8</SelectItem>
          <SelectItem value="sonnet-4-6">Sonnet 4.6</SelectItem>
          <SelectItem value="haiku-4-5">Haiku 4.5</SelectItem>
        </SelectGroup>
        <SelectSeparator />
        <SelectGroup>
          <SelectLabel>OpenAI</SelectLabel>
          <SelectItem value="gpt-5">GPT-5</SelectItem>
        </SelectGroup>
      </SelectContent>
    </Select>
  )
}

export function Placeholder() {
  return (
    <Select>
      <SelectTrigger style={{ width: 240 }}>
        <SelectValue placeholder="Choose a workspace" />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="garyx">garyx</SelectItem>
        <SelectItem value="garyx-desktop">garyx-desktop</SelectItem>
        <SelectItem value="garyx-mobile">garyx-mobile</SelectItem>
      </SelectContent>
    </Select>
  )
}

export function SmallDisabled() {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 16 }}>
      <Select defaultValue="telegram">
        <SelectTrigger size="sm" style={{ width: 180 }}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="telegram">Telegram</SelectItem>
          <SelectItem value="discord">Discord</SelectItem>
          <SelectItem value="feishu">Feishu</SelectItem>
        </SelectContent>
      </Select>
      <Select defaultValue="agent" disabled>
        <SelectTrigger style={{ width: 180 }}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="agent">Agent</SelectItem>
          <SelectItem value="team">Agent Team</SelectItem>
        </SelectContent>
      </Select>
    </div>
  )
}
