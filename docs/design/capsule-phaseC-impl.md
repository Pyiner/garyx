# Capsule Phase C — iOS Implementation Design

Scope: iOS only (`mobile/garyx-mobile/`). Gateway Phase A is already on main and exposes protected `/api/capsules*` endpoints. This phase follows `docs/design/capsule.md` §4, §5, §6, and §8.

## Goals

- Add Capsules as a top-level iOS drawer panel immediately after Automation.
- Keep route state, gateway models, gateway client calls, cache DTOs, and HTML-load state machine in `GaryxMobileCore` with SwiftPM tests.
- Keep app-target code to SwiftUI composition, model side effects, and `WKWebView` platform adapter.
- Fetch Capsule HTML through the authenticated `GaryxGatewayClient`, then render the returned string in a hardened `WKWebView` using `loadHTMLString(html, baseURL: nil)`.
- Persist only metadata in the gateway-scoped catalog cache; keep HTML bodies memory-only and invalidated by `id + revision + html_sha256`.

## Core API and type changes

### Navigation and routes

`Sources/GaryxMobileCore/GaryxMobileNavigationState.swift`

- Add `GaryxMobilePanel.capsules`.
- Label: `"Capsules"`.
- Icon: `"capsule.fill"`. This is the canonical SF Symbol requested by the design. If an older runtime ever lacks the glyph, SwiftUI simply renders a missing symbol; no per-view switch table will be added. If validation proves the target SDK rejects it, use `"shippingbox.fill"` as the fallback in this single Core property.
- No special-case redirects in `resolvedPanel`; Capsules is a normal first-level panel.

`Sources/GaryxMobileCore/GaryxMobileRouteLink.swift`

- Add `GaryxMobileRoute.capsule(String)`.
- `make(.panel(.capsules))` returns `garyx://mobile/capsules` through `pathComponent(for:)`.
- `make(.capsule(id))` returns `garyx://mobile/capsule?id=<trimmed id>`.
- `parse` accepts:
  - `/capsules` -> `.panel(.capsules)`
  - `/capsule?id=...` plus aliases `capsuleId` and `capsule_id` -> `.capsule(id)`

### Gateway models

New `Sources/GaryxMobileCore/GaryxGatewayCapsuleModels.swift`

```swift
public struct GaryxCapsulesPage: Decodable, Equatable, Sendable {
    public var capsules: [GaryxCapsuleSummary]
}

public struct GaryxCapsuleSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var title: String
    public var description: String
    public var threadId: String?
    public var runId: String?
    public var agentId: String?
    public var providerType: String?
    public var htmlSha256: String
    public var byteSize: Int
    public var revision: Int
    public var createdAt: String?
    public var updatedAt: String?
}
```

Custom `init(from:)` decodes both snake_case and camelCase for the gateway fields, matching existing task/skill model tolerance. Empty strings stay empty; optional ownership fields stay `nil` unless present.

### Gateway client

`Sources/GaryxMobileCore/GaryxGatewayClient.swift`

Add public methods near other catalog APIs:

```swift
public func listCapsules() async throws -> [GaryxCapsuleSummary]
public func deleteCapsule(id: String) async throws -> GaryxDeleteResult
public func capsuleHTML(id: String) async throws -> String
```

- `listCapsules()` decodes `GaryxCapsulesPage` from `GET /api/capsules` and returns `.capsules`.
- `deleteCapsule(id:)` calls `DELETE /api/capsules/{id}` with path-segment encoding.
- `capsuleHTML(id:)` calls `GET /api/capsules/{id}/serve` with the same auth/custom headers as JSON calls, `Accept: text/html`, retries as an idempotent request, and returns UTF-8 text. Implementation adds a private `getText(_:)` and `sendText(_:idempotent:)` path that mirrors JSON error handling and retry behavior but does not decode JSON on success.

### Catalog cache metadata

`Sources/GaryxMobileCore/GaryxMobileCatalogCache.swift`

