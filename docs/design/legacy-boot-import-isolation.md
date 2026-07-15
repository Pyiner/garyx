# Legacy Boot Import Isolation

Status: v3 for re-review (v1/v2 FAILed review in #TASK-2298; v3 fixes the
three v2 blockers — recovery-vs-marker contradiction, non-rerunning
downstream cutovers, header-only transcript acceptance — plus the archived
tombstone branch, the rewrite call-surface correction, the flock policy
contradiction, and the fault-injection seams)
Scope: `garyx/src/runtime_assembler.rs`, `garyx-gateway/src/sqlite_thread_store.rs`,
`garyx-gateway/src/garyx_db/mod.rs` (cutover gate + generation),
`garyx-gateway/src/composition/app_bootstrap.rs` (ordering only), new module
`garyx-gateway/src/legacy_boot_import.rs`,
`garyx-router/src/thread_history/store.rs` (atomic rewrite + import-specific
transcript validation), `docs/agents/repository-contracts.md`.

## Problem

The one-shot legacy archive import (#TASK-1864) is a patch woven into the
main assembly path, and its failure handling records failure as permanent
success.

Correctness defects (verified line by line, 2026-07-15):

1. `runtime_assembler.rs:54-60` — when `FileThreadStore::new` fails, the
   import source is silently replaced with an empty `InMemoryThreadStore`
   (warn only).
2. `import_thread_records_if_needed` uses `list_keys_logged` / `get_logged`
   (`garyx-router/src/store.rs:118,150`), which fold IO errors into an empty
   vec / `None`. The trait docs themselves say fallible paths should call the
   fallible methods; the import is exactly such a path.
3. The completion marker (`sqlite_thread_store.rs:494`) is written
   unconditionally: with `skipped > 0`, with transcript backfill failures,
   even with every single `write_record` failed.
4. Combined worst case: transcript backfill fails but `write_record`
   succeeds; `write_record` strips the legacy `messages` snapshot, so the
   conversation content becomes unreachable in the application truth source
   (transcripts), and the marker guarantees it is never retried.
5. The contract (`docs/agents/repository-contracts.md`) says the archive "is
   retired to `~/.garyx/backups/` after the switchover" — no retirement
   implementation exists. Migrated machines therefore commonly have the
   import marker present *and* the archive still in place.
6. Marker *check* failure (`sqlite_thread_store.rs:382`) currently chooses
   "importing", which on a transient DB error would flow the stale archive
   back over evolved SQL truth.
7. `rewrite_from_messages_file` writes the transcript with a plain
   truncating `tokio::fs::write` (`thread_history/store.rs:1459`) and
   `ThreadTranscriptStore::exists` is a bare `path.exists()`
   (`thread_history/store.rs:797`). A crash or write failure can leave an
   empty/partial jsonl that suppresses the backfill on the next pass while
   the SQL write strips `messages` — the P0 reproduces even with a
   marker-gated retry loop.

Structural defects:

- `RuntimeAssembler::assemble` constructs the legacy `FileThreadStore` on
  every boot, before anyone knows whether an import is even needed.
- `FileThreadStore::new` calls `create_dir_all` — every boot of every
  machine (including long-migrated ones and fresh installs) re-creates
  empty `data_dir/threads/` and `data_dir/sessions/` as a side effect of
  the main path.
- `assemble_sqlite_thread_store` takes a `legacy_import_source` parameter,
  mixing the migration concern into store assembly.
- Because errors are folded into emptiness, "archive is empty" and "archive
  is unreadable" are indistinguishable, which is why the safety interlock at
  `sqlite_thread_store.rs:391` had to exist at all.

## Goals

1. **Isolation**: the main assembly path knows nothing about the legacy
   archive. In steady state (lifecycle complete) boot performs one SQL
   point read and zero filesystem access for this concern.
2. **No false success**: typed errors propagate end to end; the import
   marker is written only after a fully successful import (zero failures).
3. **Fail closed**: any import-phase failure aborts startup. A gateway must
   not serve — and let SQLite records evolve — on top of a half-imported
   truth table, because the next boot's full re-import would overwrite
   evolved records with stale archive data. Only retirement (post-marker)
   is allowed to stay pending across boots.
4. **Contract fulfilled, including for existing machines**: the archive is
   retired (moved) into a backups directory after the switchover. Machines
   that already carry the import marker with the archive still in place get
   retirement-only treatment — never a re-import.
5. **Preserved ordering invariant**: boot import runs before the SQL-native
   startup cutovers, and a *forced re-import* re-runs the import-dependent
   cutovers (see Import generation) — imported records must never miss a
   cutover, on first import or on recovery.
6. **Recovery actually works**: the documented flow (restore backup dirs,
   clear the `thread_records_import` row, reboot) must produce a complete,
   cutover-correct state under this design — proven by an end-to-end test.

## Non-goals

- No change to the import's data semantics (retired-workflow discard,
  preview seeding, task-body fallback, transcript backfill from `messages`).
