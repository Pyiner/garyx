import { Textarea, Label } from 'garyx-desktop'

export function TextareaWithValue() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, maxWidth: 420 }}>
      <Label>Agent system prompt</Label>
      <Textarea defaultValue={'You are the Release Manager agent for Garyx.\nCoordinate the gateway build, run the validation suite, and post status back to the task thread.'} />
    </div>
  )
}

export function TextareaPlaceholder() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, maxWidth: 420 }}>
      <Label>Task instructions</Label>
      <Textarea placeholder="Describe what the assigned agent should do." />
    </div>
  )
}

export function TextareaDisabled() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, maxWidth: 420 }}>
      <Label>Automation trigger (read-only)</Label>
      <Textarea disabled defaultValue={'Runs every weekday at 09:00.\nThread workspace: /Users/test/repos/garyx'} />
    </div>
  )
}
