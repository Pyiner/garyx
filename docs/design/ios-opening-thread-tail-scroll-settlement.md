# iOS Opening-Thread Tail-Scroll Settlement (fix for TASK-2630 jitter)

Status: design final; implementation follow-up of #TASK-2630.

## Problem

Opening an existing thread with historical messages and no server-side change
still wobbles after the loading row disappears. Reproduced deterministically
(headless, real capture) in commit `986819b0c`:
`GaryxExistingThreadLoadingJitterReproTests` shows six delayed
`scrollTo(bottom)` writes (`[16, 40, 140, 320, 650, 1000]` ms) remain
authorized after both content reducers report a no-op.

Root cause (full analysis:
`docs/design/task-2630-ios-existing-thread-loading-jitter-reproduction.md`):

- `.openingThread` owns a long fixed retry clock
  (`GaryxConversationScrollPolicy.swift`), fired unconditionally whenever the
  reader is not touching the scroll view.
- `GaryxConversationTailScrollScheduler` cancels a token only when a *newer*
  request supersedes it. Equal snapshots produce no new request, so the chain
  has no settlement path.
- Each late write lands during independent layout convergence
  (Markdown/images/tool rows), repeatedly rewriting the offset → visible
  jitter.

Not a `render_state` identity or window problem; row sets and IDs are stable.

## Design

Principle: **a programmatic tail scroll is a means to reach a target
placement, not a timer job.** The retry clock exists only to survive late
layout; every delayed attempt must justify itself at fire time.

Single owner, explicit state machine in Core
(`GaryxConversationScrollPolicy` / `GaryxConversationTailScrollScheduler`):

```
requested → attempting(retry clock) → settled        (target placement held)
                                    → superseded     (newer request)
```

- **Fire-time need check.** A delayed attempt is authorized only if the target
  placement is not currently satisfied (bottom anchor not held) or content
  geometry changed since the last executed write. The existing
  reader-interaction gate stays; this adds the need gate on top.
- **Settlement is terminal.** Once an attempt observes the placement satisfied
  with stable geometry, the token settles and all remaining attempts for that
  token are void. Equal/no-op frames neither re-arm nor are required for
  settlement — settlement comes from placement confirmation, not from frame
  traffic.
- **Adapter stays dumb.** The SwiftUI executor
  (`GaryxMobileConversationViews.swift`) feeds the pure inputs Core needs
  (at-bottom observation, geometry-epoch signal already derived from the
  reducers) and executes only Core-authorized writes. No timing or policy
  decisions in the adapter. The retry millisecond array stays in Core
  (`TailScrollReason.retryDelayMilliseconds`, moved there by `986819b0c`).

Preserved intent (pinned by the existing 46 `GaryxConversationScrollStateTests`):

- Initial open still lands at bottom even when Markdown/image layout settles
  late — geometry movement re-qualifies the next attempt, so late convergence
  still re-anchors.
- User touch still suppresses programmatic writes.
- Newer tail requests still supersede older tokens.

Boss red line (2026-07-21) honored: jitter is solved at the behavior layer
(anchoring/settlement semantics); no transcript element moves out of the
message scroll flow; no render_state contract change.

## Impact

- `GaryxMobileCore`: scroll policy/scheduler state machine + authorization
  inputs (pure, SwiftPM-tested).
- App target: conversation view adapter feeds the new inputs; no visual or
  structural change.
- No gateway/desktop/protocol change.

## Done criteria

- `GaryxExistingThreadLoadingJitterReproTests` turns green with the same
  capture and sequence (assertion kept, `EXPECTED FAILURE` marker removed) —
  it becomes the permanent regression test.
- Existing scroll/presentation suites pass with real executed-test counts.
  Tests that encoded the buggy unconditional authorization may be updated to
  the settlement contract; tests pinning initial-placement/user-touch/
  supersede intent must stay green as-is.
- `GaryxMobile` app target compiles for iPhone 17 Pro Max / iOS 26.5.
