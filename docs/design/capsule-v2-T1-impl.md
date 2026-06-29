# Capsule v2 T1 Implementation Design

Status: implementation design for T1 only. This document intentionally covers the backend render contract and bridge write side; it does not implement desktop or iOS capsule-card UI rendering.

Source of truth: `docs/design/capsule-v2.md`, especially section 5 (chat render-state design), section 10 (T1 scope), and section 11 (risk table).

## Goals

- Persist a self-contained `capsule_attached` committed control marker for successful `capsule_create` / `capsule_update` tool results.
- Route marker writes through the bridge run-long `transcript_controls` accumulator so `build_run_record_drafts` includes the marker in both streaming partial append and terminal reconcile.
- Extend `garyx-models` render-state output with `RenderUserTurnRow.capsule_cards`, not a new row or activity enum variant.
- Keep reducer provider-agnostic: it consumes only committed `capsule_attached` controls, never raw provider tool result shapes and never the capsule DB.
- Add decoder tolerance/defaults for desktop TypeScript and iOS Swift models so missing or future fields do not drop frames.

## Non-goals / red lines

- No desktop/iOS capsule-card UI rendering in T1.
- No MCP handler/router direct append of capsule markers.
- No `range_rewrite` / `append_range_rewrite_marker` reuse for capsule business markers.
- No side-input DB join in render snapshots; no `garyx-models` dependency on gateway DB types.
- No real user names, IDs, email addresses, home directories, or tokens in docs/fixtures/tests. Fixture captures are sanitized to synthetic `thread::fixture-*`, `run::fixture-*`, `01900000-*`, `/Users/test/...`, and placeholder titles.

## Code surfaces

### Bridge write side

Files:

- `garyx-bridge/src/multi_provider/persistence.rs`
- `garyx-bridge/src/multi_provider/run_management.rs`
- `garyx-bridge/src/multi_provider/persistence/tests.rs` and/or `run_management` focused tests
- `test-fixtures/capsules/provider-results/*.json`

Planned helpers:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct CapsuleMutationAttachment {
    action: RenderCapsuleActionLike, // local bridge enum or string: created/updated
    capsule_id: String,
    title: String,
    revision: i64,
}

fn capsule_tool_action_from_name(tool_name: &str) -> Option<CapsuleAction>;
fn extract_capsule_attachment_from_tool_result(
    message: &ProviderMessage,
    tool_names_by_id: &HashMap<String, String>,
) -> Option<CapsuleMutationAttachment>;
fn capsule_attached_control_record(
    thread_id: &str,
    run_id: &str,
    attachment: CapsuleMutationAttachment,
    after_content_count: usize,
) -> RunControlRecord;
```

Implementation notes:

1. Maintain a per-run `tool_use_id -> tool_name` map in the streaming persistence worker next to `transcript_controls`.
   - On every `StreamEvent::ToolUse`, store non-empty `message.tool_use_id` and non-empty `message.tool_name`.
   - This must cover Claude Code, where the later `ToolResult` has `tool_name = None` but the prior `ToolUse` is named `mcp__garyx__capsule_create|update`.
2. When `StreamEvent::ToolResult` is applied and `is_error != Some(true)`, run the extractor after `snapshot.apply_stream_event` so `snapshot.session_messages.len()` reflects the committed content count including that tool result.
3. If extraction succeeds and the run has a `bridge_run_id`, push `RunControlRecord::new("capsule_attached", ...)` into `transcript_controls` with `after_content_count = 1 + snapshot.session_messages.len()`.
   - The `+1` accounts for the synthetic user content message included by `build_run_messages`.
   - Payload fields: `capsule_id`, `revision`, `action`, `title`.
   - Envelope must be exactly from `RunControlRecord::new`: top-level `role:"system"`, `kind:"control"`, `internal:true`, `internal_kind:"control"`, nested `control.kind:"capsule_attached"`.
4. Reuse the same helper in both the normal command drain and the `try_recv` coalescing drain to avoid divergent behavior.
5. Do not emit duplicate controls for the same completed result if a provider repeats an identical event in one run. The first implementation can track marker identity `(tool_use_id, capsule_id, revision, action)` in a `HashSet`; if `tool_use_id` is missing, use `(after_content_count, capsule_id, revision, action)`.

Provider-aware extractor details:

- Tool-name recognition accepts:
  - Codex direct `mcp:garyx:capsule_create` / `mcp:garyx:capsule_update`.
  - Claude direct/correlated `mcp__garyx__capsule_create` / `mcp__garyx__capsule_update`.
  - Payload self-identification as fallback when tool name is absent or a future provider wraps it differently.
- Payload parsing walks common real-world nested shapes and attempts to parse JSON strings recursively:
  - Direct object result.
  - Object string fields such as `result`, `text`, `content`, `output`.
  - Arrays such as MCP `content[].text`.
  - Codex `mcpToolCall` item structures containing a nested result object/string.
  - Claude `content.result` / `content.text` wrappers.
- A valid attachment requires `capsule_id` (or `id`), integer `revision`, and either a recognized capsule action or a self-identifying tool/open URL. Missing/invalid values return `None` and do not write a marker.
- `title` defaults to an empty string only if absent after sanitation; no DB lookup.

Fixture plan before coding the extractor:

- Capture a real Codex-format completed MCP result (`mcpToolCall` content with result `content[].text`) and its paired `tool_use`. If the current Codex runtime cannot directly call the Garyx MCP tool, use an already committed Codex MCP result fixture to pin Codex nesting and combine it with a real Garyx capsule JSON payload captured via `/mcp/{thread_id}/{run_id}`; keep this limitation documented in the fixture metadata.
- Run one real Claude agent/thread that calls `capsule_create`, capture the completed `tool_result` JSON and paired `tool_use`.
- Capture or derive one `capsule_update` payload with `action=updated` and `revision > 1` so update freshness is tested explicitly.
- Sanitize fixture files under `test-fixtures/capsules/provider-results/`:
  - Replace real thread/run/tool ids with synthetic fixture ids.
  - Replace capsule UUIDs with synthetic UUID-looking ids.
  - Replace titles/content/paths with synthetic placeholders.
  - Preserve only provider nesting, `tool_name`, `tool_use_id`, and result JSON shape needed by the extractor tests.
- Add tests that deserialize those fixtures and assert extraction succeeds for Codex direct and Claude anonymous-result correlation. Add one small synthetic payload-self-identification fixture for the fallback path.

### Reducer contract

File:

- `garyx-models/src/transcript_render_state.rs`

Public wire additions:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderCapsuleCard {
    pub id: String,
    pub capsule_id: String,
    pub title: String,
    pub revision: i64,
    pub action: RenderCapsuleAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderCapsuleAction {
    Created,
    Updated,
}

// On RenderUserTurnRow:
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub capsule_cards: Vec<RenderCapsuleCard>,
```

