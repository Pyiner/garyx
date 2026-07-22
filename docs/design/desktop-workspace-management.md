# Desktop Workspace Selection & Management (Codex 1:1)

Status: v2 — review findings from #TASK-2533 incorporated; approved direction
(implement now, 1:1 visual parity with the Codex app).
Scope: Mac desktop app + gateway workspace APIs. Mobile follows later from the
same server contract.

## 1. Problem

Workspaces are the primary unit of work in Garyx, but the desktop app treats
them as a thin path list:

- No pinning, no rename UI, no recency ordering.
- The new-thread workspace trigger sits at the top of the empty state instead
  of the composer.
- The remote directory browser only supports click-down/click-up navigation:
  no path input, no filtering, no git-repo signal.
- No at-a-glance workspace context (full path, thread count, git branch).
- The web settings variant still uses raw text inputs for `workspace_dir`.
- The client maintains second sources of truth (name rewriting, local
  ordering) that fight the server.

Product direction (owner decision): **copy the Codex app's project model
1:1** — wherever a Garyx surface is the structural equivalent of a Codex
surface, it must look identical (dimensions, radii, spacing, typography,
shadows, hover behavior, icons — the actual SVGs). Two Garyx-specific facts
shape the adaptation:

1. **Pure remote by default.** Workspace directories live on the gateway
   machine. Every directory operation goes through gateway APIs; native macOS
   folder panels and Finder integration are never used.
2. **Single gateway per app session.** The app renders exactly one gateway's
   workspace universe at a time. No cross-gateway aggregation ("remix").

## 2. Reference model and fidelity assets

Captured first-hand from the live Codex desktop app (ChatGPT.app embedded
Chromium) via CDP on 2026-07-21:

- **Sidebar "Projects" section**: collapsible; `+` add button; each row =
  icon + basename; hover reveals `⋯` actions + "new task in project";
  chevron expands the project's task list inline; overflow rows collapse
  behind a show-more/less toggle.
- **Project row hover card**: mark, name, pin toggle, "N conversations",
  clickable abbreviated path.
- **Project actions menu**: Pin / Reveal in Finder / Create permanent
  worktree / Rename / Archive tasks / Remove.
- **Composer project chip**: pill in the composer footer; opens a popover =
  search field + project list (current checked) + footer "New project…" and
  "Work without a project".
- **Create project dialog**: name field + "Choose folder" + info note.

### 2.1 Extracted fidelity assets

Reference assets live at `~/garyx-design-refs/codex-workspace-ux/` on the
capture machine (screenshots contain personal data — never committed):

- `screenshots/01…06.png` — sidebar, row hover + card, actions menu,
  composer chip, picker popover, create dialog.
- `style-tokens.json` + `extract-*.json` — computed styles per element.
- `icons/*.svg` — 30 raw SVGs (project folder, pin, menu items, search,
  check, work-without-project, etc.).
- `REFERENCE.md` — capture method and key measured values.

Key measured values (light mode): project row height 24px / radius 10px /
padding 4px / gap 4px / font 13px weight 445; actions menu & picker popover
radius 15px, padding 4px, frosted near-white background with 0.5px ring +
soft shadow; composer chip = pill, height 28px, padding 0 8px, font 13px.

### 2.2 Fidelity mandate

- Structurally equivalent elements are **pixel-faithful**: sizes, radii,
  paddings, gaps, font size/weight, shadows, hover states, transition
  timing, and icons (use the extracted SVGs verbatim, recolored via
  `currentColor` where needed).
- Captured tokens are light-mode. Dark mode: re-capture with the app in dark
  appearance using the same CDP method
  (`open -a ChatGPT --args --remote-debugging-port=9224`, attach, extract),
  or — if the reference app is unavailable during implementation — map the
  light-mode token structure onto Garyx's existing dark theme variables and
  flag the approximation for review.
- Where Garyx semantics diverge (pure remote, single gateway, no
  local/cloud runner, no ChatGPT memory settings), this design's adaptation
  wins; fidelity applies to everything else.
