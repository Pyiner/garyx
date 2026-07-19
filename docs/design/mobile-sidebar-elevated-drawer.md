# Mobile Sidebar Elevated Drawer Design

> Historical note (2026-07-19): this document records the pre-P0-A A5 drawer
> shape and interaction baseline. The shared horizontal-reveal state machine
> now owns these gestures, and A4c removed the former leading-edge action.

## Goal

Tune the compact iOS push drawer so the opened state reads like the ChatGPT iOS
sidebar reference: the sidebar and pushed main panel have a clear boundary, the
main panel looks raised above the sidebar, and the visible main-panel edge has
large continuous top and bottom rounding.

## Reference Observations

The reference screenshot has four relevant traits:

- The sidebar is not a full-screen replacement. The pushed main panel remains
  visible after the drawer is open.
- A fine vertical divider sits exactly at the sidebar/main-panel boundary.
- The main panel casts a soft shadow back over the sidebar, so the right surface
  reads as the higher layer.
- The main panel's visible leading corners are rounded at both the top and the
  bottom. The rounding is large enough to read as a card silhouette, not a
  small list-card radius.

The gray capsule close handle on the visible main-panel side is useful but
secondary. The core pass/fail criteria are the divider, elevation, and rounded
main-panel edge.

## Historical Local Shape

`GaryxShellView.drawerBody(width:containerSize:)` in
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileViews.swift` already uses the
right interaction model from the previous push-drawer change:

- A single horizontal rail contains `GaryxThreadSidebar` followed by
  `GaryxMainPanelView`.
- `sidebarRevealWidth(for:)` drives the rail offset, so the sidebar and main
  panel move together during interactive drag.
- The sidebar receives the closing drag; the main panel receives the opening
  drag; a clear close-capture layer preserves tap-outside and swipe-close on the
  exposed pushed panel.
- The whole drawer is clipped to the screen frame.

Two visual gaps remain:

- On compact width, `drawerSidebarWidth(for:)` currently returns the full screen
  width, so the main panel is fully pushed offscreen when the drawer is open.
- `GaryxMainPanelView` is unshaped in drawer mode, so the boundary is a hard
  vertical cut with no card silhouette or raised layer.

## Proposed Layout

Keep the rail model and gesture ownership. Change only the compact drawer
presentation.

1. Reserve a small visible main-panel peek in compact mode:
   - Add a `drawerMainPanelPeekWidth` constant.
   - Return `min(sidebarWidth, max(0, containerSize.width - drawerMainPanelPeekWidth))`
     from `drawerSidebarWidth(for:)` on compact layouts.
   - Keep the existing `min(sidebarWidth, containerSize.width * 0.92)` rule for
     non-compact non-persistent layouts.

2. Wrap the drawer-mode main panel in an elevated presentation layer:
   - Compute `drawerProgress = revealWidth / width`.
   - Use progress to interpolate the visible radius from `0` when closed to a
     large continuous radius when the drawer is open.
   - Clip the main panel to an `UnevenRoundedRectangle` that rounds
     `topLeading` and `bottomLeading` only. The trailing corners remain square
     because the panel is larger than the viewport and its trailing edge is not
     the visual boundary.
   - Apply shadow after clipping so the shadow follows the rounded silhouette.

3. Draw the divider at the same boundary:
   - Add a 1-pixel hairline overlay aligned to the main panel's leading edge.
   - Fade it in with drawer progress so the closed fullscreen state remains
     visually unchanged.
   - Use an adaptive primary-opacity hairline so both light and dark mode have a
     crisp but not heavy boundary.

4. Keep the outer screen clip:
   - Leave the outer `.clipped()` on the `drawerBody` root to prevent the rail
     from drawing outside the device bounds.
   - Do not add clipping to the `HStack` rail or the sidebar, because the main
     panel's shadow must be allowed to render leftward across the boundary.

5. Optionally add the close handle:
   - Render it only when the drawer is mostly open and there is enough exposed
     main-panel width.
   - Put it above the close-capture layer with the same close gesture so it can
     be tapped or dragged without stealing the core outside-close behavior.

## Interaction Impact

The change preserves the previous interaction contract:

- Button open and header close still use `setSidebarVisible`.
- Leading-edge swipe open was unchanged and still gated by the then-current
  leading-edge action.
- Left swipe on the visible sidebar still closes the drawer.
- Tap outside and left swipe on the exposed pushed main panel still close the
  drawer through the clear close-capture layer.

The only behavioral change is that compact phones now leave a visible pushed
main-panel strip when the drawer is open. This is required for the reference
visual language and gives the existing tap-outside area a visible target.

## Risks And Mitigations

- Shadow clipping: applying shadow inside the main-panel wrapper but keeping the
  rail unclipped allows the elevation to appear over the sidebar; the final root
  clip only cuts pixels outside the screen.
- Closed-state artifacts: radius, divider opacity, shadow opacity, and optional
  handle opacity are all driven by reveal progress, so the normal closed app
  remains rectangular and flat.
- Small screens: the peek width must not make the sidebar too narrow. Cap the
  sidebar at the existing `sidebarWidth`, but reserve only a modest fixed peek
  so compact sidebars still have enough row width.
- Gesture regression: no gesture thresholds or axis classification change, so
  vertical sidebar scrolling remains protected by the existing horizontal-axis
  gate.

## Validation Plan

- Run `swift test --package-path mobile/garyx-mobile`.
- Run the iOS Debug simulator build:
  `xcodebuild -project mobile/garyx-mobile/GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build CODE_SIGNING_ALLOWED=NO`.
- Capture simulator screenshots through the Garyx screenshot script using the
  existing debug sidebar route.
- Capture an open-sidebar screenshot and a mid-drag/closing state if the
  automation path can hold the gesture state reliably.
- Create a local side-by-side comparison image with the reference screenshot
  and the updated simulator screenshot, then use it for design QA against
  divider, elevation, radius, and interaction-state regressions.
