# Tool Projection: File Change Bodies (Write / Edit)

Status: draft for review
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
2. **The edit pair is not selectable at all.** `old_string`/`new_string` are
   two fields; a `RenderToolFieldSelector` points at one value. No key list
   entry exists for them and no selector shape can express the pair.
3. **Clients are (correctly) dumb.** Desktop `tool-trace-registry.ts` and iOS
   `GaryxToolFieldProjectionResolver` resolve exactly what the projection
   selects. iOS still contains a legacy pre-render-state fallback
   (`GaryxToolCallPresentation.diffLines`) that composed old/new diffs
   locally, but it only runs when a row has **no** projection — since the
   render-state migration the server always sends one, so that path is dead
   in practice.

## Design

Keep the projection contract exactly as it is philosophically: the server
selects fields, carries **selectors not values**, and clients resolve them
generically. No new wire fields, no schema change. Two things change: what
the server selects for file tools, and the small generic vocabulary of
"diff source" shapes both clients understand.

### Slot semantics (unchanged, now stated explicitly)

For every paired tool activity:

- `summary` — concise identity of the action (one line, collapsed row).
- `call` — the substantive call-side payload.
- `result` — the outcome.

File tools previously abused `call` for the identity (the path) because
nothing else fit. The fix is to put each value in its semantically correct
slot:

| Tool kind | `summary` | `call` | `result` |
| --- | --- | --- | --- |
| FileWrite | file path (`label: file`, `format: path`) | written content (`label: content`, `format: code`) | provider result (unchanged) |
| FileEdit | file path (`label: file`, `format: path`) | the diff (`label: diff`, `format: diff`) | provider result (unchanged) |
| FileRead | unchanged | unchanged (path is the substantive value for a read) | unchanged |

### Server: `garyx-models` selection rules

In `tool_field_projection.rs`, for `FileWrite` and `FileEdit`:

1. **Call slot — substantive body first.** New priority order:
   - `changes` / `diff` — pre-rendered diff (Codex `fileChange`; existing
     behavior, must not regress).
   - **composite edit shape** — if the call input object contains
     `old_string`/`new_string`, or an `edits` array of such pairs
     (`MultiEdit`), emit a selector pointing at the **input object itself**
     (`path` = the input prefix, e.g. `["input"]`), `format: diff`,
     `label: diff`. The selector still carries no copied values; clients
     resolve the object and compose the diff (see vocabulary below).
   - `content` / `new_source` — whole-body write (`Write`, `NotebookEdit`),
     `format: code`, `label: content`.
   - fall back to the current path keys only if none of the above exist, so
     degenerate inputs still render something.
2. **Summary slot — file path.** When the call slot selected a body, select
   the path (`file_path`, `filePath`, `AbsolutePath`, `TargetFile`,
   `notebook_path`, `path`, `file`) into `summary` with `label: file`,
   `format: path`. If no path key exists, keep the existing
   `CALL_SUMMARY_KEYS` behavior.
3. **Classification sweep.** `NotebookEdit` currently classifies as
   `Generic` (its compacted name matches no rule); add it to `FileEdit`.
4. `selector_for_value` learns: key `content`/`new_source` on the call side
   of a file tool → (`label: content`, `format: code`) instead of plain
   text.

The existing `absorb_result` dedupe (a result selector that repeats a visual
diff/image call selector is dropped) is unchanged and continues to apply.

### Wire vocabulary: generic diff sources

`format: diff` selectors may resolve to any of these shapes; both clients
must handle all of them identically:

1. a string — already unified-diff text (existing);
2. an object with a string `diff` field (existing);
3. an array of objects with `diff` fields — concatenated (existing);
4. **new:** an object with `old_string`/`new_string` — composed as
   `-`-prefixed old lines followed by `+`-prefixed new lines;
5. **new:** an object with an `edits` array of `old_string`/`new_string`
   pairs — each pair composed as (4), blocks concatenated in order.