- Acceptance includes side-by-side comparison against the captured
  screenshots for every surface in §4.

## 3. Invariants preserved

- **Workspace identity is the absolute directory path string.** No workspace
  IDs (`docs/agents/workspace-paths.md`).
- **Thread `workspace_dir` is immutable once set.** The picker changes the
  *draft* selection of an unsent new thread only.
- **"No workspace" keeps its meaning**: the user declined to choose; the
  runtime provisions a private Garyx-managed thread workspace.
- **All directory operations are remote** (gateway APIs). No native folder
  dialogs; no raw-path-only inputs as the primary control.
- **Root workspace rows come from explicit user action** (add / seed), never
  inferred from thread metadata or projections.
- **Thread condition queries go through SQL projections** derived in the
  same transaction as record writes; no read-time repair, no recurring
  backfill.

## 4. UX design

### 4.1 Sidebar Workspaces section

Codex interaction grammar; row geometry follows Garyx's own sidebar. The
original pixel-faithful 24px/4px Codex row read as misaligned next to the
Bots and Pinned sections, so workspace rows now share the bots-row metrics
(32px min-height, 10px left padding, 16px icon, 6px copy gap, 13px/400
label) while keeping the Codex hover/expand behavior:

- **Row**: extracted project-folder SVG + display name (server-provided).
  Hover reveals inline `⋯` (actions) and `+` (new thread here). Chevron
  expands an **inline thread subtree** under the row (see below).
- **Inline thread tree (replaces the secondary rail for workspaces).**
  Codex expands a project's tasks inline; we do the same: expanding a
  workspace row lists its threads (from the workspace-membership projection,
  §5.2) directly under the row. The existing workspace-triggered secondary
  conversation rail (`WorkspaceConversationSidebar` as opened by workspace
  rows) is retired for this purpose. Impact: the 980px responsive-collapse
  contract in `docs/agents/desktop-ui.md` / `CLAUDE.md` is updated in the
  same change; if the secondary rail has remaining non-workspace uses they
  are untouched, otherwise the rail and its funding/collapse rules go with
  it. No overlay variant is added.
- **Overflow**: like Codex, the section shows a bounded number of rows with
  a show-more/less toggle (match captured behavior; expansion state is local
  UI state, keyboard reachable).
- **Ordering** (server-provided, one total order everywhere — sidebar and
  picker): `pinned_at` desc → `last_activity_at` desc → `name` asc →
  normalized `path` asc as the final tie-breaker.
- **Actions menu** (`⋯`), pixel-faithful to the captured menu with adapted
  items: Pin/Unpin · Rename… · New thread · Copy path · Remove. Pure-remote
  substitution: Codex "Reveal in Finder" → **Copy path**. "Create permanent
  worktree" and "Archive tasks" are explicitly not adopted (§7). "Remove"
  soft-deletes the list entry (existing tombstone semantics), never touches
  disk; the confirm dialog states exactly that.
- **Hover card** (delay-appear, faithful layout): display name, pin toggle,
  thread count, `~`-abbreviated path (server-provided `gateway_home`, §5.2 —
  never the Mac-local HOME), git branch when the root is a git repo
  (existing `/api/workspaces/git-status`). The Codex "open in Finder" path
  button becomes a copy-path affordance.

### 4.2 Composer workspace chip + picker

- **Chip**: pill in the composer footer of a new-thread draft (Codex chip
  fidelity), showing the draft selection; "No workspace" state shows the
  captured work-without-project icon + muted label. Clicking opens the
  picker popover anchored to the chip.
- **Picker popover** (pixel-faithful to the captured popover): search field
  (filters name and path) → workspace list in the shared total order,
  current selection checked (extracted check SVG) → footer items
  `Add workspace…` and `No workspace` (extracted icons).
