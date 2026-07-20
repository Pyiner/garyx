# Desktop Workspace Selection & Management (Codex-style)

Status: draft for review
Scope: Mac desktop app + gateway workspace APIs. Mobile follows later from the
same server contract.

## 1. Problem

Workspaces are the primary unit of work in Garyx, but the desktop app treats
them as a thin path list:

- No pinning, no rename UI, no recency ordering — the sidebar shows a flat
  alphabetical-ish list from `/api/workspaces`.
- The new-thread workspace trigger sits at the top of the empty state instead
  of the composer, so the selection feels detached from the message being
  composed.
- The remote directory browser (`/api/workspaces/directories`) only supports
  click-down/click-up navigation: no path input, no filtering, no git-repo
  signal — picking a deep directory on the gateway machine is tedious.
- There is no at-a-glance workspace context (full path, thread count, git
  branch) anywhere; rows show only a basename.
- The web settings variant still uses a raw text `Input` for
  `workspace_dir`, violating `docs/agents/workspace-paths.md`.

The product direction is to copy the Codex app's project model — projects as
first-class sidebar citizens with pin/rename/actions, a searchable project
picker in the composer, and a lightweight create-project flow — adapted to two
Garyx-specific facts:

1. **Pure remote by default.** Workspace directories live on the gateway
   machine. Every directory operation goes through gateway APIs; native macOS
   folder panels and Finder integration are never used.
2. **Single gateway per app session.** The app renders exactly one gateway's
   workspace universe at a time. No cross-gateway aggregation or mixing
   ("remix") of workspace lists.

## 2. Reference model (captured first-hand from Codex app 2026-07-21)

Observed via CDP on the live Codex desktop app:

- **Sidebar "Projects" section**: collapsible, `+` add button; each row =
  mark/icon + basename, hover reveals `⋯` actions + "new task in project",
  chevron expands the project's task list. Overflow rows collapse behind
  "show less/more".
- **Project row hover card**: mark, name, pin toggle, "N conversations",
  clickable `~/abbreviated/path` (opens Finder — not applicable to us).
- **Project actions menu**: Pin project / Reveal in Finder / Create permanent
  worktree / Rename project / Archive tasks / Remove.
- **Composer project chip**: bottom toolbar shows current project; clicking
  opens a popover = search field + project list (current item checked) +
  footer items "New project…" and "Work without a project".
- **Create project dialog**: name field + "Choose folder" (native panel) +
  settings; a project may exist without a folder.
- Codex also has "Choose where to run this chat" (local vs cloud) — not
  applicable; Garyx is always the connected gateway.

## 3. Invariants preserved

These existing contracts are load-bearing and unchanged:

- **Workspace identity is the absolute directory path string.** No workspace
  IDs (`docs/agents/workspace-paths.md`). New metadata hangs off the path key
  in the gateway `workspaces` table.
- **Thread `workspace_dir` is immutable once set.** The picker changes the
  *draft* workspace of an unsent new thread; existing threads never switch.
- **"No workspace" keeps its meaning**: the user declined to choose; the
  runtime still provisions a private Garyx-managed thread workspace.
- **All directory operations are remote** (gateway APIs). No native folder
  dialogs, no raw-path-only inputs as the primary control.
- **Root workspace rows come from user action** (explicit add / seed), never
  inferred from thread metadata.
- **Thread condition queries go through SQL projections** written in the same
  transaction as record writes.

## 4. UX design

### 4.1 Sidebar Workspaces section

Keep the existing collapsible "Workspaces" section and per-workspace thread
expansion, upgraded to the Codex row grammar:

- **Row**: workspace icon (git-repo variant when the root is a git repo) +
  display name. Hover reveals two inline buttons: `⋯` (actions menu) and `+`
  (new thread in this workspace). Chevron expands that workspace's threads
  (existing behavior).
- **Ordering**: pinned first (most recently pinned on top), then by last
  thread activity (most recent first), then name. One ordering everywhere —
  sidebar and picker share it.
- **Actions menu** (`⋯`): Pin/Unpin · Rename… · New thread · Copy path ·
  Remove. "Remove" soft-deletes the list entry (existing tombstone semantics)
  and never touches the directory on disk; confirm dialog states exactly
  that. No Finder item — the pure-remote replacement for "Reveal in Finder"
  is Copy path.
- **Hover card** (Codex-style, after a short delay): display name, full
  path (`~`-abbreviated relative to the gateway home when applicable), thread
  count, current git branch when the root is a git repo (from the existing
  `/api/workspaces/git-status`), pin toggle.

### 4.2 Composer workspace chip + picker

The workspace selection for a new thread moves into the composer footer
toolbar (this also aligns with `docs/agents/desktop-ui.md`, which already
places workspace/branch context in the composer footer):

- **Chip**: `⌂ garyx` style button in the composer footer of a new-thread
  draft. Shows the draft workspace name; "No workspace" state shows a muted
  label. Clicking opens the picker popover anchored to the chip.
- **Picker popover** (replaces the current centered `WorkspaceSelectDialog`
  presentation; same component contract, new anchor and layout):
  - Search field (filters by name and path).
  - Workspace list in the shared ordering, current draft selection checked.
    Each row: icon + name + dimmed abbreviated path.
  - Footer: `Add workspace…` (opens the add flow, §4.3) and `No workspace`.
- **Mode control**: the existing local/worktree mode select renders next to
  the chip, only when the selected workspace is worktree-capable (existing
  gating).
- **Defaulting**: a fresh new-thread draft preselects the most recently used
  workspace (first non-pinned-biased row of the shared ordering: i.e. the
  workspace with the latest thread activity). "Create thread in X" from the
  sidebar preselects X. Explicit "No workspace" is respected and not
  overridden.
