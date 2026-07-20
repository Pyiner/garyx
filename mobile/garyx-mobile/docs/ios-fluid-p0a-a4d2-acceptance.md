# iOS Fluid P0-A A4d-2 Acceptance Record

Date: 2026-07-19

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
  leaves the draft intact, and durably creates one owner-scoped backpressure
  chip.
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
  envelope through `PayloadConflictSet`, terminalizes the delivery as
  `abandoned/restoredToDraft`, removes its host reference, and releases both
  delivery quotas. Multi-stage create ownership remains on its explicit create
  ambiguity path. Legacy A4d-1 envelopes lacking attachment snapshots recover
  their text and publish a durable warning to reattach the missing files.
- The gateway paths and request bodies remain compatible: start-chat still
  carries the existing message, attachments, workspace, and metadata fields,
  including `client_intent_id`. Existing low-level sends without a durable
  delivery handle retain their prior transport path. Existing busy-send,
  direct-follow-up, and Queue-Steer run-tracker semantics remain in place.

## Evidence and explicit ambiguous exits

- Committed history and per-thread stream frames feed their authenticated
  `origin_id` values into a body-free `DeliveryEvidenceIngress`. Matching uses
  exact gateway scope plus correlation ID and can acknowledge an ambiguous
  record without depending on the active composer Entry.
- An unresolved record is rendered inline as **Send status unknown**. The two
  explicit actions are atomic durability transactions: restore the envelope as
  a separate `GaryxPayloadConflictSet` candidate, or resend a clearly labelled
  duplicate-risk copy with a fresh client intent ID. Late evidence can still
  claim the original correlation without undoing either user disposition.
- Conversation creation persists `createPending`, `threadCreated`, optional
  `bindingCompleted`, and `chatStartAttempted` separately. Lost create,
  binding, or chat responses become ambiguous at the exact durable stage. A
  lost create response is presented as **Conversation creation status
  unknown**; the product does not promise that a server-side conversation
  cannot exist. Restore and duplicate-risk rebuild settle create plus message
  state together, and a rebuild changes both client intents when a new
  conversation may be required.

## Durable feedback chips

- Pending operation feedback is projected only for the exact host Entry that
  currently owns interaction. Presentation transitions `pending -> presented`
  durably, so route changes and relaunch cannot silently lose the chip.
- Backpressure and storage feedback acknowledgement removes the Entry
  reference and advances the feedback record in one action transaction.
  Retry-upload and remove-upload actions likewise include their acknowledgement
  and attachment/operation settlement in the owning transaction.
- Gateway-scope revocation archives destination-invalid feedback and removes
  its live Entry references. The inline chip UI exposes native labelled
  buttons with at least 44-point hit regions and no competing interaction
  owner.

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

The full SwiftPM run executes all 22 real-process durability suites. No case
uses an in-memory reconstruction as a relaunch substitute, and every recovered
delivery reaches acknowledged, a surfaced/reclaimed safe retry, or an explicit
user exit rather than remaining hidden in `notDispatched` or
`transportAttempted`.

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

## UI and accessibility acceptance

The production notice stack is exercised through an isolated app-hosted
fixture using the same view and action types. Five XCUITests cover:

- unknown-send restore through a conflict without overwriting the current
  draft;
- duplicate-risk resend warning and fresh-intent action;
- durable feedback acknowledgement, upload retry, and upload removal;
- lost-create restore and duplicate-risk conversation rebuild; and
- VoiceOver sufficient-description plus hit-region audits for every send exit
  and chip action.

All five passed on iPhone 17 Pro / iOS 26.5 with zero accessibility audit
failures. The only product-visible additions are the inline unknown-state exits
and durable feedback chips.

## Compatibility and validation

No file under `garyx-gateway` changed. The generated Xcode project has no
post-generation drift. The final validation gate is:

```sh
cd mobile/garyx-mobile
xcodegen generate
swift test
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  CODE_SIGNING_ALLOWED=NO
xcodebuild test -project GaryxMobile.xcodeproj \
  -scheme GaryxMobileFluidRoutes \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro,OS=26.5' \
  -only-testing:GaryxMobileUITests/DurableDeliveryInteractionTests \
  CODE_SIGNING_ALLOWED=NO
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Debug \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
xcodebuild build -project GaryxMobile.xcodeproj \
  -scheme GaryxMobile -configuration Release \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO
cd ../../desktop/garyx-desktop
npm run test:unit -- \
  src/renderer/src/conversation-state-conformance.test.mjs
```

Final results:

- SwiftPM passed 1,401 of 1,401 tests with zero failures, including all 22
  real-process durability suites and the true `SQLITE_FULL` case.
- The complete app-hosted `GaryxMobile` suite passed 133 of 133 tests with zero
  failures, including a production SQLite attachment send/ambiguity/restore
  round trip and a live attempt-marker failure that proves URLSession never
  starts before the envelope is reclaimed.
- The focused durable UI/VoiceOver suite passed 5 of 5 tests with zero failures
  in 80.596 seconds.
- The desktop canonical conformance suite passed all 27 tests.
- Generic simulator Debug and Release builds both completed successfully with
  code signing disabled.
