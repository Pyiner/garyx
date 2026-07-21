import XCTest
@testable import GaryxMobileCore

final class GaryxPresentationLeaseLeakReproTests: XCTestCase {
    func testAbandonedLeaseHasNoOwnerLivenessRecoveryBeforeExplicitTerminalCallback() {
        let abandoned = GaryxPresentationLeaseToken(rawValue: "abandoned-presenter")
        let unknown = GaryxPresentationLeaseToken(rawValue: "unknown-presenter")
        var leases = GaryxPresentationLeaseTree()

        XCTAssertTrue(leases.acquire(abandoned))
        leases.markPresented(abandoned)

        XCTAssertFalse(leases.acquire(abandoned), "duplicate acquisition is a no-op")

        // Exercise every non-terminal mutation available to a lease whose
        // presenter owner has vanished. Also churn unrelated leases through
        // their full lifecycle; neither new acquisitions nor their terminals
        // prove that the abandoned presenter dismissed.
        for index in 0..<8 {
            leases.markPresented(abandoned)
            leases.markDismissing(abandoned)
            leases.recordResult(abandoned)
            leases.recordNoResult(abandoned)
            leases.markPresented(unknown)
            leases.markDismissing(unknown)
            leases.recordResult(unknown)
            leases.recordNoResult(unknown)

            let unrelated = GaryxPresentationLeaseToken(rawValue: "unrelated-\(index)")
            XCTAssertTrue(leases.acquire(unrelated))
            leases.markPresented(unrelated)
            leases.dismissalCompleted(unrelated)
            XCTAssertEqual(leases.garbageCollectReleased(), 1)
            XCTAssertTrue(leases.hasBarrier)
        }

        XCTAssertEqual(leases.records.count, 1)
        XCTAssertEqual(leases.records[abandoned]?.joinState, .dismissing)
        XCTAssertEqual(leases.records[abandoned]?.releaseCount, 0)
        XCTAssertFalse(leases.records[abandoned]?.released == true)
        XCTAssertTrue(
            leases.hasBarrier,
            "REPRO: an ownerless non-terminal lease has no liveness-based recovery"
        )

        var dismissalRecovery = leases
        dismissalRecovery.dismissalCompleted(abandoned)
        XCTAssertFalse(dismissalRecovery.hasBarrier)

        var forcedRecovery = leases
        forcedRecovery.presentationFailed(abandoned)
        XCTAssertFalse(forcedRecovery.hasBarrier)

        print(
            "PRESENTATION_LEASE_CORE_REPRO state=\(String(describing: leases.records[abandoned]?.joinState)) "
                + "released=\(leases.records[abandoned]?.released == true) "
                + "barrier=\(leases.hasBarrier) nonTerminalRecovery=false"
        )
    }

    func testAbandonedLeasePermanentlyQueuesOrdinaryNavigationUntilExplicitDismissal() {
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

        for _ in 0..<32 {
            coordinator.setTransactionStatus(.nonTerminal)
            XCTAssertEqual(
                coordinator.nextAdmissionAction(presentationBarrier: leases.hasBarrier),
                .waitForTransactionTerminal
            )
            coordinator.setTransactionStatus(.terminal)
            XCTAssertEqual(
                coordinator.nextAdmissionAction(presentationBarrier: leases.hasBarrier),
                .waitForPresentationBarrier
            )
            XCTAssertTrue(
                coordinator.drainAdmissible(presentationBarrier: leases.hasBarrier).isEmpty
            )

            leases.markPresented(abandoned)
            leases.markDismissing(abandoned)
            XCTAssertEqual(leases.garbageCollectReleased(), 0)
            XCTAssertTrue(leases.hasBarrier)
        }

        XCTAssertEqual(coordinator.queued, [intent])
        XCTAssertEqual(
            coordinator.nextAdmissionAction(presentationBarrier: leases.hasBarrier),
            .waitForPresentationBarrier
        )

        leases.dismissalCompleted(abandoned)
        XCTAssertFalse(leases.hasBarrier)
        XCTAssertEqual(
            coordinator.drainAdmissible(presentationBarrier: leases.hasBarrier),
            [intent]
        )

        print(
            "PRESENTATION_LEASE_NAV_REPRO cycles=32 queuedBeforeDismissal=true "
                + "nextAction=waitForPresentationBarrier admittedBeforeDismissal=false"
        )
    }
}
