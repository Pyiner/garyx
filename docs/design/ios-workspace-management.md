# iOS Workspace Management

Status: v1 — iOS adaptation of the shipped desktop workspace-management
contract (`docs/design/desktop-workspace-management.md`). Design is final per
the no-design-review flow; implementation follows this document.
Scope: iOS app + `GaryxMobileCore` + one additive gateway change to
`/api/thread-summaries`. Everything else consumes the already-shipped server
contract.

## 1. Problem

The desktop feature shipped a full workspace model — server-owned ordering
(pinned → recency → name → path), pin/rename point mutations, rich workspace
summaries (`thread_count`, `last_activity_at`, `git_repo`, `gateway_home`),
a typed-error remote directory browser, tri-state draft workspace selection,
and server-owned thread workspace provenance (`workspace_origin`,
`root_workspace_path`). iOS still renders the old thin model:

- Drawer workspace rows are basenames of a paths-only cache, sorted
  client-side (`uniqueSortedWorkspacePaths`) — a second source of truth that
  fights the server order.
- No pin, no rename, no remove, no workspace context (count/activity/git)
  anywhere.
- The workspace drilldown groups threads by runtime `workspace_dir`, so
  worktree threads group under their private worktree path instead of their
  root workspace.
- The new-thread draft workspace is a bare `String` where `""` conflates
  "explicitly no workspace" and "not chosen yet".
- The directory browser ignores `git_repo`, renders only
  `error.localizedDescription`, and has no typed-path entry or filtering.
- `~` abbreviation is a hardcoded local string heuristic, not server
  `gateway_home`.

## 2. Direction and fidelity

- The Mac app is the source of truth for information architecture, labels,
  field meaning, and data models. iOS renders the same model with **native
  iOS patterns** (grouped lists, context menus, sheets, toolbar actions).
  The desktop Codex pixel-parity mandate does not transfer to iOS; porting
  desktop card/hover layouts is explicitly out.
- iOS and the gateway ship together. No compatibility shims, no dual-read
  fallbacks for old shapes.

## 3. Invariants preserved (identical to desktop design §3)

- Workspace identity is the absolute directory path string; no IDs.
- Thread `workspace_dir` is immutable once set; the picker only changes the
  draft selection of an unsent new thread.
- "No workspace" means the user declined to choose; the runtime provisions a
  private managed directory.
- All directory operations are remote gateway APIs.
- Root workspace rows come from explicit user action only.
- Thread condition queries go through SQL projections derived in the record
  write transaction; no read-time repair.
- Clients render server name and server order verbatim. No client-side
  re-sorting, renaming, or path filtering of the workspace catalog.

## 4. Server change (additive, this feature's only gateway work)

`/api/thread-summaries` becomes membership-aware:

- **Filter param**: `workspace_dir` is replaced by `root_workspace_path`.
  The filter matches the `thread_meta` projection's `root_workspace_path`
  column (worktree threads therefore group under their source workspace;
  implicit threads match no workspace filter). The old runtime-dir filter
  semantics retire with the param — the iOS drilldown was its only real
  consumer (the CLI probe passes no filter); no alias or fallback param.
- **Row fields**: summary rows additionally emit `root_workspace_path` and
  `workspace_origin`, read from the same projection row the query already
  scans.
- **Indexes**: the summary query family gains `root_workspace_path` sibling
  partial indexes matching the existing `workspace_dir` index shapes
  (`visible` / `non_task` / `task`); the retired `workspace_dir` summary
  indexes are dropped in the same versioned migration.
- **Cursor scope**: `thread_summary_scope` hashes the new param; a cursor
  from another scope keeps rejecting exactly as today.
- Pagination, visibility rules (side chats hidden), and every other field
  are unchanged.

No other server work: workspaces list/pin/rename/directories/git-status and
thread-detail provenance already shipped.

## 5. UX design (iOS)

### 5.1 Drawer "Workspaces" section

- Rows render the server list verbatim: server `name`, server order. Pinned
  rows show a small pin glyph after the name (monochrome). Row icon stays
  the folder glyph; `git_repo` does not change the drawer row.