- The old empty-state top trigger is removed. The empty state keeps Resume
  session and other content; workspace context lives only in the composer.
- **Immutability stays visible**: on threads that already have a workspace,
  the footer keeps the existing read-only workspace/branch context (no chip
  affordance to change it).

All other `WorkspacePathPicker` embedders (agent form, automation dialog, MCP
settings, gateway settings, tasks panel) keep the field-trigger form factor
but open the same upgraded picker content. The web settings variant's raw
text inputs are replaced with the same picker — closing the known
contract divergence.

### 4.3 Add workspace flow (remote directory browser v2)

One dialog, Codex "create project" adapted to remote browsing:

- **Path bar**: editable breadcrumb. Displays the current path as segments
  (click any segment to jump); focus/typing turns it into a text field where
  an absolute path can be pasted or typed, Enter navigates. Invalid or
  non-directory paths show an inline error and stay put.
- **Directory list**: gateway-listed subdirectories (existing API), with a
  git-repo badge on entries that are git repository roots. A local filter
  field narrows the current listing client-side.
- **Name field**: defaults to the selected directory's basename, editable
  before confirming (persisted as the workspace display name).
- **Confirm**: `Add workspace` — upserts `{path, name}` via the existing
  `POST /api/workspaces`.
- Hidden directories stay filtered out; no directory creation in v1 (the
  gateway filesystem is browse-only from this surface).

### 4.4 Single-gateway scoping (explicit non-remix)

- Workspace list, ordering metadata, directory browsing, and git status are
  all scoped to the currently connected gateway. Switching gateways swaps the
  entire workspace universe.
- Non-goals: aggregating workspaces from multiple gateways into one list,
  per-workspace gateway tags, cross-gateway pin/recency merging. The app
  holds one gateway connection; workspace UX assumes it.

## 5. Server changes

### 5.1 Schema (`garyx_db/workspaces.rs`)

Add to the `workspaces` table (versioned migration):

- `pinned_at` (nullable RFC3339 UTC): set on pin, cleared on unpin.

No workspace ID, no new tables for workspace identity.

### 5.2 `/api/workspaces` response

Extend each workspace entry (additive):

```json
{
  "name": "garyx",
  "path": "/Users/pyiner/repos/garyx",
  "pinned": true,
  "thread_count": 122,
  "last_activity_at": "2026-07-20T16:44:00Z" | null,
  "git_repo": true
}
```

- `thread_count` / `last_activity_at` come from a SQL projection keyed by
  workspace path. If the current thread projections lack a queryable
  `workspace_dir` column, add the column and its write-path derivation (same
  transaction as record writes, per repository contracts) — no read-time
  scans, no backfill jobs beyond the standard versioned cutover that
  populates the new column once.
- `git_repo` is computed at list time by the gateway (root `.git` check —
  the list is small; this is a handful of stats).
- Server returns entries pre-sorted in the shared ordering (pinned by
  `pinned_at` desc, then `last_activity_at` desc, then name); clients render
  in order.

### 5.3 Pin endpoint

`POST /api/workspaces/pin` with `{path, pinned: bool}` (or an extension of
the existing upsert body — implementer's choice, but pinning must not be
conflated with rename upserts that also reset `name`). Rename continues via
the existing upsert.

### 5.4 `/api/workspaces/directories`

Extend the listing (additive):

- Each entry gains `git_repo: bool` (entry root contains `.git`).
- The endpoint already accepts an arbitrary absolute `path`; it becomes the
  target for the editable path bar. Error contract for
  missing/non-directory paths stays a structured 400 the client can render
  inline.
- Keep: directories only, hidden filtered, 500-entry cap, name sort.

## 6. Client changes (desktop)

- `WorkspaceThreadSidebar`: row hover actions, actions menu, hover card,
  shared ordering (server order), pin/rename wiring.
- `WorkspacePathPicker` family: picker popover layout (search + ordered list
  + footer actions), composer chip trigger for new-thread drafts,
  field-trigger form for embedders, add-workspace dialog v2 (editable path
  bar, git badges, filter, name field).
- `NewThreadEmptyState`: remove top workspace trigger; move workspace chip +
  mode select into the composer footer toolbar.
- `main/garyx-client/workspaces.ts` + `shared/contracts/workspace.ts`: map
  the extended fields (`pinned`, `thread_count`, `last_activity_at`,
  `git_repo`) instead of hardcoding.
- `WebSettingsPage`: replace raw `workspace_dir` text inputs with the shared
  picker.
- i18n for all new strings; keyboard navigation in picker and path bar.

## 7. Non-goals

- Multi-gateway workspace aggregation (explicitly out — single gateway per
  app session).
- Directory creation / any filesystem mutation from the browser surface.
- Workspace mark colors, archiving of workspaces, folder nesting of the
  sidebar list.
- Changing thread `workspace_dir` immutability.
- Mobile UI work (server contract is shared; mobile adapts later).

## 8. Testing

- Gateway: unit tests for migration, pin endpoint, extended list response
  (ordering: pinned/activity/name), projection column derivation
  (thread create/delete updates count and activity in the same
  transaction), directories `git_repo` flag, path-bar error contract.
- Desktop: renderer unit tests for picker ordering/filtering, chip
  state transitions (draft → sent immutability), add-dialog path bar
  navigation (segments, typed path, invalid path), and removal
  confirmation copy. Packaged-app CDP pass for the composer chip, popover,
  sidebar hover surfaces, and web settings picker convergence.