- Bump `GaryxMobileCatalogCacheSnapshot.currentVersion` from 2 to 3, because the Codable snapshot gains a non-optional `capsules` array.
- Add `var capsules: [GaryxCachedCapsule]` to the snapshot initializer and Codable payload.
- Add `GaryxCachedCapsule` with exactly the metadata fields from `GaryxCapsuleSummary`. It intentionally does not store HTML.
- Update snapshot construction/restoration in the app extension to include `capsules`.

### HTML memory cache and panel state machine

New Core type, preferably in `GaryxGatewayCapsuleModels.swift` or a small sibling if it grows:

```swift
public struct GaryxCapsuleHTMLCacheKey: Hashable, Equatable, Sendable {
    public var id: String
    public var revision: Int
    public var htmlSha256: String
}

public struct GaryxCapsuleHTMLLoadState: Equatable, Sendable {
    public private(set) var selectedCapsuleId: String?
    public private(set) var requestedKey: GaryxCapsuleHTMLCacheKey?
    public private(set) var loadedKey: GaryxCapsuleHTMLCacheKey?
    public private(set) var html: String?
    public private(set) var isLoading: Bool
    public private(set) var errorMessage: String?

    public mutating func select(_ capsule: GaryxCapsuleSummary?)
    public mutating func beginHTMLLoad(for capsule: GaryxCapsuleSummary) -> GaryxCapsuleHTMLCacheKey
    public mutating func applyHTML(_ html: String, for key: GaryxCapsuleHTMLCacheKey) -> Bool
    public mutating func applyHTMLFailure(_ message: String, for key: GaryxCapsuleHTMLCacheKey) -> Bool
    public mutating func applyCachedHTML(_ html: String, for key: GaryxCapsuleHTMLCacheKey) -> Bool
    public mutating func remove(id: String)
}
```

State rules:

- Selecting a different capsule clears stale loaded HTML/error and records the new id.
- `beginHTMLLoad` records `requestedKey`, sets `isLoading = true`, clears the current error, and preserves loaded HTML only when the loaded key still equals the request key.
- `applyHTML` and `applyHTMLFailure` ignore stale completions whose key is no longer the current requested key or selected capsule id.
- `applyCachedHTML` immediately installs a cache hit without entering loading.
- Cache key is `id + revision + html_sha256`, so either metadata update invalidates the body.

The actual memory dictionary lives in `GaryxMobileModel` because it is runtime-only app state:

```swift
var capsuleHTMLCache: [GaryxCapsuleHTMLCacheKey: String] = [:]
@Published var capsuleHTMLState = GaryxCapsuleHTMLLoadState()
```

## App model changes

`App/GaryxMobile/GaryxMobileModel.swift`

- Add `@Published var capsules: [GaryxCapsuleSummary] = []`.
- Add `@Published var capsuleHTMLState = GaryxCapsuleHTMLLoadState()`.
- Add non-published `var capsuleHTMLCache: [GaryxCapsuleHTMLCacheKey: String] = [:]`.
- Reset them on gateway runtime switch.
- Debug snapshot can seed synthetic Capsules only if needed for screenshots; not required for this implementation.

`App/GaryxMobile/GaryxMobileModel+CatalogCache.swift`

- Restore `capsules = snapshot.capsules.map(\.model)`.
- Persist `capsules: capsules`.

`App/GaryxMobile/GaryxMobileModel+Gateway.swift`

- Add `async let capsulesResult = garyxCaptureCatalog { try await gateway.listCapsules() }` to `refreshRemoteState()`.
- Include it in `cacheableResults` so a complete stale-while-refresh snapshot includes Capsules with the same gateway-scoped semantics as tasks/skills.
- On success, assign `capsules`.

New `App/GaryxMobile/GaryxMobileModel+Capsules.swift`

