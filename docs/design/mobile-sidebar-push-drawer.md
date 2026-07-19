# Mobile Sidebar Push Drawer Design

> Historical note (2026-07-19): this document records the pre-P0-A A5 drawer
> implementation and its one-time migration plan. The former app-layer drag
> helpers and state described below have been removed; current behavior is
> owned by the shared horizontal-reveal state machine.

## Goal

Change the compact iOS sidebar from an overlay drawer into a push drawer, and
allow a left swipe that starts on the visible sidebar to close it.

## Historical Implementation Anchor

`GaryxShellView` owns the mobile shell layout in
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileViews.swift`.

On regular-width layouts it already uses a persistent sidebar:

- `HStack(spacing: 0)` lays out `GaryxThreadSidebar` beside
  `GaryxMainPanelView`.
- The sidebar is always present and `sidebarVisible` is reset to `false`.

On compact drawer layouts, `drawerBody(width:containerSize:)` then used a
`ZStack`:

- `GaryxMainPanelView` stays fixed at `x = 0`.
- A dimming layer is drawn above the main panel and handles tap-to-close plus
  the closing drag.
- `GaryxThreadSidebar` is drawn above the main panel and moved with
  `offset(x: revealWidth - width)`.
- A narrow clear strip near the reveal edge also receives the closing drag.

The former drag state supplied these inputs:

- A local drag offset drove interactive progress.
- `sidebarRevealWidth(for:)` normalizes open/closed progress.
- The opening helper only opened from the leading edge and delegated nested
  page behavior through the then-current leading-edge action.
- The closing helper handled leftward drags with a
  horizontal-axis threshold, but it is not attached to the sidebar content.

## Proposed Layout

Keep the persistent-width branch unchanged. Replace only the compact
`drawerBody` composition with a translated horizontal drawer rail:

```swift
ZStack(alignment: .topLeading) {
    HStack(spacing: 0) {
        GaryxThreadSidebar(showsInlineCloseButton: true)
            .frame(width: width)
            .simultaneousGesture(legacyCloseGesture(sidebarWidth: width))

        GaryxMainPanelView()
            .frame(width: containerSize.width, height: containerSize.height)
            .simultaneousGesture(legacyOpenGesture(sidebarWidth: width))
    }
    .frame(width: width + containerSize.width, height: containerSize.height)
    .offset(x: revealWidth - width)

    if revealWidth > 1, containerSize.width > revealWidth {
        Color.clear
            .frame(width: containerSize.width - revealWidth, height: containerSize.height)
            .offset(x: revealWidth)
            .contentShape(Rectangle())
            .onTapGesture { closeSidebar() }
            .simultaneousGesture(legacyCloseGesture(sidebarWidth: width))
    }
}
```

This produces these positions:

- Closed: rail offset is `-width`, sidebar is offscreen, main panel is at
  `x = 0`.
- Dragging open: rail offset follows `sidebarRevealWidth`, so the sidebar and
  main panel move together.
- Open: rail offset is `0`, sidebar is at `x = 0`, main panel begins at
  `x = width`.

For compact phones, `width == containerSize.width`, so the main panel is pushed
fully offscreen while the sidebar is open. For non-persistent wider layouts,
`width` remains capped at the current sidebar width, so the pushed main panel is
still partially visible to the right.

## Closing Interaction

The migration attached the former closing helper directly to
`GaryxThreadSidebar` with `simultaneousGesture`.

The current axis gate is already appropriate for avoiding vertical-list
conflicts:

- Minimum distance is `18`.
- A drag becomes horizontal only after the dominant movement is at least `14`.
- Horizontal movement must be greater than vertical movement by the current
  `1.5` ratio.
- Closing requires leftward movement.

This means vertical sidebar scrolling remains vertical, while a deliberate
leftward swipe closes the drawer. No row-level left-swipe actions are added.

Keep a clear, invisible close-capture layer only over the visible pushed main
panel area when `revealWidth > 1` and the pushed main panel is still visible in
the viewport:

- Tap outside the sidebar still closes the drawer.
- Left swipe on the visible pushed main panel still closes the drawer.
- The layer starts at `x = revealWidth`, uses
  `width = containerSize.width - revealWidth`, and is transparent, so the main
  panel is not visually covered.

The old visual dimming layer should be removed from compact drawer mode because
it reinforces the previous overlay behavior.

## State And Contracts

No gateway, routing, or Core data contract changes are needed. The change is
local SwiftUI composition:

- `sidebarVisible` remains the source of truth for open/closed state.
- The then-current leading-edge action remained the right-swipe-open guard,
  preserving the previous rule that the leading gesture matches the current
  page's top-left action.
- Existing button open, close button, tap-outside close, and leading-edge
  swipe-open remain supported.

## Validation Plan

Use the narrowest mobile checks:

- `swift test --package-path mobile/garyx-mobile` as a broad mobile Core
  regression check, not as direct coverage of this app-target shell diff.
- `xcodebuild -project mobile/garyx-mobile/GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build CODE_SIGNING_ALLOWED=NO`
- Install/launch the simulator app and capture a debug sidebar screenshot with
  the existing screenshot script.
- Exercise a left swipe from the visible sidebar in the simulator and capture
  the closed result or an equivalent observable state transition.

## Risks

- A too-aggressive sidebar gesture could interfere with vertical list scrolling.
  Reusing the existing horizontal-axis classifier keeps that risk bounded.
- A fully open compact phone drawer has no visible outside region, so the new
  sidebar left-swipe is the important close path in addition to the header close
  button.
- The pushed main panel may show a narrower visible area on medium-width
  devices. That is expected because the sidebar width is unchanged.
