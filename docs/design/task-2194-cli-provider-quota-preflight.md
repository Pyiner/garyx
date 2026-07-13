# TASK-2194: CLI task-create provider quota preflight

Status: implementation design; no production code yet

## Outcome and boundary

`garyx task create --agent <id>` performs a best-effort quota check before
`POST /api/tasks`. Only a fresh, credential-aligned, explicit zero blocks task
creation. The blocking path returns exit 1 and never sends the task POST. Every
unsupported or indeterminate case is fail-open.

The change is limited to `garyx-models` and the `garyx` CLI. It does not change
the gateway, bridge, router, HTTP contracts, desktop, or iOS, and it does not
require a gateway rebuild or restart.

## Shared pure evaluator

Add `garyx-models/src/quota.rs` and re-export its public types from `lib.rs`.
The module owns only serializable snapshot shapes and deterministic quota
semantics; it performs no I/O and has no provider probe registry.

The core API is:

```rust
pub fn evaluate_quota(
    snapshot: &CodingUsageSnapshot,
    provider: &ProviderType,
    model: Option<&str>,
    credential_scope: QuotaCredentialScope,
    now: DateTime<Utc>,
) -> Result<QuotaStatus, QuotaCheckError>;
```

The types are:

- `QuotaStatus::{Ok, Exhausted { provider, scope, reset_at }, Unsupported}`.
- `QuotaScope::{Window { name }, Model { name }}`.
- `QuotaCredentialScope::{DefaultLocal, Customized}`.
- `QuotaCheckError::{TimedOut, SourceUnavailable, Indeterminate,
  CredentialScopeMismatch, ModelNotFound}`. Variants may carry a sanitized
  reason/model for user-facing warnings, never an environment value, account,
  response body, or token.
- `CodingUsageSnapshot`, `CodingProviderUsage`, `CodingUsageWindow`, and
  `CodingModelUsage` mirror the existing `/api/usage/coding` response without
  changing that response.

Every numeric snapshot field is `Option<f64>`/`Option<i64>`. No numeric field
uses a serde default that can turn absence into zero. Missing, non-finite, or
out-of-range values become `Indeterminate`; they never become `Exhausted`.
`stale` may retain `#[serde(default)]` because the gateway intentionally omits
the false boolean. The mirrored wire field is named `resets_at`, exactly like
the gateway response; only the evaluated `QuotaStatus::Exhausted` field is
named `reset_at`. Unknown response fields remain ignored.

## Provider-neutral evaluation

The evaluator joins a target provider to response entries with
`ProviderType::from_slug(entry.id)`. This intentionally maps usage id `codex`
to canonical agent provider `codex_app_server`. There is no provider switch or
provider-id equality table.

The decision order is:

1. Reject `Customized` as `CredentialScopeMismatch` before inspecting quota
   values.
2. Find exactly one normalized provider entry. No matching entry means
   `Unsupported`, so the supported set is derived from the response. Duplicate
   normalized entries are `Indeterminate`.
3. Require `available == true` and `stale == false`; unavailable is
   `SourceUnavailable` and stale is `Indeterminate`.
4. Select behavior from data shape. A non-empty window shape evaluates
   windows; a non-empty model-bucket shape evaluates the exact target model.
   Both shapes at once, or neither shape, are `Indeterminate`.

Window evaluation considers the existing `session` and `weekly` fields only.
It does not recreate Codex window classification; the gateway's
`parse_codex_usage` remains the sole place that decides whether a Codex window
is approximately seven days. Any present window with a valid remaining value
of zero is exhausted. Other present windows cannot negate that zero. If no
current zero exists, malformed present windows or zeros whose parsed reset is
already before `now` produce `Indeterminate`; otherwise at least one valid
positive window is `Ok`. A machine-precision epsilon may absorb floating-point
zero, but ordinary small positive values remain `Ok`.

Bucket evaluation trims and exactly matches the target model against bucket
`id` or `name`; it never uses substring/fuzzy matching. Only the matched bucket
is evaluated. Its valid current zero is exhausted, a positive value is `Ok`,
and another bucket's zero is irrelevant. A missing target model or unmatched
bucket is `ModelNotFound`. A zero with a reset before `now` is
`Indeterminate`. An absent reset is allowed; a present but unparsable reset on
an apparent zero is indeterminate. `CustomAgentProfile.model` may legitimately
be empty and the profile response does not carry a separate provider-default
model; an empty bucket-model target is therefore `ModelNotFound` and safely
fail-open. Only an explicit exact id/name match may block a bucket-shaped
provider.

`reset_at` remains an optional RFC 3339 value in the result. The CLI converts
it to local wall-clock display; comparison stays in UTC inside the evaluator.

