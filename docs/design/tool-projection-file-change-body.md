# Tool Projection: File Change Bodies (Write / Edit)

Status: v3 for review (v2 FAIL findings addressed)
Date: 2026-07-18

## Problem

Claude Code `Write` and `Edit` tool rows on both Mac and iOS show only the
file path and the provider's textual result ("File created successfully at
..."). The actual change — the written content, or the `old_string` →
`new_string` replacement — is never rendered anywhere. The user cannot see
what an agent changed without opening the file.

Codex `fileChange` already renders its diff (its committed body carries a
`changes[].diff` field the projection selects). The gap is specific to tools
whose call input carries the change as structured fields instead of a
pre-rendered diff: Claude Code `Write`, `Edit`, `MultiEdit`, `NotebookEdit`,
and equivalent shapes from other providers.

## Root cause

Tool row field selection is server-owned through
`RenderToolFieldProjection` (`garyx-models/src/tool_field_projection.rs`).
Three defects compound:

1. **Single substantive slot, path-first key order.** For
   `FileWrite`/`FileEdit` the call-selector key list is
   `[file_path, filePath, AbsolutePath, path, file, changes, diff, content]`
   and `select_object_field` takes the first hit. Claude's `Write` input is
   `{file_path, content}` and `Edit` is
   `{file_path, old_string, new_string, replace_all}` (real captured shapes,
   verified against committed transcripts) — `file_path` always wins, so the
   one call slot is spent on the path and the body is never selected.
2. **The edit pair is not expressible.** `old_string`/`new_string` are two
   fields; a `RenderToolFieldSelector` points at one value. No selector
   shape can express "compose these two values as a diff".
3. **Clients are (correctly) dumb.** Desktop `tool-trace-registry.ts` and
   iOS `GaryxToolFieldProjectionResolver` resolve exactly what the
   projection selects. iOS still contains a legacy pre-render-state fallback
   (`GaryxToolCallPresentation.diffLines`) that composed old/new diffs
   locally, but it is dead: with a projection, `projectedSections` returns
   early; without one, the mapper leaves input/result/path empty and
   `isProviderNeutralFallback` returns the neutral empty state (captured-
   frame tests pin this).

## Design

The projection contract stays: the server selects fields, carries
**selectors not values**, and clients resolve them generically — including
for diffs. A diff is a *composed* value, so the wire gains an explicit,
server-owned composition recipe instead of overloading a scalar selector.
Clients gain one generic composition operator and zero knowledge of
provider field names.

### Slot semantics

For every paired tool activity:

- `summary` — concise identity of the action (one line, collapsed row).
- `call` — the substantive scalar call-side payload.
- `diff` — the substantive *composed* change body (new, see below).
- `result` — the outcome.

File tools previously abused `call` for the identity (the path) because
nothing else fit. Each value moves to its semantically correct slot:

| Tool kind | `summary` | `call` | `diff` | `result` |
| --- | --- | --- | --- | --- |
| FileWrite | file path (`label: file`, `format: path`) | — | one `pair` segment: `old` absent, `new` → content | provider result (unchanged) |
| FileEdit (old/new shapes) | file path | — | one `pair` segment per edit | provider result (unchanged) |
| FileEdit (pre-rendered, e.g. Codex `fileChange`) | unchanged (none today) | — | one `unified` segment per change | dropped by the merge rule (call side wins) |
| FileRead | unchanged | unchanged (path is the substantive value for a read) | — | unchanged |
| any kind with a pre-rendered `diff`/`changes` result value | unchanged | unchanged | `unified` segments, `source: tool_result` | that value no longer occupies `result` |

Rendering a whole written file as an all-added pair matches the CLI
conventions of the providers themselves (a `Write` *is* a pure insertion)
and gives file-change bodies exactly one rendering path.

### Wire contract: `RenderToolDiffRecipe`

New projection field, additive on the wire:

```rust
#[serde(rename_all = "snake_case")]
pub enum RenderToolDiffSource {
    ToolUse,
    ToolResult,
}

#[serde(rename_all = "snake_case")]
pub enum RenderToolDiffSegment {
    /// Text that is already unified-diff-style (`+`/`-`/context lines).
    Unified { text: RenderToolFieldSelector },
    /// A replacement pair: `old` renders as removed lines, `new` as added
    /// lines. An absent side contributes nothing (pure insert / delete).
    Pair {
        old: Option<RenderToolFieldSelector>,
        new: Option<RenderToolFieldSelector>,
    },
}

/// Server-owned composition recipe for one rendered diff. Segments carry
/// selectors, never values. `source` names the message body every selector
/// in this recipe resolves against — clients resolve strictly against that
/// body, with no cross-body fallback guessing.
pub struct RenderToolDiffRecipe {
    pub source: RenderToolDiffSource,
    pub segments: Vec<RenderToolDiffSegment>,
}

pub struct RenderToolFieldProjection {
    // ... existing fields ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<RenderToolDiffRecipe>,
}
```

Array-valued inputs are enumerated **server-side** at derive time (the
deriver already holds the committed message): `MultiEdit`
`{edits: [{old_string, new_string}, …]}` becomes one `Pair` per element
with indexed selectors (`["input", "edits", "0", "old_string"]`, …);
Codex `fileChange` `{changes: [{path, diff}, …]}` becomes one `Unified`
per element (`["changes", "0", "diff"]`). Clients never learn the field
names `old_string`, `new_string`, `edits`, `changes`, or `diff`.

**Merge semantics.** `from_message` derives at most one recipe per
derivation pass (`source: tool_use` for the call pass, `tool_result` for
the result pass). `absorb_result` applies one rule — the call side wins:

| call-pass recipe | result-pass recipe | merged |
| --- | --- | --- |
| none | none | none |
| some | none | call recipe |
| none | some | result recipe |
| some | some (equal or not) | call recipe |

There is no structural-equality comparison and no concatenation across
sides; a result-side recipe is adopted only when the call pass produced
none. This subsumes today's `fileChange` "result repeats the call diff"
dedupe. The existing `repeats_visual_call` clause keeps only its `Image`
case.

**`RenderToolFieldFormat::Diff` and `RenderToolFieldLabel::Diff` retire.**
All diff rendering flows through `diff` recipes; there is exactly one way
to express a diff on the wire. The scalar selector formats keep only
non-composed presentations (text/code/path/json/image). Wire safety for
this removal is guaranteed by the lossy projection boundary (see
Compatibility), not by keeping legacy tokens decodable.

Composition semantics (identical on both clients, pinned by tests):

- Segments resolve against the recipe's `source` body and concatenate in
  wire order.
- `Unified`: resolved text splits into lines classified by `+`/`-` prefix
  (`+++`/`---` are context), exactly today's behavior.
- `Pair`: every line of resolved `old` renders as removed, then every line
  of resolved `new` as added. Raw strings are preserved — no trimming, and
  empty/whitespace-only lines survive (diff bodies depend on whitespace).
- An absent selector, or one that resolves to nothing, contributes zero
  lines. A segment contributing zero lines renders nothing. If **all**
  segments contribute nothing (including when the `source` body is not
  loaded), the diff section is omitted and the row renders from the
  remaining slots.
- An empty-string `old` with non-empty `new` is a pure insert (added lines
  only); the reverse is a pure delete.

### Server: `garyx-models` derivation rules

All current producers of Diff-formatted selectors are migrated; the
implementation must grep-enumerate every `RenderToolFieldFormat::Diff` /
`RenderToolFieldLabel::Diff` production and consumption site and leave
none behind.

1. **Kind-independent pre-rendered diffs.** Wherever the current key lists
   select `changes`/`diff` (the FileWrite/FileEdit call keys *and* the
   result-side key list that applies to every kind, including Generic),
   those values now derive `Unified` recipe segments for that pass instead
   of a Diff-formatted scalar selector. Values that are not diff-composable
   (not a string / array-of-`{diff}` shape) fall back to the ordinary
   text/JSON selector they would get without the `changes`/`diff` keys.
2. **Structured pair shapes — file kinds only.** For `FileWrite`/`FileEdit`
   call passes, in preference order after pre-rendered diffs:
   `old_string`/`new_string` on the input → one `Pair`; `edits` array
   (`MultiEdit`) → one `Pair` per element; `content`/`new_source`
   (`Write`, `NotebookEdit`) → one `Pair` with `old` absent. A `content`
   key outside these kinds keeps its existing output/text meaning.
3. **No JSON fallback when a recipe exists.** The whole-input JSON
   `Parameters` fallback for the call slot is suppressed once the call pass
   derived a recipe; the call slot stays empty for these rows.