- No new runtime file mode, no dual-write mirror (contract unchanged).
- No retroactive self-healing for machines where an older binary already
  recorded a false-success marker: those records cannot be safely
  re-imported automatically (SQLite may have evolved past the archive).
  Recovery remains the explicit manual flow, now actually functional and
  documented in the contract.

## Design

### Ordering contract

```
construct SQLite store
  -> run_legacy_boot_import        (must fully succeed; Err aborts startup)
  -> SQL-native startup migrations (generation-gated; see Import generation)
  -> AppState build / serving
```

`assemble_sqlite_thread_store` becomes a pure constructor: the
`import_source` parameter and both the import call and the
`run_thread_data_startup_migrations` call are removed from it. The cutover
keeps its existing home in `AppStateBuilder::build`
(`app_bootstrap.rs:303`, panic-on-failure), which the assembler reaches
only after the import has succeeded — the effective order is unchanged
from today. The existing ordering regression test
(`assembly_migrates_task_kind_only_after_boot_import`,
`sqlite_thread_store.rs:813`) is preserved and re-pointed at the new seam.

`RuntimeAssembler::assemble`:

```rust
let sqlite_store = garyx_gateway::assemble_sqlite_thread_store(
    garyx_db.clone(), transcript_store.clone(), &bridge)?;
garyx_gateway::run_legacy_boot_import(
    &garyx_db, &sqlite_store, &transcript_store,
    Path::new(&session_data_dir),
).await?;                      // Err = abort startup (goal 3)
// ... builder.build() runs the SQL cutovers afterwards, as today
```

### Module boundary

New module `garyx-gateway/src/legacy_boot_import.rs` owning the entire
migration lifecycle. Everything import-related moves out of
`sqlite_thread_store.rs`: marker constants, `ThreadRecordImportSummary`,
`import_thread_records_if_needed`, and their tests. The assembler contains
zero archive knowledge beyond passing the data dir.

All filesystem touchpoints — archive probe, lifecycle-lock open/acquire,
retirement moves — go through a small internal `ArchiveFs` seam (real
implementation by default, recording fake in tests) so "zero FS access in
steady state" is provable by asserting on the seam, including the lock
path, not by absence of observable side effects.

### Markers and lifecycle (fixes v2 blocker 1)

- `thread_records_import` / v1 — existing import marker; name and version
  frozen forever (renaming/re-versioning would re-import stale archives
  over evolved SQLite truth fleet-wide).
- `legacy_archive_retirement` / v1 — new; records that the archive
  directories have been fully moved to backups.

The entry point reads **both markers in one SQL query** and dispatches on
the pair — there is no early return that consults only one of them:

| import | retirement | meaning | action |
|---|---|---|---|
| 1 | 1 | lifecycle complete | `Complete`; return; zero FS access |
| 1 | 0 | migrated, archive not yet retired (every existing machine today) | retirement-only: never re-import; move dirs; write retirement marker |
| 0 | 0 | first boot / fresh install | full import, then retirement |
| 0 | 1 | **recovery intent**: operator cleared the import row per the documented flow | under the lock, transactionally clear the retirement marker, then run the full import + retirement path |

The `(0,1)` row is what makes the documented recovery flow — "clear the
import row only" — actually reach the import instead of short-circuiting
as `Complete`. The operator contract stays one-row; the state machine owns
the retirement-marker reset.

