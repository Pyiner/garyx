import XCTest
@testable import GaryxMobileCore

final class GaryxPresentationLeaseLeakReproTests: XCTestCase {
    func testOwnerLossTerminatesAbandonedLeaseAndAuditsCause() {
        let abandoned = GaryxPresentationLeaseToken(rawValue: "abandoned-presenter")
        let unknown = GaryxPresentationLeaseToken(rawValue: "unknown-presenter")
        var leases = GaryxPresentationLeaseTree()

        XCTAssertTrue(leases.acquire(abandoned, resultBearing: true))
        leases.markPresented(abandoned)
        leases.ownerPresentationEnded(unknown)

        XCTAssertTrue(leases.hasBarrier)
        leases.ownerPresentationEnded(abandoned)

        XCTAssertEqual(leases.records.count, 1)
        XCTAssertEqual(leases.records[abandoned]?.joinState, .released)
        XCTAssertEqual(leases.records[abandoned]?.releaseCount, 1)
        XCTAssertEqual(leases.records[abandoned]?.result, .explicitNoResult)
        XCTAssertEqual(leases.records[abandoned]?.terminalCause, .ownerLoss)
        XCTAssertFalse(leases.hasBarrier)

        leases.dismissalCompleted(abandoned)
        leases.presentationFailed(abandoned)
        leases.ownerPresentationEnded(abandoned)
        XCTAssertEqual(leases.records[abandoned]?.releaseCount, 1)
        XCTAssertEqual(leases.records[abandoned]?.terminalCause, .ownerLoss)

        print(
            "PRESENTATION_LEASE_CORE_HEALTH state=\(String(describing: leases.records[abandoned]?.joinState)) "
                + "released=\(leases.records[abandoned]?.released == true) "
                + "barrier=\(leases.hasBarrier) terminalCause=ownerLoss"
        )
    }

    func testOwnerLossAdmitsNavigationQueuedBehindAbandonedLease() {
        let scope = GaryxGatewayScope(identity: "synthetic-gateway", epoch: 1)
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        let abandoned = GaryxPresentationLeaseToken(rawValue: "abandoned-navigation-barrier")
        var leases = GaryxPresentationLeaseTree()
        XCTAssertTrue(leases.acquire(abandoned))
        leases.markPresented(abandoned)

        let intent = GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: "open-thread-after-abandonment"),
            effect: .ordinaryNavigation(.conversation(threadID: "synthetic-thread")),
            dependency: .absolute
        )
        var coordinator = GaryxNavigationIntentCoordinator()
        let ticket = coordinator.beginPreparation(
            intentID: intent.id,
            key: intent.coalescingKey,
            scope: scope
        )

        XCTAssertEqual(
            coordinator.completePreparation(
                ticket,
                outcome: .ready(intent),
                scopes: scopes,
                routeState: .init(),
                presentationBarrier: leases.hasBarrier
            ),
            .queued
        )
        XCTAssertEqual(
            coordinator.nextAdmissionAction(presentationBarrier: leases.hasBarrier),
            .waitForPresentationBarrier
        )
        XCTAssertTrue(coordinator.drainAdmissible(presentationBarrier: true).isEmpty)

        leases.ownerPresentationEnded(abandoned)

        XCTAssertFalse(leases.hasBarrier)
        XCTAssertEqual(leases.records[abandoned]?.terminalCause, .ownerLoss)
        XCTAssertEqual(
            coordinator.drainAdmissible(presentationBarrier: leases.hasBarrier),
            [intent]
        )
        XCTAssertTrue(coordinator.queued.isEmpty)

        print(
            "PRESENTATION_LEASE_NAV_HEALTH queuedBeforeOwnerLoss=true "
                + "barrierAfterOwnerLoss=false admittedAfterOwnerLoss=true"
        )
    }
}
