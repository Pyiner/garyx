# iOS Send-Anchor Review Debt

Status: recorded during the adversarial review of `#TASK-2680`. The
send-anchored transcript implementation passed review. The finding below is
adjacent and pre-existing, is not caused by that implementation, and requires
separate investigation.

## New-thread title rewrite

During the required iPhone 17 Pro Max / iOS 26.5 light-mode walkthrough, a new
thread's first-send transcript behaved correctly, but its navigation title was
later replaced by a server-generated title unrelated to the submitted prompt.
The transcript rows, row identity, thinking handoff, send anchor, and scroll
geometry remained correct.

This observation is outside the send-anchor scope:

- The reviewed change does not modify thread title generation, title metadata,
  or title rendering.
- The behavior occurs after the transcript's local-send presentation and does
  not affect the send-anchored state machine.
- No title-path root cause was established during the scroll-focused review.

Disposition: investigate new-thread title ownership and server title
generation in an independent task with its own deterministic reproduction. Do
not fold a title fix into the send-anchored transcript change.
