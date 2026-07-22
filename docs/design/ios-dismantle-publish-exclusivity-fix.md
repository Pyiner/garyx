# iOS Representable Teardown Publish: Deferred Observable Settlement Design

Status: approved for implementation.
Repro evidence: #TASK-2566, commit `2206e5287` on branch `garyx/c7c093e4`
(`GaryxProductionRouteIntentIntegrationTests
.testBuild158BackgroundSceneTeardownDoesNotPublishDuringDismantle`,
deterministic `Fatal access conflict detected` SIGABRT). Production crash:
TestFlight build 158 `.ips`, background scene reclaim.

## Problem

Representable lifecycle callbacks — make, update, and dismantle — run inside
SwiftUI graph updates; dismantle can run inside graph teardown while a window
or scene deallocates (now a repo contract in AGENTS.md/CLAUDE.md). The route
stack's detach path violates this:

- `GaryxProductionRouteStack.dismantleUIViewController`
  → `GaryxMobileModel.detachGlobalRevealHostOccurrence`
  → `GaryxHorizontalRevealInteraction.detachHostOccurrence` (current owner)
  → `forceTerminal` → `publish()` → `@Published presentation` write
  → `ObservableObjectPublisher.send` re-enters the graph being invalidated
  → `swift_beginAccess` exclusivity abort. This is the build 158 crash.
- The same publish chain is reachable in the foreground:
  `updateUIViewController` → `Coordinator.transitionRootSurface` →
  detach/attach; `attachHostOccurrence`'s `.superseded` branch also runs
  `forceTerminal` → `publish()`. Update callbacks are graph-update contexts
  too — there it is "modifying state during view update" undefined behavior
  rather than a guaranteed abort.
- `GaryxProductionRouteStore.detach` → `GaryxPresentationLeaseCoordinator
  .detach` → `presentationBarrierStateChanged(false)` synchronously writes
  `@Published hasPresentationBarrier`. Conditionally poisonous: it publishes
  exactly when a presentation barrier is active at teardown (measured in the
  repro).

Root cause: the detach/attach entry points mix two responsibilities —
ownership bookkeeping (must be immediate) and observable presentation/barrier
settlement (must never run inside a graph update).

## Design

### 1. Split bookkeeping from observable settlement

In `GaryxHorizontalRevealInteraction`, the host-ownership entry points that
representable lifecycle callbacks reach (`attachHostOccurrence`,
`detachHostOccurrence`, `applyRootSurfaceOccurrenceTransition`) settle
`hostOwnership` (non-observable state) synchronously — ownership truth never
lags. When such a transition requires a terminal presentation settle, the
observable part (`forceTerminal`'s state transition publish) is scheduled
outside the current graph update via a main-actor async hop. No wall-clock
timers, no caller-supplied context flags: these entry points defer uniformly,
because their callers are representable lifecycle callbacks by construction.

Gesture-driven paths (`beginGesture`, `updateGesture`, `endGesture`,
`cancelGesture`, display-link settle frames) keep synchronous publication:
they run from UIKit event contexts where publishing is legal and per-frame
latency matters.

### 2. Stale settlement must not touch newer state

The deferred settle captures the host occurrence (and reveal generation) it
settles for. If, by execution time, a newer host occurrence attached or a new
gesture owns the presentation, the stale settle must not mutate current
occurrence bookkeeping (same principle as the occurrence-scoped async route
preparation contract). It may only complete the terminal event for its own
detached occurrence; a superseded settle is a no-op beyond its own cleanup.
Terminal zero-residue assertions move with the deferred settle so they assert
after the publish actually lands, not mid-defer.

### 3. Barrier publication on coordinator detach defers the same way

`GaryxPresentationLeaseCoordinator.detach` must not synchronously write
`hasPresentationBarrier` from a dismantle context; its barrier-state
publication defers off the current graph update identically. All other
`synchronizeBarrier` call sites (lease acquire/terminal flows driven by
binding setters, dismissal callbacks, and deferred owner-loss settlement)
already run in legal publish contexts and stay synchronous.

### 4. Explicitly not in scope

- No change to gesture publication timing or reveal physics.
- No repo-wide sweep of other `@Published` writes; anything adjacent found
  during implementation goes to a debt doc entry with call sites, filed
  separately.
- The just-shipped lease owner-loss settlement (`ownerPresentationEnded`)
  already publishes from a deferred task context and is unaffected.

## Verification

- The #TASK-2566 repro test is already written as the healthy assertion
  (`...DoesNotPublishDuringDismantle`): port `2206e5287` onto current main;
  the fix flips it red → green on the identical teardown path (no abort, no
  synchronous publication inside the dismantle window).
- New foreground coverage: root-surface transition through
  `updateUIViewController` performs no synchronous publish inside the update
  window; the deferred terminal publish lands within bounded main-actor
  turns; ownership residue is zero.
- `store.detach` with an active barrier: no synchronous publish; barrier
  falls within a turn.
- Staleness: detach-then-immediate-reattach (and detach-then-new-gesture)
  proves a stale deferred settle cannot clobber the newer occurrence.
- Existing reveal interaction, route stack, lease, and owner-loss suites stay
  green.

## Impact

- `GaryxHorizontalRevealInteraction`: deferred observable settlement for
  host-ownership entry points + stale-settle guard.
- `GaryxPresentationLeaseCoordinator.detach`: deferred barrier publication.
- `GaryxProductionRouteStack` dismantle/update paths become publish-free by
  construction.
- User-visible: the background teardown crash class is eliminated; terminal
  reveal/barrier settles land one main-actor turn later, which is invisible.
