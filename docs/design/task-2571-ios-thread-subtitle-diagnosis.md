# #TASK-2571 iOS Thread Subtitle Diagnosis

Status: diagnosis and deterministic red reproductions only. No production fix
is included.

## Reproductions

The gateway tests drive the real HTTP and persistence path: create a thread
with `noWorkspace=true`, submit a user message, block the provider, read both
list projections, release the provider, and read both projections again.

```sh
cargo test -p garyx-gateway \
  task_2571_new_thread_preview_is_missing_while_first_run_is_active \
  --lib -- --ignored --nocapture

cargo test -p garyx-gateway \
  task_2571_recent_and_summary_routes_disagree_after_completed_run \
  --lib -- --ignored --nocapture
```

Both commands are intentionally red. The first fails with an empty active
Recent preview instead of `Latest user sentence`. The second fails with
`Latest user sentence` from Recent versus `Assistant answer` from Summary.
The tests are in `garyx-gateway/src/chat_tests.rs` and are ignored in normal
test runs.

The sanitized route capture is checked in at
`mobile/garyx-mobile/Tests/GaryxMobileCoreTests/Fixtures/task-2571-thread-subtitle-capture.json`.
It retains the exact endpoint envelopes and row fields while replacing the
random UUIDs, temporary directory, and timestamps with synthetic values. Feed
it through the production Core decoders, shared cache, and subtitle presenter:

```sh
cd mobile/garyx-mobile
TASK_2571_REPRO=1 swift test --filter GaryxThreadSubtitleReproTests
```

This command is also intentionally red, with two failures:

1. `thread--00000000-0000-4000-8000-000000002571` is rendered instead of the
   accepted user sentence while the first run is active.
2. Replaying adjacent source arrivals renders
   `Latest user sentence -> Assistant answer -> Latest user sentence` for the
   same thread.

Without `TASK_2571_REPRO=1`, the two desired-contract assertions skip so normal
SwiftPM validation remains green.

## Captured Gateway State

| Point | Source | message count | active run | preview |
| --- | --- | ---: | --- | --- |
| provider blocked | `/api/recent-threads` | 2 | set | `""` |
| provider blocked | `/api/thread-summaries` | 2 | set | null |
| run committed | `/api/recent-threads` | 5 | null | `Latest user sentence` |
| run committed | `/api/thread-summaries` | 5 | null | `Assistant answer` |

The created row has `workspace_origin=implicit`,
`root_workspace_path=null`, and a private workspace whose basename is the
sanitized thread id. No captured `last_message_preview` contains the thread id.

## Symptom 1: New Thread Shows Its ID

The displayed ID is a client presentation fallback, not a value written into
`last_message_preview`.

The concrete chain is:

1. Commit `9acc37d33290e88332621c090bd54cd27c0bca48` made the iOS `.none`
   workspace selection send `noWorkspace=true`
   (`GaryxMobileModel+Composer.swift:1060-1082`). Before that commit, the same
   selection omitted both `workspaceDir` and `noWorkspace`, allowing an agent
   default workspace to substitute. After it, the gateway intentionally skips
   the agent default (`garyx-gateway/src/agent_identity.rs:140-149`). This is
   the recent regression trigger for users whose agent has a default workspace.
2. The gateway creates an implicit private directory ending in the sanitized
   thread id (`garyx-gateway/src/workspace_mode.rs:25-49,147-163`). This behavior
   itself dates to `69065cea7936c1c90c49a674d64a98e08b7054a0`.
3. Commit `b946e9a07d02ac0eca2a328ae64014e758d4ba32` introduced the current
   subtitle recipe. It places `workspacePath`'s basename before the preview and
   does not suppress that raw path when the workspace is implicit
   (`GaryxHomeThreadListPresentation.swift:174-190`). This is the latent UI
   defect that turns the private directory into a visible thread id.
   `fdd042880c69936f4adc1561fbb10a0d267a41e8` later preferred
   `rootWorkspacePath`, but an implicit thread deliberately has no root, so it
   still falls through to the raw private `workspacePath`.
