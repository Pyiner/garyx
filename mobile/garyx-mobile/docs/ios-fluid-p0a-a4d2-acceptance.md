# iOS Fluid P0-A A4d-2 Acceptance Record

Date: 2026-07-19
Updated: 2026-07-22 — composer inline status presentation retired; durable
send, retry, recovery, and outbox behavior remains unchanged.

This record covers the v41 A4d-2 slice: the A4d-1 concrete durability domain
is connected to the production iOS composer, existing gateway transport,
authenticated delivery evidence, and durable user-facing exits. No gateway
code or endpoint contract changes are part of this slice.

## Environment

- Xcode 26.6 (17F113), iOS Simulator SDK 26.5
- Apple Swift 6.3.3
- iPhone 17 Pro simulator, iOS 26.5

## Production send and transport wiring

- A composer send captures the latest reducer state, runs the delivery quota
  preflight, seals a `GaryxSendCommitBarrier`, and atomically publishes the
  reservation ledger, immutable envelope, delivery record, payload clear, and
  follow-up generation before transport can run. Every accepted send owns a
  distinct delivery handle; an active run therefore does not collapse a busy
  follow-up into an earlier record.
- The sealed barrier captures immutable attachment upload snapshots as well as
  their IDs. Relaunch and an ambiguous restore therefore retain the exact
  transport envelope without consulting mutable staging state. Barriers
  written by the earlier A4d-1 schema decode a missing snapshot field as an
  empty list.
- The quota gate is the A4d-1 canonical `GaryxDeliveryQuota`: at most 64 live
  records per gateway scope and 256 globally. Rejection occurs before sealing,
  leaves the draft intact, and durably records owner-scoped backpressure
  feedback without projecting it into composer copy.
- `GaryxGatewayClient` exposes a before-dispatch callback at the existing
  create-thread and start-chat request sites. The callback commits
  `transportAttempted` immediately before URLSession dispatch. For a newly
  created conversation, the message attempt and current create stage advance
  in the same SQLite transaction.
- A successful gateway response records acknowledgement. An error after the
  attempt gate records ambiguity; relaunch recovery also promotes an
  interrupted `transportAttempted` record to `ambiguous`. Pre-encoding and
  other pre-dispatch failures never claim that transport was attempted.
- A bare message killed after `commitSend` but before the attempt marker is not
  left as a hidden `notDispatched` record and is never silently sent. Relaunch,
  or a live attempt-marker storage failure, atomically restores the immutable
  envelope to composer ownership, terminalizes the delivery as
  `abandoned/restoredToDraft`, removes its host reference, and releases both
  delivery quotas. Placement is automatic: a host with only whitespace and no
  attachments or in-flight operation adopts the envelope in the same
  transaction; a meaningful host stays byte-for-byte intact while the envelope
  remains a separately rooted durable payload. Deferred payloads are adopted in
  recovery-generation order after the newer draft is committed to the durable
  delivery pipeline, or on a later activation/relaunch once its host is blank.
  No recovery-choice notice or action is projected. Multi-stage create
  ownership remains on its explicit create ambiguity path. Legacy A4d-1
  envelopes lacking attachment snapshots recover their text and publish a
  durable warning to reattach the missing files.
- The gateway paths and request bodies remain compatible: start-chat still
  carries the existing message, attachments, workspace, and metadata fields,
  including `client_intent_id`. Existing low-level sends without a durable
  delivery handle retain their prior transport path. Existing busy-send,
  direct-follow-up, and Queue-Steer run-tracker semantics remain in place.

## Evidence and non-presentational recovery

- Committed history and per-thread stream frames feed their authenticated
  `origin_id` values into a body-free `DeliveryEvidenceIngress`. Matching uses
  exact gateway scope plus correlation ID and can acknowledge an ambiguous
  record without depending on the active composer Entry.
- An unresolved record remains durable evidence, but no delivery phase is
  projected into composer copy. The restore and duplicate-resend transactions
  remain available to the durability workflow, and late evidence can still
  claim the original correlation without undoing either user disposition.
- Conversation creation persists `createPending`, `threadCreated`, optional
  `bindingCompleted`, and `chatStartAttempted` separately. Lost create,
  binding, or chat responses become ambiguous at the exact durable stage. A
  lost create response remains durable state and is not presented in the
  composer; the product does not promise that a server-side conversation
  cannot exist. Restore and duplicate-risk rebuild still settle create plus
  message state together, and a rebuild changes both client intents when a new
  conversation may be required.

## Durable feedback state

- Pending operation feedback remains in the durability snapshot, but the
  composer neither projects it into inline copy nor advances it solely for
  presentation.
- Backpressure and storage feedback acknowledgement removes the Entry
  reference and advances the feedback record in one action transaction.
  Retry-upload and remove-upload actions likewise include their acknowledgement
  and attachment/operation settlement in the owning transaction.
- Gateway-scope revocation still archives destination-invalid feedback and
  removes its live Entry references.

## Scope settlement

