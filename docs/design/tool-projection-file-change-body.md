# Tool Projection: File Change Bodies (Write / Edit)

Status: v7 for review (v6 FAIL findings addressed)
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

The `diff` slot is orthogonal to `call`/`result`: a row may carry any
combination (a Generic tool can have a `call`, an `output` result, *and* a
diff). Detail rendering order on both platforms is
**File (path summary) → Call → Diff → Result**.

File tools previously abused `call` for the identity (the path) because
nothing else fit. Each value moves to its semantically correct slot:

| Tool kind | `summary` | `call` | `diff` | `result` |
| --- | --- | --- | --- | --- |
| FileWrite | file path (`label: file`, `format: path`) | — | one `pair` segment: `old` absent, `new` → content | provider result (unchanged) |
| FileEdit (old/new shapes) | file path | — | one `pair` segment per edit | provider result (unchanged) |
| FileEdit (pre-rendered, e.g. Codex `fileChange`) | unchanged (none today) | — | one `unified` segment per change | dropped by the merge rule (call side wins) |
| FileRead | unchanged | unchanged (path is the substantive value for a read) | — | unchanged |
| any kind, either side, with a pre-rendered `diff`/`changes` value | unchanged | unchanged (scalar selection no longer consumed by the diff value) | `unified` segments for that side | e.g. `{output, diff}` keeps `output` as the result |

Rendering a whole written file as an all-added pair matches the CLI
conventions of the providers themselves (a `Write` *is* a pure insertion)
and gives file-change bodies exactly one rendering path.

### Wire contract: value selectors and `RenderToolDiffRecipe`

Selector factoring: **location and presentation separate**. A diff segment
operand only locates a raw value; the segment kind itself is the
presentation. Reusing the display selector would force meaningless
`format`/`label` fields onto operands (with `Diff` retired there is nothing
sensible to put there) and would route resolution through display-oriented
filters.

The block below documents the **wire shape only** — in code it is the raw
serde shadow DTO, not the domain API (see the invariants section for the
domain types that make illegal states unrepresentable):

