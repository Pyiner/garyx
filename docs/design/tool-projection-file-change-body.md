# Tool Projection: File Change Bodies (Write / Edit)

Status: v2 for review (v1 FAIL findings addressed)
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
server-owned composition structure instead of overloading a scalar
selector. Clients gain one generic composition operator and zero knowledge
of provider field names.

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
| FileEdit (pre-rendered, e.g. Codex `fileChange`) | unchanged (none today) | — | one `unified` segment per change | dedupe unchanged |
| FileRead | unchanged | unchanged (path is the substantive value for a read) | — | unchanged |

Rendering a whole written file as an all-added pair matches the CLI
conventions of the providers themselves (a `Write` *is* a pure insertion)
and gives file-change bodies exactly one rendering path.

### Wire contract: `RenderToolDiffSegment`

New projection field, additive on the wire:

```rust
/// Server-owned composition recipe for one rendered diff. Segments carry
/// selectors, never values; clients resolve and concatenate in order.
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

pub struct RenderToolFieldProjection {
    // ... existing fields ...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff: Vec<RenderToolDiffSegment>,
}
```

Array-valued inputs are enumerated **server-side** at derive time (the
deriver already holds the committed message): `MultiEdit`
`{edits: [{old_string, new_string}, …]}` becomes one `Pair` per element
with indexed selectors (`["input", "edits", "0", "old_string"]`, …);
Codex `fileChange` `{changes: [{path, diff}, …]}` becomes one `Unified`
per element (`["changes", "0", "diff"]`). Clients never learn the field
names `old_string`, `new_string`, `edits`, `changes`, or `diff`.

**`RenderToolFieldFormat::Diff` and `RenderToolFieldLabel::Diff` retire.**
All diff rendering flows through `diff` segments; there is exactly one way
to express a diff on the wire. The scalar selector formats keep only
non-composed presentations (text/code/path/json/image).

Composition semantics (identical on both clients, pinned by tests):

- Segments resolve and concatenate in wire order.
- `Unified`: resolved text splits into lines classified by `+`/`-` prefix
  (`+++`/`---` are context), exactly today's behavior.
- `Pair`: every line of resolved `old` renders as removed, then every line
  of resolved `new` as added. Raw strings are preserved — no trimming, and
  empty/whitespace-only lines survive (diff bodies depend on whitespace).
- An absent selector, or one that resolves to nothing, contributes zero
  lines. A segment contributing zero lines renders nothing. If **all**
  segments contribute nothing, the diff section is omitted entirely (the
  row degrades to summary/result as today).
- An empty-string `old` with non-empty `new` is a pure insert (added lines
  only); the reverse is a pure delete.

### Server: `garyx-models` selection rules

In `tool_field_projection.rs`, for `FileWrite` and `FileEdit`:

1. **Diff slot.** Derive `diff` segments, in preference order:
   - pre-rendered `changes`/`diff` values → `Unified` segments (one per
     array element, indexed);
   - `old_string`/`new_string` on the input → one `Pair`;
   - `edits` array of such pairs (`MultiEdit`) → one `Pair` per element;
   - `content` / `new_source` (`Write`, `NotebookEdit`) → one `Pair` with
     `old` absent.
2. **No JSON fallback when segments exist.** The existing whole-input JSON
   `Parameters` fallback for the call slot is suppressed once `diff` is
   non-empty; the call slot stays empty for these rows.
3. **Summary slot — file path.** When `diff` is non-empty, select the path
   (`file_path`, `filePath`, `AbsolutePath`, `TargetFile`, `notebook_path`,
   `path`, `file`) into `summary` with `label: file`, `format: path`. If no
   path key exists, keep the existing `CALL_SUMMARY_KEYS` behavior.
4. **Classification sweep.** `NotebookEdit` currently classifies as
   `Generic` (its compacted name matches no rule); add it to `FileEdit`.
5. **Result dedupe.** `absorb_result`'s "result repeats the visual call"
   rule generalizes: a result-side derivation whose `diff` segments equal
   the call-side segments is dropped (this is today's `fileChange`
   behavior, expressed on segments). The `Image` clause is unchanged.

### Desktop (`tool-trace-registry.ts`, `tool-trace.tsx`)

