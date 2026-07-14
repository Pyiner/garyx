# Capsule Favorites: All / Favorites Gallery Tabs

Status: draft v2 (post-review revision; v1 review = #TASK-2276 FAIL, all blockers addressed)
Date: 2026-07-14

## Problem

The capsule gallery (desktop `CapsulesPanel`, iOS `GaryxCapsulesView`) is a single
flat list ordered by `updated_at DESC`. As capsules accumulate, the user cannot
mark important ones or view only those. Requirement: add a favorite toggle per
capsule and an **All / Favorites** two-tab switch on both gallery surfaces.

## Design Summary

| Layer | Change |
| --- | --- |
| DB (`garyx-gateway/src/garyx_db/mod.rs`) | Add nullable `favorited_at TEXT` column to `capsules` (NULL = not favorited). New `set_capsule_favorite(id, bool)` point write. No `revision` bump, no `updated_at` touch. |
| HTTP (`capsules.rs` + `route_graph.rs`) | `PUT /api/capsules/{id}/favorite` and `DELETE /api/capsules/{id}/favorite`. `GET /api/capsules` gains no params; each record now serializes `favorited_at`. |
| Desktop | `CapsulesPanel` gallery header gets an All/Favorites segmented control (reuse `tasks-segmented` pattern). Gallery card restructured into a non-interactive container with an inner open button and a star toggle. Favorite mutations go through a per-capsule desired-state reducer (pure TS, unit-tested). Filtering is client-side. |
| iOS | `GaryxCapsulesView` gets a native segmented `Picker`. Favorite toggle in the card long-press context menu and the focused-preview ellipsis menu; star badge on favorited cards. Filter + favorite-mutation reducer + response DTO live in `GaryxMobileCore` with SwiftPM tests; the persisted catalog projection (`GaryxCachedCapsule`) round-trips `favoritedAt`. |
| MCP | Capsule `summary_json` (shared by `capsule_create` / `capsule_update` responses and `capsule_list`) additionally exposes `favorited` (read-only). No agent-facing write path for favorites. |

## Key Decisions

### D1. Column on `capsules`, not a join table

Thread pins use a separate `thread_pins` table because thread truth lives in
`thread_records` with its own store abstraction. Capsules are already a plain
gateway-owned SQLite table, so a nullable `favorited_at TEXT` column is the
smallest correct surface: `list_capsules` returns the flag in the same row read,
no join, no second query. One column expresses both state (NULL vs set) and
recency of favoriting.

Migration follows the existing `PRAGMA table_info` probe + `ALTER TABLE ... ADD
COLUMN` pattern (`ensure_thread_meta_projection_columns` precedent), invoked
right after `capsules` table creation. Idempotent; STRICT-compatible (verified:
adding a nullable TEXT column to a STRICT table is legal).

### D2. Favorite toggle is a dedicated point write — NOT `update_capsule`

`update_capsule` bumps `revision` and touches `updated_at` on every write.
Clients key cached capsule HTML on `(id, revision)`
(`GaryxCapsuleHTMLCacheKey`), and thumbnails on `(id, revision, rendition,
schema)` (desktop `capsule-thumbnail-store.ts`, iOS
`GaryxCapsuleThumbnailRendering.swift`). Favoriting is metadata-only; routing
it through `update_capsule` would needlessly invalidate those caches and
re-download up-to-5MiB payloads, and would reorder the `updated_at DESC`
gallery. Therefore:

```rust
// garyx_db: new method
fn set_capsule_favorite(&self, id: &str, favorited: bool)
    -> Result<Option<CapsuleRecord>, ...>
// favorited == true:  UPDATE capsules SET favorited_at = COALESCE(favorited_at, ?now) WHERE id = ?
// favorited == false: UPDATE capsules SET favorited_at = NULL WHERE id = ?
// returns the fresh record (None if id unknown). revision and updated_at untouched.
```

`COALESCE` keeps repeated PUTs idempotent (first-favorite time preserved).
`CapsuleUpdateParams` / `CapsuleUpdateDraft` / MCP `capsule_update` are NOT
extended — favoriting is a user action, not an agent action.

### D3. HTTP contract

```
PUT    /api/capsules/{id}/favorite  -> 200 {"favorited": true,  "capsule": {...}}
DELETE /api/capsules/{id}/favorite  -> 200 {"favorited": false, "capsule": {...}}
                                       404 {"error": "capsule not found"}
```

Both idempotent on repeat. The PUT/DELETE toggle route *shape* follows the
thread-pin precedent; the 404-on-unknown-id behavior deliberately follows the
existing capsule DELETE route instead (thread-pin DELETE returns 200 for
unknown targets — capsules have a real existence check, so 404 is correct
here). Registered in `route_graph.rs` next to the existing capsule routes,
behind gateway auth. `CapsuleRecord` serialization now includes
`favorited_at: Option<String>` (RFC3339 UTC, consistent with the other
timestamps; clients use it only as a flag).

### D4. Two tabs are client-side filters; no server filter param

`GET /api/capsules` already returns the full list unpaginated. Adding a server
filter would be a second source of truth for no gain. Both tabs render from the
one fetched page:

- **All**: every capsule, `updated_at DESC` (unchanged).
- **Favorites**: `favorited_at != null`, same `updated_at DESC` order. Keeping
  one ordering avoids a second sort mental model; the tab is a filter, not a
  different gallery.

Tab selection is local view state (not routed, not persisted). Default = All.

### D5. Sort order unchanged on both tabs

Server keeps `ORDER BY updated_at DESC, id ASC`. Favorites tab filters that
same order. (`favorited_at` recency ordering was considered and rejected to
keep one consistent gallery order.)

### D6. Favorite writes: per-capsule desired-state reducer + refresh merge

HTTP idempotency does not solve client-side response reordering. Rapid
PUT→DELETE double-taps, out-of-order responses, and a full list refresh racing
an in-flight favorite write can all resurrect a stale value over the user's
last intent. Both clients therefore run the same **desired-state** rule set,
implemented once per platform as a pure, unit-testable reducer (no timers, no
network inside):

The reducer tracks, per capsule id,
`{ serverFavoritedAt, desiredFavorited, inFlight }`, plus one global
`favoritesGeneration` counter that is bumped every time any favorite mutation
**starts** and every time one **settles** (success or failure). List refreshes
capture the generation at *request send time*; a mismatch at response time
marks that response's favorite fields stale (see refresh merge below). This is
what stops a list GET that was issued before/while a mutation flew from
resurrecting the pre-mutation value after settle — HTTP idempotency and a
list-vs-list request id cannot catch that cross-operation reorder.

- **Toggle (user tap)**: set `desiredFavorited` to the new intent; UI renders
  desired state immediately. If no request is in flight for this id, emit a
  send effect; if one is in flight, do not emit — at most one favorite request
  per capsule is ever in flight (per-capsule serialization).
- **Response arrived (success)**: record the returned `capsule.favorited_at`
  as `serverFavoritedAt`. If `desiredFavorited` no longer matches the server
  state (user toggled again while the request flew), emit a follow-up send for
  the current desired state; otherwise settle (`inFlight = false`, desired
  becomes authoritative = server).
- **Response arrived (failure)**: revert `desiredFavorited` to
  `serverFavoritedAt`, settle, surface a non-blocking error.
- **List refresh landed**: the response carries the `favoritesGeneration`
  captured when the request was sent.
  - If that generation still equals the current one **and** the capsule has no
    pending mutation (`inFlight == false` and desired == server), adopt the
    refreshed record wholesale, including `favorited_at`.
  - Otherwise (stale generation — a mutation started or settled after this
    request was sent — or a pending mutation on this capsule), adopt the
    refreshed record's *other* fields but keep the reducer's favorite state
    (`serverFavoritedAt` / `desiredFavorited`), which is strictly newer than
    the response.
  - When no mutation is active the generation is stable, so ordinary refreshes
    (including favorite changes made by another client) are adopted normally —
    eventual consistency is preserved.
  This merge happens wherever the refreshed list is written (`loadCapsules` on
  desktop, `refreshCapsules` / gateway full-refresh on iOS), **before** any
  persistence, so a stale response can never be written into the iOS catalog
  snapshot.

Placement: iOS reducer lives in `GaryxMobileCore` (SwiftPM-tested); desktop
reducer is a pure TS module under the renderer, unit-tested via
`npm run test:unit`. View layers only dispatch taps and render reducer output.

## Client Changes

### Desktop (`desktop/garyx-desktop`)

- `shared/contracts/capsule.ts`: `DesktopCapsuleSummary` gains
  `favoritedAt: string | null`; new `SetCapsuleFavoriteInput { capsuleId, favorited }`
  and a `SetCapsuleFavoriteResult { favorited, capsule }` mirroring the HTTP
  body.
- `main/garyx-client/capsules.ts`: map `favorited_at` in `mapCapsuleSummary`;
  new `setCapsuleFavorite` hitting PUT/DELETE.
- preload + IPC: expose `setCapsuleFavorite` alongside the existing five
  capsule bridges.
- `CapsulesPanel.tsx`:
  - **Card restructure (required)**: today the whole `CapsuleGalleryCard` is
    one `<button>`; nesting a star `<button>` inside it is invalid HTML and
    would double-trigger open-preview. The card becomes a non-interactive
    container with two separate interactive children: the main open control
    (covering the preview/title area) and a star toggle button. Star button:
    `aria-pressed` reflects favorited state, accessible label
    Favorite/Unfavorite, favorited state **always visible** (filled star),
    the toggle reachable on `:hover` and `:focus-within` (not hover-only).
  - Gallery header: segmented control `All | Favorites` following the
    `tasks-segmented` markup/CSS recipe (feature stylesheet `capsules.css`,
    not shell chrome).
  - Favorite mutations dispatch through the D6 reducer module; the existing
    list request-id guard stays for list-vs-list races, and `loadCapsules`
    results pass through the D6 refresh-merge before hitting state.
  - Favorites tab empty state: "No favorite Capsules yet."
  - Filtering: pure function over the merged capsule list, unit-tested.

### iOS (`mobile/garyx-mobile`)

- `GaryxMobileCore/GaryxGatewayCapsuleModels.swift`: `GaryxCapsuleSummary`
  gains `favoritedAt` (decode both `favorited_at` / `favoritedAt` like the
  existing keys); `isFavorited` computed helper. New Core DTO
  `GaryxCapsuleFavoriteResponse { favorited, capsule }` decoding the PUT/DELETE
  body.
- `GaryxMobileCore/GaryxGatewayClient.swift`: `setCapsuleFavorite(id:favorited:)`
  → PUT/DELETE, returns the DTO.
- `GaryxMobileCore` — all favorite business logic sits in Core so SwiftPM
  actually compiles and tests it (SwiftPM builds only `GaryxMobileCore`, not
  the app target):
  - gallery tab filter as a pure function (e.g. `GaryxCapsuleGalleryTab.filter(_:)`);
  - the D6 favorite-mutation reducer (toggle / success / failure /
    refresh-merge transitions over the capsule array + per-id mutation state).
- **Persisted catalog projection (required)**: `GaryxCachedCapsule` in
  `GaryxMobileCatalogCache.swift` gains `favoritedAt`, with both directions of
  the mapping updated — restore (`GaryxMobileModel+CatalogCache.swift` restore
  path) and save (snapshot path). After a favorite mutation settles
  successfully, the catalog snapshot is re-persisted so offline /
  stale-while-refresh restores don't drop favorite state. SwiftPM round-trip
  test: summary → cached → summary preserves `favoritedAt`.
- `GaryxMobileModel+Capsules.swift`: thin orchestration only — dispatch to the
  Core reducer, perform the network effect it emits, feed responses back, and
  persist the catalog snapshot on settle. Must NOT touch the HTML/thumbnail
  caches (favorite flips don't change `revision`, so existing prune paths keep
  `(id, revision)` entries alive — validated by an explicit regression test).
- `GaryxCapsulesView`: segmented `Picker` (All / Favorites) in the glass top
  bar area, native `.pickerStyle(.segmented)`, monochrome (no green tint).
  Favorite toggle entry points: the gallery card's existing long-press
  `contextMenu` (favorite listed above the destructive Delete, not adjacent),
  and the focused preview's ellipsis menu. Favorited cards show a star badge.
  Favorites empty state.
- If any new Core/App file is added: run `xcodegen generate` and commit the
  pbxproj.

### MCP (read-only surface)

Capsule `summary_json` is shared: `capsule_list` items **and** the
`capsule_create` / `capsule_update` tool responses all go through it, so
`"favorited": bool` appears in every capsule MCP summary. This is intentional
(read-only visibility, no split mapper). Tests: MCP summary output carries
`favorited`; `capsule_update` params reject/ignore any favorite field (no
agent write path).

## Out of Scope

- Server-side filter/pagination params on `GET /api/capsules`.
- Favorite write access for agents (MCP).
- Any change to capsule HTML storage, serve, CSP, thumbnails, or deletion.
- Sorting favorites by `favorited_at`.
- Legacy-gateway compatibility shims (desktop/gateway ship together).

## Validation Plan

- **Gateway** (`cargo test -p garyx-gateway --lib`):
  - migration: fresh DB has column; pre-existing DB gains column; re-run is a
    no-op.
  - `set_capsule_favorite`: set → record has `favorited_at`; repeat set keeps
    original timestamp; unset → NULL; unknown id → None; `revision` and
    `updated_at` unchanged across all of these.
  - routes: PUT/DELETE happy path + 404 + auth; `GET /api/capsules` body
    carries `favorited_at`.
  - MCP: summary output carries `favorited`; update params carry no favorite
    field.
- **Desktop** (`npm run test:unit`): `mapCapsuleSummary` maps `favorited_at`;
  gallery filter pure function (all/favorites/empty); D6 reducer — double-tap
  PUT→DELETE emits serialized sends and settles on final intent, failure
  reverts, refresh-merge keeps pending desired state and adopts settled
  server state. Generation matrix: refresh sent before mutation → mutation
  settles → late refresh lands (must not clobber); refresh sent during pending
  mutation, landing after settle (must not clobber); refresh sent after settle
  (adopted normally).
- **iOS** (`swift test` in GaryxMobileCore, then `xcodebuild` app compile):
  summary decode with/without `favorited_at`; favorite response DTO decode;
  tab filter; D6 reducer transitions (same matrix as desktop, including the
  three-case generation matrix); cached-capsule round-trip preserves
  `favoritedAt`; stale-refresh merge output is what gets persisted — a late
  pre-mutation list response never pollutes the catalog snapshot; regression:
  favorite settle leaves `(id, revision)` HTML/thumbnail cache entries intact
  through the prune path.
- **End-to-end**: create two capsules via MCP, favorite one via PUT, verify
  `GET /api/capsules` flags it, flip tabs in both clients (desktop packaged
  `dist:dir` pass required — preload/IPC wiring changed).