Internal shape:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct CapsuleMark {
    seq: u64,
    capsule_id: String,
    title: String,
    revision: i64,
    action: RenderCapsuleAction,
}
```

Reducer algorithm:

1. Pre-scan committed input records for valid `capsule_attached` controls to build only `latest_by_capsule` within the reducer input window. The main loop below builds the ordered, seq-bearing `capsule_marks` side list exactly once.
2. During the main loop, before skipping control messages:
   - If `message.control.kind == "capsule_attached"`, flush the current tool group, push the valid mark to `capsule_marks`, and continue.
   - Other controls remain skipped.
3. Keep `visible_blocks` free of capsule marks. Call `apply_tool_group_statuses`, `build_rows_with_capsule_marks`, and `derive_tail_activity` using only visible message/tool blocks.
4. Attribute marks by physical sequence interval:
   - A user turn starts at its user block seq; an orphan turn starts at its first activity block seq.
   - A turn ends before the next user block seq.
   - Marks in the interval attach to that turn.
5. Deduplicate per turn by `capsule_id`, keeping the highest revision and its action/title, while preserving the first mark sequence for ordering.
6. Freshness: each emitted card uses the `latest_by_capsule` revision/title from the reducer input window so later updates refresh older cards in full snapshots.
7. Busy gate: if the turn is trailing and `run_state.busy`, emit no `capsule_cards` for that trailing turn. This mirrors final-message deferral and prevents cards appearing before the final answer.
8. Capsule cards do not contribute to `visible_message_ids`, placeholder filtering, tool group status, active tool group id, tail activity, or progress locus.

### Router/gateway snapshot behavior

No production path changes planned. Existing `render_snapshot_at_seq` and `render_snapshot_in_window` feed committed records into `garyx-models`; `based_on_seq` remains the max seq in the snapshot input. Tests will prove:

- A snapshot at a seq before the marker does not backfill a card.
- A floor/window that excludes the marker omits the card.

### Desktop contract tolerance only

Files:

- `desktop/garyx-desktop/src/shared/contracts.ts`
- `desktop/garyx-desktop/src/renderer/src/render-view-model.ts`
- Related focused renderer tests if an existing test seam is available.

Changes:

- Add `RenderCapsuleCard` type and optional `capsule_cards?: RenderCapsuleCard[]` on `RenderUserTurnRow`.
- Add an optional `capsuleCards` field to the internal `UserTurnRow` view model only if this can be done without UI rendering; otherwise keep TypeScript contract-only in T1. If added, do not render it yet and do not count it as represented message ids.
- Keep unknown top-level/activity kind handling as skip-only in mapper code, not a hard throw.

### iOS contract tolerance only

Files:

- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileRenderState.swift`
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxMobileRenderRows.swift`
- `mobile/garyx-mobile/Tests/GaryxMobileCoreTests/GaryxMobileRenderStateMapperTests.swift`

Changes:

- Add `GaryxRenderCapsuleCard` and `GaryxRenderCapsuleAction` with snake-case wire keys and `Identifiable` conformance.
- Add `capsuleCards: [GaryxRenderCapsuleCard]` to `GaryxRenderUserTurnRow`; decode missing as `[]`; encode only with the row model.
- Add `capsuleCards` to `GaryxMobileTurnRow` and map it by pure passthrough from render state only if this falls out naturally from decoder/model tests; otherwise leave mobile turn-row passthrough to T3. Do not render SwiftUI card UI in T1.
- Implement tolerant unknown-kind decoding using an `unknown` enum case (or lossy row/activity array wrapper) so future unknown row/activity kinds do not throw away the full frame. Unknown rows map to `nil`; unknown activities map to `nil`.
- Ensure `messageRefs` ignores capsule cards so history resolution is unchanged.

## Tests

### Required Rust reducer tests

- `capsule_card_after_final_for_create`: marker between tool result and final assistant; emitted in `capsule_cards`, final assistant remains `RenderStepRow.final_message`.
- `capsule_card_waits_until_not_busy`: same records without terminal run completion plus busy run state emits no card; terminal/idle emits it.
- `same_run_create_then_update_dedupes_to_latest_revision`: one card, update revision/action wins, first mark order retained.
- `multiple_capsules_order_by_first_mark_seq`: two cards preserve first marker order.
- `later_run_update_bumps_revision_on_all_cards`: full snapshot updates historical card revision/title from later marker.
- `marker_below_render_floor_omits_card`: reducer/router window without marker has no card.
- `non_capsule_control_does_not_emit_card`.
- `marker_seq_advances_frame_cursor_without_backfilling_old_snapshots`: `based_on_seq` / at-seq behavior remains deterministic.
- `capsule_attached_event_ingestion_is_tolerant`: desktop/iOS event ingestion treats the new committed control event as an unknown/opaque event and still advances cursor/cache without throwing.
- `capsule_mark_does_not_break_tail_activity_or_tool_group_status`: marker at tail or between tool result and final does not become a tail block or active tool group blocker.

### Required bridge/persistence tests

- Extractor tests from sanitized real Codex/Claude fixtures:
  - Codex completed MCP result with direct `tool_name="mcp:garyx:capsule_create|update"` emits attachment.
  - Claude completed MCP result with result-side `tool_name=None` correlates via `tool_use_id -> mcp__garyx__capsule_create|update` and emits attachment.
  - Payload self-identification fallback emits attachment when tool name is missing.
- `capsule_attached_run_control_has_control_envelope`: `RunControlRecord::new` envelope contains top-level `kind/internal_kind=control` and nested `control.kind="capsule_attached"`.
- `capsule_attached_survives_terminal_reconcile`: after streaming partial append and terminal reconcile with the same `transcript_controls`, committed controls include `capsule_attached` and no extra `range_rewrite` is generated for the marker path.
- A negative test documents that direct external marker append is not the supported path, or at minimum that authoritative reconcile keeps only controls supplied through `transcript_controls`.

### Decoder/model tests

- Desktop type/model test if existing JS test coverage can be updated cheaply: `capsule_cards` optional/missing does not change existing row output; unknown kinds are skipped.
- iOS SwiftPM tests:
  - Missing `capsule_cards` decodes to `[]`.
  - Present `capsule_cards` decodes and maps to `GaryxMobileTurnRow.capsuleCards`.
  - Unknown top-level row/activity kinds do not throw the full `GaryxRenderSnapshot` decode and do not create unresolved refs.

### Commands

Focused required commands:

```bash
cargo test -p garyx-models transcript_render_state --all-targets
cargo test -p garyx-bridge capsule --all-targets
cargo test -p garyx-router render_snapshot --all-targets
cd mobile/garyx-mobile && swift test
```

If Swift model changes affect app-target compilation, also run:

```bash
cd mobile/garyx-mobile
xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug build CODE_SIGNING_ALLOWED=NO
```

## Rollout / review gates

1. Get cross-model review PASS on this implementation design before code.
2. Capture/sanitize real provider fixtures before implementing extractor logic.
3. Implement tests first around extractor and reducer seams, then production code.
4. Run required focused validation.
5. Open a separate Claude code-review task against the final diff and keep iterating until PASS.
6. Commit only task-related files using repo git identity. Merge main only after code review PASS.