- `resolveMergedToolTrace` resolves `projection.diff` through the generic
  composition operator into the diff detail; `projectionDiffText`'s
  shape-sniffing (`{diff}` objects, arrays) is deleted with the retired
  format.
- Collapsed row: the path badge (`pathTail`) currently derives from
  `call.format === 'path'`; it now also derives from
  `summary.format === 'path'`. A path-formatted summary feeds the badge
  only — it is not additionally rendered as the summary text line.
- Expanded detail: a path-formatted summary renders as a `File` row above
  the diff/call/result sections, so the full path stays available expanded.
- `diffStats` (declared, never populated) is computed from the composed
  segment lines (added/removed counts) and shown in the collapsed row via
  the existing `DiffStatsLabel`.

### iOS (`GaryxMobileCore`)

- `GaryxToolFieldProjectionResolver` resolves `diff` segments through the
  same generic operator into `GaryxToolCallDiffLine`s; its `{diff}`-key /
  array shape-sniffing is deleted with the retired format.
- **Canonical path mapping** (`GaryxMobileRenderState.swift`): the mapper's
  `primaryPath`/`primaryPathBadge` derivation must include a path-formatted
  `summary` selector, so everything downstream of `primaryPath` keeps
  working when the path moves out of the call slot — the transcript
  tool-group counting ("Edited N files", `GaryxMobileTranscriptModel`) and
  written-image thumbnails (`GaryxToolCallPresentation.imageRefs`) are the
  two known consumers and both get regression tests.
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
and no version gating (repo policy). The `diff` field is additive; retiring
`format: diff` means:

- stale mobile build + new gateway: file rows degrade to path + result (no
  diff) until the app updates;
- new mobile build + older gateway: `format: "diff"` selectors fail enum
  decode, so `fileChange` rows degrade to the neutral state until the
  gateway updates.

Both degradations are display-only and accepted; no decode leniency is
added to preserve them.

## Affected surfaces

- `garyx-models/src/tool_field_projection.rs` (+ tests) — wire contract
  and selection rules.
- Gateway render frames: shape additive, no gateway-side code change
  expected.
- `desktop/garyx-desktop/src/shared/contracts` (projection type),
  `src/renderer/src/tool-trace-registry.ts`, `tool-trace.tsx` (+ unit
  tests).
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxToolFieldProjection.swift`,
  `GaryxMobileRenderState.swift`, `GaryxMobileTranscriptModel.swift`
  consumers, `GaryxToolCallPresentation.swift` (+ SwiftPM tests). If any
  file is added, run `xcodegen generate` and commit the pbxproj.

## Validation

Headless-first, per the repo UI direction. The core oracle: the same real
captured `Write`/`Edit` fixtures must **fail on the parent commit and pass
after the implementation**, at every layer that changes.

1. **Rust unit tests** with real captured (sanitized, synthetic-data)
   Claude Code shapes:
   - `Edit` `{file_path, old_string, new_string, replace_all}` → summary =
     path selector, `diff` = one Pair with old/new selectors; wire JSON
     contains none of the old/new text (selector-only invariant).
   - `Write` `{file_path, content}` → summary = path, `diff` = one Pair
     with absent `old`.
   - `MultiEdit` `{file_path, edits: […]}` → indexed Pair per edit.
   - `NotebookEdit` classifies `FileEdit` and selects `new_source`.
   - Codex `fileChange` → indexed Unified segments; result dedupe drops the
     repeated result-side segments.
   - Degenerate inputs (missing path, empty strings, empty `edits`) follow
     the composition semantics above.
2. **Desktop unit tests** (`npm run test:unit`): segment composition
   (Unified, Pair, absent sides, empty lines preserved, ordering), badge
   from path summary, `File` row presence, diffStats counts.
3. **iOS**: SwiftPM tests for the resolver operator, `primaryPath` from
   path summary, tool-group counting, image refs, section order
   (File → Diff → Result), collapsed path tail. Run **both** `swift test`
   and an iOS-simulator `xcodebuild` test pass unconditionally — this
   change touches Core/app integration regardless of whether project files
   change (validation contract).
4. **End-to-end**: drive `render_state` from a committed transcript
   containing real `Write`/`Edit` records and assert the mapped desktop and
   iOS view models show the composed diff. One packaged-app desktop check
   (`npm run dist:dir`) at acceptance.