- Logout/revocation first advances the persisted scope registry and
  `revokedThroughEpoch`, then settles every delivery according to its own
  phase: unattempted work is cancelled, attempted work retains bounded
  evidence, and acknowledged work becomes terminal evidence.
- Unfinished create correlations and destination-scoped feedback settle in the
  same scope transaction. The watermark rejects activation or mutation from a
  revoked epoch after relaunch.
- Delivery evidence remains an independent ingress. Authenticated late
  evidence may advance an attempted revoked tombstone, but it cannot resurrect
  the scope, payload, or archived feedback.

## Fifth-layer real-process harness

`GaryxComposerDurabilityCrashHarness` opens the production SQLite store in a
separate process. Kill cases use real `SIGKILL`; every assertion after death
opens the same main/WAL files in a newly launched process.

The complete retained A4d-1 matrix remains green. A4d-2 adds and pins:

- all 24 physical commit-send boundaries under SIGKILL, ENOSPC, and fsync
  failure, with every post-commit bare `notDispatched` message reclaimed as a
  visible conflict draft and its live quota released;
- every SQLite boundary of that automatic recovery transaction under real
  process death, followed by a second idempotent relaunch asserting recovered
  text, conflict visibility, zero host delivery references, and zero live
  delivery quota;
- a real SQLite page-limit exhaustion that returns primary result code 13
  (`SQLITE_FULL`), rolls back revision zero atomically, and reopens with no
  Entry or delivery residue;
- attempt, response-loss ambiguity, and acknowledgement before/after-commit
  deaths, including a process killed immediately after the attempted marker;
- restore-draft and duplicate-risk resend before/after their own commit, plus
  both orderings of each exit against late authenticated evidence;
- create-response, binding-response, and chat-response loss, each followed by
  process death and atomic restore/rebuild exit assertions; and
- scope revoke before/after commit for not-dispatched, attempted, ambiguous,
  and acknowledged records, followed by epoch-gate and late-evidence checks.

The full SwiftPM run executes all real-process durability suites. No case uses
an in-memory reconstruction as a relaunch substitute, and delivery recovery
continues to settle through the durable protocol without relying on composer
status presentation.

## Canonical cross-platform contract

- `docs/agents/conversation-state.md` defines `DurableDeliveryState`, its
  evidence and user-disposition axes, authenticated evidence rules, and the
  multi-stage create contract.
- `spec/conversation-state/states.json` owns the exact delivery/create raw
  values. `spec/conversation-state/scenarios/durable-delivery.json` owns the
  shared transition fixtures.
- iOS runs every durable delivery and create fixture against
  `GaryxMobileCore`. The desktop conformance suite loads the same vocabulary
  and fixture, and both clients assert the `implemented` consumer marker. The
  Mac reducer now consumes the canonical conflict-set admission and
  scope-authenticated multi-record evidence rules rather than maintaining a
  second client-local spec.

## Headless composer presentation acceptance

`GaryxMobileCore` drives the production durable states through an outbox
commit, a stuck transport attempt, ambiguous response loss during network
jitter, and durable recovery. The SwiftPM regression asserts that every stage
projects an empty composer-notice collection. The app-hosted notice fixture and
its XCUITests were removed with the production notice view; no visual fixture
can reintroduce a second presentation source.

## Compatibility and validation

No file under `garyx-gateway` changed. Xcode project references to the retired
notice view, debug fixture, and UI test were removed. The 2026-07-22
task-specific validation was:

```sh
cd mobile/garyx-mobile
swift test --filter \
  GaryxDurableDeliveryActionsTests/testComposerProjectsNoInlineStatusAcrossNetworkFailureRecoverySequence
swift test --filter \
  'GaryxDurableDeliveryActionsTests|GaryxComposerDurabilityRecoveryTests|GaryxComposerDeliveryProtocolTests'
swift test
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro Max,OS=26.5' \
  CODE_SIGNING_ALLOWED=NO
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro Max,OS=26.5' \
  -only-testing:GaryxMobileTests/GaryxComposerRuntimeIntegrationTests \
  CODE_SIGNING_ALLOWED=NO
```

The focused regression failed before the production change with
`Send status unknown` at the response-loss stage, then passed after the mapping
and view path were removed. The focused durable suites passed 57 tests, the
complete SwiftPM run passed 1,521 tests, the exact simulator build completed,
and the composer runtime integration suite passed, all with zero failures.

Original 2026-07-19 results (before the zero-inline-notice update):

- SwiftPM passed 1,401 of 1,401 tests with zero failures, including all 22
  real-process durability suites and the true `SQLITE_FULL` case.
- The complete app-hosted `GaryxMobile` suite passed 133 of 133 tests with zero
  failures, including a production SQLite attachment send/ambiguity/restore
  round trip and a live attempt-marker failure that proves URLSession never
  starts before the envelope is reclaimed.
- The now-retired durable notice UI/VoiceOver suite passed 5 of 5 tests with
  zero failures in 80.596 seconds.
- The desktop canonical conformance suite passed all 27 tests.
- Generic simulator Debug and Release builds both completed successfully with
  code signing disabled.
