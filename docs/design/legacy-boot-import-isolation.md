# Legacy Boot Import Isolation

Status: v5 for re-review (v1–v4 FAILed review in #TASK-2298; v5 fixes the
three v4 residuals: tombstone pre-check now deletes leftover transcripts,
recovery probe validates backup destinations against partial restores, and
the atomic-replace failure semantics are stated per stage)
Scope: `garyx/src/runtime_assembler.rs`, `garyx-gateway/src/sqlite_thread_store.rs`,
`garyx-gateway/src/garyx_db/mod.rs` (cutover gate + generation),
`garyx-gateway/src/composition/app_bootstrap.rs` (ordering only), new module
`garyx-gateway/src/legacy_boot_import.rs`,
`garyx-router/src/thread_history/store.rs` and `reconcile.rs` (atomic
replace + import-specific transcript validation),
`docs/agents/repository-contracts.md`.

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
   This includes recovery: a recovery boot that finds no restored archive
   is a failure, never `NothingToImport`.
3. **Fail closed**: any import-phase failure aborts startup. A gateway must
   not serve — and let SQLite records evolve — on top of a half-imported
   truth table, because the next boot's full re-import would overwrite
   evolved records with stale archive data. Only retirement (post-marker)
   is allowed to stay pending across boots.
4. **Contract fulfilled, including for existing machines**: the archive is
   retired (moved) into a backups directory after the switchover. Machines
   that already carry the import marker with the archive still in place get
   retirement-only treatment — never a re-import.
5. **Preserved ordering invariant, crash-safe on recovery**: boot import
   runs before the SQL-native startup cutovers, and a forced re-import
   re-runs the import-dependent cutovers via a generation that is owned
   outside the markers, survives every crash window, and only ever moves
   forward.
6. **Recovery actually works**: the documented flow (restore backup dirs,
   clear the `thread_records_import` row, reboot) must produce a complete,
   cutover-correct state under this design — proven by an end-to-end test —
   and must fail closed when the operator forgot to restore the archive.

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
implementation by default, recording fake in tests) so steady-state and
failure-path filesystem behavior is provable by asserting on the seam's
call log (which calls happened, in which order), not by absence of
observable side effects.

### Markers and lifecycle

- `thread_records_import` / v1 — existing import marker; name and version
  frozen forever (renaming/re-versioning would re-import stale archives
  over evolved SQLite truth fleet-wide).
- `legacy_archive_retirement` / v1 — new; records that the archive
  directories have been fully moved to backups.
- `legacy_import_generation` — new, **independent** persistent row (see
  Import generation). It is never deleted by recovery or by the state
  machine; it only increments, transactionally with import-marker writes.

The entry point reads **both markers in one SQL query** and dispatches on
the pair — there is no early return that consults only one of them:

| import | retirement | meaning | action |
|---|---|---|---|
| 1 | 1 | lifecycle complete | `Complete`; return; zero FS access |
| 1 | 0 | migrated, archive not yet retired (every existing machine today) | retirement-only: never re-import; move dirs; write retirement marker |
| 0 | 0 | first boot / fresh install | full import, then retirement |
| 0 | 1 | **recovery intent**: operator cleared the import row per the documented flow | full import from the restored archive; **incomplete restore = abort** (see below). The retirement marker is NOT touched up front: a successful import clears it in the same transaction that writes the import marker, so `(0,1) → (1,0)` is atomic and every failure or crash preserves the recovery-intent state |

The `(0,1)` state is sticky by construction: nothing is deleted on entry,
so a crash at any point before the import commits leaves `(0,1)` intact
and the next boot retries recovery. And because recovery *requires* a
fully restored archive, the probe validates **both sides**: at least one
source directory must exist AND no backup destination may still exist. An
operator who cleared the row but forgot the restore, restored only one of
the two directories, or copied instead of moved (leaving the destination
in place) gets an abort with an explicit message — never a
`NothingToImport` or partial-import false success. Partial restores are
real states, not operator exotica: a crash between the two retirement
renames followed by a one-directory restore produces exactly this shape.
This is the true replacement for the old empty-source interlock's
recovery scenario.

