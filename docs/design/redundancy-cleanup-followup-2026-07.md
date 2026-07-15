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
| 2 | `FileThreadStore` write half (set/delete/update/clear/size/lock/atomic-write) dead in production | PARTIAL — set/delete/update pinned by `ThreadStore` trait; write-half removal sequenced after legacy boot-import isolation v5 | R2 |
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
- `garyx-core/src/lib.rs` module declarations AND re-exports for the three
  modules (the `mod` items plus the `pub use` block, lines 1–46 region)
- Orphan fixtures: `tests/fixtures/keys_fixtures.json`,
  `tests/fixtures/label_fixtures.json`,
  `tests/fixtures/route_resolver_fixtures.json`, and the whole
  `tests/fixtures/generate_fixtures.py` (it only serves these modules).
  `tests/fixtures/stateful_routing_fixtures.json` showed zero content
  references in pre-review probes — re-verify exhaustively during
  implementation; delete only if confirmed orphan.
- Now-unused `garyx-core/Cargo.toml` dependencies after the module removal:
  `regex`, `tracing`, `thiserror`, `once_cell` (verified:
  `slash_commands.rs` and `lib.rs` use none of them; keep `garyx-models`
  and `serde_json`).

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

## Batch R2 — `FileThreadStore` dead write half (two phases; phase 2 blocked by legacy v5)

Verified constraint: `set`/`delete`/`update` are required `ThreadStore`
trait methods with no default bodies, so they cannot be deleted while
`FileThreadStore` implements `ThreadStore`. The canonical narrowing is
already specified by `docs/design/legacy-boot-import-isolation.md` (v5):
`assemble_sqlite_thread_store` becomes a pure constructor (loses its
`import_source` parameter) and the import moves to a dedicated
`legacy_boot_import.rs` seam with an injectable source double for fallible
`list_keys`/`get` coverage (v5 doc :116, :461). That refactor is designed
but NOT yet implemented (no `legacy_boot_import.rs` in tree); inventing a
competing narrowing here would conflict with it.

Phase 1 (this cleanup round — safe now):
- Delete `clear()` and `size()` from `garyx-router/src/file_store.rs`,
  removing only `test_clear` and the specific `size()` assertions in
  `file_store/tests.rs`.
- `with_options()` is NOT deletable in phase 1: it is the configuration
  entry for four live behavior tests
  (`file_store/tests.rs:165/200/274/320` — two write-lock tests,
  cache-TTL, lock-free read). Demote it to a `#[cfg(test)]` constructor
  (same-crate tests only) or a test-module helper instead of deleting.
- Phase 2 deletes the two write-lock tests together with the write half;
  the cache-TTL and lock-free-read tests stay, on disk-seeded data plus
  the test constructor.

Phase 2 (sequenced AFTER the legacy v5 implementation lands; execute as
its final step, not as an independent competing refactor):
- Read-only seam per v5: `pub(crate) LegacyArchiveReader` exposing only
  fallible `list_keys` + `get`, injectable double included. No
  "concrete type" shortcut. The full archive walk stays legal ONLY inside
  boot import (repository contract; no `list_keys`-scan condition queries
  anywhere else, no runtime JSON backend resurrection).
- Then remove the ENTIRE `impl ThreadStore for FileThreadStore`
  (`file_store.rs:333` region) — `set`/`delete`/`exists`/`update` are all
  required trait methods with no default bodies (`store.rs:27`), so a
  partial deletion cannot compile. `exists` is deleted with the impl
  (zero production callers). `get`/`list_keys` become inherent (or
  narrow-interface) methods consumed by the gateway's
  `pub(crate) LegacyArchiveReader` read-only adapter. Also delete the lock
  machinery (`acquire_lock`, `release_lock`, `check_stale_lock`,
  `lock_file_for_path`, `resolve_write_lock_thread_file`) and
  `atomic_write`.
- Update `docs/design/legacy-boot-import-isolation.md` in the same change:
  its test-seam section (:461 region) still says `dyn ThreadStore` —
  align it with the `LegacyArchiveReader` seam so the two design documents
  do not conflict.

Keep (production boot import uses list+get only — verified at
`sqlite_thread_store.rs:366/408`):
- `get`, `list_keys` and their helpers: `resolve_thread_file`,
  `legacy_thread_file`, `legacy_compat_thread_file`, `thread_file`,
  `decode_key`, `stem_to_key`, `file_mtime`, `evict_if_needed`,
  `deep_clone`, `storage_roots`, `CacheEntry`/cache,
  `encode_thread_storage_key`, `thread_storage_file_name`.

Test migration (phase 2; complete consumer list from design review):
- Gateway full read/write `ThreadStore` contract case for `FileThreadStore`
  (`sqlite_thread_store.rs:635` region) — delete that case; add a
  disk-JSON-driven read-only archive-reader contract in its place.
- Gateway import tests seed the legacy source through `ThreadStore::set`
  in MULTIPLE places, not one (multiline call syntax — sweep by method
  name `.set(`, never by receiver): `cleared_import_state_forces_a_reimport`
  (`sqlite_thread_store.rs:659`, seeds at :669/:682 — keep its forced
  re-import coverage), `boot_import_migrates_the_archive_once` (:711,
  seeds at :723/:740/:759 — keep single-import, transcript backfill and
  projection assertions), the ordering test (seed at :847), plus any other
  cfg(test) `.set(` seeding that targets the legacy `FileThreadStore`
  source — classify every `.set(` receiver in the module before migrating.
  Migration split: the real archive-reader/import contract and the `garyx`
  startup integration test seed by writing legacy JSON layout files on
  disk; import-logic, ordering, and fault-injection tests use the
  injectable `LegacyArchiveReader` fake. Do not keep a pub write path just
  for tests.