```
run_legacy_boot_import(db, store, transcripts, data_dir):
  1. read both markers (one SQL query)
       Err(e)              -> Err (abort startup)
       (1,1)               -> Complete. Return. Zero filesystem access.
       else                -> continue
  2. acquire per-data-dir exclusive lifecycle lock
       (non-blocking try-flock on <data_dir>/legacy-boot-import.lock,
       opened via ArchiveFs)
       open/acquire Err or busy -> Err (abort startup: a concurrent boot
       on one data dir is a deployment fault; fail closed, do not race
       and do not wait unboundedly)
  3. re-read both markers under the lock (double-check)
       (1,1) now           -> Complete (another process finished first)
       (0,1)               -> transactionally delete retirement marker
                              (recovery intent), proceed as (0,0)
       (1,0)               -> skip to step 7 (retirement-only)
       (0,0)               -> continue
  4. archive probe (metadata on data_dir/threads, data_dir/sessions,
     via ArchiveFs; no directory creation)
       probe IO error      -> Err (abort startup)
       neither dir exists  -> write import marker (generation++; see
                              Import generation); go to step 8
       else                -> continue
  5. open FileThreadStore::new(data_dir)
       Err(e) -> Err (abort startup; never substitute an empty source)
  6. full import using FALLIBLE store methods:
       list_keys() Err          -> Err (abort startup)
       per key:
         get() Err              -> failed += 1
         get() Ok(None)         -> failed += 1  (listed key with no
                                   readable record is a failure)
         retired workflow       -> transcript delete Err -> failed += 1
                                   else discarded += 1
         transcript backfill    -> import-specific validation + atomic
                                   write (see Transcript backfill); Err
                                   -> failed += 1, write_record SKIPPED
                                   for this key
         write_record Err(Archived)
                                -> discarded += 1  (recovery case: the
                                   thread was archived after the original
                                   migration; the tombstone wins — the
                                   repo contract forbids resurrecting
                                   archived ids. Never counted as failed,
                                   or recovery could never complete.)
         write_record other Err -> failed += 1
       failed > 0  -> Err (abort startup; no marker; log full summary).
                      Retry next boot is safe: the gateway never served
                      on the partial state, so no SQLite record has
                      evolved past the archive.
       failed == 0 -> write import marker (generation++)
                      marker write Err -> Err (abort startup)
  7. retirement (import marker durably present; archive untouched by any
     primary path):
       for each src in [threads, sessions]:
         src absent                -> done (already moved / never existed)
         src present, dest absent  -> fs::rename (atomic; same filesystem
                                      by construction — destination is
                                      data-dir-local)
         src present, dest present -> conflict: do NOT overwrite or merge;
                                      warn; retirement stays pending
                                      (manual resolution)
       all done -> continue; any pending -> warn and return Ok
       (retirement failure never blocks startup: SQLite is already the
       complete truth source; the next boot lands in retirement-only and
       retries the move)
  8. write retirement marker
       Err -> warn, return Ok (retried next boot via retirement-only)
```

Outcome (for logs/tests): `Complete | ImportedAndRetired(summary) |
ImportedRetirementPending(summary) | RetirementOnly{pending: bool} |
NothingToImport` — every `Err` aborts startup; only steps ≥ 7 degrade to
warn.

`ThreadRecordImportSummary` splits today's `skipped` into `discarded`
(retired-workflow drops + archived tombstones) vs `failed`.

Crash windows: rename is atomic per directory; a crash between the two
renames leaves one source absent (done) and one present (retried). A crash
between import marker and retirement marker lands in retirement-only. A
crash before the import marker re-runs the import — safe by goal 3. A
crash after the recovery reset (step 3, `(0,1)`→`(0,0)`) but before the
import marker lands in `(0,0)` — plain full import, still correct.

### Import generation: re-import re-runs dependent cutovers (fixes v2 blocker 2)

Today every SQL-native cutover gates on "my projection_states row exists"
(`garyx_db/mod.rs:1187`) — correct for steady state, wrong after a forced
re-import: freshly re-imported legacy rows would permanently miss
`recent_task_thread_kind_v1`, `endpoint_holder_dedup_v1`, and any future
import-dependent cutover (the pinned test
`recent_task_thread_kind_migration_records_zero_and_never_reruns` proves
the skip).