4. During the first active run, the captured preview is empty. Preview fields
   are populated from the transcript at terminal persistence
   (`garyx-bridge/src/multi_provider/persistence.rs:1580-1619`), while the
   streaming persistence block at lines 1275-1350 does not refresh them. The
   workspace basename therefore becomes the entire subtitle until terminal
   persistence. After completion, the same row renders
   `thread--... · Latest user sentence`, still leading with the id.

Root attribution is therefore composite: `9acc37d332` is the current behavior
change that forces newly explicit No-workspace threads onto the private path;
`b946e9a07` is the presentation commit that exposes that implementation path.
On installations without an agent default workspace, the latent `b946e9a07`
behavior could already occur before `9acc37d332`.

`8faa07957b2a1afb6df50c2c47baf2881bda0eee` introduced the write-time
`last_user_preview` / `last_assistant_preview` fields at terminal persistence.
It contributes the active-run empty-preview window, but it neither stores nor
derives a thread id as preview. The later `23f53429b8a0aebafb63ef1eab2bd69d3e481895`
removed the record `messages` snapshot; before removal, streaming persistence
only ensured an empty array and terminal persistence rebuilt it, so that commit
also did not create an ID-valued preview regression.

## Symptom 2: Existing Subtitle Flickers

Two gateway projections define “last message” differently for the same
canonical record:

- Recent is user-first:
  `last_user_preview.or(last_assistant_preview)` in
  `garyx-gateway/src/recent_thread_projection.rs:171-174`. This ordering dates
  to `e2002823d4a9961ac52743945d9a1b8202503d82`.
- Summary is assistant-first:
  `last_assistant_message.or(last_user_message)` in
  `garyx-gateway/src/thread_meta_projection.rs:33-37`. This ordering dates to
  `393769eda54781fc84a5aef9259718f86d10fc7f`.

Those incompatible definitions became an iOS-visible race in two steps:

1. `5b8f5f5ae555d00046546aadcada197c332c97bb` added the
   `/api/thread-summaries` slice (and enhanced favorites summary lookup) backed
   by `thread_meta`, exposing the assistant-first value as a second list source.
2. `c050ea8b31f2abe68b5426a6c196a68e97f64859` added the shared iOS
   `GaryxThreadSummaryCache` and enhanced favorites owner. Its cache replaces
   the complete summary for an id on every write, with no source authority or
   version comparison (`GaryxThreadSummaryCache.swift:79-81,123-132`). The
   favorites owner deliberately lets the Summary row win over its embedded
   Recent row (`GaryxFavoritesMembershipProvider.swift:103-120,244-251`).

Home refresh starts the favorites snapshot in an unawaited task and then starts
the Recent refresh (`GaryxMobileModel+ThreadList.swift:39-73`). Whichever
request completes last overwrites the shared entry; the main Recent commit is
at lines 548-578, while the enhanced favorites completion is launched from
`GaryxMobileModel+ThreadFavorites.swift:118-156`. Repeated refreshes can
therefore produce the deterministic red-test sequence user -> assistant ->
user, with network completion order deciding the observed transition.

`00679da733526505cb0e69dd42fd0d69b4f5a8cc` later broadened the unified list
ownership integration, but it is not the first bad commit: the exact
last-writer cache and enhanced favorites write path are already present in its
ancestor `c050ea8b31`.

## Suspects Ruled Out

`47198f1c313250844b2f59b42f3e00377d78114b` and
`970a52a0a2790d3780c0f618abae6cb89e1202d6` retire recent-exclusion membership
state and then remove its skeleton. Their relevant diffs change inclusion and
DTO fields, but do not change user/assistant preview selection or cache write
precedence. They are not causal for either reproduced value transition.

## Read-only Existing-data Check

A read-only aggregate query against the local gateway database on 2026-07-22
found:

- 569 implicit rows also present in Recent;
- 51 of those had an empty Recent preview;
- 124 had a workspace basename equal to the sanitized thread id;
- 6 had both conditions, which is the exact id-only subtitle shape;
- 2,402 joined rows with both user and assistant previews had different Recent
  and Summary `last_message_preview` values;
- 0 Recent previews equaled either the raw or sanitized thread id.

Only aggregate counts were collected; no thread ids or message bodies are
included here.
