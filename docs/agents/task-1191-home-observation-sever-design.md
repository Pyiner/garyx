# Task 1191 Home Observation Sever Design

> Historical note (2026-07-19): this inventory describes the pre-P0-A route
> observation graph. A4c completed the container migration, so “currently”
> below means the TASK-1191 implementation baseline rather than current code.

## Scope

This task implements only the object-observation severing step described in
`docs/design/home-list-rebuild-v4.md` section 2. The task label calls this M4;
the merged design table currently labels the same step as M3. The implementation
does not cut over the home render source, does not stop or rewrite selected
thread streams, does not migrate `GaryxMobileModel` itself to `@Observable`, and
does not replace `LazyVStack` with native `List`.

## Reachable View Inventory After Pop To Home

Static home route:

- `GaryxRootView`: always mounted. It currently observes `GaryxMobileModel` for
  `hasGatewaySettings`, `connectionState`, root actions, debug gateway-switcher
  binding, URL handling, settings sheet state, and connect refresh task startup.
- `GaryxShellView`: mounted when the connection gate passes. It already observes
  narrow stores only: `GaryxShellChromeStore`, `GaryxNavigationDrawerStore`,
  the former root-path observation store, `GaryxRouteNotFoundStore`, and
  `GaryxHomeThreadListStore`. It has no `@EnvironmentObject GaryxMobileModel`.
- `GaryxRootNavigationView`: home root `NavigationStack`. It observed
  the former root-path observation store, `GaryxRouteNotFoundStore`, and
  `GaryxHomeThreadListStore`. When `popToHome` has completed the path is empty,
  so `GaryxRootRouteContentView` and its route destinations are not mounted.
- `GaryxHomeThreadListView`: home list. It observes `GaryxHomeThreadListStore`.
  It passes row data and actions as values/closures.
- `GaryxHomeThreadButton`, `GaryxSidebarThreadRowView`,
  `GaryxHomeHeaderView`, section headers, empty/loading rows, swipe rows, and
  row badges: no `@EnvironmentObject GaryxMobileModel` on the home root path.
- `GaryxSidebarThreadAutoLoadFooter`: mounted at the bottom of the home list.
  It currently observes `GaryxMobileModel` for `isLoadingMoreThreads`,
  `hasMoreThreadSummaries`, and `loadMoreThreads()`.
- `GaryxNavigationDrawerView`: mounted beside the home list even while closed.
  It observes `GaryxNavigationDrawerStore` and gets the debug gateway-switcher
  binding from `GaryxShellView`. It has no `@EnvironmentObject GaryxMobileModel`.
- `GaryxSidebarGatewayIdentityControl`: mounted in the drawer. It currently gets
  a binding that originates from `$model.debugShowsGatewaySwitcher`.
- `GaryxGlobalErrorToastHost`: overlaid by `GaryxRootView` and also by gateway
  setup views. The home overlay is mounted in steady state and currently
  observes `GaryxMobileModel` for `lastError`. There are three current
  callsites: the root overlay and two gateway setup overlays.

Reachable only after navigation, sheets, or setup state:

- `GaryxRootRouteContentView` and conversation/panel destinations observe the
  model, but they are not mounted after `popToHome` because the navigation path
  is empty.
- `GaryxWorkspaceBotsView` and the older sidebar section helpers under it
  observe the model, but they are panel content, not the home root list.
- `GaryxGatewaySwitcherSheet` observes the model, but it is sheet content
  created only after the user opens the switcher.
- `GaryxGatewaySetupView` observes the model when no gateway is configured or
  when the settings sheet is presented; that is outside the ready home steady
  state.

## Proposed Shape

Add one narrow Swift Observation store in `GaryxMobileCore`:

- `GaryxHomeObservationStore` is `@MainActor @Observable`.
- It owns four public value fields used by the ready home surface:
  `isGatewayConfigured`, `connectionState`, `debugShowsGatewaySwitcher`, and
  `showsSettings`.
- It owns global error state used by the home toast host: `lastError`.
- It also owns pagination values needed by the footer:
  `isLoadingMoreThreads` and `hasMoreThreadSummaries`.
- Its setters are equality-gated so reapplying identical values does not notify
  observers.
- It keeps an ignored diagnostic `publishCount` for tests. Equality-gated
  no-op reapplies must not bump it; changed mirrored values must bump it.

Bridge `GaryxMobileModel` into the narrow store:

- Add `let homeObservationStore = GaryxHomeObservationStore()` to the model.
- Synchronize it from the existing gateway URL and connection-state `didSet`s,
  and add new `didSet`s for `isLoadingMoreThreads`,
  `hasMoreThreadSummaries`, `showsSettings`, and
  `debugShowsGatewaySwitcher`.
- Add a `lastError` setter hook so every existing `model.lastError = ...` write
  updates the narrow store without replacing all call sites.
