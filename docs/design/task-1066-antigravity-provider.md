# TASK-1066 Antigravity Provider Design

## Goal

Add a Garyx provider backed by the local Antigravity CLI (`agy`) so custom
agents can run through the user's local Antigravity OAuth login. The provider
defaults to `Claude Opus 4.6 (Thinking)`.

The implementation must use the `agy` CLI process and Antigravity transcript
files. It must not use the `google-antigravity` Python SDK because that SDK does
not reuse the local OAuth subscription path.

## CLI Contract

First run:

```text
agy -p "<prompt>" --model "Claude Opus 4.6 (Thinking)" --dangerously-skip-permissions
```

Continuation:

```text
agy -p "<prompt>" --model "Claude Opus 4.6 (Thinking)" --conversation "<id>" --dangerously-skip-permissions
```

`--continue` is not used because it resumes the most recent global
conversation, not the Garyx thread-bound conversation.

`ANTIGRAVITY_CONVERSATION_ID` was tested locally and ignored by the current
CLI. Garyx therefore cannot precompute the transcript path on first run. The
provider discovers the new conversation id after spawn, stores it as
`sdk_session_id`, and uses `--conversation` on later runs.

## Provider Surface

- Add `ProviderType::Antigravity`.
- Canonical slug: `antigravity`.
- Alias: `agy`.
- Add a built-in custom-agent profile with `agent_id: "antigravity"`.
- Add optional default provider registration in bridge lifecycle for config keys
  `antigravity` and `agy`.
- Add a provider-model catalog with the static `agy models` entries, defaulting
  to `Claude Opus 4.6 (Thinking)`. The Gemini entries can remain selectable for
  users who explicitly choose them, but the recommended/default model stays
  Claude because Gemini Antigravity calls can fail with location-gated Google
  AI Platform errors.
- Reuse `AgentProviderConfig` fields for `workspace_dir`, `default_model`,
  `model`, `timeout_seconds`, and `env`.
- Add provider-specific shared config fields: `antigravity_bin` and optional
  `antigravity_brain_root`. The default brain root is derived from
  `$HOME/.gemini/antigravity-cli/brain`, with an env override allowed for tests.
- Add `AntigravityCliConfig` in `garyx-models/src/provider.rs`.

Custom-agent model tri-state semantics stay unchanged: absent preserves on
update, empty string clears to provider default, and a non-empty value sets the
model.

## Run Flow

Implement `garyx-bridge/src/antigravity_provider.rs`.

For each run:

1. Resolve `run_id` from metadata using the same fallback order as the Gemini
   provider.
2. Resolve cwd from run options, provider config, or current directory.
3. Resolve binary from `antigravity_bin`, then `agy`.
4. Resolve model from metadata `model`, config `model`, config `default_model`,
   then built-in default `Claude Opus 4.6 (Thinking)`.
5. Build the prompt with existing Garyx prompt helpers. File/image payloads are
   converted to local attachment instructions using existing staging helpers.
6. Spawn `agy -p <prompt> --model <model> --dangerously-skip-permissions`.
   Add `--conversation <id>` when the thread already has a bound session id,
   add `--add-dir <workspace>` when a workspace is available, and add
   `--log-file <temp-run-log>` to make first-run diagnostics deterministic.
7. Pass `--print-timeout` from the provider timeout so `agy` and Garyx agree on
   the maximum run duration.
8. Merge config env, task CLI env, and provider-specific metadata env
   (`desktop_antigravity_env`) into the child environment.
9. Register the child process by run id for abort/shutdown.
10. Tail transcript rows while the child runs.
11. Emit `SessionBound` once the conversation id is known.
12. On process exit, drain remaining transcript rows, emit `Done`, unregister
    the child, and return `ProviderRunResult`.

## Conversation Discovery

Known session id:

```text
$HOME/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript.jsonl
```

Fresh first run:

1. Record run start time before spawning `agy`.
2. Hold a short provider-local fresh-session discovery lock. First runs without
   a known conversation id are the only ambiguous case, and serializing just
   this discovery path avoids cross-claiming two new Antigravity conversations.
3. Prefer parsing the conversation id from the per-run `--log-file` if the CLI
   writes it there. If not present, scan
   `$HOME/.gemini/antigravity-cli/conversations/*.db` for files created or
   modified after run start.
4. Prefer the newest candidate whose matching brain log path exists and whose
   turn-0 `USER_INPUT` matches the prompt envelope for this run.
5. Once found, emit `SessionBound { sdk_session_id: <id> }` and persist the id
   in the provider's thread session map.

If `--conversation <id>` fails with a session-not-found style error, clear the
thread's session id and retry once as a fresh conversation. Do not retry on
ordinary model/tool/runtime failures.

Forking is out of scope for the first implementation. If
`sdk_session_fork=true` is requested, return `SessionError` explaining that
Antigravity exposes resume-by-id but no safe local fork primitive.