```
run_legacy_boot_import(db, store, transcripts, data_dir):
  1. read both markers (one SQL query)
       Err(e)              -> Err (abort startup)
       (1,1)               -> Complete. Return. Zero filesystem access.
       else                -> continue
  2. acquire per-data-dir exclusive lifecycle lock
       (non-blocking try-flock on <data_dir>/legacy-boot-import.lock,
       opened via ArchiveFs)
       open/acquire Err or busy -> Err (abort startup)
  3. re-read both markers under the lock (double-check)
       Err                 -> Err (abort startup)
       (1,1) now           -> Complete (another process finished first)
       (1,0)               -> skip to step 7 (retirement-only)
       (0,1)               -> recovery=true, continue
       (0,0)               -> recovery=false, continue
  4. archive probe (metadata on data_dir/threads, data_dir/sessions —
     and, when recovery=true, on both backup destinations under
     <data_dir>/backups/legacy-archive-v1/ — via ArchiveFs; no directory
     creation)
       probe IO error      -> Err (abort startup)
       recovery=true:
         any backup destination still exists
                           -> Err (abort startup: restore is incomplete —
                              only one directory moved back, or the
                              operator copied instead of moving. Importing
                              now would commit a partial archive as a
                              successful recovery and serve on it, while
                              retirement-only could never import the
                              missed directory later.)
         no source exists  -> Err (abort startup: "recovery intent but no
                              archive restored")
         else              -> continue (>=1 source present, all backup
                              destinations moved away)
       recovery=false:
         neither dir exists-> commit import marker + generation++ in one
                              transaction; go to step 8 (fresh install)
         else              -> continue
  5. open FileThreadStore::new(data_dir)
       Err(e) -> Err (abort startup; never substitute an empty source)
  6. full import using FALLIBLE store methods:
       list_keys() Err          -> Err (abort startup)
       per key:
         tombstone pre-check (SQL point read: is this id archived?)
              Err               -> Err (abort startup)
              archived          -> idempotent transcript_store.delete(key)
                                   (the product archive flow's transcript
                                   delete is best-effort with errors
                                   ignored — routes.rs:1175 — so an
                                   archived thread may still have a live
                                   transcript on disk, and a failed
                                   residual-branch cleanup from a previous
                                   pass leaves the same shape);
                                   delete Ok / not-found -> discarded += 1
                                   delete Err            -> failed += 1
                                   Either way SKIP backfill and
                                   write_record (repo contract forbids
                                   resurrecting archived ids; a leftover
                                   transcript must not survive behind a
                                   completion marker)
         get() Err              -> failed += 1
         get() Ok(None)         -> failed += 1
         retired workflow       -> transcript delete Err -> failed += 1
                                   else discarded += 1
         transcript backfill    -> import-specific validation + atomic
                                   replace (see Transcript backfill); Err
                                   -> failed += 1, write_record SKIPPED
         write_record Err(Archived)   [defensive residual branch]
                                -> delete the transcript this pass just
                                   backfilled for the key;
                                   delete Ok  -> discarded += 1
                                   delete Err -> failed += 1 (abort; the
                                   next pass's tombstone pre-check hits
                                   the same key and retries the delete,
                                   so the leftover cannot be sealed by a
                                   later completion marker)
         write_record other Err -> failed += 1
       failed > 0  -> Err (abort startup; no marker; log full summary)
       failed == 0 -> ONE transaction: write import marker AND
                      generation := generation + 1 AND (if recovery)
                      delete retirement marker
                      commit Err -> Err (abort startup)
  7. retirement (import marker durably present; archive untouched by any
     primary path):
       for each src in [threads, sessions]:
         src absent                -> done
         src present, dest absent  -> fs::rename (atomic; same filesystem
                                      by construction)
         src present, dest present -> conflict: do NOT overwrite or merge;
                                      warn; retirement stays pending
       all done -> continue; any pending -> warn and return Ok
  8. write retirement marker
       Err -> warn, return Ok (retried next boot via retirement-only)
```

Outcome (for logs/tests): `Complete | ImportedAndRetired(summary) |
ImportedRetirementPending(summary) | RetirementOnly{pending: bool} |
NothingToImport` — every `Err` aborts startup; only steps ≥ 7 degrade to
warn. `ThreadRecordImportSummary` splits today's `skipped` into
`discarded` (retired workflows + archived tombstones) vs `failed`.

Crash windows, exhaustively: rename is atomic per directory; a crash
between the two renames leaves one source absent (done) and one present
(retried). A crash between the import-commit transaction and the
retirement marker lands in retirement-only. A crash anywhere before the
import-commit transaction leaves the marker pair exactly as it was —
`(0,0)` re-imports, `(0,1)` retries recovery (nothing was deleted up
front) — and the generation has not moved, because it only moves inside
that same committed transaction. There is no window in which markers and
generation disagree.