Mechanism (generic, not a hardcoded marker list):

- The import marker row carries a monotonically increasing **import
  generation**, incremented on every successful import-marker write
  (first import, `NothingToImport`, and every recovery re-import).
- Import-dependent cutovers record, transactionally with their own
  completion marker, the import generation they ran against
  (`based_on_import_generation`).
- The shared cutover gate becomes: *skip iff my marker exists AND its
  `based_on_import_generation` equals the current import generation* —
  otherwise run (all these cutovers are idempotent single-transaction
  passes). New cutovers registered through the same helper inherit the
  semantics automatically; nothing in `legacy_boot_import.rs` enumerates
  cutover names.
- Backward compatibility for existing machines (import marker present, no
  generation recorded, cutover markers present without `based_on`): seed
  generation = 1 and treat a missing `based_on` as 1. Existing machines
  therefore see no re-runs — the pinned never-reruns behavior is preserved
  verbatim until a genuine recovery re-import advances the generation.
- Storage shape (companion `projection_states` row vs added column) is an
  implementation detail; the requirements are: read/write transactional
  with the markers they describe, and one source of truth for the current
  generation.

### Fail closed on import errors

Every failure in steps 1–6 returns `Err` from `run_legacy_boot_import`,
which `RuntimeAssembler::assemble` propagates — the gateway refuses to
start. Rationale (review's counterexample, adopted in v2): if the gateway
served after a partial import, imported records would evolve in SQLite;
the next boot's full re-import would overwrite the evolved records with
stale archive bodies. `write_record` idempotence makes *retries of an
unserved pass* safe; it does not make *overwriting evolved truth* safe.
Only retirement (steps 7–8) may stay pending across boots.

### Transcript backfill: import-specific validation + atomic write (fixes v2 blocker 3)

Two layers in `garyx-router/src/thread_history/store.rs`:

**1. Atomic rewrite (shared).** `rewrite_from_messages_file` switches from
truncating `fs::write` to write-temp-then-rename in the same directory,
with fsync on the file *and its parent directory* before reporting
success. Production callers: this boot import **and** the local provider
session import route (`routes.rs:797`) — v2's "single caller" claim was
wrong (grep output was truncated). Atomicity is a strict improvement for
both; the route's behavior is otherwise unchanged and gets a regression
test.

**2. Import-specific gate (new, import-only).**
`ensure_transcript_backfilled(thread_id, legacy_messages) ->
Result<BackfillOutcome, ThreadHistoryError>` replaces the bare `exists()`
check. Define `target` = the record sequence produced from
`legacy_messages` (reuse the existing `reconcile_rewrite_records`
conversion with empty `existing`). Rules:

| observed transcript state | action |
|---|---|
| file absent, or empty file | atomic full write of `target` |
| tail line torn (incomplete JSON / no trailing newline) AND the parsed prefix is a strict prefix of `target` | torn artifact from a pre-fix binary → atomic rewrite |
| any other parse failure | `Err` → key fails (never auto-overwrite possibly-evolved data) |
| header thread_id ≠ expected, or unsupported header version | `Err` → key fails |
| header-only (zero records) while `legacy_messages` is non-empty | atomic rewrite (a legally-created empty transcript cannot satisfy a non-empty archive) |
| records == `target` | no-op, done |
| records are a strict prefix of `target` | incomplete → atomic rewrite to `target` |
| diverged (neither equal nor prefix) | existing transcript wins — it can only have evolved at runtime, and transcripts are the content truth source; skip backfill, done (not a failure) |

Every rewrite goes through layer 1. A crash mid-backfill leaves only a
temp file — never a truncated target that would suppress the retry — and
validation runs on every pass, so no torn artifact survives a retry
un-repaired. `exists()` keeps its current semantics for other callers;
the import never calls it.

### Concurrency

Per-data-dir exclusive **non-blocking** try-flock (step 2), opened via the
`ArchiveFs` seam; busy or open failure → `Err`, abort startup. Two gateways
booting on one data dir is a deployment fault — fail closed is consistent
with the rest of the design, and avoids unbounded blocking behind a
long-running import. All archive filesystem access (probe, store
construction, retirement moves) happens strictly inside the lock, with
both markers re-checked under it — closing the probe/construct TOCTOU
noted in the v1 review. With errors never folded into emptiness and mutual
exclusion guaranteed, a genuinely absent archive is unambiguous, and
writing the import marker over a populated table in that case is a correct
no-op — the old empty-source interlock (`sqlite_thread_store.rs:391`) is
retired on those grounds, and its recovery scenario is now handled
explicitly by the `(0,1)` marker state instead of implicitly by a scan
guard.

### Fault-injection seams (makes the test plan honest)

- `GaryxDbService` gains an internal test-only lifecycle/fault seam so
  marker reads and writes can deterministically return errors (today there
  is no public way to poison it; "closed DB handle" is not a reproducible
  path). Shape: `#[cfg(any(test, feature = "test-seams"))]` injectable
  error hook on the projection-state read/write entry points.
- `ArchiveFs` covers probe, lock open/acquire, and rename, so lock
  failures and move failures are directly injectable and the zero-FS
  assertion covers the lock path.
- The archive source is a `dyn ThreadStore` test double for `list_keys` /
  `get` faults; transcript faults are injected through a store wrapper.

### Contract text

Update `docs/agents/repository-contracts.md`:

- retirement destination: `<data_dir>/backups/legacy-archive-v1/`
  (`threads/`, `sessions/` inside). Data-dir-local keeps the move a
  same-filesystem atomic rename and stays correct for non-default
  `sessions.data_dir`. For the default data dir:
  `~/.garyx/data/backups/legacy-archive-v1/`.
- failed imports abort startup and retry next boot; they are never marked
  complete.
- manual recovery = move (not copy) the backup dirs back + clear the
  `thread_records_import` row + reboot; the system resets the retirement
  marker itself and re-runs import-dependent cutovers via the import
  generation. Archived-thread tombstones win over restored archive
  records (no resurrection). False-success markers written by pre-fix
  binaries are not self-healed.

## Blast radius

- `garyx/src/runtime_assembler.rs`: FileThreadStore block deleted; one
  fallible import call; `assemble` error path covers import failure.
- `garyx-gateway/src/sqlite_thread_store.rs`: import machinery moves out;
  `assemble_sqlite_thread_store` loses `import_source` and both trailing
  calls. Call sites: production (`runtime_assembler.rs:88`) and the
  ordering test (`sqlite_thread_store.rs:850`) — both updated; `lib.rs`
  re-exports adjusted. No other callers (re-verified without output
  truncation).
- `garyx-gateway/src/legacy_boot_import.rs`: new, self-contained; owns
  lock, marker pair, generation bump, probe seam, import loop, retirement.
- `garyx-gateway/src/garyx_db/mod.rs`: shared generation-aware cutover
  gate; generation seed for existing machines; both existing cutovers
  moved onto the helper.
- `garyx-gateway/src/composition/app_bootstrap.rs`: unchanged behavior;
  ordering guaranteed by the assembler sequence.
- `garyx-gateway/src/routes.rs`: no code change; local session import
  route inherits atomic rewrite (regression test added).
- `garyx-router/src/thread_history/store.rs`: atomic rewrite +
  `ensure_transcript_backfilled`.
- Cross-crate compile and tests of the `garyx` crate are part of
  validation (fast Rust tiers do not cover it).

## Test plan (deterministic, no UI; faults injected via the GaryxDbService
seam, ArchiveFs seam, ThreadStore double, transcript wrapper)

