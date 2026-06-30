# Claude Code SDK Notes

- In bidirectional Claude Code SDK sessions, every CLI `control_request` must
  receive a `control_response`, even when the subtype is unsupported or newer
  than this SDK. Dropping one can leave the CLI waiting forever.
- Normal streaming completion should close stdin and wait for the Claude CLI
  process to exit; force-closing the transport can race Claude's local
  transcript flush and break later `--resume` behavior.
- Claude SDK approval tests need a `can_use_tool` callback, which adds
  `--permission-prompt-tool stdio`; changing only `permission_mode` may not
  make Claude send `can_use_tool` requests to the SDK.
- The standalone Claude SDK should preserve Claude Code's built-in system prompt
  by default.
- Pass `--system-prompt` only when Garyx intentionally replaces Claude Code's
  default prompt.
- Use `--append-system-prompt` to keep default tool behavior.

## Workflow SDK

- Garyx workflows are SDK-first. User TypeScript owns loops, branching,
  ordering, deduplication, budgets, and failure handling.
- Workflow task creation in product UI is text-first: it should send one
  plain-text input string. Do not build product forms directly from workflow
  definition metadata; a workflow that needs structured input should structure the
  text in its own first step.
- CLI workflow task creation is text-first through a single `--input <text>`
  flag; a workflow that needs structured data parses that text in its first
  step. There are no `--input-file` / `--input-json` variants.
- Do not add a Garyx interpreter for Claude Code workflow scripts, and do not
  try to execute Claude Code's generated workflow files directly.
- The gateway side of workflows should provide observability, hidden child
  threads, provider execution, structured `submit_result`, and persisted run
  records.
- Treat structured results as a thread-run capability, not a workflow-only
  primitive. The child thread metadata carries the required result schema, and
  the MCP server exposes `submit_result` dynamically for the current thread with
  the schema fields as direct tool arguments. Do not use workflow-specific result
  tokens or a generic `{ payload: ... }` wrapper.
