# iOS Fluid P0-A A4d-1 Acceptance Record

Date: 2026-07-18

This record covers the v41 A4d-1 concrete durability slice only. It introduces
no UI wiring and does not connect the existing composer or transport path;
A4b and A4d-2 consume this layer later.

## Environment

- Xcode 26.6 (17F113), iOS Simulator SDK 26.5
- Apple Swift 6.3.3
- iPhone 17 Pro simulator, iOS 26.5

## Concrete durability domain

- `GaryxSQLiteComposerDurabilityStore` owns one SQLite database in WAL mode
  with `synchronous=FULL`. One metadata table and all 16 composer record-family
  tables live in that database; no attached or secondary database is opened.
- `commitSend(_:)` publishes the committed reservation ledger, payload
  generation/clear, send barrier, immutable delivery envelope, and optional
  `producerDrained` records under one `BEGIN IMMEDIATE`/`COMMIT` transaction.
- Payload, outbox, reservation ledger, operation manifest, replacement,
  feedback, discard convergence/tombstones, conflicts, aliases, lineages,
  create delivery, and staged-file quota/owner state use the same reducer and
  transaction domain.
- The concrete store rejects durable reservation descendants without their
  ledger. A ledger mutation is first in `commitSend`; producer-drained records
  and the ledger are published atomically before any transport gate can run.
- Payload generations and send reservation IDs use a durable hi-lo allocator:
  block refill pre-raises the persisted high watermark, in-block allocations
  perform no database commit, and relaunch skips every unused prior-process ID.
- Protected staging reserves owner and bytes before copying, uses a protected
  app-private/excluded-from-backup directory, file fsync + atomic rename +
  directory fsync, durable condemned-file cleanup, and launch removal of
  interrupted `.partial-*` copies.

## Reviewer pins

- **L-1:** the committed replacement swap retains O1 only as a `superseded`
  audit record and clears `O1.stagedAssetID` plus `O1.reservedBytes` in that
  same transaction. O2 is the sole manifest/file/quota owner. A concrete
  SQLite pre-commit failure restores the empty prior state; relaunch after a
  successful commit preserves this exact owner shape.
- **L-2:** `removeDiscardConvergence` is rejected by the shared apply layer
  unless the stored lifecycle is `.discarded`. The successful removal path
  first JSON round-trips the complete snapshot containing its tombstone, then
  performs GC; the process harness independently decodes committed tombstones
  from SQLite in a new process.

## Fifth-layer process harness

`GaryxComposerDurabilityCrashHarness` is a separate executable. Every crash
case starts a process against a fixture database, verifies termination by real
`SIGKILL`, then opens the same SQLite/WAL files in a newly launched process.
No test reconstructs an in-memory store to simulate relaunch.

The retained matrix covers:

- commit-send at 24 physical SQLite boundaries: 24 SIGKILL cases plus the same
  24 boundaries under ENOSPC and fsync failure (48 I/O-failure cases);
- attempted-before/after, transport-response ambiguity, and acknowledgement
  before/after commit, with every recovery disposition constrained to
  acknowledged, safe retry, or user-terminable;
- the seven-mutation startup synthetic-revocation transaction at 27 physical
  boundaries under SIGKILL, ENOSPC, and fsync (81 cases), followed by two
  relaunches that keep `T+U`, a revoked target mapping, and exactly one close;
- nine operation-state/attempt combinations × four reservation outcomes
  (`nil`, unsettled, committed, revoked) × three scope lifecycles = 108 sealed
  manifest recovery cases;
- every operation state × destination discard, each killed after durable
  admission and again during convergence, ending with zero owner, manifest,
  replacement, feedback, quota, cleanup, or payload residue;
- cross-promotion S1 close-pending-ack + S2 live/finalizing discard under
  active and revoked scope at every committed step, including new-process JSON
  decoding of both finalization tombstones;
- a mixed three-phase delivery fixture (not dispatched, attempted, acknowledged)
  plus sealed reservation, two sessions, and alias, killed after each of eight
  convergence commits; final records are respectively cancelled-by-discard,
  evidence, and terminal-evidence;
- seven protected-staging boundaries under SIGKILL, ENOSPC, and fsync, with
  relaunch proving no final file, partial file, owner, quota, manifest, or
  cleanup tombstone remains; and
- 500 multi-session cross-promotion discards followed by relaunch, with a
  bounded tombstone pool and zero durable resource residue.

## Deletion and wiring checklist

- No file under `App/GaryxMobile` and no existing send/transport call site is
  changed. App behavior is intentionally unchanged for this slice.
- No parallel persistence domain, fallback file store, read-time repair, or
  second outbox path is introduced.
- `GaryxFakeComposerDurabilityStore` remains only as the A3 contract test seam;
  the concrete crash harness, staging, launch recovery, and transport gate all
  consume the same `GaryxComposerDurabilityStore` protocol using the SQLite
  implementation.
- A4b composer-path replacement/deletion and A4d-2 transport/ambiguous UX
  deletion are not performed early in A4d-1.

## Reproduction

```sh
cd mobile/garyx-mobile
xcodegen generate
swift test
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Debug \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
```

The final clean SwiftPM run passed 1,344 of 1,344 tests with zero failures in
199.834 seconds. The generated Xcode project passed Debug and Release generic
iOS Simulator builds, and the `GaryxMobile` app-hosted suite passed 89 of 89
tests on iPhone 17 Pro / iOS 26.5. Build warnings were pre-existing app-source
deprecations; the A4d-1 files emitted no warnings.
