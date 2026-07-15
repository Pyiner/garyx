# Legacy Boot Import Isolation

Status: v2 for re-review (v1 FAILed review in #TASK-2298; all four blockers
and the interlock/open-question rulings are incorporated below)
Scope: `garyx/src/runtime_assembler.rs`, `garyx-gateway/src/sqlite_thread_store.rs`,
`garyx-gateway/src/composition/app_bootstrap.rs` (ordering only), new module
`garyx-gateway/src/legacy_boot_import.rs`,
`garyx-router/src/thread_history/store.rs` (atomic rewrite),
`docs/agents/repository-contracts.md` (contract wording).

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
   startup cutovers (`recent_task_thread_kind_v1`,
   `endpoint_holder_dedup_v1`, …), exactly as today and as the contract
   requires. Imported records must not miss the cutovers.

## Non-goals

- No change to the import's data semantics (retired-workflow discard,
  preview seeding, task-body fallback, transcript backfill from `messages`).
- No new runtime file mode, no dual-write mirror (contract unchanged).
- No retroactive self-healing for machines where an older binary already
  recorded a false-success marker: those records cannot be safely
  re-imported automatically (SQLite may have evolved past the archive).
  Recovery remains the explicit manual flow — restore/keep the backup,
  clear the `projection_states` import row, reboot — now documented as
  such in the contract.

## Design

### Ordering contract (fixes review blocker 1)

```
construct SQLite store
  -> run_legacy_boot_import        (must fully succeed; Err aborts startup)
  -> SQL-native startup migrations (run_thread_data_startup_migrations)
  -> AppState build / serving
```

`assemble_sqlite_thread_store` becomes a pure constructor: the
`import_source` parameter and both the import call and the
`run_thread_data_startup_migrations` call are removed from it. The cutover
keeps its existing home in `AppStateBuilder::build`
(`app_bootstrap.rs:303`, panic-on-failure), which the assembler reaches
only after the import has succeeded — so the effective order is unchanged
from today. The existing ordering regression test
(`assembly_migrates_task_kind_only_after_boot_import`,
`sqlite_thread_store.rs:813`) is preserved and re-pointed at the new seam:
store → import → builder cutover.

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

Filesystem probing and the retirement mover go through a small internal
`ArchiveFs` seam (real implementation by default, recording fake in tests)
so "zero FS access in steady state" is provable by assertion, not by
absence of observable side effects.

### Two markers, one lifecycle (fixes review blocker 3)

- `thread_records_import` / v1 — the existing import marker. Name and
  version are frozen: renaming or re-versioning it would re-import stale
  archives over evolved SQLite truth on every migrated machine.
- `legacy_archive_retirement` / v1 — new. Records that the archive
  directories have been fully moved to backups.

```
run_legacy_boot_import(db, store, transcripts, data_dir):
  1. retirement marker check (SQL point read)
       Ok(true)  -> Complete. Return. Zero filesystem access.
       Err(e)    -> Err (abort startup)
       Ok(false) -> continue
  2. acquire per-data-dir exclusive lifecycle lock
       (flock on <data_dir>/legacy-boot-import.lock; see Concurrency)
       failure/busy -> Err (abort startup; concurrent boot must not race)
  3. re-check BOTH markers under the lock (double-check)
       retirement marker now present -> Complete (another process finished)
  4. import marker check
       Err(e)    -> Err (abort startup; NEVER "check failed -> import")
       Ok(true)  -> skip to step 7 (retirement-only path: this is every
                    already-migrated machine in the field today)
       Ok(false) -> continue
  5. archive probe (metadata on data_dir/threads, data_dir/sessions,
     via ArchiveFs; no directory creation)
       probe IO error      -> Err (abort startup)
       neither dir exists  -> write import marker; go to step 8
                              (fresh install: nothing to move either)
       else                -> continue
  6. open FileThreadStore::new(data_dir)
       Err(e) -> Err (abort startup; never substitute an empty source)
  7'. (from step 5/6) full import using FALLIBLE store methods:
       list_keys() Err          -> Err (abort startup)
       per key:
         get() Err              -> failed += 1
         get() Ok(None)         -> failed += 1   (listed key with no
                                   readable record is a failure, not a
                                   discard)
         retired workflow       -> transcript delete Err -> failed += 1
                                   else discarded += 1
         transcript backfill    -> uses the atomic path below; any Err
                                   -> failed += 1, write_record SKIPPED
                                   for this key (leaving the record
                                   unimported keeps the archive
                                   authoritative for it)
         write_record Err       -> failed += 1
       failed > 0  -> Err (abort startup; no marker; log full summary).
                      Next boot retries the whole pass — safe because
                      the gateway never served on the partial state, so
                      no SQLite record has evolved past the archive.
       failed == 0 -> write import marker
                      marker write Err -> Err (abort startup)
  7. retirement (import marker durably present; archive untouched by any
     primary path):
       for each src in [threads, sessions]:
         src absent                    -> done (already moved / never existed)
         src present, dest absent      -> fs::rename (atomic; same
                                          filesystem by construction, the
                                          destination is data-dir-local)
         src present, dest present     -> conflict: do NOT overwrite or
                                          merge; warn; retirement stays
                                          pending (manual resolution)
       all sources done -> continue; any pending -> warn and RETURN OK
       (retirement failure never blocks startup: SQLite is already the
       complete truth source and the marker gates re-import; the next
       boot lands in the retirement-only path and retries the move)
  8. write retirement marker
       Err -> warn, return Ok (retried next boot via retirement-only path)
```

