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

        XCTAssertFalse(store.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: nil),
            willTransitionRootSurface: { transition in
                XCTFail("same root surface emitted \(transition)")
            }
        ))
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

    func test_rootSurfaceOccurrenceEndsBeforeTheVisibleBranchChanges() throws {
        let store = GaryxHomeObservationStore(
            isGatewayConfigured: true,
            connectionState: .checking
        )
        var transitions: [GaryxRootSurfaceOccurrenceTransition] = []

        XCTAssertTrue(store.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "first"),
            willTransitionRootSurface: { transition in
                XCTAssertEqual(store.rootSurface, .gatewaySetup)
                transitions.append(transition)
            }
        ))
        let firstOccurrence = try XCTUnwrap(store.rootSurface.navigationShellOccurrenceID)
        XCTAssertEqual(transitions, [.navigationShellBegan(firstOccurrence)])

        XCTAssertTrue(store.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "same-occurrence"),
            willTransitionRootSurface: { transition in
                XCTFail("ready-to-ready replaced the Shell: \(transition)")
            }
        ))
        XCTAssertEqual(store.rootSurface, .navigationShell(firstOccurrence))

        XCTAssertTrue(store.applyConnection(
            isGatewayConfigured: true,
            connectionState: .checking,
            willTransitionRootSurface: { transition in
                XCTAssertEqual(store.rootSurface, .navigationShell(firstOccurrence))
                transitions.append(transition)
            }
        ))
        XCTAssertEqual(store.rootSurface, .gatewaySetup)
        XCTAssertEqual(transitions.last, .navigationShellEnded(firstOccurrence))

        XCTAssertTrue(store.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "second"),
            willTransitionRootSurface: { transitions.append($0) }
        ))
        let secondOccurrence = try XCTUnwrap(store.rootSurface.navigationShellOccurrenceID)
        XCTAssertNotEqual(secondOccurrence, firstOccurrence)
        XCTAssertEqual(transitions.last, .navigationShellBegan(secondOccurrence))
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

private extension GaryxRootSurface {
    var navigationShellOccurrenceID: GaryxRootSurfaceOccurrenceID? {
        guard case .navigationShell(let occurrenceID) = self else { return nil }
        return occurrenceID
    }
}