- `GaryxMobileModel` remains the write-side source of truth for
  `showsSettings` and `debugShowsGatewaySwitcher`. Root bindings read from the
  store but set the model; the model `didSet`s mirror back to the store. The
  store is not allowed to become a second independent source for those values.
- Do not synchronize `messages` or `renderSnapshotsByThread` into the home
  observation store. Their didSet paths must not call any home-observation
  setter.

Rewrite the static home leaks:

- The app root injects both `.environmentObject(model)` for non-home
  descendants and `.environment(model.homeObservationStore)` for static home
  chrome.
- `GaryxRootView` deletes `@EnvironmentObject private var model` and receives
  `GaryxMobileModel` as a non-observing initializer parameter from
  `GaryxMobileApp`. It reads `@Environment(GaryxHomeObservationStore.self)` for
  the connection gate, settings sheet binding, and debug binding. It uses the
  plain model reference only for actions, async closures, URL handling, and
  binding setters. Do not replace this with `@ObservedObject`, `@StateObject`,
  or another property wrapper that subscribes to `objectWillChange`.
- `GaryxRootView` builds the settings/debug bindings as
  `get: { store.value }` and `set: { model.value = $0 }`. Keeping
  `.environmentObject(model)` at the app root is mandatory so pushed routes,
  sheets, setup, and conversation views still resolve their existing model
  environment object.
- `GaryxSidebarThreadAutoLoadFooter` reads the narrow store and receives a
  load-more action from root. This can be a closure threaded through
  `GaryxShellView`/`GaryxRootNavigationView`/`GaryxHomeThreadListView`, or an
  environment action analogous to existing root actions; either way the footer
  must not read `GaryxMobileModel`.
- `GaryxGlobalErrorToastHost` reads the narrow store and receives a clear-error
  closure from all three root/setup callers. The closure takes the visible
  message and clears the model only if it still matches, preserving the current
  stale-error guard.
- `GaryxSidebarGatewayIdentityControl` keeps a `Binding<Bool>`, but the binding
  is built with a store read and a model write instead of `$model`.

## Observation Gate

Add two deterministic test layers:

- Core SwiftPM tests cover the `GaryxHomeObservationStore` contract directly:
  equality-gated reapply of the same values must not bump `publishCount` and
  must not notify; a changed home value must bump `publishCount` and notify
  through `withObservationTracking`.
- Add a small app unit-test target for the real model bridge. It instantiates
  `GaryxMobileModel` with isolated defaults, tracks the exact static ready-home
  reads on `model.homeObservationStore`, then runs two fresh tracking cycles:
  1. write real conversation data through `model.setRenderSnapshot(...)` and
     `model.setMessages(...)`; `onChange` must not fire;
  2. re-register the same reads, then write home data through model-owned state
     such as pagination or `lastError`; `onChange` must fire.
- The app bridge test requires an Xcode unit-test target because
  `GaryxMobileModel` lives in the app target, not in `GaryxMobileCore`. Add a
  `bundle.unit-test` target in `mobile/garyx-mobile/project.yml`, set
  `TEST_TARGET_NAME: GaryxMobile`, add it to the `GaryxMobile` scheme `test`
  action, run `xcodegen generate`, and commit the regenerated `.xcodeproj`
  changes. A UI-test target or SwiftPM `swift test` cannot cover this bridge.

This is not a visual SwiftUI test. `withObservationTracking` can prove only
Swift Observation read invalidation; it cannot observe an accidental
`ObservableObject.objectWillChange` subscription. Therefore the actual sever
proof is the combination of the app bridge test above, the compile gate, and the
grep gate proving no ready-home static view still observes `GaryxMobileModel` or
uses `$model`. The code-review checklist must also verify that the
`messages`/`renderSnapshotsByThread` write paths do not call any
home-observation setter.

## Validation

- `rg '@(EnvironmentObject|ObservedObject|StateObject)\\b.*GaryxMobileModel|@Environment\\(\\s*GaryxMobileModel|\\$model\\.'`
  over the mobile app, then re-check each hit against the inventory above. The
  known static-home hits at `GaryxMobileViews.swift:13`,
  `GaryxMobileSidebarViews.swift:846`, and
  `GaryxMobileStatusComponents.swift:93`, plus root `$model` bindings, must be
  gone. Remaining hits must classify as route, sheet, panel, setup,
  conversation, or picker content outside the popToHome steady-state tree.
- `cd mobile/garyx-mobile && swift test`
- `cd mobile/garyx-mobile && xcodegen generate`
- `cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -scheme GaryxMobile -sdk iphonesimulator -destination 'platform=iOS Simulator,name=iPhone 16' -configuration Debug CODE_SIGNING_ALLOWED=NO test`
- `cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -scheme GaryxMobile -sdk iphonesimulator -configuration Debug CODE_SIGNING_ALLOWED=NO build`
