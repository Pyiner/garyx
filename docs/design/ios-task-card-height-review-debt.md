# iOS Task-Card Height Review Debt

Status: recorded during the adversarial review of the iOS task-notification
card-height fix. The fix passed review. The relationship below is an adjacent
observation for separate investigation, not a defect in this change.

## Possible relationship to the scroll-to-bottom bug

The height bug and the parallel scroll-to-bottom report share a rendering
surface: both involve a thread containing the same short and long task
notification cards. The committed test fixture sanitizes that source thread as
`thread::task-notification-card-height-repro`.

The reviewer independently rendered the short card through the app-level path
on iPhone 17 Pro Max with iOS 26.5. Before the fix, the two-line card measured
336 points tall under the parent view's expanding proposal; after the fix it
measured 132 points. A card that over-reports its height inflates transcript
content size, so a shared height-measurement cause for the scroll symptom is
plausible.

This change does not establish that shared root:

- It changes only the collapsed card's displayed body height and the
  `GaryxMobileCore` presentation policy.
- It does not modify scroll, bottom-anchor, content-offset, content-size, or
  `ScrollViewReader` behavior.
- The natural Markdown measurement reported upstream remains unchanged.

Disposition: keep the scroll-to-bottom bug as an independent task. Its own
deterministic reproduction must establish whether card over-measurement affects
bottom anchoring. Do not fold a scroll fix into the task-card height change.

## Other adjacent observations

None.
