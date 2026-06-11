# Transcript Monitor Event Provenance

## Problem

When a Claude Code run starts a background watcher with the harness `Monitor`
tool, every subsequent watcher event is pushed into the agent as an injected
notification, not pulled by a tool call. The agent typically reacts to each
event with a short assistant text segment.

In the transcript UI this renders as a series of assistant bubbles that appear
out of thin air: no tool call, no user message, nothing between them. A user
watching the thread cannot tell what triggered each segment, whether the run is
stuck, or whether the agent is still waiting on something. The thread status
shows a generic "thinking" indicator for the whole wait.

This is a real, recurring pattern for long-running agent work (build watchers,
log tailers, batch-job monitors), not an edge case.

## Observed Example

A quant agent fired a 10-seed simulation batch, then started a watcher:

```
Monitor: tail -f data/repair_batch.out \
  | grep -E --line-buffered "KILLED|PASSED|ERROR|fail|RESULTS" | ...
→ Monitor started (task bpwv0bfpn, timeout 3600000ms).
  You will be notified on each event. Keep working — do not poll or sleep.
```

For the next ~30 minutes the run produced zero tool calls. Each time the
batch log emitted a matching line, the agent received a notification and wrote
one short paragraph:

| Transcript bubble (assistant text)          | Actual trigger (invisible)              |
| ------------------------------------------- | --------------------------------------- |
| "三因子组合反而稀释（0.81 vs 1.36）…剩 3 发" | 11:26:44 `[KILLED] seed_07 sharpe=0.81` |
| "剩 2 发。"                                  | 11:30:43 `[KILLED] seed_08 sharpe=0.35` |
| "最后 1 发（capex+cash 双因子）。"            | 11:36:36 `[KILLED] seed_09 sharpe=0.83` |

The gateway per-thread event log confirms the gap: the last `[tool]` entry for
the run is `Monitor started`; the three text segments above have no logged
trigger at all.

## Mechanism (current pipeline)

1. The Claude Code SDK runs the watcher as a background task. Each matched
   line arrives on the SDK stream as a `Message::System` with subtype
   `task_notification`. The payload carries `task_id`, `tool_use_id` (of the
   originating Monitor tool call), `status`, and a `summary` string containing
   the event content. (`task_started` / `task_updated` are the sibling
   subtypes for lifecycle changes.)
2. `garyx-bridge/src/claude_provider.rs` consumes these messages silently. It
   uses them for exactly two internal purposes:
   - `update_claude_background_tasks` tracks which background tasks are still
     active so the bridge knows the run is alive.
   - A `task_notification` resets `result_seen = false` so the bridge keeps
     reading the stream after an intermediate Result message.
3. The notification is never forwarded as a `StreamEvent`, never appended to
   `session_messages`, never persisted in the transcript, and never written to
   the per-thread event log. Only the assistant text that follows it survives.

So the trigger is consumed at the bridge layer and the product surfaces
(desktop, mobile, channel streams) have no way to render it today.

## Design Goal

Make event-driven assistant output legible: a reader of the transcript should
be able to answer "what prompted this message?" and "what is the run waiting
on right now?" without opening gateway logs.

Concretely, design how to:

1. **Represent monitor/background-task events in the transcript.** Likely a
   low-emphasis system row (analogous to a collapsed tool-call row) between
   assistant segments, e.g. `⏳ monitor event · [KILLED] seed_08 sharpe=0.35`,
   attributable to the originating Monitor tool call via `tool_use_id`.
2. **Represent the waiting state.** While one or more background tasks are
   active and no tokens are streaming, "thinking" is misleading. The thread
   status could read as waiting on a named monitor (with elapsed time and
   timeout), on both the desktop transcript and mobile.
3. **Define the persistence contract.** Decide whether these rows are
   transcript messages (persisted via the normal thread-store write path, so
   history replays correctly) or ephemeral run-state decorations (live-only,
   reconstructed from run snapshots). Note the transcript is currently
   persisted at run end; live rendering and persisted history must agree.
4. **Keep noise bounded.** A chatty watcher can emit many events; consider
   coalescing consecutive events with no assistant text between them, and how
   channel surfaces (Telegram/Discord stream policies in
   `garyx-channels/src/plugin_tools.rs`) should treat these events — probably
   suppressed there, shown only in first-party transcript UIs.

## Constraints

- The bridge must keep its current internal uses (task liveness tracking,
  `result_seen` reset) regardless of presentation.
- Provider-agnostic naming: other providers may grow equivalent push events;
  avoid hard-coding Claude `task_notification` vocabulary into the UI model.
- Mobile presentation mapping belongs in `GaryxMobileCore` with SwiftPM tests;
  desktop and mobile should share one identity/labeling approach per existing
  UI contracts (`docs/agents/desktop-ui.md`, `docs/agents/mobile-ui.md`).
- Per repository contracts, transcript persistence changes must go through the
  thread-store write path, not read-route repairs.

## Open Questions

- Should monitor event rows be expandable to the full event payload, or is the
  one-line summary enough?
- Do `task_started` / terminal `task_updated` (completed, killed, timeout)
  deserve their own rows, so the start and end of a watch window are visible?
- How should multiple concurrent monitors be distinguished (label by command,
  by task id, by originating tool call)?
- Is there value in surfacing these events through the gateway API for
  third-party clients, or is this purely a first-party rendering concern?