## Transcript Reader

Read compact `transcript.jsonl` first. When the corresponding
`transcript_full.jsonl` exists, keep it available for row replacement. Local
evidence showed compact and full files can differ by small encoding/formatting
amounts without a stable `is_truncated` flag on every row, so the reader should
replace a compact row from full only when:

- compact row has `is_truncated: true`, or
- compact row is missing `content`, `thinking`, `tool_calls`, or `error` that
  exists on the full row with the same `step_index`.

Rows are processed once by `step_index`; before a continuation run starts, record
the current max `step_index` and emit only rows with `step_index` greater than
that baseline. If a row is rewritten while being tailed, the later complete
parse wins before emission.

## Event Mapping

| Antigravity row | Garyx behavior |
| --- | --- |
| `USER_INPUT` | Skip. Garyx already persists the user row. |
| `CONVERSATION_HISTORY` | Skip. |
| `SYSTEM_MESSAGE` | Skip. |
| `CHECKPOINT` | Skip. Checkpoints contain internal summaries and may include local transcript paths. |
| `PLANNER_RESPONSE.thinking` | Store in assistant message metadata as provider reasoning, but do not stream as visible text. |
| `PLANNER_RESPONSE.content` | Emit `StreamEvent::Delta { text }` and append/merge an assistant `ProviderMessage` with source `antigravity`. |
| `PLANNER_RESPONSE.tool_calls[]` | Emit one `ToolUse`. Tool call objects have `name` and `args`; `args` is a JSON string when parseable. |
| `RUN_COMMAND`, `LIST_DIRECTORY`, and other model-source tool result rows | Emit `ToolResult` with raw row content, tool name from row `type`, timestamp, status-derived `is_error`, and source metadata. |
| `ERROR_MESSAGE.error` | Emit a failed `ToolResult` when the run already has useful context; otherwise surface it as the run error. |
| Unknown user/system rows | Skip with debug logging. |
| Unknown model rows with content/error | Emit `ToolResult` to keep provider activity visible. |

The visible run response is only the concatenation of emitted
`PLANNER_RESPONSE.content` strings. The provider must not surface checkpoint,
history, or system rows as visible assistant text.

Tool result rows do not carry explicit call ids in the observed schema. The
provider should assign synthetic ids to `ToolUse` events and pair result rows
with the most recent unmatched tool call of the same normalized name, falling
back to FIFO order for unknown/parallel cases.

## Error And Timeout Handling

- `initialize` runs `agy models`; failure returns `ProviderNotReady`.
- Spawn failure returns an internal spawn error.
- Transcript discovery timeout returns `RunFailed`.
- Invalid partial JSONL reads are retried until the row becomes parseable or
  the child exits.
- Non-zero child exit returns a failed `ProviderRunResult` when transcript
  messages were captured; otherwise it returns `RunFailed` with stderr/stdout
  context.
- Obvious overload/rate-limit strings can map to `BridgeError::Overloaded` when
  no useful partial transcript exists.
- Garyx timeout kills the child and returns `BridgeError::Timeout`.
- `abort(run_id)` kills the child and returns `true` when a child existed.

## Tests

Unit tests:

- `ProviderType` slug/alias round trips for `antigravity` and `agy`.
- Config defaults and builder precedence for model, binary path, workspace,
  env, timeout, and brain-root override.
- Command construction for first run, continuation, `--add-dir`, `--log-file`,
  and print timeout.
- Transcript parser:
  - skips user/history/system/checkpoint rows
  - maps planner content to delta and final response
  - maps planner `tool_calls` to `ToolUse`
  - parses JSON-string tool args when possible
  - maps `RUN_COMMAND` and `LIST_DIRECTORY` to `ToolResult`
  - maps `ERROR_MESSAGE.error`
  - replaces compact row data from full transcript when needed
- Conversation discovery using temp Antigravity home directories.
- Baseline `step_index` filtering for continuation runs.
- Abort removes and kills the registered child.

Focused validation:

```text
cargo test -p garyx-models provider
cargo test -p garyx-bridge antigravity_provider
cargo test -p garyx-gateway provider_models
cargo test -p garyx agent_upsert
cargo check --workspace
```

End-to-end validation:

1. Build/install the patched local CLI when validating through the managed
   gateway.
2. Restart the managed gateway only for installed-binary validation.
3. Create a synthetic custom agent bound to provider `antigravity`.
4. Run `garyx thread create --agent-id <agent>` and `garyx thread send` with a
   deterministic prompt.
5. Confirm the Garyx transcript streams the Claude reply.
6. Confirm the thread stores an Antigravity conversation id as `sdk_session_id`.
7. Send a second message on the same thread and confirm the same Antigravity
   transcript receives the new rows.
8. Run a harmless temporary-directory prompt that triggers listing/command
   activity and confirm Garyx shows tool use and result rows.