```rust
/// Pure location: which root and path inside one message body.
pub struct RenderToolValueSelector {
    pub root: RenderToolFieldRoot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<String>,
}

/// Display selector = location + presentation. `value` is flattened so the
/// scalar selector wire shape (root/path/format/label) is byte-identical
/// to today.
pub struct RenderToolFieldSelector {
    #[serde(flatten)]
    pub value: RenderToolValueSelector,
    pub format: RenderToolFieldFormat,
    pub label: RenderToolFieldLabel,
}

#[serde(rename_all = "snake_case")]
pub enum RenderToolDiffSource {
    ToolUse,
    ToolResult,
}

#[serde(rename_all = "snake_case")]
pub enum RenderToolDiffSegment {
    /// Text that is already unified-diff-style (`+`/`-`/context lines).
    Unified { text: RenderToolValueSelector },
    /// A replacement pair: `old` renders as removed lines, `new` as added
    /// lines. An absent side contributes nothing (pure insert / delete).
    Pair {
        old: Option<RenderToolValueSelector>,
        new: Option<RenderToolValueSelector>,
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

**Structural invariants** (unrepresentable, not advisory): a recipe holds
at least one segment, and a `Pair` carries at least one side. The public
domain API exposes only types that cannot express violations:

```rust
// Domain API (sketch; names indicative). All fields private.
pub struct RenderToolDiffPair { /* old/new, ≥1 side, private */ }
impl RenderToolDiffPair {
    pub fn old_new(old: RenderToolValueSelector, new: RenderToolValueSelector) -> Self;
    pub fn insert(new: RenderToolValueSelector) -> Self;   // old absent
    pub fn delete(old: RenderToolValueSelector) -> Self;   // new absent
}
pub enum RenderToolDiffSegment { Unified(RenderToolValueSelector), Pair(RenderToolDiffPair) }
pub struct RenderToolDiffRecipe { /* source + non-empty segments, private */ }
impl RenderToolDiffRecipe {
    pub fn new(source: RenderToolDiffSource, segments: Vec<RenderToolDiffSegment>) -> Option<Self>; // None when empty
}
```

Serialization goes through the shadow DTO (`From<&Recipe>`), so the server
*cannot* build or serialize an illegal recipe — there is no public
constructor or field access that admits one. Deserialization decodes the
shadow DTO and validates through the same constructors (`TryFrom`), so an
illegal recipe *fails decode* while the wire shape stays exactly as
documented above. On the client, a failing recipe decode is a malformed
projection and takes the lossy boundary (below). Plain derive on public
shapes would accept empty `Vec`s and double-`None` pairs; that is
explicitly not sufficient.

**Zero-contribution pruning (derive time).** The deriver holds the
committed raw values, so visual emptiness is decided where the values are
known, not at merge time. A segment side whose raw value is an empty
string, missing, or not a string contributes nothing; whitespace-only
values still contribute. Segments with no contributing side are dropped at
derivation; a derivation whose segments all drop yields `None` (empty
`edits`/`changes` fall out of this rule too). This closes the call-wins
hole where a structurally valid but visually empty call recipe (`Write`
with `content: ""`, `Edit` with both strings empty) would suppress a valid
result-side recipe and render nothing. Render-time composition semantics
below still cover values that later resolve empty on the client (e.g. the
source body is not loaded).

**Composability predicate (one predicate drives everything).** Each
candidate value under a `changes`/`diff` key (and each structured
file-change value) is judged by a single type-aware predicate,
**container-atomically**:

- a string (including the empty string) is composable;
- a `{diff: string}` object is composable;
- an array is composable only if **every meaningful member** is
  composable — zero-contribution members (empty strings, empty `{diff:
  ""}`) do not block atomicity, they simply prune, but a meaningful
  unsupported member (`{note: "keep"}`) makes the **whole container**
  non-composable;
- anything else (`diff: {note: …}`, numbers, …) is non-composable.

A non-composable container derives **no** recipe segments at all and its
value stays fully available to today's scalar rules (whole-object fallback
included) — no partial consumption ever strands a meaningful member behind
a suppressed fallback. This same predicate gates recipe derivation,
consumption, and slot exclusivity (below), so the three rules cannot
disagree.

**Consumed set.** "Consumed" is defined *after* pruning: the consumed set
of a pass is exactly the set of value locations referenced by the
**surviving** segments of that pass's recipe. A pass whose recipe pruned
to `None` has an empty consumed set — `{foo, bar, diff: ""}` and
`{foo, bar, changes: []}` therefore keep today's whole-object fallback and
lose nothing. Within a composable container, pruning is per-element: in
`changes: [{diff: "+x"}, {diff: ""}]` the container is composable (the
second member is zero-contribution, not unsupported), one segment
survives, and the fallback is suppressed.

**Raw resolution path.** Clients resolve diff operands with a dedicated
raw-string resolution: recipe `source` body + value selector → the exact
raw string. No trimming, no whitespace-visibility filtering (iOS
`hasVisibleContent` and similar display-side filters do not apply), no
JSON-unwrap heuristics beyond what scalar path traversal already does.

Array-valued inputs are enumerated **server-side** at derive time (the
deriver already holds the committed message): `MultiEdit`
`{edits: [{old_string, new_string}, …]}` becomes one `Pair` per element
with indexed selectors (`["input", "edits", "0", "old_string"]`, …);
Codex `fileChange` `{changes: [{path, diff}, …]}` becomes one `Unified`
per element (`["changes", "0", "diff"]`); a `{diff: "…"}` object value
becomes a selector into its `diff` key. Clients never learn the field
names `old_string`, `new_string`, `edits`, `changes`, or `diff`.

**Merge semantics.** `from_message` derives at most one recipe per
derivation pass (`source: tool_use` for the call pass, `tool_result` for
the result pass); a pass whose derivation is structurally empty yields
`None` (per the invariants). `absorb_result` applies one rule — the call
side wins:

| call-pass recipe | result-pass recipe | merged |
| --- | --- | --- |
| none (incl. empty derivation) | none | none |
| some | none | call recipe |
| none (incl. empty derivation) | some | result recipe |
| some | some (equal or not) | call recipe |

There is no structural-equality comparison and no concatenation across
sides. This subsumes today's `fileChange` "result repeats the call diff"
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

1. **Orthogonal pre-rendered diff scan — every pass, every kind.** Each
   derivation pass (call side and result side, all kinds including
   Generic/MCP) independently scans its payload object's `changes`/`diff`
   keys — *not* via the scalar key lists. Composable shapes — a string, a
   `{diff: string}` object, or an array whose elements are either —
   enumerate into `Unified` segments with value selectors pointing at the
   exact strings. Non-composable values produce no segments and remain
   available to scalar selection as ordinary values.
2. **Scalar selection is independent, loses nothing else, and never
   re-renders consumed values.** The `changes`/`diff` entries leave the
   scalar key lists; scalar call/result selection excludes the values the
   diff scan consumed. `{output, diff}` therefore projects `output` as the
   result *and* a diff recipe. Because a location-only selector cannot
   express "this object minus its consumed keys", **the whole-object JSON
   fallback is suppressed whenever the pass consumed a value**: only
   precisely selectable scalars (the ordinary key lists) remain eligible.
   A Generic call `{diff: "+x"}` gets a diff recipe and an empty call —
   not a whole-arguments JSON selector that would repeat the diff; a
   Generic call `{foo: 1, bar: 2, diff: "+x"}` gets the recipe plus
   whatever single key the ordinary lists select (here: nothing), never
   the enclosing object.
3. **Structured pair shapes — file kinds only.** For `FileWrite`/`FileEdit`
   call passes, in preference order after pre-rendered diffs:
   `old_string`/`new_string` on the input → one `Pair`; `edits` array
   (`MultiEdit`) → one `Pair` per element; `content`/`new_source`
   (`Write`, `NotebookEdit`) → one `Pair` with `old` absent. A `content`
   key outside these kinds keeps its existing output/text meaning.
4. **Summary owns the path; `call` is forced empty — a shape invariant,
   not a recipe-survival condition.** For a `FileWrite`/`FileEdit` call
   pass whose input matches a **recognized file-change shape** — any of
   the structured keys (`old_string`/`new_string`, `edits`,
   `content`/`new_source`) or pre-rendered keys (`changes`/`diff`) is
   present **with a value that passes the composability predicate**
   (legal-but-empty values qualify; unsupported types such as
   `diff: {note: …}` do not) — the path (`file_path`, `filePath`,
   `AbsolutePath`, `TargetFile`, `notebook_path`, `path`, `file`) belongs
   exclusively to `summary` (`label: file`, `format: path`) and **`call`
   is `None`**; scalar call selection does not run. Recognition is
   deliberately independent of pruning survival but *not* of type: keying
   on survival would let an empty `Write`/`Edit` fall back to path-in-call
   (and the "summary equals call" filter would then drop the summary),
   ending up `summary=None, call=path, diff=result` once a result-side
   recipe merged in; keying on bare key presence would swallow
   non-composable values (`FileEdit {file_path, diff: {note: …}}` would
   force `call=None` with nothing to show). With the predicate-gated shape
   invariant, an empty `Write` still projects `summary=path, call=None`,
   and unsupported-type inputs keep today's scalar behavior untouched. If
   no path key exists, keep the existing `CALL_SUMMARY_KEYS` behavior.
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
- Expanded detail (`tool-trace.tsx`): sections render File (path summary)
  → Call → Diff → Result.
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
  operator producing `GaryxToolCallDiffLine`s via the raw resolution path;
  its `{diff}`-key / array shape-sniffing is deleted with the retired
  format.
- **Lossy projection decode boundary** (`GaryxMobileRenderState.swift`):
  the projection is a presentation hint. An undecodable projection —
  unknown enum value, unknown segment discriminator, missing source,
  invariant-violating recipe — must decode as `nil` projection for that
  entry, never fail the entry, the tool group, or the frame. Today
  selector enums decode strictly, so a retired or future token would drop
  whole tool groups from full snapshots (lossy arrays) and mark delta
  upserts malformed, triggering gap/replay. This boundary is a general
  invariant of the render contract (presentation vocabulary may evolve
  server-side), not a legacy shim; it is what makes the `format: diff`
  retirement — and any future vocabulary change — display-only.
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
- `GaryxToolCallPresentation.projectedSections` renders File (path
  summary) → Call → Diff → Result.
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

Both degradations are display-only and pinned by the mismatch tests below.

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
   - Slot exclusivity: for all four structured tools (`Write`, `Edit`,
     `MultiEdit`, `NotebookEdit`) assert `summary == path selector` **and**
     `call == None` — both for the call-only projection and for the final
     merged projection; and for the empty-body variants (`content: ""`,
     both strings empty) assert the same slots with `diff == None`, plus
     the merged case where a valid result recipe is adopted while
     `summary`/`call` stay `path`/`None`.
   - Orthogonal scan: Generic call `{diff: "+x"}` → recipe with empty
     call (no JSON degradation); `{foo, bar, diff}` on the call side and
     on the result side → recipe plus only precisely selectable scalars,
     never the enclosing object (consumed-value fallback suppression);
     Generic result `{output, diff}` → `output` result selector *and* a
     `tool_result` recipe; `{diff: {diff: "…"}}` object shape → selector
     into the inner key; non-composable `diff` values produce no recipe
     and stay available to scalar selection. No `Format::Diff`/
     `Label::Diff` production remains (grep-pinned).
   - Invariants and pruning: empty `edits`/`changes` derive `None`;
     `Write` `content: ""` and `Edit` with both strings empty derive
     `None` (zero-contribution pruning), and the merge tests "empty call
     derivation + valid result recipe → result adopted" and "empty-string
     call body + valid result recipe → result adopted" pass;
     whitespace-only values still derive segments. All four merge-table
     cases.
   - Consumed set and container atomicity: `{foo, bar, diff: ""}` and
     `{foo, bar, changes: []}` on both sides keep the whole-object
     fallback (pruned recipe ⇒ empty consumed set);
     `changes: [{diff: "+x"}, {diff: ""}]` is composable, derives one
     segment, and suppresses the fallback;
     `changes: [{diff: "+x"}, {note: "keep"}]` on **both** sides is
     non-composable as a container — no recipe, fallback preserved, the
     unsupported member reachable.
   - Predicate-gated shape recognition: `FileEdit {file_path, diff: {note:
     …}}` (unsupported type) keeps today's scalar behavior — no forced
     `call=None`; the same key with a legal zero-contribution value
     (`diff: ""`) triggers exclusivity (`summary=path, call=None,
     diff=None`).
   - Enforcement: illegal raw JSON (empty `segments`, double-`None` pair)
     fails recipe decode in Rust; the checked constructors make illegal
     recipes unrepresentable, pinned by a compile-visible API (no public
     field construction) plus a serializer test that only legal recipes
     exist on the wire.
   - Scalar selector wire shape unchanged by the `RenderToolValueSelector`
     flatten refactor (serde round-trip byte-equality on existing
     fixtures).
2. **Desktop unit tests** (`npm run test:unit`): segment composition
   (Unified, Pair, absent sides, whitespace-only and empty lines
   preserved, ordering, source-body targeting), badge from path summary,
   File→Call→Diff→Result order, diffStats counts, row-equality includes
   `diff`, unknown-format leniency.
3. **iOS**: SwiftPM tests for the raw-resolution segment operator
   (whitespace-only values survive), lossy decode boundary with raw JSON
   fixtures for **each** shape: legacy `format: "diff"`, unknown segment
   discriminator, unknown/missing `source`, invariant-violating recipe —
   in both a full snapshot and a delta upsert, and after a lossy delta the
   next valid delta still applies (no gap/replay entered). A frozen
   pre-change decoder fixture pins the stale-mobile direction: new-wire
   `Write`/`Edit` decodes to path + result, new-wire `fileChange` to a
   bare titled row. Plus `primaryPath` from path summary, tool-group
   counting, image refs, signature incorporates diff (late-body flip
   test), section order, collapsed path tail. Run **both** `swift test`
   and an iOS-simulator `xcodebuild` test pass unconditionally — this
   change touches Core/app integration regardless of whether project files
   change (validation contract).
4. **End-to-end**: drive `render_state` from a committed transcript
   containing real `Write`/`Edit` records and assert the mapped desktop and
   iOS view models show the composed diff. One packaged-app desktop check
   (`npm run dist:dir`) at acceptance.
