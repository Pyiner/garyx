import {
  Field,
  FieldLabel,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLegend,
  FieldSeparator,
  FieldSet,
  FieldContent,
  FieldTitle,
  Input,
  Textarea,
  Checkbox,
} from 'garyx-desktop'

export function FieldVertical() {
  return (
    <div style={{ maxWidth: 420 }}>
      <Field>
        <FieldLabel htmlFor="agent-name">Agent name</FieldLabel>
        <Input id="agent-name" defaultValue="Release Manager" />
        <FieldDescription>
          Shown in the task forest and channel identity rows.
        </FieldDescription>
      </Field>
    </div>
  )
}

export function FieldWithError() {
  return (
    <div style={{ maxWidth: 420 }}>
      <Field data-invalid="true">
        <FieldLabel htmlFor="gateway-url">Gateway URL</FieldLabel>
        <Input id="gateway-url" aria-invalid defaultValue="gateway.local" />
        <FieldError errors={[{ message: 'Must be an absolute https:// URL.' }]} />
      </Field>
    </div>
  )
}

export function FieldHorizontalCheckbox() {
  return (
    <div style={{ maxWidth: 460 }}>
      <FieldGroup>
        <Field orientation="horizontal">
          <Checkbox id="notify-thread" defaultChecked />
          <FieldContent>
            <FieldLabel htmlFor="notify-thread">Notify current thread</FieldLabel>
            <FieldDescription>
              Return review results to the task thread instead of a bot channel.
            </FieldDescription>
          </FieldContent>
        </Field>
        <Field orientation="horizontal">
          <Checkbox id="auto-restart" />
          <FieldContent>
            <FieldLabel htmlFor="auto-restart">Restart gateway after build</FieldLabel>
            <FieldDescription>
              Reinstall the signed binary, then restart the managed gateway.
            </FieldDescription>
          </FieldContent>
        </Field>
      </FieldGroup>
    </div>
  )
}

export function FieldSetGroup() {
  return (
    <div style={{ maxWidth: 460 }}>
      <FieldSet>
        <FieldLegend>Workflow task</FieldLegend>
        <FieldDescription>
          Configure how this development-loop run executes.
        </FieldDescription>
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="wf-input">Input</FieldLabel>
            <Textarea id="wf-input" placeholder="Plain-text task description" />
          </Field>
          <FieldSeparator>Advanced</FieldSeparator>
          <Field>
            <FieldLabel htmlFor="wf-bun">Bun binary</FieldLabel>
            <Input id="wf-bun" placeholder="$GARYX_WORKFLOW_BUN_BIN" />
            <FieldDescription>
              Resolved from env, a bundled sibling, then PATH.
            </FieldDescription>
          </Field>
        </FieldGroup>
      </FieldSet>
    </div>
  )
}

export function FieldTitleCard() {
  return (
    <div style={{ maxWidth: 420 }}>
      <Field>
        <FieldTitle>Structured results</FieldTitle>
        <FieldDescription>
          The child thread carries the result schema; submit_result is exposed
          dynamically from the current MCP thread context.
        </FieldDescription>
      </Field>
    </div>
  )
}