### Import generation: re-import re-runs dependent cutovers, crash-safely

Today every SQL-native cutover gates on "my projection_states row exists"
(`garyx_db/mod.rs:1187`) — correct for steady state, wrong after a forced
re-import: freshly re-imported legacy rows would permanently miss
`recent_task_thread_kind_v1`, `endpoint_holder_dedup_v1`, and any future
import-dependent cutover.

Mechanism (generic, not a hardcoded marker list):

- **Generation owner**: a dedicated persistent `legacy_import_generation`
  row, separate from both markers. It is never deleted — not by the state
  machine, not by the documented recovery flow (which clears only the
  `thread_records_import` row). It increments by exactly 1 inside the same
  transaction that writes the import marker (step 6/step 4-fresh-install
  commit). Because deletion of the retirement marker also happens inside
  that transaction (recovery case), no crash window can observe a bumped
  generation without its import marker or vice versa.
- **Current generation** := the row's value; if the row is absent: 1 when
  the import marker exists (pre-v4 migrated machine — seeded lazily), else
  0 (fresh DB, in-memory DB, direct `AppStateBuilder` construction in
  tests). This makes the gate well-defined for every builder call that
  never runs `run_legacy_boot_import`.
- **Cutover gate** (shared helper; both existing cutovers move onto it,
  future ones inherit it): *skip iff my completion marker exists AND its
  recorded `based_on_import_generation` equals the current generation* —
  otherwise run (these cutovers are idempotent single-transaction passes).
  The cutover records `based_on_import_generation = current generation`
  transactionally with its own completion marker. A cutover marker without
  a recorded `based_on` (pre-v4) is treated as `based_on = 1`.
- **Compatibility**: existing migrated machines (import marker present, no
  generation row, cutover markers without `based_on`) resolve to
  generation 1 vs `based_on` 1 — no re-runs; the pinned
  `recent_task_thread_kind_migration_records_zero_and_never_reruns`
  behavior is preserved verbatim. Only a genuine recovery re-import
  advances the generation (1 → 2) and triggers exactly one re-run of each
  dependent cutover.

Worked recovery crash case (the v3 counterexample, closed): generation=1,
cutover `based_on=1`; operator clears the import row → `(0,1)`; boot
starts recovery and crashes anywhere before the import commit →
generation still 1, markers still `(0,1)`; next boot retries recovery;
the commit lands marker + generation=2 + retirement-marker clear
atomically; cutovers see `based_on=1 ≠ 2` and re-run.

### Fail closed on import errors

Every failure in steps 1–6 returns `Err` from `run_legacy_boot_import`,
which `RuntimeAssembler::assemble` propagates — the gateway refuses to
start. Rationale (v1-review counterexample, adopted): if the gateway
served after a partial import, imported records would evolve in SQLite;
the next boot's full re-import would overwrite the evolved records with
stale archive bodies. `write_record` idempotence makes *retries of an
unserved pass* safe; it does not make *overwriting evolved truth* safe.
Only retirement (steps 7–8) may stay pending across boots.

### Transcript backfill: identity-based validation + atomic replace

Reality checks that shape this section: legacy `messages` frequently carry
no timestamp, and the record conversion fills missing timestamps with
`Utc::now()` (`reconcile.rs:86`), while `ThreadTranscriptRecord` derives
`PartialEq` including `timestamp` (`model.rs:17`). Any rule built on
whole-record equality is therefore unstable across passes: a prefix
written by pass 1 can never `==`-match a target regenerated in pass 2.
Comparison must use logical message identity — the existing
`message_identity` helper (`reconcile.rs:452`) that the reconcile path
already uses for exactly this reason.

Three layers in `garyx-router`:

**1. Atomic replace primitive (new, low-level).**
`replace_transcript_atomic(thread_id, records)`: serialize header +
records, write to a temp file in the same directory, fsync the file,
rename over the target, fsync the parent directory, then report success.
It does **not** parse the existing target file — the current
`rewrite_from_messages_file` re-reads the target first
(`store.rs:1407`) and therefore cannot repair a torn file. Each stage
failure is a typed error with stage-specific postconditions:

- temp write / file fsync / rename failure → the old target is untouched;
- parent-fsync failure → `Err`, but the rename has already happened: in
  the current process the target IS the complete new file (only its
  directory-entry durability across power loss is unproven). No marker is
  written this pass; the retry validates the target and no-ops or
  continues — it must not treat this shape as an error.
- Observable on-disk states at any crash/failure point: old target,
  complete new target, or a stray temp file — never a partial target.
- On any stage failure the store's in-memory cache for the thread is
  invalidated; on success the same store instance immediately serves the
  new content.

**2. Shared rewrite path.** `rewrite_from_messages_file` keeps its
read-reconcile-write semantics for well-formed files but delegates its
write to the atomic primitive. Production callers: this boot import's
full-write cases and the local provider session import route
(`routes.rs:797`) — atomicity is a strict improvement for both; the
route's behavior is otherwise unchanged and gets a regression test.

**3. Import-specific gate (import-only).**
`ensure_transcript_backfilled(thread_id, legacy_messages) ->
Result<BackfillOutcome, ThreadHistoryError>` replaces the bare `exists()`
check.

Structural validation first (independent of content comparison — a
structurally broken file is never classified as "diverged"):

- exactly one session header, first line, supported version, matching
  `thread_id`;
- every record line parses, carries the expected `thread_id`, and `seq`
  is strictly increasing;
- a torn **tail** (final line incomplete JSON / missing trailing newline)
  is recoverable *iff* the parsed prefix passes the checks above AND
  identity-matches a prefix of the legacy messages (see below); any other
  structural damage → `Err` → key fails (never auto-overwrite
  possibly-evolved data).

Content comparison on `message_identity` sequences (not record equality):
let `E` = identities of parsed existing records, `L` = identities of
`legacy_messages`.

| state | action |
|---|---|
| file absent / empty file | atomic write of the full legacy conversion |
| `E == L` | no-op, done (timestamps/seq of the existing file are preserved — no rewrite) |
| `E` strict prefix of `L` (incl. header-only `E = []` with non-empty `L`, and the recoverable torn-tail case) | complete: keep existing records verbatim (their `seq`/`run_id`/`timestamp` untouched), convert and append only the missing tail, atomic replace |
| `E` diverged from `L` (neither equal nor prefix) | existing transcript wins — it can only have evolved at runtime, and transcripts are the content truth source; skip backfill, done |

A crash mid-backfill leaves only a temp file — never a truncated target —
and validation runs on every pass, so torn artifacts from pre-fix binaries
are repaired, prefixes are completed idempotently across passes regardless
of generated timestamps, and structural corruption fails loudly instead of
being silently overwritten or misread as divergence. `exists()` keeps its
current semantics for other callers; the import never calls it.

### Concurrency

Per-data-dir exclusive **non-blocking** try-flock (step 2), opened via the
`ArchiveFs` seam; busy or open failure → `Err`, abort startup. Two gateways
booting on one data dir is a deployment fault — fail closed is consistent
with the rest of the design, and avoids unbounded blocking behind a
long-running import. All archive filesystem access (probe, store
construction, retirement moves) happens strictly inside the lock, with
both markers re-checked under it — closing the probe/construct TOCTOU
noted in the v1 review. The old empty-source interlock
(`sqlite_thread_store.rs:391`) is retired because each of its jobs now has
an explicit owner: unreadable-archive ambiguity → typed probe/open errors;
concurrent-boot races → the lock; the recovery scenario → the sticky
`(0,1)` state that aborts on a missing archive.

### Fault-injection seams (makes the test plan honest)

- `GaryxDbService` gains an internal test-only fault seam
  (`#[cfg(any(test, feature = "test-seams"))]` injectable error hook on
  the projection-state read/write entry points) so marker-pair reads,
  marker/generation commits, and tombstone pre-checks can
  deterministically fail — today there is no public way to poison the
  service.
- `ArchiveFs` covers probe, lock open/acquire, and rename; its recording
  fake logs call order so tests can assert e.g. "lock open happened, then
  nothing" rather than just "no FS access".
- The archive source is a `dyn ThreadStore` test double for `list_keys` /
  `get` faults; transcript faults are injected through a store wrapper
  that can fail each atomic-replace stage (temp write, file fsync, rename,
  parent fsync) independently.

### Contract text

Update `docs/agents/repository-contracts.md`:

- retirement destination: `<data_dir>/backups/legacy-archive-v1/`
  (`threads/`, `sessions/` inside). Data-dir-local keeps the move a
  same-filesystem atomic rename and stays correct for non-default
  `sessions.data_dir`. For the default data dir:
  `~/.garyx/data/backups/legacy-archive-v1/`.
- failed imports abort startup and retry next boot; they are never marked
  complete. Recovery with a missing restored archive aborts.
- manual recovery = move (not copy) the backup dirs back + clear the
  `thread_records_import` row + reboot; the system completes
  `(0,1) → (1,0)` atomically and re-runs import-dependent cutovers via
  the import generation. Archived-thread tombstones win over restored
  archive records (no resurrection — record, projection, or transcript).
  False-success markers written by pre-fix binaries are not self-healed.

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
  lock, marker pair, generation commit, tombstone pre-check, probe seam,
  import loop, retirement.
- `garyx-gateway/src/garyx_db/mod.rs`: generation row + lazy seed;
  generation-aware shared cutover gate; both existing cutovers moved onto
  it; single transaction for marker+generation(+retirement-clear).
- `garyx-gateway/src/composition/app_bootstrap.rs`: unchanged behavior;
  generation 0 semantics for builder-only construction documented.
- `garyx-gateway/src/routes.rs`: no code change; local session import
  route inherits atomic replace (regression test added).
- `garyx-router/src/thread_history/{store,reconcile}.rs`: atomic replace
  primitive, rewrite delegation, `ensure_transcript_backfilled`,
  identity-sequence comparison (reusing `message_identity`).
- Cross-crate compile and tests of the `garyx` crate are part of
  validation (fast Rust tiers do not cover it).

## Test plan (deterministic, no UI; faults injected via the GaryxDbService
seam, ArchiveFs seam, ThreadStore double, transcript wrapper)

Lifecycle & gating:
1. Both markers present → `Complete`; ArchiveFs call log empty (no lock
   open, no probe).
2. Fresh install → import committed (marker + generation=1), retirement
   marker written, no dirs created; second run → `Complete`.
3. Marker-pair read `Err` (initial and under-lock re-read, separately) →
   abort.
4. Import-commit transaction `Err` (marker+generation) → abort; next run
   re-imports; generation advanced exactly once overall.
5. Fresh-install commit `Err` → abort; retry completes.
6. Retirement-marker write `Err` → warn, startup OK, retried next boot
   (pair read errors always abort — there is no separately degradable
   retirement *check*).
7. Lock open `Err` / lock busy → abort; call log shows lock open then
   nothing (no probe, no store construction, no moves).
8. Archive probe metadata `Err` → abort.

Import failures (each: abort, no marker movement, generation unchanged,
archive intact; next run after repair completes fully — the original P0
end to end):
9. `FileThreadStore::new` failure.
10. `list_keys` `Err`.
11. Per-key `get` `Err` and `get` `Ok(None)` → both `failed`.
12. Retired-workflow transcript delete `Err` → `failed`; clean pass counts
    `discarded` and still commits.
13. Backfill `Err` → record NOT written; archive `messages` still
    authoritative; retry lands transcript + record.
14. Nth `write_record` fails mid-batch → no marker; full retry equals a
    clean single-pass result.

Tombstones:
15. Tombstone pre-check hit with a pre-existing orphan transcript on disk
    (the best-effort product-archive delete failed) → orphan is deleted,
    `discarded`; record, projection, and transcript all absent after
    commit. Pre-check hit with no transcript → not-found, `discarded`.
16. Tombstone pre-check `Err` → abort; pre-check transcript delete `Err`
    → `failed`, abort.
17. Residual `write_record(Archived)` branch, chained across passes:
    pass 1 residual cleanup delete `Err` → `failed`, abort; pass 2
    tombstone pre-check hits the key and retries the delete → success →
    `discarded`, commit; end state has no resurrected transcript
    anywhere in the chain.

Transcript validation (import-specific gate; all legacy fixtures WITHOUT
timestamps, all idempotency assertions across two passes):
18. Absent / empty file → written atomically; second pass → no-op.
19. Header-only + non-empty legacy → completed; second pass → no-op.
20. Torn tail over an identity-prefix → repaired; crash mid-backfill
    leaves only a temp file → retried cleanly.
21. Valid header + garbage middle line, wrong-thread_id record line,
    non-monotonic seq, duplicate header → each fails the key; nothing
    overwritten.