```swift
func refreshCapsules() async
func openCapsule(_ capsule: GaryxCapsuleSummary) async
func loadSelectedCapsuleHTML(forceRefresh: Bool = false) async
func deleteCapsule(_ capsule: GaryxCapsuleSummary) async
```

- `refreshCapsules()` calls `client().listCapsules()`, updates `capsules`, persists the snapshot, and preserves/updates selected detail if possible.
- `openCapsule` selects the capsule via `capsuleHTMLState.select(capsule)`, then calls `loadSelectedCapsuleHTML()`.
- `loadSelectedCapsuleHTML` checks `capsuleHTMLCache[key]` unless forced. On miss, it calls `capsuleHTML(id:)`, then stores `cache[key] = html` and applies state only if the request is still current and gateway runtime generation still matches.
- `deleteCapsule` calls `deleteCapsule(id:)`, removes the row locally, removes any memory cache entries for that id, clears selected state if it was open, persists metadata snapshot, and refreshes on failure only through user-visible error.
- All async methods capture `gatewayRuntimeGeneration` and ignore completions after a gateway/profile switch.

`App/GaryxMobile/GaryxMobileModel+Navigation.swift`

- Route `.capsule(id)` opens the Capsules panel, refreshes Capsules (or uses restored metadata first), finds the id, and calls `openCapsule`. If missing after refresh, use the existing route-not-found sheet.
- `clearRouteDrivenDetailState()` clears selected Capsule HTML state when route-driven detail state is reset.

## SwiftUI structure

### Sidebar entry

`App/GaryxMobile/GaryxMobileSidebarViews.swift`

- In `GaryxRootRouteContentView.panelContent(for:)`, add `.capsules: GaryxCapsulesView()`.
- In `GaryxNavigationDrawerView`, insert `GaryxSidebarNavigationRow(panel: .capsules, ...)` immediately after Automation and before Agents.

### New feature surface

New `App/GaryxMobile/GaryxMobileCapsuleViews.swift`

`GaryxCapsulesView`

- Use a native grouped `List` with `.insetGrouped`, `.scrollContentBackground(.hidden)`, and `.garyxPageBackground()`.
- Top chrome can use `GaryxPanelScaffold` only if it does not wrap the list in an extra ScrollView; to keep a real native `List`, implement page-local top chrome with `garyxAdaptiveTopBar` matching the panel scaffold header.
- Pull-to-refresh calls `model.refreshCapsules()`.
- Empty/loading/error states mirror existing panel state views:
  - initial cache empty and `isRemoteStatePending` => loading.
  - empty => “No capsules yet.”
- Row tap calls `model.openCapsule(capsule)`.
- Row trailing ellipsis uses `GaryxRowActionMenu` with destructive Delete, no left-swipe actions.
- Metadata line: relative updated timestamp, byte size, revision, and owner badge. The owner badge is derived through `GaryxProviderPresentation.make(agentId:providerType:fallbackName:)` or local agent lookup; no local provider switch table.

`GaryxCapsuleDetailView`

- Shows title, description, metadata chips, loading/error states, and the `GaryxCapsuleWebView` once HTML is loaded.
- Header actions: Reload HTML and Delete via toolbar/menu. Reload uses `forceRefresh: true`.
- Detail presentation: full-screen cover from list row selection, using `GaryxFormSheet` or a custom top chrome. The detail hosts the runner full-height, so a custom page with `garyxPageBackground()` is preferable to a form card.

`GaryxCapsuleWebView`

- `UIViewRepresentable` backed by `WKWebView`.
- Configuration:
  - `WKWebViewConfiguration.websiteDataStore = .nonPersistent()`
  - `defaultWebpagePreferences.allowsContentJavaScript = true`
  - `preferences.javaScriptCanOpenWindowsAutomatically = false`
  - no `addUserScript`
  - no `WKScriptMessageHandler`
  - `loadHTMLString(html, baseURL: nil)`
