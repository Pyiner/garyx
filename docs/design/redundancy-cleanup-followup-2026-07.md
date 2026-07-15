# Redundancy Cleanup Follow-up Batch Plan (2026-07)

Follow-up to the 2026-07 repository-wide redundancy cleanup. Covers 7
"zero-consumer / dead chain" candidates. Every claim was independently
re-verified before this plan was written (exhaustive `rg` without
truncation, `--hidden` included, production vs test consumers listed
separately). Only verified-CONFIRMED surfaces are scheduled for deletion;
PARTIAL claims are scheduled with their verified preconditions.

## Verification Summary

| # | Claim | Verdict | Batch |
|---|-------|---------|-------|
| 1 | `garyx-core` keys/label/route_resolver test island (787 impl + 649 test lines, zero production consumers) | CONFIRMED | R1 |
| 2 | `FileThreadStore` write half (set/delete/update/clear/size/lock/atomic-write) dead in production | PARTIAL — set/delete/update pinned by `ThreadStore` trait; needs import-source narrowing first | R2 |
| 3 | Bridge `ProviderHealth`: per-run write-lock updates, both getters zero callers | CONFIRMED | R3 |
| 4 | Mobile `HomeProjectionActor` per-boundary full legacy-projection parity compare, parity outputs have zero production consumers | CONFIRMED | M1 |
| 5 | Mobile 8 zero-consumer `GaryxGatewayClient` wrappers | CONFIRMED (all 8 mirror shipped Mac features — see Decision Point) | M2 |
| 6 | Desktop 11 renderer-zero-consumer preload → IPC → runtime chains | CONFIRMED (exactly 11; dynamic/indirect call paths ruled out) | D1 |
| 7 | Desktop `AgentAvatar.tsx` / `card.tsx` zero importers; tsconfig unused gate off with 142 diagnostics | PARTIAL — `.design-sync` barrel holds live references; must be co-updated | D2 / D3 |

Non-goals: no behavior change anywhere; no removal of the mobile legacy
home projection (`GaryxHomeThreadListStore`) or its kill-switch fallback
(separate future decision); no change to message routing semantics.

---

## Batch R1 — Delete `garyx-core` keys / label / route_resolver islands

Delete:
- `garyx-core/src/keys.rs`, `garyx-core/src/keys/` (`parsing.rs`, `tests.rs`)
- `garyx-core/src/label.rs`, `garyx-core/src/label/tests.rs`
- `garyx-core/src/route_resolver.rs`, `garyx-core/src/route_resolver/tests.rs`
- `garyx-core/src/lib.rs` re-exports for the three modules (lines 7–46 region)
- Orphan fixtures: `tests/fixtures/keys_fixtures.json`,
  `tests/fixtures/label_fixtures.json`, and the corresponding sections of
  `tests/fixtures/generate_fixtures.py` (verify first whether the whole
  script only serves these modules; if so delete the script).

Keep (production consumers exist):
- `garyx-core/src/slash_commands.rs` + its tests + its `lib.rs` re-exports.
  Consumers: `garyx-gateway/src/application/chat/prepare.rs:308`,
  `garyx-router/src/router/run/execution.rs:33`,
  `garyx-router/src/router/run/planning.rs:381`.

Also update:
- `garyx-router/src/router/message/routing.rs:12-19` comments referencing
  `garyx_core::route_resolver` ("simplified until garyx-core is ready") —
  reword to describe current behavior as intended; do NOT change the
  `resolve_agent_for_channel` default-agent behavior itself.

Validation: `cargo test -p garyx-core`; `cargo test -p garyx-router
--all-targets`; exhaustive `rg` (no truncation, `--hidden`) proving zero
remaining references to every deleted pub symbol.

## Batch R2 — Narrow boot-import source, then delete `FileThreadStore` write half

Precondition refactor (verified: `set`/`delete`/`update` are required
`ThreadStore` trait methods with no default bodies, so they cannot be
deleted while `FileThreadStore` implements `ThreadStore`):
- Narrow the legacy boot-import source from `Arc<dyn ThreadStore>` to a
  read-only surface (small trait with `list_keys` + `get`, or the concrete
  read-only type). Touch points: `garyx/src/runtime_assembler.rs:42-92`,
  `garyx-gateway` `assemble_sqlite_thread_store` /
  `import_thread_records_if_needed` signatures
  (`sqlite_thread_store.rs:323/334/360-509`). Direction matches
  `docs/design/legacy-boot-import-isolation.md`.

