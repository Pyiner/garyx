# TASK-2212: strict desktop gateway wire parsing

## Outcome

The Electron main-process gateway client will parse exactly the serialization
shape emitted by the same-repository gateway. Compatibility aliases, invented
missing-key placeholders, and computed replacements for required wire fields
will be removed. A missing or wrongly cased required field will raise a descriptive
contract error at the endpoint boundary instead of producing a plausible but
incorrect desktop model.

This is a parser-contract cleanup. It does not change gateway routes, shared
desktop models, transcript materialization, or stream reconnection/watchdog
logic. Two existing valid-response behaviors change intentionally. First,
automation activity rows are currently parsed as snake_case even though the
gateway emits camelCase, so their identities and timestamps collapse to
placeholders. The strict camelCase activity parser fixes that bug. Second, the
legal `failed_dropped` automation status currently falls through to `success`;
the exhaustive mapping correctly surfaces it as `failed`. Each correction gets
a dedicated regression fixture. Other valid-response mappings stay
behavior-compatible.

## Baseline evidence

Before any source edit, authenticated responses were captured from the local
gateway into an analysis-only worktree directory. Raw captures are excluded by
the repository's local Git `info/exclude` file and are never committed because
they can contain private runtime content. `git check-ignore` is part of the
pre-staging hygiene check.

| Capture | Size | SHA-256 | Contract observation |
| --- | ---: | --- | --- |
| `/api/tasks?include_done=true` | 28,642 bytes | `2f76a3cdea92fd6a6758e0b23dbeb8ad7bf5b0f5d88c54e080011334e500f6dd` | `tasks`, `total`, and `has_more`; task DTOs are snake_case only |
| `/api/threads?include_hidden=true&limit=1000` | 1,686,219 bytes | `a732b3a5007ee11e75030d4d45875693f3030aadbe4814b109b9f164f2211e22` | page and thread DTOs are snake_case only |
| bounded thread SSE capture | 103,419 bytes | `fff3275a312ad622eb9a2b49af3d311280a1cca568f8ddc8e491dc04ad716d07` | envelopes are snake_case; render snapshots retain their documented explicit camelCase renames |

The SSE capture completed three frames before the deliberate 12-second curl
timeout. Its outer keys were `type`, `thread_id`, `events`, and either
`render_state` or `render_delta`. CamelCase seen below the envelope belonged to
one of two current contracts: the complete explicit render rename set
`tailActivity`, `activeToolGroupId`, `rateLimit`, `resetAt`, and
`willAutoResend`, or opaque provider-native message body content such as command
metadata. Neither category is a legacy gateway alias.

The pre-edit deterministic baseline is 590 unit tests passing and a clean
`npx tsc --noEmit`. Source inspection also proves that `STATE_FILE_NAME` and
`LEGACY_STATE_FILE_NAME` are the same literal, so `migrateLegacyStateFile()`
cannot ever select different source and destination paths.

## Canonical casing by endpoint

“Single parser” means one current shape per endpoint, not globally forcing a
case that contradicts a server-side `serde` rename. The authoritative Rust
serializer determines the following matrix.

| Client area | Current gateway shape | Evidence |
| --- | --- | --- |
| tasks and task forest | snake_case | router/model structs and live task sample |
| thread pages, summaries, pins, and worktree metadata | snake_case | route JSON and live thread sample |
| thread SSE envelope and committed-event ledger fields | snake_case | route implementation and live SSE sample |
| render snapshot/delta payload | snake_case except the five documented explicit renames | `transcript_render_state.rs` render contracts and live SSE sample |
| start/input/interrupt chat HTTP responses | camelCase | gateway chat contracts use `rename_all = "camelCase"` |
| provider models and coding usage | snake_case | response structs use default/snake serialization |
| recent local provider sessions | camelCase | `RecentLocalProviderSession` uses `rename_all = "camelCase"` |
| automation summaries, activity entries, activity page metadata, and run-now | camelCase | automation response structs and route JSON |
| workspace root list and git status | snake_case | route JSON/default serialization |
| directory/file browsing, preview, upload, and attachment results | camelCase | `WorkspaceDirectoryListing` and workspace-file response structs use `rename_all = "camelCase"` |
| capsules | snake_case | `CapsuleRecord` default serialization |
| custom agents and provider icon descriptors | snake_case | `CustomAgentProfile` plus `custom_agent_response()` |
| skill catalog rows | snake_case | `SkillInfo` default serialization |
| skill tree/editor/file documents | camelCase outer documents, snake_case nested `skill` row | skill response structs |
| shortcut commands and MCP server records | snake_case | route `json!` construction |
| thread log chunks | camelCase | `ThreadLogChunk` uses `rename_all = "camelCase"` |