- Navigation delegate:
  - Allow initial document load.
  - For link-activated or top-frame external navigations with `http`, `https`, or `mailto`, cancel and call `UIApplication.shared.open(url)` (Safari/system handler).
  - Cancel `file:` and other unknown top-frame navigations.
  - Allow subframe/resource navigation as decided by WebKit/CSP.
- The loaded key uses the Core cache key plus HTML count/hash to avoid unnecessary reloads.

## File diff plan

Core:

- Modify `Sources/GaryxMobileCore/GaryxMobileNavigationState.swift`
- Modify `Sources/GaryxMobileCore/GaryxMobileRouteLink.swift`
- Add `Sources/GaryxMobileCore/GaryxGatewayCapsuleModels.swift`
- Modify `Sources/GaryxMobileCore/GaryxGatewayClient.swift`
- Modify `Sources/GaryxMobileCore/GaryxMobileCatalogCache.swift`

App target:

- Modify `App/GaryxMobile/GaryxMobileModel.swift`
- Modify `App/GaryxMobile/GaryxMobileModel+CatalogCache.swift`
- Modify `App/GaryxMobile/GaryxMobileModel+Gateway.swift`
- Modify `App/GaryxMobile/GaryxMobileModel+Navigation.swift`
- Add `App/GaryxMobile/GaryxMobileModel+Capsules.swift`
- Modify `App/GaryxMobile/GaryxMobileSidebarViews.swift`
- Add `App/GaryxMobile/GaryxMobileCapsuleViews.swift`
- Regenerate and commit `GaryxMobile.xcodeproj/project.pbxproj` with `xcodegen generate`

Tests:

- Modify `Tests/GaryxMobileCoreTests/GaryxMobileRouteLinkTests.swift` for panel/detail route round-trips and query alias parse.
- Modify `Tests/GaryxMobileCoreTests/GaryxMobileCatalogCacheTests.swift` for metadata cache roundtrip without HTML body.
- Add `Tests/GaryxMobileCoreTests/GaryxGatewayCapsuleModelsTests.swift` for snake_case/camelCase decode.
- Add `Tests/GaryxMobileCoreTests/GaryxCapsuleHTMLLoadStateTests.swift` for keying, cache hit, stale completion ignore, failure, selection reset, and deletion reset.
- Add/extend `Tests/GaryxMobileCoreTests/GaryxGatewayClientTests.swift` for `capsuleHTML(id:)` path, auth header, `Accept: text/html`, and UTF-8 response.

## Validation plan

Design-gated before implementation:

1. Open Claude design review against this doc and wait for explicit PASS.

Implementation validation:

1. `cd mobile/garyx-mobile && swift test`
2. `cd mobile/garyx-mobile && xcodegen generate`
3. Verify `GaryxMobile.xcodeproj/project.pbxproj` includes the new Core/app files.
4. `cd mobile/garyx-mobile && xcodebuild -project GaryxMobile.xcodeproj -target GaryxMobile -sdk iphonesimulator -configuration Debug CODE_SIGNING_ALLOWED=NO build`
5. Run the simulator app enough to verify the Capsules drawer entry, list, and opening a Capsule page. If the active gateway has no capsules, create or use a synthetic local Phase A capsule through the gateway/MCP only if safe; otherwise verify the UI loads/empty state and document the limitation. Do not touch gateway code.
6. Open Claude code review with current commit/diff and require true `swift test` + app-target `xcodebuild`; loop until PASS.
7. Commit with repo git identity, merge into main, and stop without changing parent task status.

## Non-goals and guardrails

- No gateway, desktop, MCP, DB, or API changes in this phase.
- No persistent HTML cache and no cookies/local storage for Capsule HTML.
- No JS-native bridge, no user scripts, no `WKScriptMessageHandler`, no non-nil base URL.
- No local provider/channel switch tables in views.
- No real personal data in fixtures, docs, or commits; tests use synthetic ids and `/Users/test` paths.