Then delete from `garyx-router/src/file_store.rs`:
- `impl ThreadStore for FileThreadStore` write methods (`set`/`delete`/
  `update`) — replaced by the narrowed read-only impl
- `clear()`, `size()`, `with_options()` (non-trait, test-only consumers)
- Lock machinery: `acquire_lock`, `release_lock`, `check_stale_lock`,
  `lock_file_for_path`, `resolve_write_lock_thread_file`
- `atomic_write`

Keep (production boot import uses list+get only — verified at
`sqlite_thread_store.rs:366/408`):
- `get`, `list_keys` and their helpers: `resolve_thread_file`,
  `legacy_thread_file`, `legacy_compat_thread_file`, `thread_file`,
  `decode_key`, `stem_to_key`, `file_mtime`, `evict_if_needed`,
  `deep_clone`, `storage_roots`, `CacheEntry`/cache,
  `encode_thread_storage_key`, `thread_storage_file_name`.

Test migration:
- Gateway `contract_tests` currently seed legacy data via `source.set(...)`
  (`sqlite_thread_store.rs:847`, cfg(test)). Rework seeding to write legacy
  JSON layout files directly on disk (more faithful to the real
  pre-upgrade scenario). Do not keep a pub write path just for tests.
- `garyx-router/src/file_store/tests.rs`: write-path tests are deleted with
  the dead code; read-path tests (list/get/eviction/key codec) stay and
  switch to on-disk seeding.

Contract check (verified): boot import (list+get) and backup-restore
(archived JSON on disk + forced re-import) are both read-only over the
archive; deleting the write half breaks neither. No runtime JSON backend
may be (re)introduced.

Validation: `cargo test -p garyx-router --all-targets`; `cargo test -p
garyx-gateway --lib`; `scripts/test/rust_tier1_fast.sh --changed`.

## Batch R3 — Delete `ProviderHealth` dead-write cluster

Delete:
- `garyx-bridge/src/multi_provider/topology.rs:85-119`: both getters
  (`get_provider_health`, `get_all_provider_health` — zero callers
  repo-wide) and `record_health_success` / `record_health_failure`
- `garyx-bridge/src/multi_provider/run_management.rs` call sites at
  906/913/1082/1539/1546/1629
- `garyx-bridge/src/multi_provider/state.rs:86` `provider_health` field
- `garyx-bridge/src/multi_provider/lifecycle.rs:230-232` retain block
- `garyx-bridge/src/provider_trait.rs:222-` `ProviderHealth` struct + impl
  (`record_success`/`record_failure`), `lib.rs:14` re-export,
  `provider_trait/tests.rs:34-105`

Keep: the `/health/detailed` chain (`route_graph.rs:39`, `routes.rs:622`,
`health.rs:41` `HealthChecker`) — verified independent of topology.

Same-name traps (verified, do not touch): `claude_provider.rs:1319`
`record_failure` and `recent_threads.rs` `record_successful_page` are
unrelated methods.

Validation: `cargo test -p garyx-bridge --all-targets`; exhaustive `rg`
proving `ProviderHealth` zero remaining references.

## Batch M1 — Remove mobile runtime parity chain

Delete from `mobile/garyx-mobile/Sources/GaryxMobileCore/HomeProjectionActor.swift`:
- Types `HomeProjectionParityMismatch`, `HomeProjectionCheckpoint`,
  `HomeProjectionSnapshotCounters`, `HomeProjectionLiveLegacyDiagnostics`
- `HomeProjectionBoundaryResult` fields `parityMismatchCount`,
  `latestParityMismatch`, `liveLegacyDiagnostics`
- Actor state `checkpointStore`, `parityMismatchCount`,
  `latestParityMismatch`; the `finishBoundary` parity block (lines 325–343
  region: ~4 O(N) passes per boundary purely for parity)
- The `liveLegacySnapshot` parameter threading (278/292/315/322/338/425/
  431/437/472/476 region) — production always passes nil
- Gateway `parityMismatchCount` (384/494 region)