4. **Summary slot — file path.** When the call pass derived a recipe,
   select the path (`file_path`, `filePath`, `AbsolutePath`, `TargetFile`,
   `notebook_path`, `path`, `file`) into `summary` with `label: file`,
   `format: path`. If no path key exists, keep the existing
   `CALL_SUMMARY_KEYS` behavior.
5. **Classification sweep.** `NotebookEdit` currently classifies as
   `Generic` (its compacted name matches no rule); add it to `FileEdit`.

### Desktop

- `tool-trace-registry.ts`: `resolveMergedToolTrace` resolves
  `projection.diff` through the generic composition operator against the
  recipe's source body; `projectionDiffText`'s shape-sniffing (`{diff}`
  objects, arrays) is deleted with the retired format.
- Collapsed row: the path badge (`pathTail`) currently derives from
  `call.format === 'path'`; it now also derives from
  `summary.format === 'path'`. A path-formatted summary feeds the badge
  only — it is not additionally rendered as the summary text line.
- Expanded detail (`tool-trace.tsx`): a path-formatted summary renders as a
  `File` row above the diff/call/result sections.
- `diffStats` (declared, never populated) is computed from the composed
  segment lines (added/removed counts) and shown in the collapsed row via
  the existing `DiffStatsLabel`.
- **Row invalidation**: `thread-render-row-equality.ts` compares only
  summary/call/result selectors today; the memo comparator must include
  the `diff` recipe, otherwise a frame that changes only segments skips
  re-render. Pinned by an "only segments changed → re-renders" test.
- Desktop's runtime JSON handling is structurally lenient (TS types are
  compile-time; unknown enum strings fall through switches). Pin that
  property with a test: a projection carrying an unknown selector format
  degrades that field, never the row.

### iOS (`GaryxMobileCore` + app target)

- `GaryxToolFieldProjection.swift`: the resolver gains the generic segment
  operator producing `GaryxToolCallDiffLine`s; its `{diff}`-key / array
  shape-sniffing is deleted with the retired format.
- **Lossy projection decode boundary** (`GaryxMobileRenderState.swift`):
  the projection is a presentation hint. An undecodable projection —
  unknown enum value, unknown segment shape, malformed recipe — must decode
  as `nil` projection for that entry, never fail the entry, the tool
  group, or the frame. Today selector enums decode strictly, so a retired
  or future token would drop whole tool groups from full snapshots (lossy
  arrays) and mark delta upserts malformed, triggering gap/replay. This
  boundary is a general invariant of the render contract (presentation
  vocabulary may evolve server-side), not a legacy shim; it is what makes
  the `format: diff` retirement — and any future vocabulary change —
  display-only. Pinned by full-snapshot and delta-upsert tests feeding
  legacy `format: "diff"` payloads.
- **Canonical path mapping** (`GaryxMobileRenderState.swift`): the mapper's
  `primaryPath`/`primaryPathBadge` derivation must include a path-formatted
  `summary` selector, so everything downstream of `primaryPath` keeps
  working when the path moves out of the call slot — the transcript
  tool-group counting ("Edited N files", `GaryxMobileTranscriptModel`) and
  written-image thumbnails (`GaryxToolCallPresentation.imageRefs`) are the
  two known consumers and both get regression tests.
- **Update signature** (`GaryxMobileToolTraceBuilder.swift`, consumed by
  `GaryxMobileModel+Messages.swift` to drop no-op updates): the bounded
  signature must incorporate the resolved diff lines. Without it, a Codex
  row with no summary/call/result whose render frame arrives before the
  message body would never show its diff once the body lands. Pinned by a
  "body arrives late → diff flips empty→non-empty" regression test.
- `GaryxToolCallPresentation.projectedSections` prepends a `File` section
  (plain monospace) from the path-formatted `summary`, then the diff
  section, then result — mirroring the desktop expanded layout.
- Collapsed list row: a path-formatted summary displays as its path tail,
  consistent with the existing `primaryPathBadge` presentation, not the
  full absolute path.