This matrix explains why a mechanical “make every property snake_case” edit
would itself break current wire behavior. Each old dual parser is replaced by
the one entry in this table.

### Thread route shapes

Thread casing is uniformly snake_case, but the routes intentionally emit three
different field sets. They will no longer share one union mapper:

| Route family | Dedicated parser contract |
| --- | --- |
| `GET /api/threads`, `POST /api/threads`, and thread `PATCH` | standard summary from `thread_summary()` / `thread_summary_from_meta()`: `thread_id`, `thread_key`, `thread_type`, and every `json!` summary key are present; nullable presentation values must still have a key |
| `GET /api/recent-threads` | `RecentThreadRecord`: `title`, `last_active_at`, `recorded_at`, and `run_state` are required; it does not require standard-summary `label` or `created_at` |
| `GET /api/threads/{key}` | raw stored thread metadata plus injected required `thread_id`, `thread_key`, `thread_type`, and `thread_runtime`; stored presentation keys may be absent, are read only under their canonical names, and are not substituted from a different route shape |
| thread pin routes | the current `thread_ids` array is required; the redundant `pins` projection is not used as a fallback |

This removes the `label || title`, `created_at || recorded_at ||
last_active_at`, and other cross-route chains. The never-emitted `session_key`
and old `sessions` page aliases are deleted explicitly. Each caller selects the
mapper for its route instead of asking a union mapper to guess the response
kind.

### Task route shapes

Task list/forest rows and task create/detail envelopes are all snake_case, but
they are not the same field set. List/forest rows require their serialized
`runtime_agent_id` and `reply_count`. `GET /api/tasks/{task_id}` requires both
the `task` object and backing `thread` object; its desktop runtime and reply
projections come from raw stored `thread.agent_id` (absent/null maps to the
existing empty runtime id) and required `thread.message_count`. `POST
/api/tasks` instead requires the outer
`runtime_agent_id`, `number`, and `status` projections alongside `task`. A
successful create owns a brand-new backing thread, so its desktop `replyCount`
is deterministically zero; this is endpoint semantics, not a fallback for a
missing field. The outer create number/status are checked against the nested
task rather than silently trusting an inconsistent response.

## Parsing policy

### Required fields

A small `GatewayContractError` and typed assertion helpers will be added to the
main-process HTTP module. Endpoint mappers will use them for fields that the
current gateway always serializes. Errors identify the endpoint/type and field,
for example `Gateway contract violation: task summary.thread_id must be a
non-empty string`.

Required means that a missing field, `null` where null is not in the Rust type,
an invalid enum value, or a wrong primitive/container type is a schema break.
These cases throw; they do not become an empty string, epoch timestamp, zero,
`false`, inferred complement percentage, or filtered-out row. Collections and
page counters emitted unconditionally by a route are required even when empty
or zero. Nullable fields that are serialized unconditionally must be present
and may contain `null`.

The strict checks cover every field consumed by the touched mappers, including:

- task/page identities, principals, status, timestamps, counters, roots, and
  forest node structure;
- stream frame identity and required render/event containers;
- provider identity/capabilities, required quota percentages, and recent
  session identity;
