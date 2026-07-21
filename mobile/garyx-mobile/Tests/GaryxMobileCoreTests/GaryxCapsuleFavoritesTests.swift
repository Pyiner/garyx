import XCTest
@testable import GaryxMobileCore

final class GaryxCapsuleFavoritesTests: XCTestCase {
    func testGalleryTabFilterKeepsServerOrderAndHandlesEmpty() {
        let capsules = [
            capsule("a"),
            capsule("b", favoritedAt: "2026-07-14T01:00:00Z"),
            capsule("c"),
        ]
        let state = GaryxCapsuleFavoriteReducerState()
        XCTAssertEqual(GaryxCapsuleGalleryTab.all.filter(capsules, favoriteState: state).map(\.id), ["a", "b", "c"])
        XCTAssertEqual(GaryxCapsuleGalleryTab.favorites.filter(capsules, favoriteState: state).map(\.id), ["b"])
        XCTAssertEqual(GaryxCapsuleGalleryTab.favorites.filter([], favoriteState: state), [])
    }

    func testDoubleTapSerializesPutThenDeleteAndSettlesOnFinalIntent() throws {
        var capsules = [capsule("a")]
        var state = GaryxCapsuleFavoriteReducerState()

        var transition = GaryxCapsuleFavoriteReducer.toggle(
            capsules: capsules,
            state: state,
            capsuleId: "a",
            favorited: true
        )
        capsules = transition.capsules
        state = transition.state
        XCTAssertEqual(transition.effect, GaryxCapsuleFavoriteEffect(capsuleId: "a", favorited: true))
        XCTAssertEqual(state.favoritesGeneration, 1)
        XCTAssertTrue(GaryxCapsuleFavoriteReducer.isFavorited(capsules[0], state: state))

        transition = GaryxCapsuleFavoriteReducer.toggle(
            capsules: capsules,
            state: state,
            capsuleId: "a",
            favorited: false
        )
        capsules = transition.capsules
        state = transition.state
        XCTAssertNil(transition.effect)
        XCTAssertEqual(state.favoritesGeneration, 1)
        XCTAssertFalse(GaryxCapsuleFavoriteReducer.isFavorited(capsules[0], state: state))

        transition = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: capsules,
            state: state,
            capsuleId: "a",
            response: response(capsule("a", favoritedAt: "2026-07-14T02:00:00Z"))
        )
        capsules = transition.capsules
        state = transition.state
        XCTAssertEqual(transition.effect, GaryxCapsuleFavoriteEffect(capsuleId: "a", favorited: false))
        XCTAssertEqual(state.favoritesGeneration, 3)
        XCTAssertFalse(GaryxCapsuleFavoriteReducer.isFavorited(capsules[0], state: state))

