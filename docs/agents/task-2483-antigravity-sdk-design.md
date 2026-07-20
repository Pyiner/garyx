# TASK-2483 Antigravity SDK Extraction Design

## Goal

Extract Antigravity CLI process and private-disk protocol handling from
`garyx-bridge` into a standalone workspace crate, `antigravity-sdk`.

The new crate owns process lifecycle and the existing
`transcript.jsonl`/`.db` transport. It has no dependency on Garyx crates and no
knowledge of Garyx config, threads, persistence, prompts, or stream types.
`garyx-bridge` remains the product adapter from the SDK's structured events to
Garyx semantics.

This is an ownership refactor. It deliberately keeps the current tailing and
conversation-discovery mechanism.

## Current Boundary Inventory

Move to `antigravity-sdk`:

- CLI command construction, spawn, stdout/stderr capture, child registration,
  normal wait, abort, and shutdown.
- The `agy models` readiness probe.
- Antigravity path rules below a caller-provided brain root, including compact
  and full transcript locations and the conversations directory.
- Compact `transcript.jsonl` parsing and the richer-row overlay from
  `transcript_full.jsonl`.
- Baseline step detection, step deduplication, post-exit drain polls, and the
  existing poll timing.
- Fresh-conversation serialization, run-log UUID discovery, `.db` filename and
  mtime discovery, and prompt matching.
- Private row classification, tool-call parsing, normalized tool/result
  pairing, pending reasoning association, and invalid-conversation
  classification.
- Encoding a caller's explicit permission decision into Antigravity CLI flags.

Keep in `garyx-bridge`:

- `AntigravityCliConfig`, model default hot reload, metadata overrides, Garyx
  runtime environment injection, and workspace/brain-root default resolution.
- Attachment staging, native skill expansion, Gary instructions, memory
  context, and initial-context prompt composition.
- Garyx thread-to-provider-session state, unsupported fork semantics, stale
  session eviction, and the single fresh-session retry.
- The explicit Garyx approval policy currently equivalent to
  `--dangerously-skip-permissions`.
- `AntigravityEvent` to `StreamEvent`/`ProviderMessage` mapping, assistant
  response aggregation, provider-reasoning metadata, `ProviderRunResult`, and
  `BridgeError` mapping.
- `ProviderRuntime` integration, run-to-thread ownership used by
  `clear_session`, graph-engine callbacks, and persistence-worker behavior.

The bridge will not parse transcript rows, scan Antigravity files, construct
Antigravity CLI protocol arguments, or retain `tokio::process::Child` values.

## Crate Shape

```text
antigravity-sdk/
  Cargo.toml
  src/
    lib.rs          public exports
    client.rs       probe, run registry, lifecycle, timeout, abort/shutdown
    transcript.rs   private file schema, full overlay, decoder, tail state
    discovery.rs    run-log and conversations `.db` discovery
    types.rs        request/result/event/callback types
    error.rs        typed transport/protocol failures
```

Dependencies are limited to protocol/runtime utilities such as `tokio`,
`serde`, `serde_json`, `thiserror`, and `uuid`. In particular, the crate has no
`garyx-*` dependency and contains no Garyx thread, config, prompt, or provider
types.

The workspace root adds `antigravity-sdk` as a member and workspace dependency;
`garyx-bridge` depends on it.

## Public Protocol Surface

The exact field spelling may be adjusted during implementation, but the
ownership and decision flow are fixed by this design.