- automation identity, schedule, timing, status, and page metadata;
- thread page metadata, pin arrays, worktree fields consumed by the desktop, and
  log chunk fields;
- workspace, capsule, agent, skill, shortcut, and MCP identities and their
  unconditionally serialized presentation fields.

Successful malformed responses must remain visible. Existing transport
fallback catches in chat control calls and thread lookup will rethrow
`GatewayContractError` while retaining their present behavior for transport or
HTTP failures. SSE contract failures continue through the existing stream-gap
error path; no reconnect or watchdog mechanics change.

### Raw stored-record routes

Two detail routes return persisted JSON records rather than a fixed response
DTO. Their injected envelope/runtime fields stay strict, but optional stored
keys are not schema violations:

- task detail permits an absent or null backing `thread.agent_id` and maps it
  to the list route's existing empty runtime id;
- thread detail permits absent `channel_bindings`, `workspace_dir`, and
  `worktree`, mapping them to `[]`, `null`, and `null`; fields absent inside a
  stored binding keep the existing empty/null canonical projections without
  consulting alternate field names.

These exceptions are keyed to the raw-record route, not a compatibility
version. A present value with the wrong primitive/container type still throws.

### Fields that may be omitted

Outside the raw stored-record routes above, only fields with a current
serializer-side `skip_serializing_if` may be absent from a valid response and
receive a local optional value. `serde(default)` on a deserializer does not make
a serialized field omittable.

| Contract | Allowed missing-field behavior |
| --- | --- |
| task summary | `assignee`, `source`, and `executor` -> `null` |
| anchored task forest | omitted `active_count` -> `null`; omitted node `depth` -> `null` |
| provider model option | omitted `description` and `default_reasoning_effort` -> `null`; omitted `recommended` -> `false`; omitted option lists -> `[]` |
| provider reasoning-effort option | omitted `description` -> `null`; omitted `recommended` -> `false` |
| provider model catalog | omitted default model/reasoning and `error` -> `null`; server-defaulted capability lists/flags -> `[]`/`false` only where the serializer can omit them |
| coding usage | omitted `stale` -> `false`; omitted `plan`, windows, reset metadata, description, and `error` -> `null`; omitted model list -> `[]`; present `reset_after_seconds` is a signed Rust `i64` |
| automation summary/activity | omitted summary target/thread/run timestamps and unread hint, plus activity `finishedAt`, `durationMs`, and `excerpt`, -> their corresponding optional desktop value |
| custom agent | omitted `provider_env` -> `{}`; omitted default workspace -> empty optional desktop value |

No other missing-key default is accepted in response parsing.

### Fields that must be present but may be null

An `Option` serialized without `skip_serializing_if`, or an unconditional
`json!` key containing an option, must still exist on the wire. Its value may be
`null`; a missing key throws.

| Contract | Required-present nullable fields |
| --- | --- |
| workspace directory listing | `parentPath` |
| workspace file entry | `size`, `modifiedAt`, `mediaType` |
| workspace file preview | `modifiedAt`, `text`, `dataBase64`; `mediaType` and `size` are required non-null, and every uploaded workspace/chat attachment field is required non-null |
| skill tree/file | `children` is a required array even when empty; file `dataBase64` is required and may be null |
| shortcut command | `prompt` |
| MCP server | `working_dir`, `url`, and `bearer_token_env`; arrays/maps and all other route-inserted keys are also required |
| standard/recent thread summaries and present worktree objects | every fixed-route key, including nullable labels, timestamps, provider/agent/run ids, and the list summary's nullable `worktree` object; raw detail omissions follow the exception above |
| capsule | `thread_id`, `run_id`, `agent_id`, and `provider_type` |
| custom agent response | `avatar_data_url` and `provider_icon`, which are inserted unconditionally and may be null |

Request-construction defaults and local input aliases such as a shared
`threadId`/`sessionId` argument are not response parsing and are outside this
cleanup. Client-invented `sessionId` response fields are removed because no
current chat response emits them.