Lifecycle & gating:
1. Both markers present → `Complete`; recording ArchiveFs proves zero FS
   calls including no lock open (assert on the seam).
2. Fresh install → both markers written, generation = 1, no dirs created;
   second run → `Complete`.
3. Marker-pair read `Err` (initial and under-lock re-read, separately) →
   abort; nothing imported, no marker.
4. Import-marker write `Err` after a clean pass → abort; next run
   re-imports and succeeds.
5. `NothingToImport` with marker write `Err` → abort.
6. Retirement-marker check/write `Err` → warn, startup OK, retried next
   boot.
7. Lock open `Err` and lock busy → abort, no archive FS access performed.

Import failures (each: abort, no import marker, archive intact; next run
after repair completes fully — the original P0 end to end):
8. `FileThreadStore::new` failure.
9. `list_keys` `Err`.
10. Per-key `get` `Err` and `get` `Ok(None)` → both `failed`, never
    `discarded`.
11. Retired-workflow transcript delete `Err` → `failed`; clean pass counts
    `discarded` and still writes the marker.
12. Backfill `Err` → record NOT written; archive `messages` still
    authoritative; retry lands transcript + record.
13. Nth `write_record` fails mid-batch → no marker; full retry equals a
    clean single-pass result.
14. `write_record` → `Archived` → `discarded`, marker still written on an
    otherwise-clean pass; tombstone body unchanged.