22. Wrong header thread_id / unsupported version → key fails.
23. Identity strict prefix → completed to full conversation; existing
    records' seq/run_id/timestamp byte-preserved; second pass → no-op
    (timestamp instability must not re-classify as diverged).
24. Diverged identities → existing transcript preserved byte-identical;
    backfill skipped; key succeeds.
25. Atomic-replace stage failures with stage-specific postconditions:
    temp write / file fsync / rename `Err` → old target byte-identical;
    parent-fsync `Err` → target is the complete new file, no marker,
    retry validates and no-ops; every failure path → cache invalidated
    (next read re-reads disk); success → same store instance immediately
    serves the new content; no observable state is ever a partial target.
26. Local provider session import route regression on the atomic path.

Retirement:
27. Full success → both markers; dirs under
    `<data_dir>/backups/legacy-archive-v1/`; re-boot → `Complete`, dirs
    not recreated.
28. Existing-machine shape (import=1, retirement=0, archive in place) →
    retirement-only; evolved SQLite bodies untouched; archive moved; no
    cutover re-runs (generation unchanged).
29. First dir moves, second fails → pending, startup OK; next run moves
    only the remainder.
30. Destination conflict → no overwrite/merge, pending, startup OK, both
    trees intact.

Recovery (end to end):
31. Full pre-v4 upgrade chain: machine with pre-v4 markers (no generation
    row, cutover markers without `based_on`) boots v4 → lazy seed,
    generation=1, zero re-runs (pinned never-reruns preserved) → then
    operator restores backup + clears import row → recovery re-import →
    generation=2; re-imported legacy task regains `thread_kind=task` in
    record AND projection; endpoint dedup re-ran; archived thread stays
    dead (record+projection+transcript); `(0,1) → (1,0)` atomic; archive
    re-retired.
32. Recovery with missing archive (operator forgot to restore) → abort
    with the explicit recovery message; marker pair still `(0,1)`;
    generation unchanged; SQLite untouched.
33. Partial-restore aborts, three shapes, each asserting `(0,1)`
    preserved, generation unchanged, SQLite untouched, and no directory
    creation: only `threads/` restored (its backup destination gone,
    `sessions/` destination still present); only `sessions/` restored;
    copy-instead-of-move (source AND destination both present).
34. Crash after recovery entry but before import commit → state still
    `(0,1)`; retry works (nothing was deleted up front).
35. Lazy generation-seed write `Err` → abort; no marker movement.

Concurrency & ordering:
36. Concurrent second boot on one data dir → lock busy → abort; after the
    first completes, a retry sees `Complete`.
37. Import-before-cutover ordering test preserved
    (`assembly_migrates_task_kind_only_after_boot_import`, re-pointed).
38. Builder-only construction (in-memory DB, no import run) → current
    generation 0; cutovers run once and gate correctly.

Validation: `cargo test -p garyx-gateway --all-targets`,
`cargo test -p garyx-router --all-targets`, `cargo test -p garyx`, then
`scripts/test/rust_tier1_fast.sh --changed`.

## Decisions

1. **Startup on import failure: refuse to start.** All import-phase errors
   abort `assemble`. Only retirement may stay pending.
2. **Retirement destination: data-dir-local.**
   `<data_dir>/backups/legacy-archive-v1/{threads,sessions}`; recovery
   restores by moving (not copying) the dirs back.
3. **Naming: `legacy_boot_import`.** Entry `run_legacy_boot_import`.
   Import marker `thread_records_import`/v1 frozen; retirement marker
   `legacy_archive_retirement`/v1; independent `legacy_import_generation`
   row (never deleted, monotonic, committed with the import marker) drives
   dependent-cutover re-runs. No retroactive self-heal for pre-fix
   false-success markers.
4. **Recovery contract stays one-row** (clear the import row); the state
   machine completes `(0,1) → (1,0)` atomically at import commit and
   aborts on any incomplete restore: no source present, any backup
   destination still present (partial move-back or copy-instead-of-move).
5. **Lock policy: non-blocking.** Busy → abort startup.
6. **Archived tombstones win totally** — record, projection, AND
   transcript; the tombstone pre-check deletes any leftover transcript
   (product-archive delete is best-effort, so leftovers are real) and
   fails closed if that delete fails; the residual race branch cleans up
   its own write and chains into the next pass's pre-check on failure.
7. **Diverged transcripts win** over archive `messages`; divergence is
   judged on `message_identity` sequences, never on timestamp-bearing
   record equality; structural corruption is a failure, not divergence.