Other keys in those objects (`file_path`, `replace_all`, …) are ignored by
the diff resolver. This is a shape vocabulary — exactly the same category of
rule as the existing `{diff}`-key handling — not a provider switch table.

### Desktop (`tool-trace-registry.ts`, `tool-trace.tsx`)

- `projectionDiffText` learns shapes (4) and (5).
- Collapsed row: the path badge (`pathTail`) currently derives from
  `call.format === 'path'`; it now also derives from
  `summary.format === 'path'`. A path-formatted summary feeds the badge
  only — it is not additionally rendered as the summary text line (no
  duplicated path in the header).
- Expanded detail: a path-formatted summary renders as a `File` row above
  the call/result sections, so the full path stays available expanded.
- `diffStats` (the already-declared, currently never-populated
  `MergedToolTrace.diffStats`) is computed from the resolved diff text
  (count of `+`/`-` lines, excluding `+++`/`---`) and shown in the collapsed
  row via the existing `DiffStatsLabel`.

### iOS (`GaryxMobileCore`)

- `GaryxToolFieldProjectionResolver.diffText` learns shapes (4) and (5).
- `GaryxToolCallPresentation.projectedSections` prepends a `File` section
  (plain monospace) resolved from a path-formatted `summary`, before the
  call/result sections — mirroring the desktop expanded layout.
- Collapsed list row: a path-formatted summary displays as its path tail
  (last components), consistent with the existing `primaryPathBadge`
  presentation, not the full absolute path.
- **Legacy fallback:** the pre-render-state local derivation in
  `GaryxToolCallPresentation.detail` (the `isFileEdit`/`isCommand`/…
  branches and the local `diffLines(input:inputText:)`) duplicates logic the
  server now owns. Verify reachability: if projections are present for every
  tool row the render state emits (expected), delete the legacy branches in
  a dedicated commit. If a real projection-less path exists, leave the
  fallback untouched and document why — do not extend it.

### Compatibility

Desktop and gateway ship from the same repo in lockstep; no compat shims.
The wire schema (field names, enum values) is unchanged — only selector
targets change — so a stale mobile build renders a degraded-but-safe view
(the composite selector resolves as JSON text) until it updates. Per iOS
policy, no version gating.

## Affected surfaces

- `garyx-models/src/tool_field_projection.rs` (+ tests) — server contract.
- Gateway render frames: values unchanged, selectors differ. No
  gateway-side code change expected.
- `desktop/garyx-desktop/src/renderer/src/tool-trace-registry.ts`,
  `tool-trace.tsx` (+ unit tests).
- `mobile/garyx-mobile/Sources/GaryxMobileCore/GaryxToolFieldProjection.swift`,
  `GaryxToolCallPresentation.swift` (+ SwiftPM tests). If any file is
  added, run `xcodegen generate` and commit the pbxproj.

## Validation

Headless-first, per the repo UI direction:

1. **Rust unit tests** with real captured (sanitized, synthetic-data) Claude
   Code shapes:
   - `Edit` `{file_path, old_string, new_string, replace_all}` → summary =
     path selector, call = input-object diff selector; wire JSON contains
     none of the old/new text (selector-only invariant).
   - `Write` `{file_path, content}` → summary = path, call =
     `["input","content"]` code/content.
   - `MultiEdit` `{file_path, edits: […]}` → composite diff selector.
   - `NotebookEdit` classifies `FileEdit` and selects `new_source`.
   - Codex `fileChange` regression: unchanged selectors, result dedupe
     intact.
2. **Desktop unit tests** (`npm run test:unit`): diff composition for shapes
   (4)/(5), badge from path summary, `File` row presence, diffStats counts.
3. **iOS SwiftPM tests**: resolver diff composition, section order
   (File → Diff/Content → Result), collapsed path tail. Validate via
   `xcodebuild`, not `swift test` alone, if project files change.
4. **End-to-end**: drive `render_state` from a committed transcript
   containing real `Write`/`Edit` records and assert the mapped desktop and
   iOS view models show the body. One packaged-app desktop check
   (`npm run dist:dir`) at acceptance.
