# Chat Transcript Scroll Reliability

## Scope

This design covers the macOS desktop transcript in
`desktop/garyx-desktop/src/renderer/src/app-shell/AppShell.tsx` and
`desktop/garyx-desktop/src/renderer/src/app-shell/components/ThreadPage.tsx`,
plus the iOS transcript in
`mobile/garyx-mobile/App/GaryxMobile/GaryxMobileConversationViews.swift`.

The goal is to make tail updates deterministic:

- Sending a message keeps the transcript pinned to the newest visible content.
- Receiving remote input, assistant deltas, run snapshots, or history reconcile
  while the selected thread is visible keeps the transcript pinned to the newest
  content unless the user is intentionally browsing older history.
- Opening or switching to an iOS thread renders the first selected transcript
  frame with visible content instead of an empty LazyVStack viewport that only
  repairs after a manual drag.

## Current macOS Behavior

`AppShell.tsx` has the right primitives but applies them inconsistently:

- `requestMessagesBottomSnap(threadId, true)` forces sticky bottom state and a
  one-shot bottom snap. It is used for local sends, queued steering, new-thread
  draft promotion, and selected-thread identity changes.
- `requestMessagesBottomSnap(threadId)` only records a pending snap. Whether it
  scrolls depends on `shouldStickMessagesToBottomRef.current`.
- `onMessagesScroll` rewrites `shouldStickMessagesToBottomRef.current` from the
  current DOM distance to the bottom.
- Older-history pagination stores `pendingMessagesPrependAnchorRef` and then
  adjusts `scrollTop` after prepending, which correctly avoids pulling the user
  to the tail while browsing older turns.

The fragile paths are:

- Selected-thread history load calls `requestMessagesBottomSnap(threadId)` in
  `onBeforeLoad`.
- Passive selected-thread polling calls `requestMessagesBottomSnap(threadId)`
  before `applyRemoteTranscript`.
- WebSocket `user_ack` and remote pending input materialization also call
  `requestMessagesBottomSnap(threadId)` without force.
- `scheduleHistoryRefresh` applies canonical or merged transcript data without
  expressing whether that incoming transcript belongs to the active tail.

Trigger condition: after any event that sets `shouldStickMessagesToBottomRef` to
`false` (manual upward scroll, prepend preservation, or a geometry measurement
while content is mid-layout), the next remote tail update can change the
transcript without scrolling because the pending snap is non-forced. The user
then sees a half-way position and must scroll manually to reach the newest
message.

## Current iOS Behavior

`GaryxMobileConversationViews.swift` already uses `ScrollViewReader`, a bottom
anchor, a tail-thinking anchor, geometry preferences, and a retrying
`scheduleScrollToConversationTail(...)`. This improves ordinary cases but still
leaves two timing gaps:

- `LazyVStack` does not have to materialize every row during the first layout
  pass. A `proxy.scrollTo` issued while the bottom or thinking anchor has not
  been realized can be ignored by SwiftUI.
- The retry loop is gated by `isNearConversationBottom` or
  `shouldRepairVisibleTailGap`. Those values are derived from geometry
  preferences that are also unavailable or stale during the first selected
  thread frame. A failed first scroll can therefore stop retrying before the
  actual content height is known.

The white-screen symptom matches this timing: the selected thread has messages
in the model, but the initial scroll target was requested before the lazy stack
had laid out the rows. A manual drag invalidates the scroll view layout, the
lazy rows materialize, and the messages appear.

## Proposed Fix

### macOS

Keep the existing scroll container and pagination preservation. Add a small
tail-scroll scheduler with two properties:

- A forced snap stays alive for several animation frames instead of a single
  immediate `scrollTo`. This covers React commits, streamed Markdown growth,
  images/files loading, and composer-height updates.
- Tail updates use the forced mode when they are selected-thread tail events,
  while older-history prepends keep the existing anchor preservation path.

Concrete changes:

1. Extend `scheduleMessagesScrollToLatest` to retry over a short frame budget
   when sticky bottom is enabled or the caller requested a force snap.
2. Keep `forceMessagesBottomSnapRef` true until the retry budget has run or a
   real user scroll marks the view away from the bottom.
3. Call `requestMessagesBottomSnap(threadId, true)` for selected-thread
   `user_ack`, remote pending input materialization, passive selected-thread
   polling, and selected-thread history load when it is not an older-page
   prepend.
4. Do not force scroll in `applyOlderRemoteTranscriptPage`; the existing
   prepend anchor remains the protection for intentional upward browsing.

Tradeoff: a selected-thread remote update will pull the user to the tail even if
they had scrolled up in that same visible thread. That matches the task goal for
new messages, and the explicit older-history prepend path still preserves
position while loading historical pages.

### iOS

Keep the view-local implementation. Add an explicit tail scroll request model so
first layout and data changes are not inferred solely from geometry:

1. Track a pending tail scroll generation and a request kind:
   `openingThread`, `tailUpdate`, or `manual`.
2. Schedule retries on the next run loop plus several delayed main-actor passes
   for first open and tail updates.
3. During first open or a model transition from empty to non-empty, allow retry
   attempts even when `isNearConversationBottom` has not been measured yet.
4. Add a hidden non-lazy bottom sentinel after the `LazyVStack` content and
   scroll to it for normal tail positioning. The existing thinking-anchor path
   can remain for active empty-thinking states, but the bottom sentinel is the
   stable target after transcript rows appear.
5. Keep `shouldPreserveScrollForPrependedHistory` as the only path that blocks
   a tail scroll when older rows are inserted above the current viewport.

Tradeoff: retries add a few scheduled main-actor scroll calls during selected
thread open and tail updates. They are short-lived, view-local, and cancelled by
generation when the selected thread changes.

## Validation Plan

- Desktop: run renderer build and smoke tests for the Electron app:
  `cd desktop/garyx-desktop && npm run build:ui && npm run test:smoke`.
- iOS: run Swift package tests and a simulator build:
  `swift test --package-path mobile/garyx-mobile` and
  `xcodebuild -project mobile/garyx-mobile/GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build CODE_SIGNING_ALLOWED=NO`.
- Screenshot evidence: use `garyx-app-screenshots` against the real local
  gateway for macOS and the iOS simulator. Capture a selected thread before and
  after sending a message, and capture iOS immediately after switching/opening a
  thread without manual dragging.

## Impact

The change is scoped to transcript scroll orchestration. It does not change
thread data contracts, rendering models, message merging, pagination limits,
composer behavior, or gateway APIs.
