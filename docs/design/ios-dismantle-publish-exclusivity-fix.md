# iOS Representable Teardown Publish: Deferred Observable Settlement Design

Status: historical `#TASK-2587` design, converged with `#TASK-2586` after
`#TASK-2587` landed first. The combined implementation keeps this document's
lifecycle-safety and stale-detach contracts, with
`GaryxObservableStateSettler` replacing the original per-site generation and
`Task` deferrals.
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
`hostOwnership`, driver invalidation, and terminal semantic state synchronously
— business truth never lags. `GaryxObservableStateSettler` independently
projects that semantic state immediately in ordinary event contexts or on the
next main-queue turn for representable lifecycle callers. Production make,
update, and dismantle choose the graph-safe timing explicitly at their
composition points; there are no wall-clock timers or per-site async tasks.

Gesture-driven paths (`beginGesture`, `updateGesture`, `endGesture`,
`cancelGesture`, display-link settle frames) keep synchronous publication:
they run from UIKit event contexts where publishing is legal and per-frame
latency matters.

### 2. Stale settlement must not touch newer state

The settler owns one coalesced deferred projection flush. If a newer host
occurrence attaches or a new gesture changes semantic state before that flush,
execution reads the latest semantic value rather than a captured detach value.
The obsolete occurrence can therefore never mutate current bookkeeping or
overwrite the newer projection. Terminal zero-residue assertions apply to the
synchronously settled semantic state; targeted tests separately assert the
observable projection's eventual convergence.

### 3. Barrier publication on coordinator detach defers the same way

`GaryxPresentationLeaseCoordinator.detach` must not synchronously write
`hasPresentationBarrier` from a dismantle context; it records semantic false
through the same settler and defers only projection. Production attach/make
also selects graph-safe projection timing, closing the active-barrier to
barrier-free remount counterexample. Other lease acquire/terminal flows run in
legal event contexts and retain immediate projection.

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

- `GaryxHorizontalRevealInteraction`: synchronous semantic terminalization plus
  settler-owned observable projection for host-ownership entry points.
- `GaryxPresentationLeaseCoordinator`: the same settler-backed barrier
  projection across attach and detach.
- `GaryxProductionRouteStack` make/update/dismantle paths become publish-free
  by construction.
- User-visible: the background teardown crash class is eliminated; terminal
  reveal/barrier settles land one main-actor turn later, which is invisible.