        transition = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: capsules,
            state: state,
            capsuleId: "a",
            response: response(capsule("a"))
        )
        XCTAssertNil(transition.effect)
        XCTAssertEqual(transition.state.favoritesGeneration, 4)
        XCTAssertFalse(try XCTUnwrap(transition.state.mutations["a"]).inFlight)
        XCTAssertNil(transition.capsules[0].favoritedAt)
    }

    func testFailureRevertsDesiredStateToServerState() {
        let initial = GaryxCapsuleFavoriteReducer.toggle(
            capsules: [capsule("a")],
            state: GaryxCapsuleFavoriteReducerState(),
            capsuleId: "a",
            favorited: true
        )
        XCTAssertTrue(GaryxCapsuleFavoriteReducer.isFavorited(initial.capsules[0], state: initial.state))

        let failed = GaryxCapsuleFavoriteReducer.failed(
            capsules: initial.capsules,
            state: initial.state,
            capsuleId: "a"
        )
        XCTAssertEqual(failed.state.favoritesGeneration, 2)
        XCTAssertFalse(GaryxCapsuleFavoriteReducer.isFavorited(failed.capsules[0], state: failed.state))
    }

    func testRefreshMergeKeepsPendingDesiredAndAdoptsSettledServerState() {
        let toggled = GaryxCapsuleFavoriteReducer.toggle(
            capsules: [capsule("a")],
            state: GaryxCapsuleFavoriteReducerState(),
            capsuleId: "a",
            favorited: true
        )
        let pending = GaryxCapsuleFavoriteReducer.mergingRefresh(
            currentCapsules: toggled.capsules,
            refreshedCapsules: [capsule("a", title: "Refreshed")],
            state: toggled.state,
            capturedGeneration: toggled.state.favoritesGeneration
        )
        XCTAssertEqual(pending.capsules[0].title, "Refreshed")
        XCTAssertNil(pending.capsules[0].favoritedAt)
        XCTAssertTrue(GaryxCapsuleFavoriteReducer.isFavorited(pending.capsules[0], state: pending.state))

        let failed = GaryxCapsuleFavoriteReducer.failed(
            capsules: pending.capsules,
            state: pending.state,
            capsuleId: "a"
        )
        let settled = GaryxCapsuleFavoriteReducer.mergingRefresh(
            currentCapsules: failed.capsules,
            refreshedCapsules: [capsule("a", favoritedAt: "2026-07-14T03:00:00Z")],
            state: failed.state,
            capturedGeneration: failed.state.favoritesGeneration
        )
        XCTAssertEqual(settled.capsules[0].favoritedAt, "2026-07-14T03:00:00Z")
        XCTAssertTrue(GaryxCapsuleFavoriteReducer.isFavorited(settled.capsules[0], state: settled.state))
    }

    func testRefreshSentBeforeMutationCannotClobberSettledFavorite() {
        var capsules = [capsule("a")]
        var state = GaryxCapsuleFavoriteReducerState()
        let captured = state.favoritesGeneration
        let toggled = GaryxCapsuleFavoriteReducer.toggle(
            capsules: capsules,
            state: state,
            capsuleId: "a",
            favorited: true
        )
        capsules = toggled.capsules
        state = toggled.state
        let settled = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: capsules,
            state: state,
            capsuleId: "a",
            response: response(capsule("a", favoritedAt: "2026-07-14T04:00:00Z"))
        )
        let merged = GaryxCapsuleFavoriteReducer.mergingRefresh(
            currentCapsules: settled.capsules,
            refreshedCapsules: [capsule("a")],
            state: settled.state,
            capturedGeneration: captured
        )
        XCTAssertEqual(merged.capsules[0].favoritedAt, "2026-07-14T04:00:00Z")
    }

    func testRefreshSentDuringPendingCannotClobberSettledFavorite() {
        let toggled = GaryxCapsuleFavoriteReducer.toggle(
            capsules: [capsule("a")],
            state: GaryxCapsuleFavoriteReducerState(),
            capsuleId: "a",
            favorited: true
        )
        let captured = toggled.state.favoritesGeneration
        let settled = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: toggled.capsules,
            state: toggled.state,
            capsuleId: "a",
            response: response(capsule("a", favoritedAt: "2026-07-14T05:00:00Z"))
        )
        let merged = GaryxCapsuleFavoriteReducer.mergingRefresh(
            currentCapsules: settled.capsules,
            refreshedCapsules: [capsule("a")],
            state: settled.state,
            capturedGeneration: captured
        )
        XCTAssertEqual(merged.capsules[0].favoritedAt, "2026-07-14T05:00:00Z")
    }

    func testRefreshSentAfterSettleAdoptsExternalFavoriteState() {
        let toggled = GaryxCapsuleFavoriteReducer.toggle(
            capsules: [capsule("a")],
            state: GaryxCapsuleFavoriteReducerState(),
            capsuleId: "a",
            favorited: true
        )
        let settled = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: toggled.capsules,
            state: toggled.state,
            capsuleId: "a",
            response: response(capsule("a", favoritedAt: "2026-07-14T06:00:00Z"))
        )
        let merged = GaryxCapsuleFavoriteReducer.mergingRefresh(
            currentCapsules: settled.capsules,
            refreshedCapsules: [capsule("a")],
            state: settled.state,
            capturedGeneration: settled.state.favoritesGeneration
        )
        XCTAssertNil(merged.capsules[0].favoritedAt)
        XCTAssertFalse(GaryxCapsuleFavoriteReducer.isFavorited(merged.capsules[0], state: merged.state))
    }

    func testLateRefreshMergedOutputPersistsSettledFavorite() throws {
        let toggled = GaryxCapsuleFavoriteReducer.toggle(
            capsules: [capsule("a")],
            state: GaryxCapsuleFavoriteReducerState(),
            capsuleId: "a",
            favorited: true
        )
        let settled = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: toggled.capsules,
            state: toggled.state,
            capsuleId: "a",
            response: response(capsule("a", favoritedAt: "2026-07-14T07:00:00Z"))
        )
        let merged = GaryxCapsuleFavoriteReducer.mergingRefresh(
            currentCapsules: settled.capsules,
            refreshedCapsules: [capsule("a")],
            state: settled.state,
            capturedGeneration: 0
        )
        let snapshot = snapshot(capsules: merged.capsules)
        let data = try JSONEncoder().encode(snapshot)
        let restored = try JSONDecoder().decode(GaryxMobileCatalogCacheSnapshot.self, from: data)
        XCTAssertEqual(restored.capsules.first?.model.favoritedAt, "2026-07-14T07:00:00Z")
    }

    func testFavoriteSettleKeepsHTMLAndThumbnailCacheKeys() {
        let original = capsule("a", revision: 3)
        let toggled = GaryxCapsuleFavoriteReducer.toggle(
            capsules: [original],
            state: GaryxCapsuleFavoriteReducerState(),
            capsuleId: "a",
            favorited: true
        )
        let settled = GaryxCapsuleFavoriteReducer.succeeded(
            capsules: toggled.capsules,
            state: toggled.state,
            capsuleId: "a",
            response: response(capsule("a", revision: 3, favoritedAt: "2026-07-14T08:00:00Z"))
        )

        let htmlKey = GaryxCapsuleHTMLCacheKey(id: "a", revision: 3)
        let html = GaryxCapsuleHTMLCachePruner.pruned(
            cache: [htmlKey: "<html/>"],
            validCapsules: settled.capsules
        )
        XCTAssertFalse(html.didEvict)
        XCTAssertEqual(html.cache[htmlKey], "<html/>")

        let thumbnailKeys = [
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 3, rendition: .gallery),
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 3, rendition: .chatCard),
        ]
        let thumbnails = GaryxCapsuleThumbnailCachePruner.pruned(
            keys: thumbnailKeys,
            validCapsules: settled.capsules
        )
        XCTAssertEqual(thumbnails.keep, thumbnailKeys)
        XCTAssertTrue(thumbnails.evict.isEmpty)
    }

    private func capsule(
        _ id: String,
        title: String? = nil,
        revision: Int = 1,
        favoritedAt: String? = nil
    ) -> GaryxCapsuleSummary {
        GaryxCapsuleSummary(
            id: id,
            title: title ?? "Capsule \(id)",
            htmlSha256: String(repeating: "a", count: 64),
            byteSize: 42,
            revision: revision,
            createdAt: "2026-07-14T00:00:00Z",
            updatedAt: "2026-07-14T00:00:00Z",
            favoritedAt: favoritedAt
        )
    }

    private func response(_ capsule: GaryxCapsuleSummary) -> GaryxCapsuleFavoriteResponse {
        GaryxCapsuleFavoriteResponse(favorited: capsule.isFavorited, capsule: capsule)
    }

    private func snapshot(capsules: [GaryxCapsuleSummary]) -> GaryxMobileCatalogCacheSnapshot {
        GaryxMobileCatalogCacheSnapshot(
            agents: [],
            workspaceCatalog: .empty,
            skills: [],
            capsules: capsules,
            automations: [],
            slashCommands: [],
            mcpServers: [],
            channelEndpoints: [],
            configuredBots: [],
            configuredBotAccounts: [],
            botConsoles: [],
            channelPlugins: [],
            savedAt: Date(timeIntervalSince1970: 1)
        )
    }
}
