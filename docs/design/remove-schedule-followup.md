# Remove `schedule_followup` MCP Tool And Its Followup Chain

Date: 2026-07-23. Requested by the user: delete the `schedule_followup` MCP
tool. This is a full removal of the tool and its dedicated support chain — no
dead code, no compatibility shims, no deprecation stubs.

## Goal

The Garyx MCP server no longer exposes `schedule_followup`. Everything that
exists only to serve that tool is deleted: the tool handler, the
system-scheduled one-shot followup job kind, the followup dispatch/retry
driver, the followup metadata body builder, prompt guidance, docs, and tests.

## Scope: Delete

- `garyx-gateway/src/mcp/tools/schedule_followup.rs` — entire file.
- `garyx-gateway/src/mcp/tools/mod.rs` — module declaration and the
  `followup_job_id` test-visibility note.
- `garyx-gateway/src/mcp.rs` — `ScheduleFollowupParams`, the
  `schedule_followup` tool method, and the tool name in the server
  description string (`"Garyx MCP server. Tools: ..."`).
- `garyx-gateway/src/automation/engine/execution.rs` — the followup retry
  driver: `build_followup_body`, `dispatch_internal_followup_with_retry`,
  `dispatch_internal_followup_once`, `run_followup_with_retry`,
  `FollowupAttemptError`, and the firing path that renders
  `<garyx_followup_metadata>` synthetic user turns. If any helper is shared
  with non-followup jobs, keep the shared part and delete only the followup
  entry points — verify actual call graph, do not guess.
- `garyx-gateway/src/automation/engine/mod.rs` — `build_followup_body`
  re-export, the "system-managed jobs are filtered out of listings" behavior,
  and the `(thread_id, run_id)` dedupe helper, in each case **iff** their only
  consumer is the followup chain. The system-managed marker itself may have
  other consumers (e.g. legacy quota-resend cron handling); check before
  removing the marker concept.
- `garyx-gateway/src/automation/engine/model.rs` — the one-shot-jobs-disabled
  / dropped-followup-is-terminal doc contract, and any followup-specific
  variants.
- `garyx-models/src/config.rs` — the system-scheduled internal-dispatch
  payload struct (`SystemScheduledPayload` area, lines ~934–1058): the
  followup job kind variant and its fields. Delete the serde variant
  outright.
- `garyx-bridge/src/gary_prompt.rs` — the prompt line instructing agents to
  call `mcp__garyx__schedule_followup` for delayed resume. `ScheduleWakeup`
  remains disabled in this runtime; if agents still need to be told not to
  rely on `ScheduleWakeup`, keep that half of the sentence without pointing
  at a replacement. Update `garyx-bridge/src/gary_prompt/tests.rs`
  accordingly.
- `garyx-models/src/provider.rs` (~line 495) — comment/logic that justifies
  disabling `ScheduleWakeup` by pointing to `schedule_followup`; reword to
  stand on its own (the wakeup mechanism is still absent in single-turn
  provider mode).
- `garyx-gateway/src/automation/debug_api.rs` — followup-triage rationale
  comments and any followup-specific debug surface. The debug API itself
  stays if it serves regular automations.
- Tests referencing the tool or the followup chain:
  `garyx-gateway/src/automation/engine/tests.rs`,
  `garyx-gateway/src/routes/api_tests.rs`,
  `garyx-bridge/src/multi_provider/tests.rs`,
  `garyx-bridge/src/multi_provider/run_management/tests.rs`.
- Docs: delete `docs/schedule-followup.md` and
  `docs/schedule-followup-observability.md`; update `docs/concepts/mcp.md`
  tool list; update the Time And Timezone bullet in
  `docs/agents/repository-contracts.md` (drop "`schedule_followup` responses,
  followup metadata" from the human-readable-sinks list). If the root
  `AGENTS.md`/`CLAUDE.md` mention the tool, update both in the same commit
  (they are intentionally identical mirrors).

## Scope: Keep (do not touch)

- The automation engine's regular cron/product automations (Daily/Monthly,
  user cron jobs) and their execution/retry machinery.
- The `internal_inbound` internal-dispatch front door. It has other synthetic
  user-turn producers (task notifications etc.); only the followup producer
  goes away. Reword its "followups, ..." example comment.
- MCP tools `status`, `search`, `capsule_create`, `capsule_update`,
  `capsule_list`.
- Unrelated files whose "followup" is generic test/fixture naming (next-turn
  semantics), e.g. `garyx-models/src/transcript_run_state.rs`,
  `garyx-channels/src/committed_replay.rs`,
  `claude-agent-sdk/src/client/tests.rs`, `garyx/src/commands/meeting.rs`,
  mobile composer files. Leave them alone.
- Historical design docs under `docs/design/` that merely mention the tool as
  past context: they are point-in-time records, not living contracts.

## Data / Compatibility

- Live config check (2026-07-23): `~/.garyx/garyx.json` has `cron.jobs: []`
  and zero followup mentions — no pending followup job exists anywhere.
- Garyx policy: no backward compatibility for old versions. Delete the serde
  variant with no unknown-kind tolerance, no migration, no cleanup pass.

## Behavior Change (intentional, user-approved)

Agents lose the delayed self-resume mechanism entirely. `ScheduleWakeup` was
already disabled; after this change there is no replacement and the prompt no
longer advertises one. This is the requested product behavior.

## Validation

- `cargo test -p garyx-gateway --lib`
- `cargo test -p garyx-models`
- `cargo test -p garyx-bridge`
- Full-repo `rg -i 'schedule_followup|garyx_followup_metadata'` returns hits
  only in historical `docs/design/*.md` (including this file) — nothing in
  code, living docs, prompts, or tests.
- Workspace builds clean (`cargo check --workspace` or tier1 fast script).
