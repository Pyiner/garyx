# Thread Runtime Model Snapshot

## Problem

Thread headers show the effective `model`, `model_reasoning_effort`, and
`model_service_tier` from `thread_runtime`. For threads without per-thread
override metadata and without stored runtime fields, the gateway currently falls
through to the live provider defaults from the current config. Changing a
provider default therefore changes how older threads are presented, even though
those threads actually ran with the old default.

The bug is not that `build_thread_runtime_summary` needs to repair data. The
bug is that the first real run never writes the thread's resolved runtime
selection into the thread record, and some run/read paths still let current
agent/provider defaults win before the thread's own stored runtime fields.

## Goal

On the first real run for a thread that is not using the corresponding explicit
override, persist the resolved runtime request selection into thread metadata:

- `metadata.model`
- `metadata.model_reasoning_effort`
- `metadata.model_service_tier`

After that, thread display and later runs use those stored fields before the
current bound agent profile or provider defaults. This is deliberately a
thread-runtime pin, not only a display cache: profile/default changes affect new
or still-unpinned threads, while already pinned threads continue to run with
their stored baseline until the user applies an explicit thread override.
Explicit per-thread overrides remain highest priority and remain user-editable.

This design intentionally does not rebuild history for existing threads. Existing
threads without a stored snapshot will be pinned the next time they run.

## Decisions

### 1. Write Timing And Source Of Truth

Persist the snapshot after the bridge accepts a new run and resolves the final
provider target, before the provider process/request is started and before the
streaming persistence worker writes the `run_start` control record.

The value must come from the same provider-specific resolution code that builds
the provider request, not from `build_thread_runtime_summary` and not from a
second gateway-side default calculation. The snapshot is the resolved request
selection, meaning the model/effort/tier Garyx selects and passes to the
provider request layer. It is not the provider-reported post-run
`ProviderRunResult.actual_model`.

That distinction is intentional:

- The thread header currently presents Garyx's configured effective runtime, not
  a transcript-derived actual-model audit field.
- `actual_model` is not uniformly available across providers and failure modes,
  and there is no equivalent post-run result for reasoning effort or service
  tier.
- Writing before provider execution makes the first visible run frame stable and
  avoids header jumps after a provider reports a more specific model id.

If product later wants the exact provider-reported model id, that should be a
separate lineage/audit feature or a different design that writes after the run
and defines overwrite semantics for `actual_model`. It should not be mixed into
this first-run runtime snapshot.

Add a small provider-side runtime selection contract, for example:

```rust
pub struct ProviderRuntimeSelection {
    pub model: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub model_service_tier: Option<String>,
}

fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection
```

Each provider implementation should reuse its existing request builders/helpers:

- Claude: `resolve_requested_model` and `resolve_requested_effort`.
- Codex/Traex: `resolve_codex_request_model`,
  `resolve_codex_request_reasoning_effort`, and
  `resolve_codex_request_service_tier`.
- Gemini and Antigravity: their existing `model_id` helpers.
- Native GPT/Claude/Gemini: the same `model_id` and request option resolution
  used to build `AgentLoopRunRequest`.

This keeps the snapshot tied to the value Garyx actually requests from the
provider. If a provider does not send a concrete value, leave that field unset
instead of guessing.

### 2. Idempotency And Override Semantics

Persist helper behavior:

- Normalize by trimming and dropping empty strings.
- Create `metadata` only when at least one field will be written.
- For each field, write only if the stored snapshot field is currently empty.
- Never mutate `model_override`, `model_reasoning_effort_override`, or
  `model_service_tier_override`.
- Do not overwrite an existing stored snapshot on later runs.
- Do not snapshot a field from a run where the corresponding explicit override
  is present. For example, if `model_override` is set, leave `metadata.model`
  empty and let a later non-override run pin the baseline model.

Overrides still win everywhere while present. Skipping snapshot writes for
override-controlled fields avoids turning a one-off thread override into the
permanent baseline after the user clears it. Clearing an override falls back to
the stored snapshot if one exists; if no non-override run has pinned that field
yet, the thread remains unpinned for that field and may still use the current
agent/provider default until the next non-override run writes the snapshot.

This design uses first-non-override-run-wins rather than last-run-wins. The goal
is to stop ambient config drift after a thread has established its runtime
baseline. Last-run-wins would keep headers aligned to the most recent run, but it
would also make agent/profile upgrades propagate into older threads and then
rewrite their baseline, which is the behavior this task is intended to stop.

### 3. Write Path

Thread records belong to `garyx-router`, and recent-thread projection must be
updated from write paths. The snapshot write must therefore use the bridge's
configured `ThreadStore` before run persistence begins:

1. Load the thread record from `inner.thread_store`.
2. Patch only `metadata.{model,model_reasoning_effort,model_service_tier}` and
   `updated_at` when something changed.
3. Save through `ThreadStore::set`.

Do not write or repair anything inside `build_thread_runtime_summary` or any GET
route. `RecentThreadProjectingStore::set` will keep the projection current for
gateway-backed stores.

The bridge already serializes per-thread dispatch with `thread_dispatch_guard`
before run startup. The snapshot helper should run inside that guard and before
the streaming persistence worker starts, which keeps the load-patch-set window
small and prevents the run's first persistence write from racing the snapshot
write. `ThreadStore::update` is not suitable for this nested metadata patch
because router stores only shallow-merge top-level keys.

The same helper should be used by both:

- `MultiProviderBridge::start_agent_run`
- `MultiProviderBridge::run_subagent_streaming`

### 4. Read And Run Precedence

After snapshot persistence exists, both display and future runs must prefer the
stored thread snapshot before current agent/provider defaults:

```text
explicit request metadata
> thread override metadata
> thread snapshot metadata
> current bound agent profile
> current provider default
> provider catalog fallback
```

Concrete changes:

- Update `build_thread_runtime_summary` so `metadata.model` and related stored
  snapshot fields win before `current_agent_runtime_metadata(...)`.
- Add a shared metadata merge helper next to
  `merge_thread_provider_overrides` that copies the stored snapshot fields into
  run metadata with `or_insert` semantics.
- Use that helper in gateway chat preparation and in bridge
  `backfill_bound_agent_runtime_metadata`, before applying current agent profile
  defaults.

This is not read-route repair; it is only correcting the priority order now that
thread records can carry durable runtime fields.

This is the main product tradeoff of Scheme B: once a thread has a stored
snapshot, later edits to the bound agent profile model or provider defaults do
not automatically affect that existing thread. Users can still change a single
thread through the explicit override controls. A future "reset to current agent
defaults" action could clear the snapshot fields, but that is outside this task.

### 5. Existing Threads And Backfill

Default recommendation: do not do a one-time backfill in this task.

Benefits of a backfill:

- Older threads could immediately display the model used by their last run.
- It would reduce surprise for threads that may never be run again.

Costs and risks:

- The historical model is only indirectly available in transcript/provider
  artifacts and is not uniformly present for every provider or failure mode.
- Choosing "last run actual model" versus "first run model" changes semantics
  and needs product approval.
- Scanning every thread transcript is a migration with public-repo fixture and
  data-shape risk, and it is unnecessary to stop future drift.

Recommendation: ship first-run snapshotting now, document that old unpinned
threads pin on their next run, and treat transcript-derived backfill as a
separate reviewed migration if product wants immediate historical correction.

There is one residual drift case before a field is pinned: if a provider has no
concrete default/request value during the first non-override run, the helper
leaves the field empty. If an admin later configures that default, the unpinned
field can start displaying the new default until a later run resolves and stores
a concrete value.

### 6. Desktop And iOS Impact

No desktop implementation is expected. Desktop maps gateway
`thread_runtime.model`, `thread_runtime.model_reasoning_effort`, and
`thread_runtime.model_service_tier` into `ThreadRuntimeInfo` and renders those
values in the header/composer.

No iOS data-model change is expected. iOS already decodes the same
`thread_runtime` fields. The iOS picker does have local fallback logic for
`defaultReasoningEffort(for:)` when `runtime.modelReasoningEffort` is missing,
so the backend must persist `model_reasoning_effort` whenever the provider
resolved one. With that backend fix, iOS remains a dumb renderer for this bug.

## Test Plan

Use synthetic IDs and models only.

### RED Reproduction

Add a focused gateway/router-level characterization test that proves the current
drift with no UI:

1. Build a test state with provider default model `provider-default-v1` and
   effort `high`.
2. Seed a synthetic thread with provider type, no override keys, and no stored
   snapshot fields.
3. Assert the initial summary reports `provider-default-v1/high`.
4. Rebuild or update the test state with the same thread record and provider
   default `provider-default-v2` / `max`.
5. Assert the summary now reports `provider-default-v2/max`, documenting the
   deterministic drift.

This characterization may pass on current code because it documents the existing
bug. The required RED tests are:

1. **First-run write path is missing.** Start a synthetic thread with no override
   keys and no stored snapshot, using a mock provider whose resolved runtime
   selection is `provider-default-v1/high`. Then change the provider default to
   `provider-default-v2/max` and assert the summary still reports
   `provider-default-v1/high`. On current code this fails because the first run
   never writes `metadata.model` or `metadata.model_reasoning_effort`.
2. **Stored snapshot loses to current agent profile.** Seed a thread with
   `metadata.model = provider-default-v1` and bind it to an agent profile whose
   current model is `agent-model-v2`. Assert the summary reports
   `provider-default-v1`. On current code this fails because
   `current_agent_runtime_metadata(...)` wins before the stored thread snapshot.

The already-seeded "snapshot beats changed provider default" case should remain
a regression test, not the primary RED test, because current code already lets a
seeded snapshot win over provider defaults when no agent profile model is
present.

### Regression Tests

Gateway summary tests:

- Stored snapshot wins over changed provider defaults.
- Stored snapshot wins over a changed current agent profile.
- Explicit override wins over stored snapshot.
- Clearing override falls back to stored snapshot, not live provider default.

Bridge/run tests:

- Starting a thread with no snapshot persists `metadata.model` and
  `metadata.model_reasoning_effort` from the provider runtime selection.
- Re-running after provider defaults change does not overwrite existing snapshot.
- Existing override fields are not mutated.
- A first run with `model_override` set does not write `metadata.model`; after
  clearing that override, a later non-override run can pin the baseline.
- Subsequent run metadata receives thread snapshot fields before agent/provider
  defaults.
- Sub-agent runs use the same snapshot path on the leaf child thread after
  runtime metadata is scrubbed and rehydrated from that child thread's metadata.
- Edge config where provider `model` and `default_model` differ documents that
  the first snapshot uses the provider request resolver. If the old summary had
  displayed a different provider default, the first run may correct the header to
  the value Garyx actually requested from the provider.

Validation commands:

```bash
cargo test -p garyx-gateway --lib thread_runtime
cargo test -p garyx-gateway --lib application::chat::prepare
cargo test -p garyx-bridge --all-targets runtime_snapshot
cargo test -p garyx-router --all-targets
```

If no desktop/iOS files change, no UI build is required. If iOS Core code is
touched, also run `swift test` and a real `xcodebuild` app-target build.