```rust
pub struct AntigravityClientConfig {
    pub cli_bin: String,
    pub brain_root: PathBuf,
    pub transcript_poll_interval: Duration, // current default: 250 ms
    pub discovery_timeout: Duration,        // current default: 30 s
    pub shutdown_grace: Duration,           // current default: 2 s
    pub run_timeout_grace: Duration,        // current default: 10 s
}

pub struct AntigravityRunRequest {
    pub run_id: String,
    pub prompt: String,
    pub discovery_text: String,
    pub model: String,
    pub conversation_id: Option<String>,
    pub workspace_dir: PathBuf,
    pub log_path: PathBuf,
    pub env: HashMap<String, String>,
    pub print_timeout: Duration,
    pub approval_callback: ApprovalCallback,
}

pub struct ApprovalRequest {
    pub model: String,
    pub conversation_id: Option<String>,
    pub workspace_dir: PathBuf,
}

pub enum ApprovalDecision {
    UseCliDefault,
    AcceptEdits,
    Plan,
    BypassPermissions,
    Deny { reason: String },
}

pub type ApprovalFuture = Pin<Box<
    dyn Future<Output = Result<ApprovalDecision, AntigravityError>>
        + Send
        + 'static,
>>;
pub type ApprovalCallback = Arc<
    dyn Fn(ApprovalRequest) -> ApprovalFuture + Send + Sync + 'static,
>;

pub enum AntigravityEvent {
    SessionBound {
        conversation_id: String,
    },
    AssistantDelta {
        step_index: i64,
        text: String,
        reasoning: Option<String>,
        created_at: Option<String>,
    },
    ToolUse {
        step_index: i64,
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
        created_at: Option<String>,
    },
    ToolResult {
        step_index: i64,
        tool_use_id: Option<String>,
        name: String,
        content: serde_json::Value,
        is_error: bool,
        created_at: Option<String>,
    },
    Error {
        step_index: i64,
        message: String,
        created_at: Option<String>,
    },
}

pub struct AntigravityRunOutcome {
    pub conversation_id: String,
    pub success: bool,
    pub failure: Option<AntigravityRunFailure>,
    pub duration: Duration,
}

impl AntigravityClient {
    pub async fn probe(&self) -> Result<(), AntigravityError>;
    pub async fn execute(
        &self,
        request: AntigravityRunRequest,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) -> Result<AntigravityRunOutcome, AntigravityError>;
    pub async fn abort(&self, run_id: &str) -> bool;
    pub async fn shutdown(&self);
}
```

`AntigravityRunFailure` and hard SDK errors carry a typed
`InvalidConversation` classification, so the bridge does not need to inspect
private CLI error strings when deciding whether to evict and retry a session.
Raw process output remains diagnostic context in SDK error messages, matching
today's behavior.

`env` is intentionally request-scoped. Before every run, the bridge computes
the complete environment overlay from static provider config, that run's
runtime-context identity, and `desktop_antigravity_env`; the SDK applies the
map verbatim with `Command::envs`. The client config has no static environment
map and therefore cannot accidentally retain one thread's `GARYX_*` identity
for another run. The map is opaque protocol input to the SDK: none of its keys
have SDK-specific meaning.

## Approval Boundary

Antigravity CLI 1.1.4 exposes launch modes (`--mode accept-edits`, `--mode
plan`) and `--dangerously-skip-permissions`, but its non-interactive `--print`
surface does not expose a machine-readable request/response channel for each
tool approval. Therefore the callback is a per-run launch-policy hook, not a
fictional per-tool approval protocol.

The callback is required for every run. The SDK has no fallback approval
decision. It only translates the returned value:

- `UseCliDefault`: no permission flag.
- `AcceptEdits`: `--mode accept-edits`.
- `Plan`: `--mode plan`.
- `BypassPermissions`: `--dangerously-skip-permissions`.
- `Deny`: return before spawning.

To preserve current product behavior, `garyx-bridge` supplies a callback that
returns `BypassPermissions`. That choice is visibly located in the product
adapter and can later be replaced by a real Garyx policy callback without an
SDK change. This avoids the Codex SDK's unconditional transport-level
auto-approval mistake.

## Run Flow And Behavioral Equivalence

1. Bridge resolves Garyx config, model, paths, prompt, timeout, session
   candidate, and approval callback. It also computes the complete per-run
   environment overlay, including runtime-context identity, then creates a pure
   SDK request.
2. SDK asks the approval callback before spawn, encodes only that decision, and
   applies the request's environment map verbatim to the child command.
3. SDK records the existing transcript baseline for a resumed conversation.
   Fresh runs retain the current global fresh-discovery serialization.
4. SDK spawns `agy -p`, discovers a fresh conversation from the run log first
   and recent `.db` candidates second, and emits `SessionBound`.
5. SDK repeatedly reads the compact transcript plus the current richer full
   overlay, deduplicates by `step_index`, reconstructs typed events, and invokes
   the event callback synchronously in transcript order.
6. Normal completion waits for the child, performs the existing three final
   transcript reads separated by 50 ms, then drains stdout/stderr. A request
   deadline remains `print_timeout + 10 s`; timeout aborts and reaps the child.