- Row tap keeps opening the workspace drilldown (the drilldown page is the
  iOS adaptation of desktop's inline thread subtree; no chevron-expansion
  inside the drawer).
- **Long-press context menu** (iOS action surface, replaces desktop hover
  `⋯`): `Pin` / `Unpin` · `Rename…` · `New Thread` · `Copy Path` ·
  `Remove` (destructive, separated). `Remove` confirms with copy stating it
  only removes the list entry and never touches files on the gateway
  machine (existing tombstone semantics). `New Thread` opens a new-thread
  draft seeded with `path(<workspace>)`.
- `Rename…` presents a text-field alert seeded with the current server
  name; submit calls the rename point mutation.
- Pin/rename/remove refresh the catalog from the mutation response /
  follow-up fetch; the drawer re-renders whatever order the server returns.

### 5.2 Workspace drilldown

- Header (native grouped header, the iOS adaptation of the desktop hover
  card): display name, pin state, thread count, `~`-abbreviated path
  (server `gateway_home`, tappable → copy), and git branch when
  `git_repo` (existing `/api/workspaces/git-status`).
- Toolbar ellipsis menu carries the same actions as §5.1.
- The thread list switches to the membership filter
  (`root_workspace_path=<path>`, §4): worktree threads appear under their
  root workspace. Store scope keys move to the new param accordingly.
- Thread rows: subtitle prefers `root_workspace_path` basename when
  delivered, falling back to the runtime `workspace_dir` basename (only
  pre-cutover rows lack the projection value).

### 5.3 Composer workspace chip + picker (new-thread drafts)

- The workspace **chip moves into the composer bottom bar**, next to the
  existing mode capsule, for new-thread drafts. The empty-state pill above
  the conversation is removed; the empty state keeps only its non-selection
  content. The chip shows the draft selection (`No workspace` state uses a
  muted slashed-folder glyph + label) and opens the picker sheet.
- **Picker sheet** (native sheet, replaces `GaryxWorkspaceSelectSheet`
  content): search field filtering on name and path → workspace list in
  server order, pinned glyphs shown, current selection checkmarked →
  footer actions `Add Workspace…` and `No Workspace`.
- **Draft tri-state** (mirrors desktop §4.2 semantics exactly):
  `path(<absolute path>)` | `none` | unresolved.
  - Unresolved drafts resolve **once**: first row of the server-ordered
    catalog; empty catalog → `none`; a draft created before the catalog
    loads resolves once when it arrives. After resolution the draft never
    drifts on refresh — the only sanctioned re-resolution is removal of
    the selected workspace from the catalog.
  - Explicit `none` is never overridden. Entry points that carry a
    workspace ("New Thread" on a workspace row, agent `Chat` one-off
    target) seed `path(X)`.
  - Create payload: `path` → `workspace_dir`; `none` → no `workspace_dir`.
  - Persistence: the gateway-scoped draft keys store the tri-state
    explicitly (`none` is distinct from absent/unresolved); the legacy
    bare-string key retires with the same no-compat rule (an old value
    simply reads as unresolved once).
- **Mode control**: the inline mode capsule keeps its gating; display copy
  becomes **Direct** / **Worktree** (sheet copy included). Internal enum
  values and the wire field are unchanged.
- After send, the chip disappears (thread workspace is immutable);
  sent-thread context is §5.5.

### 5.4 Add workspace flow (remote directory browser v2)

One sheet, upgraded in place (`GaryxWorkspaceDirectoryBrowser`):

- **Path bar**: tappable breadcrumb of the current path; segments jump.
  Focusing turns it into a text field accepting a typed/pasted absolute
  path; submit navigates. Server typed 400s (`invalid_path`, `not_found`,
  `not_a_directory`, `permission_denied`) render inline under the path bar
  in user language; the browser stays where it was.
- **Directory list**: server entries with a git-repo badge on repository
  roots (`gitRepo`); a local filter field narrows the current listing.
- **Name field**: appears with the confirm action, defaulting to the
  selected directory basename, editable before submit; submitted with the
  add (`POST /api/workspaces` upsert — the only tombstone-revival path).
- Directories only, hidden filtered, 500-entry cap, no filesystem mutation
  — unchanged server behavior.