- **Draft selection is an explicit tri-state**, not a nullable string:
  - `path(<absolute path>)` — user picked a workspace.
  - `none` — user explicitly chose No workspace.
  - Unset drafts resolve a default **once at draft creation**: the first
    available row of the shared total order (the gateway list arrives
    pre-sorted, so a pinned row wins over a more recently active one);
    empty list → `none`. A draft that entered before the catalog loaded
    resolves its default once, when the catalog arrives. After resolution
    the draft stores a concrete `path`/`none` and never drifts on refresh
    or list changes — the only sanctioned re-resolution of a live draft is
    removal of its selected workspace.
  - The tri-state flows through route state, draft persistence, composer
    state, and the create payload. "Create thread in X" entry points seed
    the draft with `path(X)`. An explicit `none` is never overridden. The
    `thread-home` rest route carries no workspace param by design — its
    draft composer still resolves and holds a concrete selection in state,
    and the tri-state enters the route the moment the draft becomes an
    explicit `new-thread` route.
- **Mode control**: the local/worktree mode select renders next to the chip
  when the selected workspace is worktree-capable (existing gating).
  Display copy stops saying "Local" (misleading in a pure-remote model):
  the two modes present as **"Direct"** (runs in the workspace directory on
  the gateway) and **"Worktree"**. Internal enum values are unchanged.
- **After send**: the chip disappears; the footer shows the existing
  read-only workspace/branch context. What it shows is driven by
  server-owned provenance (§5.5): `implicit` → "No workspace"; `explicit` →
  the path (displayed even when it is not in the root list — membership in
  the root list is presentation-irrelevant).
- The old empty-state top trigger is removed; Resume session stays.
- **Side chats**: a side chat forks its workspace from the source thread —
  there is nothing to choose, so the chip never renders for side chats.
  The composer shows the inherited workspace read-only from the first
  frame (its former no-op workspace callbacks are removed).
- All other picker embedders (tasks panel, agent form, MCP settings,
  automation/bot dialogs via `DirectoryInput`, gateway settings, web
  settings) keep the field-trigger form factor and open the same picker
  content.

### 4.3 Add workspace flow (remote directory browser v2)

One dialog, Codex create-project fidelity where structurally shared (title
bar, name field, primary button), with the folder chooser replaced by the
remote browser:

- **Path bar**: editable breadcrumb. Segments jump on click; focus/typing
  turns it into a text field accepting a pasted/typed absolute path, Enter
  navigates. Errors from the server (§5.4) render inline; the browser stays
  where it was.
- **Directory list**: gateway-listed subdirectories with a git-repo badge on
  entries that are repository roots. A local filter field narrows the
  current listing client-side.
- **Name field**: defaults to the selected directory's basename, editable
  before confirm; submitted with the add.
- **Confirm**: `Add workspace` — the existing `POST /api/workspaces` upsert.
  Explicit Add is the **only** operation that may revive a tombstoned row
  (§5.3).
- Hidden directories stay filtered; directories only; 500-entry cap; no
  directory creation or any filesystem mutation from this surface.

### 4.4 Single-gateway scoping and switch isolation

- Workspace list, ordering, directory browsing, git status: all scoped to
  the connected gateway. Switching gateways swaps the entire universe.
- **Epoch protocol** (async isolation): every workspace catalog, directory,
  git-status, and mutation call is tagged with the gateway connection epoch
  it was issued under. On gateway switch: open pickers/browsers/dialogs
  close, controllers reset, and responses (including mutation results)
  carrying a stale epoch are discarded — they must not update state or
  revive UI. A late response from gateway A arriving after the switch to B
  is a no-op (§8 test).
- Non-goals: multi-gateway aggregation, per-workspace gateway tags,
  cross-gateway pin/recency merging.

## 5. Server changes

### 5.1 Schema (`garyx_db/workspaces.rs`)

Versioned migration adding to `workspaces`:

- `pinned_at` (nullable RFC3339 UTC). Set on pin; cleared on unpin. A
  removed-then-re-added workspace starts unpinned (`pinned_at` does not
  survive the tombstone).

No workspace ID, no new identity tables.

### 5.2 Workspace membership projection and `/api/workspaces`