Keep:
- `GaryxHomeThreadListStore` legacy projection incl. `apply(_ input:)` —
  still the production kill-switch fallback
  (`GaryxMobileModel+Presentation.swift:112-121`) and shadow path, and the
  live display store itself. Out of scope here.
- `snapshotEmitCount` (actor + gateway) — non-parity, asserted by behavior
  tests.
- Reducer-side equivalence oracle: `assertCheckpointParity`
  (`HomeProjectionReducerTests.swift:614-624` + 5 call sites) and
  `legacyCheckpointInput()` (`HomeProjectionReducer.swift:104`) — test-time
  only, zero production cost, and it guards equivalence between the two
  still-shipping production paths (actor vs legacy fallback). Mark
  `legacyCheckpointInput()` with a doc comment stating it is test-oracle
  support.

Test migration:
- `HomeProjectionActorTests.swift:5-39`
  (`testCheckpointParityIgnoresLiveSummaryOnlyRunningMismatch`): before
  deleting, migrate the real behavior assertion at lines 30–31 ("runTracker
  busy overrides summary-only isRunning") into a direct snapshot behavior
  test; drop the parity assertions.
- Remove scattered `parityMismatchCount == 0` assertions
  (`HomeProjectionActorTests.swift:32/78/106/167/212`,
  `GaryxHomeObservationBridgeTests.swift:79/106`).

If any Swift file is added/removed, run `xcodegen generate` and commit the
pbxproj; otherwise not needed.

Validation: full `swift test` for GaryxMobileCore (report the real passed
count, no piping through tail); `xcodebuild` build.

## Batch M2 — Delete 8 zero-consumer gateway wrappers (DECISION POINT)

Delete from `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxGatewayClient.swift`
(verified zero App/Core/Widget/test consumers each):
- `threadLogs` (:393), `saveSkillFile` (:581), `createSkillEntry` (:588),
  `deleteSkillEntry` (:595), `automationActivity` (:631),
  `deleteWorkspace` (:693), `startChannelAuthFlow` (:825),
  `pollChannelAuthFlow` (:835)

Do NOT touch the look-alikes with live consumers: `createSkill`/
`updateSkill`/`toggleSkill`/`deleteSkill`, `skillEditor`/`readSkillFile`,
`validateChannelAccount`.

Decision point (default = delete): all 8 mirror shipped Mac features (skill
file editing writes and channel auth flow are "iOS has the read half"
gaps). Restoring any of them from git history when iOS wires the UI costs
~zero. If the owner wants any group kept for near-term iOS work, drop it
from this batch; no other batch depends on it.

Validation: full `swift test` + `xcodebuild` build.

## Batch D1 — Delete 11 renderer-zero-consumer IPC chains

All paths under `desktop/garyx-desktop/src`. For each chain delete: preload
entry + contract interface entry + main import line + handler registration
block (incl. inline bodies) + runtime function.

| API | preload/index.ts | contracts/desktop-api.ts | main handler | runtime |
|-----|-----|-----|-----|-----|
| getTask | :140 | :273 | index.ts:780-783 (+import :143) | garyx-client/tasks.ts:609 |
| unassignTask | :147 | :289 | index.ts:861-867 (+:178) | garyx-client/tasks.ts:742 |
| updateTaskTitle | :150 | :292 | index.ts:879-885 (+:179) | garyx-client/tasks.ts:784 |
| updateSkill | :168 | :325 | index.ts:957-963 (+:173) | garyx-client/catalog.ts:337 |
| listChannelEndpoints | :215 | :382 | index.ts:1162-1165 (inline, no runtime fn) | — |
| getWorkspaceGitDetails | :223 | :278 | index.ts:1423 (+:265) | workspace-git-runtime.ts:125 |
| commitWorkspaceChanges | :225 | :281 | index.ts:1424 (+:264) | workspace-git-runtime.ts:132 |
| pushWorkspaceBranch | :227 | :284 | index.ts:1425 (+:266) | workspace-git-runtime.ts:157 |
| probeGateway | :299 | :435 | index.ts:1411-1418 (+:161) | garyx-client/gateway.ts:171 |
| copyImageToClipboard | :319 | :459 | index.ts:1436 (+:238) | browser-runtime.ts:1193 |
| activateTerminalSession | :392 | :484 | index.ts:1468 (+:254) | terminal-runtime.ts:235 |

Constraints:
- Delete functions, never files: `workspace-git-runtime.ts`, `tasks.ts`,
  `catalog.ts`, `gateway.ts`, `terminal-runtime.ts`, `browser-runtime.ts`
  all have consumed sibling exports.
- `getTask` runtime is covered by `src/main/gary-client.test.mjs:11/328/359`
  — remove/adjust those test cases in the same commit.
- `*Input` types in `src/shared/contracts/{task,workspace,browser-terminal,catalog}.ts`
  (`GetTaskInput`, `UnassignTaskInput`, `UpdateTaskTitleInput`,
  `UpdateSkillInput`, `CommitWorkspaceChangesInput`,
  `PushWorkspaceBranchInput`, `CopyImageToClipboardInput`, …): re-verify
  references after the chain deletions, then delete the orphaned ones.

Validation: `npm run test:unit` (never `node --test` directly on `.mjs` —
`.ts` imports break) + `npx tsc --noEmit`. Since preload/IPC surface
changed, one packaged-app smoke check is required before ship — performed
once at final acceptance by the orchestrator (not per-batch; `npm run
dist:dir` auto-installs to /Applications and must not clobber an in-use
app without coordination).

## Batch D2 — Delete `AgentAvatar.tsx` / `card.tsx` with `.design-sync` co-update

Verified: zero importers in shipped app code (renderer/main/preload), but
the `.design-sync` hidden-tree barrel holds live references — deleting only
the component files breaks the design-sync converter.

Delete together (all under `desktop/garyx-desktop`):
- `src/renderer/src/app-shell/components/AgentAvatar.tsx`
  (do not confuse with `AgentAvatarEditor`, which is a different, consumed
  component)
- `src/renderer/src/components/ui/card.tsx`
- `.design-sync/entry.tsx` lines `export * from '@/app-shell/components/AgentAvatar'`
  (:30) and `export * from '@/components/ui/card'` (:11)
- `.design-sync/config.json` mappings ("AgentAvatar" :40, "Card" :23)
- `.design-sync/previews/AgentAvatar.tsx`, `.design-sync/previews/Card.tsx`
- `.design-sync/conventions.md` corresponding entries (:78, :14)
- Regenerate or prune corresponding `ds-bundle/` derived output per the
  design-sync tool's flow.

Validation: `npx tsc --noEmit` + `npm run test:unit`; design-sync tool run
(if locally runnable) must not fail on the barrel.

## Batch D3 — Enable the unused gate and clear diagnostics (AFTER D1/D2)

Ordering: strictly after D1/D2 — those deletions create new unused symbols
(orphaned `*Input` types etc.); iterate `tsc` until clean.

- Add `noUnusedLocals: true` and `noUnusedParameters: true` to
  `desktop/garyx-desktop/tsconfig.json`.
- Clear the real diagnostics (measured baseline: 142 total — TS6133 ×124,
  TS6196 ×15, TS6192 ×3; `AppShell.tsx` 65, `GatewaySettingsPanel.tsx` 33,
  remainder long tail incl. `src/main/*`, `src/web/*`).
- Principle: an unused diagnostic is a lead, not a license — confirm each
  deletion is side-effect free before removing; intentionally kept
  parameters get `_` prefixes, never a gate rollback.

Validation: `npx tsc --noEmit` green with the gate on + `npm run test:unit`;
packaged smoke folded into final acceptance.

---

## Ordering & Commits

- R1 / R2 / R3 / M1 / M2 are mutually independent.
- D1, D2 → D3 strictly ordered.
- One commit per batch (or per coherent unit within a batch); never a
  single mixed mega-commit. Commit immediately after each unit — no
  uncommitted state left in the worktree.

## Hard Gates for the Implementer

- Before touching each deletion point, re-verify it yourself: exhaustive
  `rg` with `--hidden` and without any `| head`/truncation. Any mismatch
  with this plan → stop and report on the task; when in doubt, delete less.
- Redirect verification output to a file and read the file; never chain
  multiple probes into one command and eyeball interleaved output.
- No behavior changes, no opportunistic refactors beyond this plan.
- Public-repo hygiene: no real personal data in commits or docs.
