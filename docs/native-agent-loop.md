# Garyx Native Agent Loop

Status: implemented for the `gpt` model provider.

## Goals

Garyx needs a first-party agent loop that runs inside the bridge instead of
delegating loop control to an external CLI provider. The
first model backend on this loop is the GPT provider. It should:

- use Garyx's existing streaming, transcript, interruption, task, and channel
  delivery pipeline;
- use Codex-compatible OpenAI authentication so users who already logged in to
  Codex do not need to duplicate API keys in Garyx;
- expose enough local tools for a model to inspect and modify the workspace;
- persist provider messages so a thread can continue with useful context.

Non-goals for the first version:

- replacing existing providers;
- adding a new secret store;
- implementing a separate desktop UI before the command/channel path works.

## Provider Boundary

The user-facing provider slug is `gpt`. GPT is selected by creating a custom
agent with `provider_type: "gpt"`; it is not a built-in agent id. Legacy
provider slugs `garyx_native`, `garyx`, and `native` are accepted as aliases,
but the native loop is an internal execution engine rather than a provider users
select directly.

The implementation lives in `garyx-bridge` and implements
`AgentLoopProvider`. It consumes the existing `ProviderRunOptions` and emits the
existing `StreamEvent` variants:

- `SessionBound` when a native GPT session id is created or restored;
- `Delta` for assistant text;
- `ToolUse` and `ToolResult` for normalized transcript persistence;
- `Boundary::UserAck` when queued streaming input is accepted;
- `Done` when the run is complete.

The bridge persists these messages through the existing persistence worker. To
support restart/resume quality without changing the public provider request
shape, the bridge places normalized recent session messages in
`ProviderRunOptions.metadata.garyx_session_messages`. Other providers ignore
the metadata key; the GPT backend on the native loop uses it to rebuild the next
model request.

## Authentication

The provider uses a `codex` auth source by default. Resolution order follows the
practical Codex path:

1. `CODEX_API_KEY` from the provider env or process env;
2. `OPENAI_API_KEY` from the provider env or process env;
3. `$CODEX_HOME/auth.json`, or `~/.codex/auth.json` when `CODEX_HOME` is unset.

Supported credential forms:

- `OPENAI_API_KEY` in `auth.json`: call the OpenAI Responses API at
  `https://api.openai.com/v1/responses`;
- `tokens.access_token` in `auth.json`: call the ChatGPT Codex backend at
  `https://chatgpt.com/backend-api/codex/responses` and forward
  `ChatGPT-Account-ID` when present.

Codex agent identity signing is intentionally not duplicated in this pass. If
Codex only has `agent_identity`, Garyx reports a clear configuration error.

## Model Catalog

The GPT backend follows Codex's model catalog rules instead of maintaining a
separate hand-written model list. The gateway calls the Codex `/models`
endpoint with the local Codex CLI version, sorts models by Codex `priority`,
marks the first picker-visible model as the default, and exposes each model's
own supported reasoning efforts and service tiers. The `service_tiers` values
are forwarded to the Responses API as `service_tier`; for the current Codex
catalog, Fast mode is exposed as `priority`. When the live catalog cannot be
read, Garyx uses a minimal built-in fallback so the default, reasoning controls,
and known Fast service tiers remain available offline.

## Loop

Each run performs the same basic cycle used by Codex and Claude Code:

1. Add the user input to the session transcript.
2. Send transcript, system instructions, and tool schemas to the model.
3. Stream assistant text to Garyx as it arrives.
4. If the model asks for tool calls, execute them, append tool results, and
   sample again.
5. Stop when a model turn returns assistant text without tool calls, when the
   run is interrupted, or when the configured tool-iteration budget is reached.

The provider has an in-memory session map keyed by Garyx `thread_id`. On each
run it seeds that map from the restored `sdk_session_id` and persisted
`session_messages` if needed.

## Tools

The first native tool set is deliberately small but complete enough for coding
work:

- `exec_command`: run a shell command in the active workspace with optional
  timeout;
- `read_file`: read a UTF-8 text file;
- `write_file`: write a UTF-8 text file;
- `list_dir`: list directory entries.

Tool outputs are truncated before sending them back to the model and before
persisting to the transcript.

## No Persistent Goal Mode

Garyx no longer exposes a router-native `/goal` command or thread-level
auto-continuation loop mode. Long-running work should be represented by normal
thread turns, tasks, or automations instead of hidden thread state that causes
the bridge to re-enter a run automatically.

## Tests

The implementation is test-driven in vertical slices:

1. provider/model config round-trips and built-in `garyx` agent profile;
2. bridge option hydration with persisted `session_messages`;
3. native loop with fake model: assistant-only turn;
4. native loop with fake model: tool call followed by second model request;
5. native loop with queued streaming input and interrupt;
6. auth resolution from env and Codex auth file;
7. focused package tests for touched crates.