- Other embedders of the directory picker (agent form, bot/automation
  dialogs, settings) get the same upgraded browser; their field-trigger
  form factor is unchanged.

### 5.5 Sent-thread workspace context

- The thread settings surface (title-capsule panel) shows the workspace
  read-only from thread-detail provenance: `implicit` → `No workspace`;
  `explicit` → the `~`-abbreviated root path (rendered even when the path
  is no longer in the root list) plus the worktree indicator where
  applicable. Clients never infer provenance.

### 5.6 Gateway switch isolation

- Workspace picker, directory browser, and rename/remove dialogs close on
  gateway switch. All workspace catalog reads and pin/rename/add mutations
  are guarded by the existing `gatewayRequestToken` pattern: a response
  issued under a stale token must not update state or revive UI (this
  extends the guard to the mutation paths, which lack it today).

## 6. Client architecture

**GaryxMobileCore (pure, SwiftPM-tested) owns:**

- Extended models: `GaryxWorkspaceSummary` gains `pinned`, `threadCount`,
  `lastActivityAt`, `gitRepo`; `GaryxWorkspacesPage` gains `gatewayHome`
  and `workspaceStateInitialized`; `GaryxWorkspaceDirectoryEntry` gains
  `gitRepo` (server serializes camelCase); `GaryxThreadSummary` gains
  `rootWorkspacePath` + `workspaceOrigin`.
- Client methods: `pinWorkspace(path:pinned:)`,
  `renameWorkspace(path:name:)`; `listThreadSummaries` filter param renamed
  to `rootWorkspacePath`; typed decoding of the directories 400 contract
  into a `GaryxWorkspaceDirectoryError` the UI can render per-code.
- `GaryxDraftWorkspaceSelection` tri-state + the resolve-once policy
  (catalog-arrival resolution, selected-row-removal re-resolution,
  explicit-none precedence) as pure functions.
- Path presentation: `gateway_home`-based `~` abbreviation helper replacing
  the local `Users/<x>` heuristic; picker search filtering; server-order
  passthrough presentation (the sorting/filtering half of
  `GaryxMobileWorkspacePresentation` is deleted).
- Directory-browser reducer: path-bar navigation, typed-error stay-put
  behavior, filter application.
- Catalog cache shape: the SWR resource and the persisted catalog snapshot
  carry `{gatewayHome, [GaryxWorkspaceSummary]}` (snapshot version bump;
  the old paths-only field is dropped — a stale cache cold-refreshes once).

**App layer (SwiftUI, feature-specific files):** drawer row chrome +
context menus, drilldown header/toolbar, composer chip + picker sheet,
browser sheet UI, settings read-only context — thin bindings over Core
state, using the shared glass/material and safe-area chrome helpers.

**Deleted second sources:** client workspace sort + worktree/tmp path
filtering, basename-only workspace naming, the local `~` heuristic, the
bare-string draft workspace state.

## 7. Non-goals

- Drawer inline thread subtree, hover cards, overflow show-more — desktop
  presentation, replaced by drilldown navigation on iOS.
- Swipe actions on any row (context menus only, per mobile rules).
- Multi-gateway aggregation; widget changes; directory creation; changing
  thread `workspace_dir` immutability; Codex visual fidelity on iOS.

## 8. Testing

- **Gateway**: membership filter (worktree thread groups under source
  root; runtime-dir match no longer applies), emitted `root_workspace_path`
  / `workspace_origin` fields, cursor-scope rejection across the param
  change, index/migration coverage in the existing summary test family.
- **Core (SwiftPM)**: full-field summary/page/directory decoding (camelCase
  `gitRepo`), typed-400 decoding, server-order passthrough (no client
  sorting — regression-pinned), tri-state resolve-once matrix (pre-catalog
  draft, empty catalog, explicit none precedence, selected-row removal,
  no drift on refresh), create-payload mapping, `gateway_home` abbreviation,
  browser reducer (segment jump, typed path, error stay-put, filter),
  catalog snapshot version bump restore behavior.
- **App**: xcodegen + xcodebuild build; existing workspace drilldown UI
  smoke tests updated to the membership scope; no new UI-only assertions
  where a Core test can carry the behavior.