Outcome (for logs/tests): `Complete | ImportedAndRetired(summary) |
ImportedRetirementPending(summary) | RetirementOnly{pending: bool} |
NothingToImport` — every `Err` aborts startup, and only steps ≥ 7 may
degrade to a warn.

`ThreadRecordImportSummary` splits today's `skipped` into `discarded`
(intentional retired-workflow drops) vs `failed`.

Crash windows: rename is atomic per directory; a crash between the two
renames leaves one source absent (done) and one present (retried). A crash
between import marker and retirement marker lands in the retirement-only
path. A crash before the import marker re-runs the import — safe by goal 3
(nothing served, nothing evolved).

### Fail closed on import errors (fixes review blocker 2)

Every failure in steps 1–7' returns `Err` from `run_legacy_boot_import`,
which `RuntimeAssembler::assemble` propagates — the gateway refuses to
start. Rationale (review's counterexample, adopted): if the gateway served
after a partial import, imported records would evolve in SQLite; the next
boot's full re-import would overwrite the evolved `v2` records with stale
archive `v1` bodies. `write_record` idempotence makes *retries of an
unserved pass* safe; it does not make *overwriting evolved truth* safe.
Only retirement (steps 7–8) may stay pending across boots, because by then
the import marker is durable and SQLite is the complete truth source.

### Atomic transcript backfill (fixes review blocker 4)

Two changes in `garyx-router/src/thread_history/store.rs`:

1. `rewrite_from_messages_file` switches from truncating `fs::write` to
   write-temp-then-`rename` (temp file in the same directory,
   `.<name>.tmp` style, fsync before rename). Its only production caller
   is this boot import (verified by grep; the other references are tests
   and an unrelated metric label), so the blast radius is the import plus
   strictly-better durability for the shared helper.
2. The import stops using bare `exists()`. New fallible
   `ThreadTranscriptStore::has_valid_transcript(thread_id) ->
   Result<bool, ThreadHistoryError>`: propagates IO errors (does not fold
   them into `false`), returns `false` for an empty file or an unparsable
   first line (a torn artifact from a pre-fix binary), `true` only for a
   transcript whose session header parses. Backfill runs when it returns
   `false`; `Err` fails the key.

Result: a torn transcript from any earlier crash is detected and
rewritten atomically; a crash mid-backfill leaves only a temp file, never
a truncated target that would suppress the retry.

### Concurrency (replaces the interlock; review's ruling adopted)

The v1 claim that the empty-source interlock existed *only* because of
error folding was wrong: it also guarded the "archive retired, marker
manually cleared" recovery scenario, and probe-then-construct has a TOCTOU
window (a concurrent boot could move the archive between probe and
`create_dir_all` re-creating empty dirs).

Replacement: the per-data-dir exclusive flock (step 2) plus marker
re-check under the lock (step 3). All archive filesystem access — probe,
`FileThreadStore` construction, retirement moves — happens strictly inside
the lock. A second concurrent boot blocks/aborts at step 2 rather than
racing. With errors no longer folded into emptiness and mutual exclusion
guaranteed, a genuinely absent archive is unambiguous, and writing the
import marker over a populated table in that case is a correct no-op — so
the old interlock is retired *after* these constraints hold, not as a
leap of faith. The "marker cleared while archive retired" recovery
scenario resolves in step 5 to `NothingToImport` unless the operator has
restored the backup first, which is exactly the documented recovery flow.

### Contract text

Update `docs/agents/repository-contracts.md`:

- retirement destination becomes `<data_dir>/backups/legacy-archive-v1/`
  (with `threads/` and `sessions/` inside). Data-dir-local keeps the move
  a same-filesystem atomic rename and stays correct for non-default
  `sessions.data_dir` configurations. For the default data dir this is
  `~/.garyx/data/backups/legacy-archive-v1/`.
- state that failed imports abort startup and are retried on the next
  boot, never marked complete;
- state that manual recovery = restore (move, not copy) the backup dirs
  back + clear the `thread_records_import` row + reboot, and that
  false-success markers written by older binaries are not self-healed.

## Blast radius

- `garyx/src/runtime_assembler.rs`: FileThreadStore block deleted; one
  fallible import call; `assemble` error path now covers import failure.
- `garyx-gateway/src/sqlite_thread_store.rs`: import machinery moves out;
  `assemble_sqlite_thread_store` loses `import_source` and both trailing
  calls (import, startup migrations). Call sites: production
  (`runtime_assembler.rs:88`) and the ordering test
  (`sqlite_thread_store.rs:850`) — both updated; `lib.rs` re-exports
  adjusted. No other callers (verified by grep).
- `garyx-gateway/src/legacy_boot_import.rs`: new, self-contained, owns the
  lock, markers, probe seam, import loop, retirement.
- `garyx-gateway/src/composition/app_bootstrap.rs`: unchanged behavior
  (cutover stays in `build()`); ordering now guaranteed by the assembler
  sequence instead of by `assemble_sqlite_thread_store` internally.
- `garyx-router/src/thread_history/store.rs`: atomic rewrite +
  `has_valid_transcript`.
- Cross-crate compile of the `garyx` crate is part of validation (the
  fast Rust tiers do not cover it).

## Test plan (deterministic, no UI; fault injection via the ArchiveFs seam,
a failing ThreadStore test double for the archive source, and a poisoned /
closed GaryxDbService for marker faults)

