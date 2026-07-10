# iOS Agent Avatar Persistent Cache

Task: `#TASK-1345`

This document is the final implementation design synthesized from design A,
design B, and the task裁决. It is the durable source for the shipped iOS
last-known-good avatar cache.

## Problem

`GaryxAgentAvatarView` currently renders a real avatar only when the current
row has a usable `avatarDataUrl` and `GaryxDataURLImageCache` has a decoded
`UIImage` in its volatile `NSCache`. When that cache is empty after relaunch,
memory pressure, or eviction, the view immediately shows fallback content and
does not self-invalidate when the async predecode later succeeds.

The second failure path is structural: thread, task, and widget rows often only
know `agentId`. If the in-memory catalog does not currently contain
that id, the projector passes no `avatarDataUrl`, so the row falls directly to
the placeholder even if the app previously rendered a real avatar for that id.

The existing `GaryxMobileCatalogCache` persists catalog rows, including
`avatarDataUrl`, but it is membership-bound and encoded as one large JSON blob.
It is not an identity-keyed last-known-good image store, and it must not grow
into one.

## Final Architecture

The cache has three one-way layers:

1. `GaryxMobileCatalogCache` remains the gateway-scoped catalog snapshot. It
   owns current catalog membership and current row fields.
2. `GaryxAvatarStore` is the new membership-independent persistent fallback
   truth source. Its identity is gateway scope + agent id.
   It stores raw decoded image bytes and a small index, never base64.
3. `GaryxDataURLImageCache` stays a volatile decoded-image accelerator.

Catalog apply writes through to `GaryxAvatarStore`. Rendering reads live data,
then persistent data, then placeholder. The render hot path never writes the
persistent store and never performs disk I/O or base64/JSON work.

## Identity And Scope

Core defines:

- `GaryxAvatarIdentity`: `scope`, `id`
- `GaryxAvatarStoreEntry` and `GaryxAvatarStoreIndex`

The app supplies the same gateway scope token used by `scopedSettingsKey(_:)`
and `currentGatewayScopeId`, so a shared agent id on two gateways cannot
collide.

Filesystem keys must be stable and safe for ids containing `/`, `:`, spaces,
or other path-hostile characters. Hash the full storage key for file names.

## Core Policy

Core owns all cache strategy and is UIKit-free:

- Parse only `data:image/...;base64,...` candidates.
- Reject empty strings, remote URLs, non-image media types, malformed base64,
  and per-record payloads larger than 512 KB.
- Compute the content fingerprint from decoded bytes using FNV-1a, not from
  the raw data URL string.
- Use an injected `GaryxAvatarImageValidating` protocol to decide whether live
  bytes are a real decodable image. App code implements it with image APIs;
  tests inject deterministic validators.
- Resolve display priority as valid live -> stored -> placeholder.
- Plan write-through batches as non-empty, valid, changed fingerprints only.
- Never treat an ordinary empty or invalid refreshed avatar as a tombstone.
- Delete only on explicit local delete or explicit clear-avatar intent after
  the gateway confirms the mutation.

Core also provides `GaryxInMemoryAvatarStore` for SwiftPM tests. It implements
the same no-tombstone, fingerprint, LRU, scope isolation, explicit remove, and
index round-trip rules without disk or UIKit.

## App Store

The app target implements `GaryxAvatarDiskStore` as an actor. It stores data in
the App Group container `group.com.garyx.mobile`, under an avatar-cache
subdirectory that is excluded from iCloud backup.

The store keeps:

- One raw-byte blob per avatar identity.
- One small `index.json` with no base64.
- Defaults of 256 records total, 16 MB total bytes, 512 KB per record.
- LRU pruning by `lastAccessAt` only. It must never prune by current catalog
  membership because leaving the catalog is exactly when fallback is needed.

All parsing, validation, fingerprint comparison, JSON encoding, file I/O, and
pruning run in the actor/off-main. Index writes are batched once per catalog
apply, not once per entry.

## Write Path

There is one normal write path: catalog apply.

The existing `agents` update path in `GaryxMobileModel.swift` already calls
`predecodeAgentAvatarImages()`. The new write-through runs from
that same area: the main actor assembles lightweight candidates containing
scope, id, and the Swift `String` data-url reference, then hands them to
the store actor. The actor computes fingerprints, validates bytes, diffs the
index, writes changed blobs, and prunes.

