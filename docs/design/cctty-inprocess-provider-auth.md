# In-Process cctty Provider Auth Design

> Scope: replace gateway Claude Code provider-auth login from "spawn an external
> cctty binary" with "call the inline `cctty` crate API in-process".
>
> Non-goals: do not reimplement Claude OAuth, do not change the PTY-driven
> Claude Code login core, do not change iOS or the HTTP auth contract.

## Problem

Gateway provider auth currently starts Claude Code login by resolving and
spawning a `cctty` executable. The resolver checks `GARYX_CCTTY_PATH`, a
bundled sibling binary, `PATH`, and finally `garyx __cctty`. That makes provider
auth depend on whichever external `cctty` binary wins command lookup. Machines
with an old Homebrew `cctty` such as `0.2.4` can be selected before the embedded
runner, and that old binary does not support `auth login --json-events`, causing
`502 claude_auth_failed_before_url` before a login URL is returned.

The correct boundary is:

- Garyx gateway must not resolve or spawn a `cctty` binary for provider auth.
- Gateway may call the inline `cctty` crate linked into the process.
- `cctty` itself still owns the PTY wrapper around the external official
  `claude` executable. That external `claude` lookup remains cctty's job.

## Current Code Facts

- `cctty` currently exposes only `pub async fn run_cli(argv) -> Result<i32>`.
  The CLI path is process-stdio oriented.
- `cctty::runner::run_auth_login_json_events` resolves the real `claude`, spawns
  it in a PTY, reads auth codes from process stdin, and writes JSON events to
  process stdout.
- The JSON event contract is already useful for callers:
  `started`, `authorization_url`, `input_requested`, `success`, `error`, `exit`.
- Gateway `garyx-gateway/src/provider_auth.rs` owns subprocess stdin/stdout,
  parses JSON lines, and fetches final auth status by spawning the selected
  command with `auth status --json`.
- `garyx-gateway` does not currently depend on `cctty`; only the top-level
  `garyx` binary crate depends on it for `__cctty`.

## cctty Library API

Add a public auth module, exported from `cctty/src/lib.rs`:

```rust
pub mod auth;
```

The API should be shaped around a login session, not process stdio:

```rust
pub struct AuthLoginOptions {
    pub passthrough_args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub claude_path: Option<PathBuf>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthLoginEvent {
    Started { command: String, args: Vec<String> },
    AuthorizationUrl { url: String },
    InputRequested { input: String, prompt: String },
    Success { message: String },
    Error { message: String },
    Exit { exit_code: i32 },
}

pub struct AuthLoginSession { ... }

impl AuthLoginSession {
    pub fn start(options: AuthLoginOptions) -> Result<Self>;
    pub fn input(&self) -> AuthLoginInput;
    pub fn take_events(&mut self) -> mpsc::Receiver<AuthLoginEvent>;
    pub async fn wait(self) -> Result<i32>;
}

#[derive(Clone)]
pub struct AuthLoginInput { ... }

impl AuthLoginInput {
    pub async fn submit_code(&self, code: impl Into<String>) -> Result<()>;
    pub fn close(&self);
}

pub async fn auth_status_json(options: AuthStatusOptions) -> Result<serde_json::Value>;
```

Details:

- `AuthLoginOptions::passthrough_args` is the real Claude auth args after
  stripping cctty-only `--json-events`, e.g. `["auth", "login", "--claudeai"]`.
- `cwd` defaults to the current process directory. It is also the working
  directory for the PTY process.
- `claude_path` is an optional explicit underlying Claude executable path. It
  is mainly a deterministic test seam and is equivalent in spirit to
  `CCTTY_CLAUDE_PATH`; production Garyx should leave it unset.
- `timeout` defaults to the existing one-hour `RUN_TIMEOUT`.
- `AuthLoginEvent` serializes to exactly the current JSON-line event shape.
- `auth_status_json` uses cctty's Claude resolver and runs the official
  `claude auth status --json`. It must not spawn a `cctty` binary.

Implementation should keep the current PTY core intact:

- Keep `PtyProcess::spawn`.
- Keep `interactive_claude_env` / `interactive_claude_unset_env`.
- Keep URL, prompt, success, error, timeout, and exit detection semantics.
- Refactor `process_auth_login_output` so it returns/emits
  `AuthLoginEvent` values instead of writing directly to stdout.

## cctty CLI Compatibility

`run_cli` and `auth login --json-events` must continue to behave as today.
Implement that path as a thin stdio adapter over the new library session:

1. Parse invocation exactly as now.
2. Start `AuthLoginSession` with `invocation.passthrough_args`.
3. Spawn the existing blocking stdin reader, but forward each received line via
   `AuthLoginInput::submit_code`.
4. Serialize every `AuthLoginEvent` to stdout as one JSON line.
5. Return the same exit code from the session.

This preserves:

- `garyx __cctty auth login --json-events ...`
- standalone `cctty auth login --json-events ...`
- the existing JSON event field names and order expectations
- no auth-code leakage to stdout/stderr

Normal auth passthrough commands such as `cctty auth status --json` remain
passthrough CLI behavior. The new `auth_status_json` library helper is for
in-process hosts such as gateway.

## Gateway Integration

Add `cctty = { workspace = true }` to `garyx-gateway/Cargo.toml`.

Update workspace dependency management:

- During local development, use a temporary local path to `~/repos/cctty` if
  needed for fast iteration.
- Before final commit in Garyx, push cctty master and update the workspace
  `cctty` git `rev` to that new commit.
- Do not leave a local path dependency in committed Garyx code.

Replace `provider_auth.rs` subprocess ownership with an in-process cctty
session:

- Delete `Command::new` login spawning, `ChildStdin`, stdout line parsing over a
  cctty child, stderr drain, and the `AuthCommandSpec`.
- Delete cctty command discovery:
  `resolve_auth_command`, `resolve_auth_command_from_config`,
  `explicit_cctty_path`, `bundled_cctty_path`, `executable_on_path`, and
  related constants.
- `ClaudeAuthSession` stores `cctty::auth::AuthLoginInput` instead of
  `ChildStdin`.
- `submit` calls `AuthLoginInput::submit_code`.
- `start` builds the same Claude login args, but starts
  `cctty::auth::AuthLoginSession` in-process and spawns a task to consume
  `AuthLoginEvent` values.
- Event handling keeps the current gateway state mapping:
  `authorization_url` -> `waiting_for_code` and initial `201`; `input_requested`
  -> `waiting_for_code`; `success` -> `succeeded`; `error` -> `failed`; `exit`
  -> `exit_code` plus terminal reconciliation.
- Final status fetch calls `cctty::auth::auth_status_json`, not an external
  `cctty` command.

Provider auth must be decoupled from `claude_cli_mode`:

- Do not read `AgentProviderConfig.claude_cli_mode` in provider auth.
- Do not use `GARYX_CCTTY_PATH`, `GARYX_CLAUDE_CLI_PATH`, or PATH for cctty
  binary discovery.
- `claude_cli_mode=native` still uses the in-process cctty auth helper. This is
  intentionally independent from the runtime provider's SDK execution mode.

The existing HTTP contract remains unchanged:

- `POST /api/providers/claude_code/auth/start` returns `201` with
  `login_id`, `status`, `url`, `auth_status`, `error`, `exit_code`.
- `POST /api/providers/claude_code/auth/{id}/submit` accepts `code` or `token`.
- `GET /api/providers/claude_code/auth/{id}` returns the current snapshot.
- Existing start timeout behavior and error codes remain:
  `claude_auth_start_timeout`, `claude_auth_failed_before_url`, etc.

## Tests And Verification

### cctty

Add library tests for the new auth session API using the existing fake Claude
script:

- session emits `started`, `authorization_url`, `input_requested`, `success`,
  and `exit`
- `submit_code` feeds the PTY and does not leak the code in events
- nonzero Claude exit emits `error` and `exit`
- `auth_status_json` returns parsed JSON from fake `claude auth status --json`

Keep and run existing CLI tests so `auth login --json-events` behavior is locked
against regressions.

Expected cctty validation:

```sh
cargo fmt
cargo test --lib
cargo test --test cctty_cli auth_
cargo test
CCTTY_CLAUDE_PATH=$(command -v claude) target/debug/cctty auth status --json
git diff --check
```

### Garyx

Replace provider-auth tests so they use a fake underlying `claude`, not a fake
`cctty` binary. The test should prove the new seam:

- configure the in-process cctty auth session with the fake Claude path through
  a test-only gateway store override
- start returns `201` plus an `authorization_url`
- submit transitions to `submitted`
- poll reaches `succeeded` and includes parsed `auth_status`
- a `claude_cli_mode=native` config still starts auth through cctty
- putting an old or failing `cctty` earlier in PATH does not affect auth start

Expected gateway validation:

```sh
cargo fmt
cargo test -p garyx-gateway provider_auth
cargo test -p garyx-gateway
git diff --check
```

### End-To-End Local Proof

Before handoff, run a real local gateway using the changed binary and prove:

- the gateway build links the updated cctty crate
- `PATH` can contain an old/failing `cctty` first without affecting provider
  auth
- `POST /api/providers/claude_code/auth/start` returns `201` and a real
  `authorization_url`
- the response is not `502 claude_auth_failed_before_url`

This proof should use the real endpoint contract on this machine. The auth code
does not need to be completed unless required to prove the regression is fixed;
the hard failing case is the inability to reach `authorization_url`.

## Risks

- The new API must not accidentally change event timing. Gateway relies on
  `authorization_url` to release the start request.
- `AuthLoginSession` cleanup must terminate or observe the PTY process on drop
  paths so abandoned HTTP sessions do not leave login processes behind.
- Rust test seams must avoid global process env races. Prefer explicit
  `claude_path` options over mutating `CCTTY_CLAUDE_PATH` in in-process tests.
- `auth_status_json` still spawns the official `claude`, which is acceptable.
  The forbidden dependency is spawning an external `cctty` binary or parsing
  command lookup for it.
- Cargo dependency finalization is cross-repo: Garyx must commit the pushed
  cctty rev, not a local path.

## Rollout Plan

1. Land and push cctty library API on `~/repos/cctty` master with a version
   bump.
2. Update Garyx workspace `cctty` rev to that commit and add the gateway crate
   dependency.
3. Replace gateway provider-auth implementation and tests.
4. Run cctty and gateway focused validations.
5. Run real gateway auth start proof with an old/failing `cctty` first in PATH.
6. Open implementation review against this design and iterate to pass.
7. Commit Garyx worktree and merge it back to main.
