# Claude Code SDK Notes

- In bidirectional Claude Code SDK sessions, every CLI `control_request` must
  receive a `control_response`, even when the subtype is unsupported or newer
  than this SDK. Dropping one can leave the CLI waiting forever.
- Normal streaming completion should close stdin and wait for the Claude CLI
  process to exit; force-closing the transport can race Claude's local
  transcript flush and break later `--resume` behavior.
- A Claude `result` ends one turn, not necessarily the process. After stdin is
  closed, keep consuming stdout until EOF because completed background tasks
  can inject later `task-notification` turns.
- `background_tasks_changed` is a replace-style level signal whose ordering
  relative to task edge messages is unspecified. It may control input
  availability, but must never authorize output teardown.
- The primary "may stdin close after a result" signal is the SDK-level `Stop`
  hook observation (`stop_hook_observer`): the CLI reports its authoritative
  in-flight `background_tasks` at every turn stop, delivered in-band right
  before that turn's result. The stream-event task set stays as a secondary
  hold; hook absence (older CLI) falls back to stream events only. The hook
  answer is always an immediate empty output — never block or delay a stop.
- Claude SDK approval tests need a `can_use_tool` callback, which adds
  `--permission-prompt-tool stdio`; changing only `permission_mode` may not
  make Claude send `can_use_tool` requests to the SDK.
- The standalone Claude SDK should preserve Claude Code's built-in system prompt
  by default.
- Pass `--system-prompt` only when Garyx intentionally replaces Claude Code's
  default prompt.
- Use `--append-system-prompt` to keep default tool behavior.