Cold-start `restoreCachedCatalogState()` also feeds restored catalog rows once,
so users get a first-run backfill from the pre-existing catalog snapshot.

Explicit mutation rules:

- Successful create/update with a valid avatar stores the new identity.
- Successful update with an explicit empty avatar field removes the identity
  only because the local edit was an explicit clear intent.
- Successful delete removes that identity.
- Id changes do not silently migrate bytes. The new identity is stored from a
  valid response or explicitly cleared, then the old identity is removed.
- Ordinary refresh rows with empty or invalid `avatarDataUrl` do not remove
  known-good bytes.

## Render Path

`GaryxAgentAvatarView` becomes a thin SwiftUI renderer:

- It builds a `GaryxAvatarIdentity` from environment scope + id.
- It asks an environment-injected singleton `GaryxAvatarImageProvider` for a
  synchronous `NSCache` hit.
- On a miss it shows placeholder for that single view, then runs `.task(id:)`
  keyed by identity and live fingerprint.
- The task resolves valid live bytes first, stored bytes second, placeholder
  last. On success it assigns local `@State resolvedImage`.
- Filling one avatar must not publish list-wide observable state. The provider
  exposes no per-avatar `@Published` state.

When a fresh live avatar arrives, the task id changes and the view naturally
promotes restored -> live. When identity changes, the previous local state is
invalidated.

Remote `http(s)` avatar URLs keep the existing remote-image branch and are not
persisted by this task.

## Presentation Helper

The view must not switch on provider kinds for fallback colors or icon sizes.
`GaryxProviderPresentation` in Core exposes the data the view needs:

- fallback RGB background
- foreground style hint
- icon size factor
- symbol/initials already provided today

The SwiftUI view maps Core data to `Color` and font size, with no local provider
kind switch table.

## Widget

The widget receives the same last-known-good behavior without making Core call
the disk actor:

- `GaryxRecentThreadsWidgetSnapshotProjector.widgetThreads(from:)` accepts
  `avatarFallback: [GaryxAvatarIdentity: String] = [:]`.
- When a row has no current live avatar and the injected map has a fallback,
  the projector uses it. An empty map keeps current behavior and existing tests.
- The app widget persistence actor pre-resolves missing identities from the
  avatar store off-main and passes the map into the pure projector.
- New snapshots should avoid adding new large base64 fields. The existing
  `avatarDataUrl` field is kept for migration and current widget rendering, but
  no catalog snapshot schema changes are introduced.

## Rejected Alternatives

Do not extend `GaryxMobileCatalogCache` into a permanent avatar cache. Its
membership lifetime conflicts with orphan-id fallback, and it rewrites one
large JSON blob containing base64 data.

Do not persist `NSCache` or decoded `UIImage` values. Persistence is keyed by
identity and raw decoded bytes; decoded images are rebuildable acceleration.

Do not decode or hit disk synchronously in SwiftUI body. The only synchronous
render path is an in-memory cache lookup.

Do not add gateway/router/desktop behavior. This is an iOS local continuity
optimization and must not invent a new product concept.

## Test Plan

Primary validation is headless `GaryxMobileCore` SwiftPM tests:

- Data URL parsing accepts a synthetic 1x1 PNG and rejects empty, malformed,
  non-image, remote URL, and oversize inputs.
- FNV-1a fingerprints are stable across data-url whitespace/MIME casing and
  change when decoded bytes change.
- Resolution returns live when live validates, stored when live is empty or
  invalid and a stored record exists, and placeholder only when neither exists.
- Write-through writes only non-empty valid changed avatars.
- Ordinary empty/invalid refresh rows do not tombstone records.
- Explicit clear and successful delete remove records.
- Id changes store or clear the new identity before removing the old one.
- Gateway scopes do not collide.
- LRU pruning obeys 256 records / 16 MB / 512 KB record bounds and bumps
  `lastAccessAt` on reads. It never prunes by catalog membership.
- Index encode/decode round-trips and version mismatch is discarded.
- Widget projection uses injected fallback for id-only rows and preserves
  existing nil behavior with an empty map.
- Provider presentation exposes fallback styling data without view switch
  tables.

App validation must include `xcodegen generate` after adding app-target Swift
files, followed by an app-target `xcodebuild` with code signing disabled if
signing is the only blocker.
