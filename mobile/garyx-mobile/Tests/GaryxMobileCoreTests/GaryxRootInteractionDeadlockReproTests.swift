import XCTest
@testable import GaryxMobileCore

@MainActor
final class GaryxRootInteractionDeadlockReproTests: XCTestCase {
    func testSameGatewayReconnectStrandsDrawerDragAndBlocksRemountedNavigationSurface() {
        let extent: CGFloat = 330
        let root = GaryxHomeObservationStore(
            isGatewayConfigured: true,
            connectionState: .ready(version: "before-reconnect")
        )
        var drawer = GaryxHorizontalRevealState(position: .closed, extent: extent)

        XCTAssertEqual(root.rootSurface, .navigationShell)
        drawer.beginDrag(extent: extent)
        drawer.updateDrag(logicalTranslation: 120, extent: extent)
        XCTAssertEqual(drawer.phase, .dragging)
        XCTAssertFalse(drawer.phase.allowsSurfaceHitTesting)

        // `connectAndRefresh` publishes `.checking` for a same-gateway
        // reconnect without calling `resetGatewayRuntimeState`. The root then
        // dismantles the Shell (and the recognizer that owns this drag), while
        // the model-owned reveal state survives and receives no terminal event.
        XCTAssertTrue(root.applyConnection(
            isGatewayConfigured: true,
            connectionState: .checking
        ))
        XCTAssertEqual(root.rootSurface, .gatewaySetup)

        XCTAssertTrue(root.applyConnection(
            isGatewayConfigured: true,
            connectionState: .ready(version: "after-reconnect")
        ))
        XCTAssertEqual(root.rootSurface, .navigationShell)

        // Reappearing with the same drawer extent is a no-op while the reveal
        // is non-idle. There is no settle driver for `.dragging`, and the old
        // recognizer can no longer deliver ended/cancelled, so this state has
        // no owner left that can release the shared surface hit-testing gate.
        XCTAssertFalse(drawer.reconcileRestingPositionIfIdle(.closed, extent: extent))
        XCTAssertEqual(drawer.phase, .dragging)
        XCTAssertEqual(drawer.reveal, 120)
        XCTAssertFalse(
            drawer.phase.allowsSurfaceHitTesting,
            "REPRO: the remounted home rows and their child leading-edge recognizer stay inert"
        )
    }
}