Lifecycle & gating:
1. Retirement marker present → `Complete`; recording ArchiveFs proves zero
   filesystem calls (assert on the seam, not on side-effect absence).
2. Fresh install (no archive dirs) → import marker written, retirement
   marker written, no dirs created; second run → `Complete`.
3. Import-marker check `Err` → startup aborts; nothing imported, no marker.
4. Import-marker write `Err` (after clean pass) → startup aborts; next run
   re-imports and succeeds.
5. `NothingToImport` path with marker write `Err` → startup aborts.

Import failures (each: startup aborts, no import marker, archive intact,
next run after repair completes fully — the original P0 asserted end to
end):
6. `FileThreadStore::new` failure (unreadable data dir).
7. `list_keys` `Err`.
8. Per-key `get` `Err` and `get` `Ok(None)` → both `failed`, never
   `discarded`.
9. Retired-workflow record whose transcript delete fails → `failed`;
   clean pass counts it as `discarded` and still writes the marker.
10. Transcript backfill `Err` → that key's record NOT written; archive
    `messages` still authoritative; retry lands transcript + record.
11. Torn transcript on disk (empty file / garbage first line) →
    `has_valid_transcript` = false → atomic rewrite; simulated crash
    mid-backfill leaves only a temp file, target absent → retried.
12. `has_valid_transcript` IO `Err` → key fails (not folded into false).
13. Backfill succeeds, `write_record` fails → `failed`, no marker; retry
    completes without duplicating transcript content (idempotent rewrite).
14. Nth `write_record` fails mid-batch → no marker; full retry overwrites
    the partial progress (asserted equal to a clean single-pass result).

Retirement:
15. Full success → both markers; `threads/`+`sessions/` under
    `<data_dir>/backups/legacy-archive-v1/`; re-boot → `Complete`, dirs
    not recreated.
16. Existing-machine shape: import marker present + archive in place →
    retirement-only; records in SQLite (evolved post-import) are NOT
    touched (assert bodies unchanged), archive moved.
17. First dir moves, second move fails → retirement pending, startup OK;
    next run moves only the remaining dir.
18. Destination conflict (dest already exists) → no overwrite/merge,
    pending, startup OK, both trees intact.
19. Retirement-marker write `Err` → warn, startup OK, retried next boot.

Concurrency & ordering:
20. Two concurrent `run_legacy_boot_import` calls on one data dir → lock
    serializes; exactly one imports; the second observes markers under the
    lock and does nothing.
21. Import-before-cutover ordering test preserved
    (`assembly_migrates_task_kind_only_after_boot_import`, re-pointed at
    the new assembler seam).
22. Recovery flow: restore backup dirs + clear import marker → full
    re-import succeeds (#TASK-1901 semantics preserved).

Validation: `cargo test -p garyx-gateway --lib`,
`cargo test -p garyx-router --lib`, `cargo test -p garyx` (cross-crate
assembler), then `scripts/test/rust_tier1_fast.sh --changed`.

## Decisions (formerly open questions; review rulings adopted)

1. **Startup on import failure: refuse to start.** All import-phase
   errors abort `assemble`. Only retirement may stay pending.
2. **Retirement destination: data-dir-local.**
   `<data_dir>/backups/legacy-archive-v1/{threads,sessions}`; same
   filesystem as the sources, atomic rename; contract text updated.
   Recovery restores by moving (not copying) the dirs back.
3. **Naming: `legacy_boot_import`.** Entry point `run_legacy_boot_import`.
   Import marker `thread_records_import`/v1 frozen forever; retirement
   marker `legacy_archive_retirement`/v1 added. No retroactive self-heal
   for pre-fix false-success markers (manual recovery only, documented).