- `garyx/src/main_tests.rs:2578` (`startup_runtime_assembles...` calls
  `FileThreadStore.set`) — seed by writing legacy JSON files directly;
  keep its startup-import assertions.
- Gateway ordering regression test
  (`assembly_migrates_task_kind_only_after_boot_import`,
  `sqlite_thread_store.rs:813`) — keep, re-pointed at the new seam.
- `garyx-router/src/file_store/tests.rs`: write-path tests deleted with
  the dead code; read-path tests (list/get/eviction/key codec) stay and
  switch to on-disk seeding.

Contract check (verified): boot import (list+get) and backup-restore
(archived JSON on disk + forced re-import) are both read-only over the
archive; deleting the write half breaks neither.

Validation: `cargo test -p garyx-router --all-targets`; `cargo test -p
garyx-gateway --all-targets`; `cargo test -p garyx` (the tier1 fast script
does not cover the `garyx` crate); `scripts/test/rust_tier1_fast.sh
--changed`.

## Batch R3 — Delete `ProviderHealth` dead-write cluster

Delete:
- `garyx-bridge/src/multi_provider/topology.rs:85-119`: both getters
  (`get_provider_health`, `get_all_provider_health` — zero callers
  repo-wide) and `record_health_success` / `record_health_failure`
- `garyx-bridge/src/multi_provider/run_management.rs` call sites at
  906/913/1082/1539/1546/1629
- `garyx-bridge/src/multi_provider/state.rs:86` `provider_health` field
- `garyx-bridge/src/multi_provider/lifecycle.rs:230-232` retain block
- `garyx-bridge/src/provider_trait.rs`: `ProviderHealth` struct + impl
  (`record_success`/`record_failure`, :222-) AND the bridge-local
  `HealthStatus` enum (:211) — once the cluster goes it has no consumer
  outside the cluster and its tests. Remove both from the `lib.rs:14`
  re-export, delete `provider_trait/tests.rs:34-105`, and drop imports
  orphaned by the deletion (`Instant`, `HashMap`, …). The gateway's own
  `HealthStatus` (`garyx-gateway/src/health.rs`) is a different type —
  untouched.

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
- Oracle gap fix (design-review finding): the runtime checkpoint also
  compares `selectedRecentFilter` and `recentFeedPresentation`
  (`HomeProjectionActor.swift:215` region) while the reducer oracle only
  compares sections/isLoading/isHomeVisible. Extend
  `assertCheckpointParity` to compare those two fields and add oracle
  cases exercising a `.nonTask` recent filter and a non-default feed
  presentation transition, so deleting runtime parity does not drop that
  equivalence coverage.

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
- `threadLogs` (:422), `saveSkillFile` (:610), `createSkillEntry` (:617),
  `deleteSkillEntry` (:624), `automationActivity` (:660),
  `deleteWorkspace` (:722), `startChannelAuthFlow` (:854),
  `pollChannelAuthFlow` (:864)
  (anchors re-verified 2026-07-15 after the pin-reorder API landed; line
  numbers drift with parallel work — implementer re-locates by signature,
  not line)

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
- `workspace-git-runtime.ts`: delete the WHOLE file — all four exports
  (`getWorkspaceGitDetailsForPath` :79, `getWorkspaceGitDetails` :125,
  `commitWorkspaceChanges` :132, `pushWorkspaceBranch` :157) belong
  exclusively to these dead chains (verified; `getWorkspaceGitDetailsForPath`
  is only called by the other three). The live `getWorkspaceGitStatus`
  consumed by the renderer is a separate implementation in
  `garyx-client/workspaces.ts:300` — do not touch it.
- Everywhere else delete functions, not files: `tasks.ts`, `catalog.ts`,
  `gateway.ts`, `terminal-runtime.ts`, `browser-runtime.ts` have consumed
  sibling exports.
- Go one layer deeper than the wrappers: `TerminalRuntime.activateSession`
  (`terminal-runtime.ts:142`) has no caller left once the module-level
  wrapper (:235-239) goes — delete the method too. Exported types orphaned
  by the chains (`GatewayProbeResult`, the workspace git
  detail/file/result types, and the listed `*Input` types) must be
  rg-verified and deleted: the D3 unused gate does NOT report exported
  orphans, so D1 must prove zero references itself.
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

Validation (mandatory — plain `tsc` does not cover `.design-sync`; the
tsconfig includes only `src/**`):
- `npx --no-install esbuild .design-sync/entry.tsx --bundle
  --platform=browser --outfile=/dev/null --tsconfig=tsconfig.json` must
  succeed after the deletions;
- `rg --hidden` proof of zero remaining exact references to the two
  components across `.design-sync/` (entry/config/previews/conventions)
  and `ds-bundle/`;
- `npx tsc --noEmit` + `npm run test:unit`.

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

- R1 / R2 phase 1 / R3 / M1 / M2 are mutually independent.
- R2 phase 2 is blocked by the legacy boot-import isolation v5
  implementation and ships as that work's final step — out of this round
  unless v5 lands within it.
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
