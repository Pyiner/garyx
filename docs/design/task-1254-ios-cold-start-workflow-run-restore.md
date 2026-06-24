# Task 1254: iOS Restore Guard And Gateway Thread Type Consistency

## Problem

Build 108 includes `3557c4c3` (`Fix iOS home cold-start restore`). That patch
made the iOS persisted last-opened restore path reject resolved workflow-run
destinations and clear polluted restore defaults.

The pending-intent theory is rejected. `pendingThreadId` is queued only by
URL/widget route handling before the gateway is ready, so it is deferred user
navigation, not automatic restore. Cold-start deep links to workflow runs must
continue to open workflow runs, just like warm links.

The current iOS restore path is also narrower than the second draft claimed:
iOS cold-start restore resolves missing recent-list entries through
`/api/threads/{id}`, and the app test below confirms that a workflow summary
fetched from that endpoint is rejected and clears polluted restore state. iOS
does not currently decode the `thread` or `session` summary blocks returned by
`/api/threads/history`.

There is still a real gateway contract bug worth fixing: gateway history detail
summaries can report a stored workflow-run thread as `chat`. That is independent
contract hygiene for clients and tools that do consume the history envelope, not
a proven iOS cold-start root cause.

## Evidence

Rejected iOS candidate:

- `queuePendingThreadLink` is reached through `openMobileRouteFromLink` and
  `queuePendingMobileRoute`, which are triggered from `.onOpenURL`;
- the pending route is in memory and not a persisted automatic restore source;
- blocking it would break explicit workflow-run links during cold start.

Confirmed iOS restore guard:

- `GaryxMobileModel.restoreLastOpenedThread(id:)` consults recent in-memory
  summaries, then refreshed recent threads, then `client().getThread(threadId:)`
  (`/api/threads/{id}`);
- every resolved summary goes through the existing restoration policy before the
  app opens a destination;
- `GaryxThreadTranscript` does not decode `/api/threads/history` `thread` or
  `session` blocks, so that endpoint is not a current iOS restore input.

Confirmed gateway contract split:

- list summaries derive `thread_type` from stored `thread_kind` with a `chat`
  fallback;
- `/api/threads/{id}` metadata currently clones raw metadata and does not
  normalize `thread_type`;
- `/api/threads/history` derives both `thread_type` and `session_type` from
  `infer_thread_type(thread_id)`, so a normal workflow-run id with stored
  `thread_kind: workflow_run` comes back as `chat`;
- the same fallback is inconsistent for legacy ids without `thread_kind`:
  metadata/list treat them as `chat`, while history currently treats
  `cron::...` as `cron`.

Added deterministic regression coverage:

- App coverage:
  `GaryxLastOpenedWorkflowRestoreTests.testColdLaunchRestoreRejectsWorkflowRunFetchedAfterRecentListOmission`
  uses the captured workflow fixture, starts with empty in-memory `threads`,
  stubs empty recent/pins plus `/api/threads/{id}`, and verifies restore lands on
  home and clears polluted defaults. This passes today and documents that the
  direct iOS getThread fallback is not the current hole.
- Gateway red test:
  `api::tests::test_thread_history_detail_preserves_workflow_thread_type` seeds a
  transcript-backed thread with `thread_kind: workflow_run`, calls
  `/api/threads/history?thread_id=...`, and currently fails because
  `thread.thread_type == "chat"` instead of `"workflow_run"`.
- Gateway fallback red test:
  `api::tests::test_thread_history_detail_defaults_missing_thread_kind_to_chat`
  seeds a `cron::...` shaped legacy id without `thread_kind` and verifies history
  defaults to `chat`, matching metadata/list behavior.
- Metadata red tests:
  `routes::tests::thread_metadata_preserves_workflow_thread_type` and
  `routes::tests::thread_metadata_defaults_missing_thread_kind_to_chat` verify the
  single-thread metadata response uses the same type derivation as history and
  list summaries.

## Design

Introduce one gateway helper for thread-summary type derivation:

```rust
thread_kind_from_value(data).unwrap_or_else(|| "chat".to_owned())
```

Use that helper in all JSON summary surfaces that build from a raw thread record:

- `routes::thread_summary(...)`, used by list/create/update style summaries;
- `routes::thread_metadata_response(...)`, used by `/api/threads/{id}`;
- `api::summarize_thread(...)`, used by `/api/threads/history` `thread` and
  `session` envelopes.

The helper deliberately has no id-pattern fallback. If old thread metadata lacks
`thread_kind`, every raw-record summary surface reports `chat`. That matches the
existing list fallback and eliminates the history-vs-metadata split for
`cron::...` or group-shaped legacy ids without stored kind data.

The fix is server-side because `thread_kind` is gateway/router state, and
clients should not guess workflow identity from ids or workflow metadata side
channels.

## Why This Is Not A Patch

The fix does not add an iOS-only `if workflow_run` guard and does not special
case one cold-start branch. It makes the gateway surfaces that summarize the
same raw thread record share the same type derivation and fallback.

The iOS policy remains:

- automatic persisted restore may open chat destinations only;
- workflow-run and unresolved destinations clear polluted restore state and land
  on home;
- direct user opens, including cold-start deep links/widgets, may open workflow
  runs.

This design does not claim that `/api/threads/history` is the current iOS
cold-start failure input. It preserves the existing iOS restore fix with app
coverage and fixes an adjacent gateway contract bug that could mislead history
consumers.

## Validation Plan

Before implementation:

- Red gateway test:
  `cargo test -p garyx-gateway test_thread_history_detail_preserves_workflow_thread_type -- --nocapture`
  fails with `left: String("chat")`, `right: "workflow_run"`.
- Red gateway fallback test:
  `cargo test -p garyx-gateway test_thread_history_detail_defaults_missing_thread_kind_to_chat -- --nocapture`
  fails because history reports `cron` for a `cron::...` id without
  `thread_kind`.
- Red metadata tests:
  `cargo test -p garyx-gateway thread_metadata_preserves_workflow_thread_type -- --nocapture`
  and
  `cargo test -p garyx-gateway thread_metadata_defaults_missing_thread_kind_to_chat -- --nocapture`
  fail because metadata does not normalize `thread_type`.
- App getThread fallback coverage:
  `xcodebuild test -project mobile/garyx-mobile/GaryxMobile.xcodeproj -scheme GaryxMobile -destination 'id=<simulator>' -only-testing:GaryxMobileTests/GaryxLastOpenedWorkflowRestoreTests/testColdLaunchRestoreRejectsWorkflowRunFetchedAfterRecentListOmission CODE_SIGNING_ALLOWED=NO`
  passes and confirms that branch is already protected.

After implementation:

- Green gateway regressions above.
- Green relevant gateway history tests:
  `cargo test -p garyx-gateway thread_history_detail -- --nocapture`.
- Green route metadata summary tests:
  `cargo test -p garyx-gateway thread_metadata_ -- --nocapture`.
- Green iOS restore tests:
  `xcodebuild test -project mobile/garyx-mobile/GaryxMobile.xcodeproj -scheme GaryxMobile -destination 'id=<simulator>' -only-testing:GaryxMobileTests/GaryxLastOpenedWorkflowRestoreTests CODE_SIGNING_ALLOWED=NO`.
- Green SwiftPM Core restore policy tests:
  `swift test --package-path mobile/garyx-mobile --filter GaryxLastOpenedThreadRestorationPolicyTests`.
- Real app-target compile:
  `xcodebuild build -project mobile/garyx-mobile/GaryxMobile.xcodeproj -scheme GaryxMobile -destination 'id=<simulator>' CODE_SIGNING_ALLOWED=NO`.