**Per-thread root-workspace membership.** The existing
`thread_meta.workspace_dir` is the *runtime* directory and is wrong for
grouping: worktree threads carry the Garyx-managed worktree path while the
root lives in `worktree.source_workspace_dir`. Add a projected
`root_workspace_path` column (thread-meta projection):

- explicit thread → its chosen absolute path;
- worktree thread → `worktree.source_workspace_dir`;
- implicit No-workspace thread → `NULL`.

Derived in the same transaction as every record write. Historical rows are
populated by an import-generation-aware, versioned one-shot cutover with a
durable marker (standard projection-cutover machinery); no read-time repair.
The projection never creates root workspace rows — list queries join
non-tombstoned `workspaces` rows only.

**`GET /api/workspaces` response** (additive):

```json
{
  "workspace_state_initialized": true,
  "gateway_home": "/Users/test",
  "workspaces": [
    {
      "name": "garyx",
      "path": "/Users/test/repos/garyx",
      "pinned": true,
      "thread_count": 42,
      "last_activity_at": "2026-07-20T16:44:00Z",
      "git_repo": true
    }
  ]
}
```

- `thread_count` / `last_activity_at` are SQL `COUNT` / `MAX` over the
  membership projection at query time (no maintained aggregate counters).
  Counted: task-backed threads yes; side chats hidden from lists
  (`default_list_hidden=1`) no; archived/deleted threads no (their
  projections are gone). `last_activity_at` uses the same per-thread
  activity timestamp that orders `recent_threads` — one canonical source.
- `git_repo`: root `.git` check at list time.
- `gateway_home`: the gateway machine's home directory, used by clients for
  `~` abbreviation. Clients must not use the local HOME.
- Entries are returned pre-sorted in the shared total order (§4.1); clients
  render server order verbatim.

### 5.3 Pin and rename: point mutations only

The existing upsert both resets omitted fields and clears `deleted_at`
(tombstone revival), so pin/rename must not ride on it:

- `POST /api/workspaces/pin` `{path, pinned}` — updates `pinned_at` only.
- `POST /api/workspaces/rename` `{path, name}` — updates `name` only.
- Both are **active-row-only**: they update a non-tombstoned row in place;
  unknown or tombstoned path → 404. They never insert or revive. Explicit
  Add (`POST /api/workspaces`) remains the only revival path.
- Concurrency: row-level point updates; a rename racing a remove either
  lands before the tombstone or 404s — never revives.

### 5.4 `/api/workspaces/directories`

- Entries gain `git_repo: bool`.
- **Error contract replaces the current silent fallbacks** (today: missing
  or relative paths fall back to HOME, file paths fall back to the parent —
  all 200). New contract: request without `path` → listing starts at the
  gateway home. Request with `path` → typed, structured 400s the client
  renders inline while staying put: `invalid_path` (not absolute),
  `not_found`, `not_a_directory`, `permission_denied`.
- Keep: directories only, hidden filtered, 500-entry cap, name sort.

### 5.5 Thread workspace provenance

Thread records gain a persisted, server-owned `workspace_origin`:
`explicit` (user-chosen path, including worktree threads) or `implicit`
(Garyx-managed No-workspace directory). It is written into the record
when the workspace first lands (implicit provisioning and explicit
update paths both stamp it) and never rewritten afterwards; the
projection persists it as a column and thread summaries deliver it to
clients. Records that predate the field fall back to a deterministic
server-side inference (the managed path embeds the thread's own
sanitized id), applied once by the membership cutover for existing
projection rows. Clients never compute provenance — the sent-thread
footer renders the delivered value, and an explicit path renders even
when it is not in the root list.

## 6. Client changes (desktop)

**Single source of truth cleanup (blocking prerequisite):**

- Delete the main-store basename rewrite of server names and the
  client-side `sortWorkspaces` reordering; render server name/order
  verbatim through hydration, add, rename, and pin.
- Carry `{path, name, pinned, thread_count, last_activity_at, git_repo}`
  through the shared contract, main store, IPC, preload, and renderer
  facade (today the add path forces basename and drops name).
- Contract tests pin server-order/name passthrough after every mutation.

**Platform-neutral picker architecture:**