7. Bridge maps events and builds the Garyx run result. It retries once without
   a session only when the SDK reports `InvalidConversation`, exactly as now.
8. Bridge emits Garyx `Done` only on the same result-returning path as today.

Observable protocol details remain unchanged: tool IDs retain
`antigravity-tool-<step>-<index>`, `list_dir` still pairs with
`LIST_DIRECTORY`, tool results retain the original row JSON, timestamps retain
the row value with the bridge's current fallback, and truncated compact fields
still prefer richer full rows.

The existing Garyx-specific temporary log path is provided as `log_path` by the
bridge. This avoids embedding a Garyx name or filesystem convention in the SDK
while preserving the current disk location.

## Error And Lifecycle Ownership

- Spawn, discovery, transcript, wait, timeout, callback rejection/failure, and
  process-exit errors are SDK errors or typed run failures.
- For exit classification, **visible output** means at least one emitted
  `AssistantDelta`, `ToolUse`, or `ToolResult`. `SessionBound`, a reasoning-only
  planner row, and a bare `Error` event do not count. This is exactly the
  current `!session_messages.is_empty() || !response.trim().is_empty()` test.
- A non-zero exit with no visible output remains a hard SDK process-exit error
  containing the exit status and captured stdout/stderr. This remains true for
  a run that emitted only a bare `Error` event. A non-zero exit after visible
  output returns an unsuccessful outcome so the bridge can preserve partial
  messages.
- Transcript `ERROR_MESSAGE` rows produce a typed error event/failure and keep
  the current pending-tool error-result pairing. The last transcript error
  wins among multiple `ERROR_MESSAGE` rows, matching the current mapper.
- `success` is true only when the process exits successfully and no transcript
  error was observed. On a successful process exit plus a transcript error,
  the SDK returns an unsuccessful outcome with that transcript error. On a
  non-zero exit after visible output, the last transcript error wins when one
  exists; otherwise the process-exit diagnostic is the failure. On a non-zero
  exit without visible output, the hard process-exit error wins even when a
  bare transcript error was observed.
- The SDK owns every child handle. `abort(run_id)` kills and waits; `shutdown()`
  aborts all registered runs. Normal process exit is waited naturally.
- Bridge removes its run/thread bookkeeping in a finally-style guard around
  each SDK call, including timeout, error, and cancellation paths.

## Validation

SDK tests move or strengthen the current protocol guards:

- command arguments for every approval decision, including proof that bypass
  appears only when the callback returns it;
- denial/callback failure prevents process spawn;
- a fake child observes the request-scoped environment, while two sequential
  requests with different synthetic identity values prove there is no
  client-level environment retention;
- compact/full transcript overlay and malformed-line tolerance;
- event order, step deduplication, reasoning association, tool pairing, and
  invalid-conversation classification;
- run-log discovery, multiple `.db` candidates with prompt matching, and
  resumed baseline behavior;
- fake-CLI tailing end to end, normal reaping, timeout/abort, and shutdown;
- the complete exit/error matrix: clean exit without transcript error succeeds;
  clean exit with `ERROR_MESSAGE` returns an unsuccessful outcome; non-zero
  exit after assistant/tool output returns a partial unsuccessful outcome
  (with transcript error winning when present); and non-zero exit with only a
  bare `ERROR_MESSAGE` returns a hard process-exit error with process
  diagnostics.

Bridge tests cover only Garyx mapping and orchestration:

- SDK events map to the same `StreamEvent`/`ProviderMessage` sequence and final
  `ProviderRunResult`;
- the bridge supplies the explicit bypass callback;
- the bridge request environment preserves provider-config values, applies
  per-run `GARYX_*` runtime identity over them, and applies the run's desktop
  overlay with the current precedence;
- stale-session failure evicts and retries once;
- clear/shutdown delegate process cancellation to the SDK.

Required commands:

```bash
cargo test -p antigravity-sdk
cargo test -p garyx-bridge
```

If the local authenticated `agy` CLI is available, also run a real one-turn
smoke against a synthetic temporary workspace and report the evidence. The
smoke must not commit transcript contents or personal paths.

## Implementation Scope

The refactor does not change Antigravity's data source, add a new IPC, add
session fork, change model/config behavior, alter Garyx persistence, or repair
historical transcript files. Any material deviation from this boundary returns
to design review before implementation.
