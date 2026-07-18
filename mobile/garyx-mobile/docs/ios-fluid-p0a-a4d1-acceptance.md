# iOS Fluid P0-A A4d-1 Acceptance Record

Date: 2026-07-19

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
- Protected staging reserves owner and bytes before copying, creates each
  `.partial-*` file with mode `0600`, and applies the iOS protection class plus
  backup exclusion before writing its first byte. It then streams the source,
  performs file fsync + atomic rename + final-path protection + directory
  fsync, records durable condemned-file cleanup, and removes interrupted
  partial copies at launch. The SQLite main, WAL, and SHM files receive the
  same app-private protection and backup policy.

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

## Adversarial recovery remediation

Four cross-model adversarial passes found compound-state shapes that the
original single-operation fixtures did not exercise. They are now closed as
independent acceptance surfaces:

- Scope revoke settles every operation, manifest, staged owner, replacement,
  feedback, and lineage for an Entry before removing that shared Entry in one
  transaction. A two-operation concrete staging test and active/suspended/
  revoked process-kill matrix prove that no sibling can be left referring to a
  removed Entry. The v41 child-only rows remain distinct: cancelled and
  failed-retryable children are removed while sibling text and attachments
  survive.
- Discard settlement treats the convergence copy as discovery metadata and
  re-reads the authoritative delivery ledger for each record CAS. A late
  server acknowledgement therefore advances to terminal evidence instead of
  being overwritten by a captured attempted record. The shared transaction
  engine independently rejects phase, evidence, envelope, or disposition
  regression, and an acknowledged-only discard cannot skip the required
  `acknowledged -> terminalEvidence` transition.
- Launch feedback IDs encode the complete scope/Entry/generation/reservation/
  branch/operation identity with length-delimited components. Two Entries may
  use the same local operation ID without sharing a feedback record.
- Discard releases only the captured forward alias paths from that Entry's
  stable-token sessions to its destination, including `D -> T1 -> T2`.
  Session tombstones reconstruct a per-session release list without collapsing
  duplicate ComposerKeys: live/finalizing sessions contribute one
  active-or-closing reference, while close-pending sessions additionally
  contribute one pending-ack reference. Each contribution is subtracted from
  every edge it actually traverses, and an edge is removed only when its own
  three durable counters reach zero. No incoming-edge topology, force-zero
  drain, or inferred conservative occupancy floor decides ownership. Direct
  fan-in `D1 -> T <- D2`, shared suffix `A -> X <- B, X -> D`, occupancy-only
  residuals `A -> X(1), X -> D(2)`, and same-source follow-up occupancy all
  prove that discarding one Entry cannot break a live sibling route. SQLite
  fixtures reopen the same database twice; a same-origin two-session case
  proves multiplicity survives relaunch. A discriminating nested-origin case
  leaves a residual count of one, so saturating subtraction cannot mask a
  dropped or duplicate contribution. Cross-promotion and 500-churn fixtures
  retain heterogeneous multi-hop zero-residue coverage.
- Ownerless-manifest recovery removes the corresponding Entry operation
  membership in the same transaction. Concrete double-relaunch and real
  pre-/post-commit process-kill tests prove the recovery branch cannot reject
  its own result on every subsequent launch.
- Replacement abort now settles any provisional staged-file owner operation,
  manifest, and Entry membership in the same transaction as condemned-file
  registration. An unowned provisional file uses a non-live cleanup identity,
  preserving the old operation and its canonical asset. Revoked replacement
  recovery delegates to the same whole-Entry or child-only settlement vote as
  operation recovery, so the replacement loop cannot create a transaction
  that violates the store's cleanup-owner invariant.
- A superseded O1 is lineage, not an independent payload-erase vote. The mixed
  `{O1 superseded, O2 failedRetryable} x revoked` fixture removes O2 and its
  descendants while preserving sibling text/attachments and O1's lineage-only
  audit shape.
- Scope-revoke fixtures now seed replacement, feedback, and attachment-lineage
  records in both child-only and whole-Entry paths, and assert that every
  record family is removed without disturbing unrelated siblings.
- Delivery monotonicity tests independently reject record-identity rewrites,
  envelope resurrection/regression, and illegal user-disposition transitions;
  the one allowed `payloadDiscarded -> scopeRevoked` transition remains
  covered. A server acknowledgement received after terminal evidence is an
  idempotent no-op rather than a phase regression.
- An app-hosted iOS 26.5 suite observes the required protection class after
  Foundation accepts it at the temporary copied boundary and final staged
  path. It also tampers and reopens the SQLite sidecars, then proves that the
  main database, WAL, and SHM are all re-protected and excluded from backup.

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
  manifest recovery cases, including the revoked child-only payload rule;
- one shared Entry with two durable operation/manifests under all three scope
  lifecycles, killed both before and after recovery commit, then relaunched
  twice with operation membership, owners, quota, and cleanup all settled;
- pending, aborted, and committed replacement records x active, suspended, and
  revoked scope x kill before/after the recovery commit (18 cells), each
  relaunched twice against the same SQLite/WAL files. Active/suspended
  committed replacements restore O2; every abort path atomically settles its
  provisional owner without deleting sibling text or attachments;
- a separate whole-Entry revoked replacement case, killed before and after
  recovery commit and relaunched twice, that proves replacement, feedback,
  lineage, manifest, owner, quota, membership, and physical asset cleanup;
- every operation state × destination discard, each killed after durable
  admission and again during convergence, ending with zero owner, manifest,
  replacement, feedback, quota, cleanup, or payload residue;
- cross-promotion S1 close-pending-ack + S2 live/finalizing discard under
  active and revoked scope at every committed step over `D -> T1 -> T2`,
  including new-process JSON decoding of both finalization tombstones;
- a mixed three-phase delivery fixture (not dispatched, attempted, acknowledged)
  plus sealed reservation, two sessions, and a multi-hop alias. After discard
  admission, the attempted global record receives a late server ack while the
  convergence copy remains stale; the process is killed after each of eight
  convergence commits. Final records are respectively cancelled-by-discard,
  terminal-evidence with server-acknowledged evidence, and terminal-evidence;
- an ownerless operation manifest killed before and after recovery commit,
  followed by two relaunches with both manifest and Entry membership absent;
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
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Release \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  CODE_SIGNING_ALLOWED=NO
```

The final clean SwiftPM run passed 1,368 of 1,368 tests with zero failures in
232.964 seconds; its 16 real-process durability suites passed in 225.158
seconds. The generated Xcode project had zero drift and passed Debug and
Release generic iOS Simulator builds, and the `GaryxMobile` app-hosted suite
passed 91 of 91 tests on iPhone 17 Pro / iOS 26.5. Build warnings were
pre-existing app-source deprecations; the A4d-1 files emitted no warnings.