- The picker presentation/controller is platform-free; data access goes
  through an injected adapter interface: Electron adapter backed by
  `window.garyxDesktop` IPC, and a Web adapter backed by `web-api.ts` HTTP
  (which gains workspace list/add/pin/rename/directories/git-status
  methods). The web settings surface (`use-web-settings-state.ts`,
  `WebSettingsPage`) replaces its raw `workspace_dir` text inputs with the
  same picker via the Web adapter.
- Verified embedder list (all move to the new picker content):
  `NewThreadEmptyState`, `TasksPanel`, `AgentFormDialog`,
  `McpSettingsPanel`, `AppShell` add dialog, `AutomationDialog` /
  `AddBotDialog` / `EditBotDialog` (via `DirectoryInput`),
  `WebSettingsPage` (both fields), `GatewaySettingsPanel`.

**Composer integration (owner-correct):**

- The composer footer slot is owned by `ComposerForm` (`composerContext`);
  the chip and mode control are injected through `ComposerForm` props.
  `ThreadPage` wires draft state through; `NewThreadEmptyState` loses its
  top trigger. `SideChatPanel` drops its no-op workspace callbacks and
  passes the real controller.
- Draft tri-state (§4.2) lives in the draft/route state layer; create
  payload maps `none` → no `workspace_dir`, `path` → `workspace_dir`.

**Sidebar:**

- `WorkspaceThreadSidebar`: Codex row grammar, hover actions, actions menu,
  hover card, inline thread subtree, overflow toggle, server ordering.
- Retire the workspace-triggered secondary conversation rail path
  (`WorkspaceConversationSidebar` usage, `AppLeftRail`/`AppShell` rail
  state for workspaces) and update the 980px responsive contract docs in
  the same change.

**Fidelity assets:** extracted SVGs land as shared icon components; style
tokens land as the workspace-surface stylesheet. No personal-data
screenshots enter the repo.

**Gateway epoch (§4.4):** workspace controller tags requests with the
connection epoch; switch closes open workspace UI and discards stale
responses.

## 7. Non-goals

- Multi-gateway workspace aggregation ("remix") — single gateway per app
  session; switching swaps the universe.
- Directory creation or any filesystem mutation from the browser surface.
- Codex "Create permanent worktree" (Garyx worktrees are thread-scoped via
  the mode control; no workspace-scoped permanent worktree concept) and
  "Archive tasks" (bulk thread archiving from the workspace menu) — both
  explicitly not adopted in v1.
- Reveal in Finder (pure remote) — replaced by Copy path.
- Workspace mark colors, sidebar folder nesting.
- Changing thread `workspace_dir` immutability.
- Mobile UI work (server contract is shared; mobile adapts later).

## 8. Testing

- **Gateway**: migration + cutover (marker, generation-awareness,
  worktree→source mapping, implicit→NULL); membership projection derivation
  on create/archive/delete in-transaction; count/activity queries
  (side-chat exclusion, task threads counted); pin/rename point mutations
  (404 on tombstone, no revival, remove/re-add resets pin); directories
  typed 400s (relative/missing/file/permission) and no-path HOME start;
  ordering including name and path tie-breakers; `gateway_home` presence;
  `workspace_origin` set for explicit/worktree/implicit creation and
  immutable thereafter.
- **Desktop renderer**: draft tri-state — default resolved once at draft
  creation (no drift on refresh), explicit `none` respected, route
  round-trip, create payload mapping; sent-thread footer renders from
  `workspace_origin` (explicit path not in root list still shows the path);
  server order/name passthrough after add/rename/pin (second-source
  regression tests); picker search/ordering; path-bar segment jump, typed
  navigation, inline error stay-put; epoch test — slow gateway-A response
  arriving after switch to B is discarded; side-chat chip parity.
- **Fidelity**: packaged-app CDP side-by-side against the captured
  screenshots for sidebar rows, hover card, actions menu, chip, picker,
  and add dialog (light mode; dark mode per §2.2).
- Web: settings picker works through the HTTP adapter end to end.
