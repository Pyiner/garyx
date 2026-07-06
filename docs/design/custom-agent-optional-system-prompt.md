# Custom Agent Optional System Prompt

## Problem

Custom-agent creation and editing currently require `system_prompt` even though
the provider path can run without a custom prompt. This creates two bad states:

- users must invent a prompt when they only want a named provider profile
- blank prompt values can flow far enough to override provider defaults with an
  empty string

The desired behavior is: no prompt means use the provider's own default behavior.
An empty or whitespace-only prompt is the same as no prompt and must not be
forwarded to the provider as `Some("")`.

## Contract

For custom-agent upsert requests:

| Request state | Existing agent update | New agent create |
| --- | --- | --- |
| `system_prompt` absent | keep stored prompt | store unset prompt |
| `system_prompt: "x"` | store `"x"` after trimming | store `"x"` after trimming |
| `system_prompt: ""` or whitespace | clear stored prompt | store unset prompt |

`garyx agent update` keeps merge semantics: omitting `--system-prompt` preserves
the stored prompt, while `--system-prompt ""` explicitly clears it. `garyx agent
create` can omit `--system-prompt`; omitted and empty both create an agent that
uses the provider default prompt.

## Storage

Keep `CustomAgentProfile.system_prompt` as `String` for compatibility with
existing clients, fixtures, and persisted JSON. The storage representation of
"unset" remains `""`.

The normalization happens on writes:

- `Some(nonblank)` trims and stores the trimmed value.
- `Some(blank)` stores `""`.
- `None` on create stores `""`.
- `None` on update preserves the existing value; blank stored values are treated
  as unset by read/provider paths.

This avoids a broad `Option<String>` model migration while still making the
semantic boundary explicit. Persisted agents that already contain
`"system_prompt": ""` or whitespace-only values are treated as unset by the
metadata and provider paths. No data migration is required.

## Gateway And MCP API

- Change `CustomAgentUpsertPayload.system_prompt` and
  `UpsertCustomAgentRequest.system_prompt` to `Option<String>`.
- Keep `system_prompt` accepted as snake_case and allow the existing JSON field
  to be absent.
- Remove the gateway validation error `system_prompt is required`.
- Add store tests for create without prompt, create with blank prompt, update
  without prompt preserving, update with blank prompt clearing, and update with
  nonblank prompt replacing.
- Normalization converges in the store, so existing and future gateway entry
  points share the same contract.

## CLI

- Change `garyx agent create`, `update`, and `upsert` command fields from
  required `String` to `Option<String>`.
- The request builder includes `system_prompt` only when the flag was supplied.
- If supplied, the value is trimmed and sent even when it becomes `""`; that is
  the explicit clear operation on update.
- Help text should state:
  - create/upsert create path: omit to use provider default
  - update/upsert existing path: omit to preserve; pass empty string to clear

No extra `--clear-system-prompt` flag is needed because the command already has
a natural explicit-empty representation and the model-field precedent is
absent-preserve, empty-clear.

## Bridge Behavior

### Claude Code

Builtin provider agents are not custom agents. If a run carries a builtin
provider `agent_id` such as `claude`, it must use the normal Garyx composed
instruction path instead of creating a Claude session-agent definition with an
empty prompt. This keeps the builtin Claude, task, desktop, and mobile default
paths aligned and prevents an empty prompt from overriding provider behavior.

For a custom agent with a nonblank stored prompt:

- SDK `agent`: `Some(agent_id)`
- SDK `agents`: one definition whose prompt is the trimmed custom prompt
- SDK `system_prompt`: `None`
- SDK `append_system_prompt`: `None`
- Garyx branded base instructions are not prepended to the custom prompt

For a custom agent with no prompt:

- SDK `agent`: `None`
- SDK `agents`: empty
- SDK `system_prompt`: `None`
- SDK `append_system_prompt`: `None`
- provider config `system_prompt` is not used as a fallback for this custom
  agent path

That lets Claude Code use its own default prompt. Runtime context, task
metadata, and custom-agent memory are still prepended to the first user message
through `prepend_initial_context_to_user_message`; they are not injected through
`system_prompt`.

### Codex

For a custom agent with a nonblank stored prompt, keep the current behavior:
compose Garyx developer instructions with the custom prompt and pass them in the
thread config as `developer_instructions`.

For a custom agent with no prompt:

- do not insert `developer_instructions` only to carry an empty custom prompt or
  Garyx base instructions
- still include the Garyx MCP server config when present
- runtime context, task metadata, and custom-agent memory stay in the first user
  message via `prepend_initial_context_to_user_message`

This is the Codex equivalent of not passing a custom system prompt and lets the
provider use its default instructions.

Other native provider paths already receive custom-agent prompts through
metadata that omits blank `system_prompt` values, so their behavior is unchanged
by this design.

## Desktop

The Mac app is the IA and copy source of truth:

- remove create/edit validation that requires System Prompt
- keep sending the trimmed prompt string on save; empty means clear/use provider
  default
- update empty display text from `(empty)` to provider-default wording where the
  agent detail view previews the prompt

## Mobile

Mobile follows the Mac contract:

- the SwiftUI form already allows editing an empty text area; keep it optional
  in copy and read-only display
- create sends the trimmed prompt, including `""`
- update sends the user's trimmed prompt; an explicit blank edit clears instead
  of preserving a fetched non-empty base-agent value
- put request-shaping behavior in the existing app model path and cover request
  encoding/clear behavior in SwiftPM tests

## Validation

- `cargo test -p garyx-gateway custom_agents`
- `cargo test -p garyx-gateway test_create_custom_agent_allows_omitted_system_prompt`
- `cargo test -p garyx agent_create`
- `cargo test -p garyx agent_update`
- `cargo test -p garyx agent_upsert`
- `cargo test -p garyx-bridge custom_agent`
- `cargo test -p garyx-bridge codex`
- desktop focused type/build check for the touched renderer/main files
- `swift test` in `mobile/garyx-mobile`
- app-target `xcodebuild` if mobile app wiring changed
- CLI/gateway smoke: `garyx agent create` without `--system-prompt` succeeds,
  and `garyx agent update --system-prompt ""` clears an existing prompt