### Owned exceptions

- Provider-native transcript message bodies retain their current mixed-case
  interpretation. They are opaque ledger content, not gateway DTO aliases.
- Render snapshot and delta payloads retain exactly the five documented
  camelCase renames listed above; other gateway envelope fields remain strict
  snake_case.

### Automation status mapping

The current wire enum is exhaustively recognized before adapting it to the
existing three-state desktop model. Unknown strings throw. To keep this cleanup
inside `src/main` without changing renderer/shared contracts, the mapping is:

| Wire `JobRunStatus` | Desktop status |
| --- | --- |
| `success` | `success` |
| `failed`, `failed_dropped` | `failed` |
| `running`, `never_run` | `success` |

The `running`/`never_run` row preserves the existing desktop presentation for
legal nonterminal or never-run values while making the lossiness explicit and
exhaustive. Mapping `failed_dropped` to `failed` intentionally corrects its
current fallthrough to `success`; a fixture pins that behavior. The desktop-only
`skipped` value is not accepted from gateway responses because the gateway
never emits it. A future richer UI status is a separate shared-contract change.

## File changes

- `garyx-client/http.ts`: add the contract error and narrowly typed required /
  optional assertion helpers.
- `garyx-client/tasks.ts`, `threads.ts`, `stream.ts`, `provider.ts`,
  `automations.ts`, `workspaces.ts`, and `capsules.ts`: remove dual-casing DTO
  members and parse only the endpoint’s canonical shape; replace silent
  required-field defaults with contract errors.
- `garyx-client/agents.ts` and `catalog.ts`: remove the same compatibility chain
  found by the directory-wide baseline scan even though the initial line list
  did not enumerate these two files. Leaving them would fail the requested
  directory-wide completion condition. Mixed skill document casing follows the
  explicit server matrix above.
- `garyx-client/bots.ts` and its `store.ts` configured-bot mapping: delete the
  never-emitted `displayName` response alias and read only `display_name`.
- `store.ts`: also delete the duplicate legacy filename constant, legacy path
  helper, impossible migration function, its startup call, and the now-unused
  import.
- main-process unit tests: add public-safe synthetic canonical response fixtures,
  wrong-case/missing-required rejection cases, optional whitelist cases, and a
  static guard against reintroducing the retired aliases. A dedicated automation
  activity regression test proves canonical camelCase rows retain their id,
  timestamps, duration, and thread id; a `failed_dropped` fixture separately
  asserts the desktop status is `failed`. Sparse raw-record fixtures cover a
  task without `thread.agent_id` and a thread detail without workspace/worktree
  keys or a normalized binding key; a coding-usage fixture pins signed
  `reset_after_seconds`, and a missing-agent-avatar fixture must throw. Existing
  stream tests cover the stream control path; the new cases exercise the casing
  boundary.

No raw live response, home directory, token, real person/bot identifier, or
runtime transcript will enter the repository.

## Verification and review

Focused tests will replay one canonical response per touched endpoint family
and at least one former wrong-case form per parser. A source guard will allow
the documented render/provider-content exceptions while rejecting known DTO
alias names in their former files. The store test will assert that neither the
legacy constant nor migration function remains.

Before code review:

1. run the focused main-process contract tests;
2. run `npm run test:unit` (including the gateway-mirror contract suite);
3. run `npx tsc --noEmit`;
4. run the directory-wide retired-alias source scan;
5. prove `git check-ignore .task-2212-baseline/` succeeds, then scan the staged
   diff for public-repository data hygiene;
6. compare the synthetic fixtures and parser casing against both the recorded
   live key sets and the Rust serializer declarations.

After an independent Claude-family design PASS, implementation may begin. An
independent Claude-family code review must then check both the baseline evidence
and the resulting parsers. After its PASS, fetch and rebase onto the latest
`origin/main`, reconcile any upstream changes according to their current
semantics, rerun validation, commit reconciliation if needed, and push the
result directly to remote `main`.