## CLI orchestration and interception

Add `garyx/src/commands/task_quota.rs` and wire it from `commands.rs`.
`cmd_task_create` keeps the normalized agent id separately from the executor
payload, builds the request as today, and invokes the preflight immediately
before `post_gateway_json_as_cli_actor(..., "/api/tasks", ...)`.

`check_agent_quota(gateway, agent_id)`:

- gets the authoritative profile from
  `GET /api/custom-agents/{urlencoded-id}`; it never reads
  `custom-agents.json`;
- derives canonical provider and the trimmed optional model from that profile;
- treats any non-empty `provider_env` as `Customized`, without copying or
  logging its keys or values;
- gets `GET /api/usage/coding`, deserializes `CodingUsageSnapshot`, and calls
  `evaluate_quota` with an injected/current `now`;
- uses a dedicated reqwest client path with bearer auth, short request
  deadlines, one attempt, and no retry helper; and
- wraps the whole operation in an approximately three-second constant timeout
  (with a shorter injected duration in tests). It adds no CLI flag and no
  second cache on top of the gateway's 20-second cache.

There is one irreducible conflict in the supplied constraints: immediately
starting the profile and usage GETs with `tokio::join!` necessarily sends a
usage request before the profile can reveal that the agent is Traex or has a
custom credential scope. That cannot coexist with the required assertions
that customized scope does not read the usage source. The implementation
therefore uses a profile-first sequence: fetch the profile; short-circuit a
non-empty `provider_env` as `CredentialScopeMismatch` without a usage request;
otherwise fetch usage exactly once and let the evaluator decide support from
the snapshot. Thus every `DefaultLocal` target, including Traex, performs one
usage GET; Traex then becomes `Unsupported` because no normalized entry exists.
Making Traex perform zero usage GETs would require a forbidden CLI provider
capability switch and would contradict snapshot-derived support. Both
single-shot requests remain under one three-second deadline and are never
retried. This explicit resolution supersedes the incompatible immediate
`tokio::join!` and Traex-zero-GET requirements while preserving the stronger
provider-neutral invariant.

The public `check_agent_quota(gateway, agent_id)` delegates to an internal
function that accepts `now` and the total timeout duration. Production passes
`Utc::now()` and the approximately three-second constant; deterministic tests
inject a fixed clock and millisecond timeout, so timeout coverage never sleeps
for real seconds.

Profile lookup failure, 404, response decode failure, usage 5xx/malformed
response, and timeout all become `QuotaCheckError`; the caller prints one
stderr warning and continues to the existing task POST. That preserves the
gateway's existing unknown-agent validation. `Unsupported` is silent and also
continues.

Only `Exhausted` becomes `TaskCreateQuotaExhausted`, a CLI-only typed error.
Its English message includes the agent id, canonical provider slug, model when
present, local reset time when present, and the exact phrase
`task was not created`. Because the error is returned before the POST, the
gateway never receives task creation.

`main.rs::report_cli_failure` recognizes this type independently of
`GatewayCliError`: exit code 1 and JSON kind `provider_quota_exhausted`.
Human output remains one stderr line. With `--json`, stdout is exactly one
failure envelope and best-effort warnings remain on stderr.

## Tests and evidence

`garyx-models` unit tests cover all three statuses and the error boundary:

- fresh zero and positive windows, Codex id normalization, missing windows,
  unavailable/stale readings, absent/NaN/infinite/out-of-range numbers,
  near-zero positive values, expired and future resets;
- exact Antigravity id/name matching, selected A=0 versus selected B>0,
  unmatched model, and non-selected zero buckets;
- customized credential scope before numeric inspection; and
- provider absent from the snapshot as `Unsupported`.

A sanitized, account-free `/api/usage/coding` fixture proves the real response
shape deserializes. A companion fixture with a missing numeric field must
evaluate to `Indeterminate`, never exhausted.

CLI Axum tests extend the existing `RecordedRequest`/gateway-config scaffold
with profile, usage, and task routes. They assert request counts, especially
zero `POST /api/tasks` for exhaustion and exactly one POST for healthy,
unsupported, stale, malformed, 500, timeout, customized-scope, and profile
lookup failure paths. Antigravity A/B selection is tested in both directions.
Customized scope asserts zero usage GETs and one task POST. Traex asserts one
usage GET, an `Unsupported` result, and one task POST. A process-level JSON
test asserts exit 1, `provider_quota_exhausted`, valid stdout JSON,
warning-free stdout, and zero task POSTs.

Focused validation:

```text
cargo test -p garyx-models quota --lib
cargo test -p garyx commands::task --bin garyx
scripts/test/rust_tier1_fast.sh --changed
```

No test contacts a live provider or consumes provider quota.