Transcript validation (import-specific gate):
15. Absent file / empty file → written atomically.
16. Header-only + non-empty legacy → rewritten.
17. Torn tail line over a strict prefix → rewritten; simulated crash
    mid-backfill leaves only a temp file → retried cleanly.
18. Valid header + garbage later line (non-torn parse failure) → key
    fails; nothing overwritten.
19. Wrong header thread_id → key fails.
20. Strict-prefix records → completed to `target`.
21. Diverged records → existing transcript preserved byte-identical;
    backfill skipped; key succeeds.
22. Local provider session import route still works on the atomic path
    (route-level regression).

Retirement:
23. Full success → both markers; dirs under
    `<data_dir>/backups/legacy-archive-v1/`; re-boot → `Complete`, dirs
    not recreated.
24. Existing-machine shape (import=1, retirement=0, archive in place) →
    retirement-only; evolved SQLite bodies untouched (asserted); archive
    moved; no cutover re-runs (generation unchanged).
25. First dir moves, second fails → pending, startup OK; next run moves
    only the remainder.
26. Destination conflict → no overwrite/merge, pending, startup OK, both
    trees intact.

Recovery (end to end, the v2 blocker-1/2 scenario):
27. Migrate → serve → archive a thread (tombstone) → evolve a task record
    → restore backup dirs + clear import row only → reboot:
    - state machine takes the `(0,1)` recovery path, resets retirement
      marker, re-imports;
    - re-imported legacy task regains `thread_kind=task` in record AND
      `recent_threads` projection (cutover re-ran: generation advanced);
    - endpoint dedup re-ran; canonical bodies and projections agree;
    - archived thread stays archived (`discarded`);
    - both markers restored; archive re-retired.
28. Generation seed compatibility: import marker present without a
    generation row + cutover markers without `based_on` → seeded as
    generation 1, no cutover re-runs (pinned never-reruns test preserved).

Concurrency & ordering:
29. Concurrent second boot on one data dir → lock busy → abort; after the
    first completes, a retry sees `Complete`.
30. Import-before-cutover ordering test preserved
    (`assembly_migrates_task_kind_only_after_boot_import`, re-pointed).

Validation: `cargo test -p garyx-gateway --lib`,
`cargo test -p garyx-router --lib`, `cargo test -p garyx`, then
`scripts/test/rust_tier1_fast.sh --changed`.

## Decisions

1. **Startup on import failure: refuse to start.** All import-phase errors
   abort `assemble`. Only retirement may stay pending.
2. **Retirement destination: data-dir-local.**
   `<data_dir>/backups/legacy-archive-v1/{threads,sessions}`; recovery
   restores by moving (not copying) the dirs back.
3. **Naming: `legacy_boot_import`.** Entry `run_legacy_boot_import`.
   Import marker `thread_records_import`/v1 frozen; retirement marker
   `legacy_archive_retirement`/v1; monotonic import generation drives
   dependent-cutover re-runs. No retroactive self-heal for pre-fix
   false-success markers.
4. **Recovery contract stays one-row** (clear the import row); the state
   machine owns the retirement-marker reset via the `(0,1)` recovery
   state.
5. **Lock policy: non-blocking.** Busy → abort startup.
6. **Archived tombstones win** over restored archive records
   (`discarded`, never `failed`).
7. **Diverged transcripts win** over archive `messages` (transcripts are
   the content truth source; divergence implies runtime evolution).
