import Observation
import XCTest
@testable import GaryxMobileCore

@MainActor
final class GaryxHomeObservationStoreTests: XCTestCase {
    func test_reapplyingSameValuesDoesNotInvalidateTrackedReads() {
        let store = GaryxHomeObservationStore(
            isGatewayConfigured: true,
            connectionState: .ready(version: nil),
            debugShowsGatewaySwitcher: false,
            showsSettings: false,
            lastError: nil,
            isLoadingMoreThreads: false,
            hasMoreThreadSummaries: true,
            loadMoreFooterState: .idle
        )
        var invalidations = 0

        trackStaticHomeReads(store) {
            invalidations += 1
        }

        XCTAssertFalse(store.applyConnection(isGatewayConfigured: true, connectionState: .ready(version: nil)))
        XCTAssertFalse(store.applyPagination(isLoadingMoreThreads: false, hasMoreThreadSummaries: true, loadMoreFooterState: .idle))
        XCTAssertFalse(store.setDebugShowsGatewaySwitcher(false))
        XCTAssertFalse(store.setShowsSettings(false))
        XCTAssertFalse(store.setLastError(Optional<String>.none))

        XCTAssertEqual(store.publishCount, 0)
        XCTAssertEqual(invalidations, 0)
    }

    func test_changingHomeValueInvalidatesTrackedReads() {
        let store = GaryxHomeObservationStore()
        var invalidations = 0

        trackStaticHomeReads(store) {
            invalidations += 1
        }

        XCTAssertTrue(store.applyPagination(isLoadingMoreThreads: true, hasMoreThreadSummaries: false, loadMoreFooterState: .hidden))

        XCTAssertEqual(store.publishCount, 1)
        XCTAssertEqual(invalidations, 1)
    }

    private func trackStaticHomeReads(
        _ store: GaryxHomeObservationStore,
        onChange: @escaping () -> Void
    ) {
        withObservationTracking {
            _ = store.isGatewayConfigured
            _ = store.connectionState
            _ = store.debugShowsGatewaySwitcher
            _ = store.showsSettings
            _ = store.lastError
            _ = store.isLoadingMoreThreads
            _ = store.hasMoreThreadSummaries
            _ = store.loadMoreFooterState
        } onChange: {
            onChange()
        }
    }
}
