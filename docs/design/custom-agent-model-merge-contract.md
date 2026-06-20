# Custom Agent Model Merge Contract

## Problem

`PUT /api/custom-agents/{agent_id}` and `POST /api/custom-agents` currently
treat omitted model fields as empty strings because the request payload uses
`String` fields with `#[serde(default)]`. That collapses two different client
intents:

- field absent: keep the stored value when updating an existing agent
- field present with `""`: replace the stored value with provider default

The same endpoint already has merge semantics for adjacent optional fields such
as provider environment, default workspace, and avatar data. Model selection,
reasoning effort, and service tier should use the same contract.

## Contract

For custom-agent upsert requests:

| Request state | Existing agent update | New agent create |
| --- | --- | --- |
| `model` absent | keep stored `model` | store `""` |
| `model: "x"` | store `"x"` after trimming | store `"x"` after trimming |
| `model: ""` | store `""` (provider default) | store `""` |

`model_reasoning_effort` and `model_service_tier` follow the same rules. The
camelCase aliases `modelReasoningEffort` and `modelServiceTier` remain accepted.

Stored `CustomAgentProfile` fields stay as `String`; the tri-state behavior is
only a request-layer concern.

## Design

### Gateway

- Change `CustomAgentUpsertPayload.model`, `model_reasoning_effort`, and
  `model_service_tier` from `String` to `Option<String>`.
- Change `UpsertCustomAgentRequest` the same way.
- In `CustomAgentStore::upsert_agent`, resolve each field with one helper:
  - `Some(value)` trims and stores the value, including `""`.
  - `None` keeps the existing profile value.
  - `None` on create stores `""`.
- Keep built-in-agent modification rejection before persisting changes.
- Keep provider environment, auth, workspace, avatar, native config, and prompt
  behavior unchanged.

### CLI

- Stop serializing `model`, `model_reasoning_effort`, and
  `model_service_tier` when the caller omitted the matching option. This lets
  the gateway preserve existing values on update.
- Keep create behavior unchanged: omitted keys create empty stored values, which
  means provider defaults.
- Add `--clear-model` to `garyx agent update` and `garyx agent upsert`; it sends
  `model: ""` explicitly and conflicts with `--model`.
- Keep existing explicit-empty support for reasoning effort and service tier,
  because passing `--model-reasoning-effort ""` or `--model-service-tier ""`
  still sends an explicit replacement value.
- Update help text so omission on update/upsert is described as preserve, while
  omission on create remains provider default.

### Mobile

`GaryxCustomAgentRequest` already has optional request fields, so its shape does
not change. The mobile create/edit flows must send model and reasoning effort
values whenever the user is saving those controls:

- create: send the trimmed `model` and `model_reasoning_effort` strings even
  when they are empty
- update: send the next model, next reasoning effort, and preserved service tier
  strings even when they are empty

This keeps "reset to Provider default" working under the new absent-as-preserve
contract.

Desktop already sends model values from required controls, so no desktop change
is planned.

## Tradeoffs

- Using `Option<String>` at the request boundary is more explicit than adding a
  separate clear flag to the API. JSON already has a native absent-vs-present
  distinction, and existing optional fields use that pattern.
- Keeping stored fields as `String` avoids a storage migration and preserves
  the existing provider-default representation.
- The CLI grows only the missing `--clear-model` affordance. Empty string
  values remain supported for the other two fields to avoid a broader CLI flag
  expansion.

## Rollout/compatibility

The new gateway treats absent model fields as preserve. Older iOS builds send
`nil` for empty model controls, so those builds temporarily lose the ability to
reset an agent model or reasoning effort to Provider default after the gateway
change ships. They can still preserve existing settings.

Ship the gateway and iOS app changes together so mobile regains explicit
empty-string clears. Desktop sends concrete model values from required controls,
and the CLI change adds an explicit clear path, so desktop and CLI are not
affected by this compatibility window.

## Validation

- Gateway store tests:
  - update without model fields preserves existing model settings
  - update with `Some("")` clears model settings
  - update with `Some("x")` replaces model settings
  - create without model fields stores empty strings
- Gateway build coverage: `cargo build`
- Focused gateway tests: `cargo test -p garyx-gateway custom_agents`
- CLI tests:
  - update without model options omits the three keys
  - update with `--clear-model` sends `model: ""`
- CLI end-to-end check against a real persisted agent: dump current agent,
  update without `--model`, and confirm the stored model is preserved.
- Mobile core test: `GaryxCustomAgentRequest(model: "")` encodes a present
  `"model": ""` key.
- iOS app target build: run `xcodebuild` for the app target because SwiftPM
  tests do not cover app-target wiring.