- **Legacy fallback: delete.** The pre-render-state local derivation in
  `GaryxToolCallPresentation.detail` (the `isCommand`/`isFileEdit`/… input
  re-parsing branches and `diffLines(input:inputText:)`) is dead code —
  with a projection `projectedSections` returns early, without one the
  neutral empty state returns first, and captured-frame tests pin the
  projection-less neutral state. Delete these branches and `diffLines` in a
  dedicated commit; keep (and keep green) the projection-less neutral
  degradation tests. No conditional retention.

### Compatibility

Desktop and gateway ship from the same repo in lockstep; no compat shims
and no version gating (repo policy). The `diff` field is additive, and the
lossy projection boundary makes vocabulary evolution display-only in both
directions:

- **Stale mobile + new gateway**: unknown `diff` field is ignored by the
  keyed decoder. Claude `Write`/`Edit` rows show path summary + result
  (both use existing enum values); Codex `fileChange` rows, which carry
  only a recipe, show a bare titled row until the app updates.
- **New mobile + older gateway**: legacy `format: "diff"` selectors hit the
  lossy boundary → that entry renders as a neutral projection-less row
  until the gateway updates. No dropped tool groups, no malformed-frame
  replay.

Both degradations are display-only and pinned by the mismatch tests above.

## Affected surfaces

- `garyx-models/src/tool_field_projection.rs` (+ tests) — wire contract
  and derivation rules.
- Gateway render frames: shape additive, no gateway-side code change
  expected.
- `desktop/garyx-desktop/src/shared/contracts` (projection type),
  `src/renderer/src/tool-trace-registry.ts`, `tool-trace.tsx`,
  `src/renderer/src/app-shell/components/thread-render-row-equality.ts`
  (+ unit tests).
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxToolFieldProjection.swift`,
  `GaryxMobileRenderState.swift` (decode boundary + path mapping),
  `GaryxMobileToolTraceBuilder.swift` (signature),
  `GaryxMobileTranscriptModel.swift` consumers,
  `GaryxToolCallPresentation.swift`, and the app-target update gate
  `GaryxMobileModel+Messages.swift` (+ SwiftPM tests). If any file is
  added, run `xcodegen generate` and commit the pbxproj.

## Validation

Headless-first, per the repo UI direction. The core oracle: the same real
captured `Write`/`Edit` fixtures must **fail on the parent commit and pass
after the implementation**, at every layer that changes.

1. **Rust unit tests** with real captured (sanitized, synthetic-data)
   Claude Code shapes:
   - `Edit` `{file_path, old_string, new_string, replace_all}` → summary =
     path selector, `diff` = tool_use recipe with one Pair; wire JSON
     contains none of the old/new text (selector-only invariant).
   - `Write` `{file_path, content}` → summary = path, one Pair with absent
     `old`.
   - `MultiEdit` `{file_path, edits: […]}` → indexed Pair per edit.
   - `NotebookEdit` classifies `FileEdit` and selects `new_source`.
   - Codex `fileChange` → indexed Unified segments; merge rule: call-side
     recipe retained, result-side recipe ignored; result-only derivation
     (no call pass) adopts the `tool_result` recipe.
   - Generic-kind result `{diff: …}` and `{changes: […]}` → `tool_result`
     Unified recipe; non-composable `diff` values fall back to plain
     selectors. No `Format::Diff`/`Label::Diff` production remains
     (grep-pinned).
   - Merge truth table: all four call/result recipe cases.
   - Degenerate inputs (missing path, empty strings, empty `edits`) follow
     the composition semantics above.
2. **Desktop unit tests** (`npm run test:unit`): segment composition
   (Unified, Pair, absent sides, empty lines preserved, ordering, source
   body targeting), badge from path summary, `File` row presence, diffStats
   counts, row-equality includes `diff`, unknown-format leniency.
3. **iOS**: SwiftPM tests for the resolver operator, lossy decode boundary
   (full snapshot + delta upsert with legacy `format: "diff"`),
   `primaryPath` from path summary, tool-group counting, image refs,
   signature incorporates diff (late-body flip test), section order
   (File → Diff → Result), collapsed path tail. Run **both** `swift test`
   and an iOS-simulator `xcodebuild` test pass unconditionally — this
   change touches Core/app integration regardless of whether project files
   change (validation contract).
4. **End-to-end**: drive `render_state` from a committed transcript
   containing real `Write`/`Edit` records and assert the mapped desktop and
   iOS view models show the composed diff. One packaged-app desktop check
   (`npm run dist:dir`) at acceptance.
