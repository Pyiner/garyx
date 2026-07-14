# Capsule Favorites: All / Favorites Gallery Tabs

Status: draft (pending review)
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
| HTTP (`capsules.rs` + `route_graph.rs`) | `PUT /api/capsules/{id}/favorite` and `DELETE /api/capsules/{id}/favorite`, modeled on the thread-pin routes. `GET /api/capsules` gains no params; each record now serializes `favorited_at`. |
| Desktop | `CapsulesPanel` gallery header gets an All/Favorites segmented control (reuse `tasks-segmented` pattern); gallery cards get a star toggle. Filtering is client-side. |
| iOS | `GaryxCapsulesView` gets a native segmented `Picker`; cards get favorite toggle in the existing long-press/ellipsis affordance plus a star badge. Filtering logic lives in `GaryxMobileCore` with SwiftPM tests. |
| MCP | `capsule_list` summary JSON additionally exposes `favorited` (read-only). No agent-facing write path for favorites. |

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
right after `capsules` table creation. Idempotent; STRICT-compatible.

### D2. Favorite toggle is a dedicated point write — NOT `update_capsule`

`update_capsule` bumps `revision` on every write, and clients key their cached
HTML/thumbnails on `(id, revision)` (`GaryxCapsuleHTMLCacheKey`). Favoriting is
metadata-only; routing it through `update_capsule` would needlessly invalidate
HTML caches and re-download up-to-5MiB payloads. Therefore:

```rust
// garyx_db: new method
fn set_capsule_favorite(&self, id: &str, favorited: bool)
    -> Result<Option<CapsuleRecord>, ...>
// favorited == true:  UPDATE capsules SET favorited_at = COALESCE(favorited_at, ?now) WHERE id = ?
// favorited == false: UPDATE capsules SET favorited_at = NULL WHERE id = ?
// returns the fresh record (None if id unknown). revision and updated_at untouched.
```

`COALESCE` keeps repeated PUTs idempotent (first-favorite time preserved).
`CapsuleUpdateDraft` / MCP `capsule_update` are NOT extended — favoriting is a
user action, not an agent action.

### D3. HTTP contract (mirrors thread-pin)

```
PUT    /api/capsules/{id}/favorite  -> 200 {"favorited": true,  "capsule": {...}}
DELETE /api/capsules/{id}/favorite  -> 200 {"favorited": false, "capsule": {...}}
                                       404 {"error": "capsule not found"}
```

Both idempotent. Registered in `route_graph.rs` next to the existing capsule
routes, behind gateway auth. `CapsuleRecord` serialization now includes
`favorited_at: Option<String>` (RFC3339 UTC, consistent with the other
timestamps; clients localize at render time — here it is only used as a flag).

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

## Client Changes

### Desktop (`desktop/garyx-desktop`)

- `shared/contracts/capsule.ts`: `DesktopCapsuleSummary` gains
  `favoritedAt: string | null`; new `SetCapsuleFavoriteInput { capsuleId, favorited }`.
- `main/garyx-client/capsules.ts`: map `favorited_at` in `mapCapsuleSummary`;
  new `setCapsuleFavorite` hitting PUT/DELETE.
- preload + IPC: expose `setCapsuleFavorite` alongside the existing five
  capsule bridges.
- `CapsulesPanel.tsx`:
  - Gallery header: segmented control `All | Favorites` following the
    `tasks-segmented` markup/CSS recipe (feature stylesheet `capsules.css`,
    not shell chrome).
  - `CapsuleGalleryCard`: star toggle button (top-right, hover-visible like
    existing card affordances); optimistic flip, reconcile from the returned
    `capsule` record, revert on error.
  - Favorites tab empty state: "No favorite Capsules yet."
  - Filtering: pure function over `page.capsules`, unit-testable.

### iOS (`mobile/garyx-mobile`)

- `GaryxMobileCore/GaryxGatewayCapsuleModels.swift`: `GaryxCapsuleSummary`
  gains `favoritedAt` (decode both `favorited_at` / `favoritedAt` like the
  existing keys). `isFavorited` computed helper.
- `GaryxMobileCore/GaryxGatewayClient.swift`: `setCapsuleFavorite(id:favorited:)`
  → PUT/DELETE.
- `GaryxMobileCore`: gallery tab filter as a pure Core function
  (e.g. `GaryxCapsuleGalleryTab.filter(_:)`), SwiftPM tests cover decode +
  filter + idempotent-toggle model update.
- `GaryxMobileModel+Capsules.swift`: `setCapsuleFavorite` async action —
  optimistic local update of `model.capsules`, reconcile with server response,
  revert on failure. Must NOT touch the HTML/thumbnail caches (favorite flips
  don't change `revision`).
- `GaryxCapsulesView`: segmented `Picker` (All / Favorites) in the glass top
  bar area, native `.pickerStyle(.segmented)`, monochrome (no green tint);
  card star badge when favorited; toggle action in the existing card
  context-menu/ellipsis affordance next to Delete (favorite is not
  destructive — keep it visually separate from Delete). Favorites empty state.
- If any new Core/App file is added: run `xcodegen generate` and commit the
  pbxproj.

### MCP (read-only surface)

`capsule_list` `summary_json` adds `"favorited": bool`. Nothing else changes.

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
- **Desktop** (`npm run test:unit`): `mapCapsuleSummary` maps `favorited_at`;
  gallery filter pure function (all/favorites/empty).
- **iOS** (`swift test` in GaryxMobileCore, then `xcodebuild` app compile):
  summary decode with/without `favorited_at`; tab filter; optimistic toggle
  reconcile/revert model logic.
- **End-to-end**: create two capsules via MCP, favorite one via PUT, verify
  `GET /api/capsules` flags it, flip tabs in both clients (desktop packaged
  check only if preload/IPC wiring changed — it did, so one `dist:dir` pass).
