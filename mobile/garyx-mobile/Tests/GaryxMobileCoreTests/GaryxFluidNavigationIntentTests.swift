import XCTest
@testable import GaryxMobileCore

final class GaryxFluidNavigationIntentTests: XCTestCase {
    private let scope1 = GaryxGatewayScope(identity: "gateway-1", epoch: 1)
    private let scope2 = GaryxGatewayScope(identity: "gateway-2", epoch: 1)

    func testPrepareOutcomeHasSixTypedProductSemantics() {
        let outcomes: [GaryxPrepareOutcome<GaryxPreparedNavigationIntent>] = [
            .ready(ordinary("ready", thread: "thread-1")),
            .userVisibleNotFound,
            .retryableFailure(message: "offline"),
            .authenticationRequired,
            .cancelledOrStale,
            .internalFault(code: "resolver_contract"),
        ]
        XCTAssertEqual(outcomes.count, 6)

        let expected: [GaryxNavigationQueueResult] = [
            .admittedImmediately,
            .userVisibleNotFound,
            .retryableFailure(message: "offline"),
            .authenticationRequired,
            .cancelledOrStale,
            .internalFault(code: "resolver_contract"),
        ]

        for (index, outcome) in outcomes.enumerated() {
            let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
            XCTAssertEqual(scopes.activeScope, scope1)
            var coordinator = GaryxNavigationIntentCoordinator()
            let intent = ordinary("ready", thread: "thread-1")
            let ticket = coordinator.beginPreparation(
                intentID: intent.id,
                key: .ordinaryNavigation,
                scope: scope1
            )
            XCTAssertEqual(
                coordinator.completePreparation(
                    ticket,
                    outcome: outcome,
                    scopes: scopes,
                    routeState: .init()
                ),
                expected[index]
            )
        }
    }

    func testNonTerminalTransactionOnlyQueuesUntilTerminal() {
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
        var coordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
        let intent = ordinary("deep-link", thread: "thread-1")
        let ticket = coordinator.beginPreparation(
            intentID: intent.id,
            key: intent.coalescingKey,
            scope: scope1
        )

        XCTAssertEqual(
            coordinator.completePreparation(
                ticket,
                outcome: .ready(intent),
                scopes: scopes,
                routeState: .init()
            ),
            .queued
        )
        XCTAssertTrue(coordinator.drainAdmissible().isEmpty)
        coordinator.setTransactionStatus(.terminal)
        XCTAssertEqual(coordinator.drainAdmissible(), [intent])
    }

    func testResolverIgnoringCancellationLosesTripleCASInBothCompletionOrders() {
        for completeNewestFirst in [false, true] {
            let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
            var coordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
            let first = ordinary("first", thread: "thread-a")
            let second = ordinary("second", thread: "thread-b")
            let firstTicket = coordinator.beginPreparation(
                intentID: first.id,
                key: .ordinaryNavigation,
                scope: scope1
            )
            let secondTicket = coordinator.beginPreparation(
                intentID: second.id,
                key: .ordinaryNavigation,
                scope: scope1
            )

            let firstResult: GaryxNavigationQueueResult
            let secondResult: GaryxNavigationQueueResult
            if completeNewestFirst {
                secondResult = coordinator.completePreparation(
                    secondTicket,
                    outcome: .ready(second),
                    scopes: scopes,
                    routeState: .init()
                )
                firstResult = coordinator.completePreparation(
                    firstTicket,
                    outcome: .ready(first),
                    scopes: scopes,
                    routeState: .init()
                )
            } else {
                firstResult = coordinator.completePreparation(
                    firstTicket,
                    outcome: .ready(first),
                    scopes: scopes,
                    routeState: .init()
                )
                secondResult = coordinator.completePreparation(
                    secondTicket,
                    outcome: .ready(second),
                    scopes: scopes,
                    routeState: .init()
                )
            }
            XCTAssertEqual(firstResult, .stalePreparation)
            XCTAssertEqual(secondResult, .queued)
            XCTAssertEqual(coordinator.queued, [second])
        }
    }

    func testTripleCASRejectsOldScopeAndSupersededSameIntent() {
        var scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
        var coordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
        let intent = ordinary("same", thread: "thread-a")
        let oldTicket = coordinator.beginPreparation(
            intentID: intent.id,
            key: .ordinaryNavigation,
            scope: scope1
        )
        let currentTicket = coordinator.beginPreparation(
            intentID: intent.id,
            key: .ordinaryNavigation,
            scope: scope1
        )
        XCTAssertEqual(
            coordinator.completePreparation(
                oldTicket,
                outcome: .ready(intent),
                scopes: scopes,
                routeState: .init()
            ),
            .stalePreparation
        )

        XCTAssertTrue(scopes.switchActive(to: scope2))
        XCTAssertEqual(
            coordinator.completePreparation(
                currentTicket,
                outcome: .ready(intent),
                scopes: scopes,
                routeState: .init()
            ),
            .stalePreparation
        )
    }

    func testSafetyEffectsUseDistinctIdempotentKeysAndCommute() {
        let forward = drainSafety(order: [.routeInvalidation, .logout])
        let reverse = drainSafety(order: [.logout, .routeInvalidation])

        XCTAssertEqual(forward.map(\.coalescingKey), [.routeInvalidation, .logout])
        XCTAssertEqual(reverse.map(\.coalescingKey), [.routeInvalidation, .logout])
    }

    func testSafetyEffectDropsLowerPriorityAndCannotBeSuperseded() {
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
        var coordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
        enqueue(ordinary("ordinary", thread: "thread-a"), into: &coordinator, scopes: scopes)
        enqueue(scopeChange("switch", to: scope2), into: &coordinator, scopes: scopes)
        enqueue(logout("logout"), into: &coordinator, scopes: scopes)
        enqueue(ordinary("late", thread: "thread-b"), into: &coordinator, scopes: scopes)
        XCTAssertEqual(coordinator.queued.map(\.coalescingKey), [.logout])

        coordinator.setTransactionStatus(.terminal)
        XCTAssertEqual(coordinator.drainAdmissible().map(\.coalescingKey), [.logout])
        XCTAssertTrue(coordinator.authenticationBarrier)

        let blocked = ordinary("blocked", thread: "thread-c")
        let ticket = coordinator.beginPreparation(
            intentID: blocked.id,
            key: .ordinaryNavigation,
            scope: scope1
        )
        XCTAssertEqual(
            coordinator.completePreparation(
                ticket,
                outcome: .ready(blocked),
                scopes: scopes,
                routeState: .init()
            ),
            .authenticationRequired
        )
        coordinator.authenticated(in: scope1)
        XCTAssertFalse(coordinator.authenticationBarrier)
        XCTAssertTrue(coordinator.queued.isEmpty, "blocked navigation is never auto-reprepared")
    }

    func testGatewayScopeChangeAndOrdinaryLanesAreLastWins() {
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
        var coordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
        enqueue(scopeChange("switch-a", to: scope2), into: &coordinator, scopes: scopes)
        let scope3 = GaryxGatewayScope(identity: "gateway-3", epoch: 1)
        enqueue(scopeChange("switch-b", to: scope3), into: &coordinator, scopes: scopes)
        XCTAssertEqual(coordinator.queued, [scopeChange("switch-b", to: scope3)])

        var ordinaryCoordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
        enqueue(ordinary("a", thread: "thread-a"), into: &ordinaryCoordinator, scopes: scopes)
        enqueue(ordinary("b", thread: "thread-b"), into: &ordinaryCoordinator, scopes: scopes)
        XCTAssertEqual(ordinaryCoordinator.queued, [ordinary("b", thread: "thread-b")])
    }

    func testAbsoluteDependencyRebasesWhileRelativeDependencyUsesBothRevisions() {
        var route = GaryxCanonicalRouteState()
        let base = GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: "base"),
            destination: .conversation(threadID: "thread-a")
        )
        _ = route.open(base)
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)

        var absoluteCoordinator = GaryxNavigationIntentCoordinator()
        let absolute = ordinary("absolute", thread: "thread-b")
        let absoluteTicket = absoluteCoordinator.beginPreparation(
            intentID: absolute.id,
            key: .ordinaryNavigation,
            scope: scope1
        )
        _ = route.open(GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: "new-top"),
            destination: .panel("agents")
        ))
        XCTAssertEqual(
            absoluteCoordinator.completePreparation(
                absoluteTicket,
                outcome: .ready(absolute),
                scopes: scopes,
                routeState: route
            ),
            .admittedImmediately
        )

        let staleRelative = GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: "relative"),
            effect: .ordinaryNavigation(.settingsDetail("gateway")),
            dependency: .relative(
                base: base.id,
                payloadRevision: base.payloadRevision,
                stackRevision: 1,
                mismatch: .reprepare
            )
        )
        var relativeCoordinator = GaryxNavigationIntentCoordinator()
        let relativeTicket = relativeCoordinator.beginPreparation(
            intentID: staleRelative.id,
            key: .ordinaryNavigation,
            scope: scope1
        )
        XCTAssertEqual(
            relativeCoordinator.completePreparation(
                relativeTicket,
                outcome: .ready(staleRelative),
                scopes: scopes,
                routeState: route
            ),
            .reprepareRequired
        )

        let discarded = GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: "discard"),
            effect: .ordinaryNavigation(.settingsDetail("provider")),
            dependency: .relative(
                base: base.id,
                payloadRevision: base.payloadRevision,
                stackRevision: 1,
                mismatch: .discard
            )
        )
        var discardCoordinator = GaryxNavigationIntentCoordinator()
        let discardTicket = discardCoordinator.beginPreparation(
            intentID: discarded.id,
            key: .ordinaryNavigation,
            scope: scope1
        )
        XCTAssertEqual(
            discardCoordinator.completePreparation(
                discardTicket,
                outcome: .ready(discarded),
                scopes: scopes,
                routeState: route
            ),
            .dependencyDiscarded
        )
    }

    func testScopeLifecyclePreservesSuspendedPartitionsAndRevocationWatermark() {
        var registry = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
        XCTAssertTrue(registry.switchActive(to: scope2))
        XCTAssertEqual(registry.lifecycle(of: scope1), .suspended)
        XCTAssertEqual(registry.admitDomainEvent(from: scope1), .acceptedSuspendedPartition)
        XCTAssertTrue(registry.switchActive(to: scope1))
        XCTAssertEqual(registry.lifecycle(of: scope1), .active)

        XCTAssertTrue(registry.revoke(scope1))
        XCTAssertEqual(registry.revokedThroughEpoch[scope1.identity], 1)
        XCTAssertEqual(registry.admitDomainEvent(from: scope1), .rejectedRevoked)
        XCTAssertFalse(registry.switchActive(to: scope1))

        let reauthenticated = GaryxGatewayScope(identity: scope1.identity, epoch: 2)
        XCTAssertTrue(registry.switchActive(to: reauthenticated))
        XCTAssertEqual(registry.activeScope, reauthenticated)
        XCTAssertEqual(registry.admitDomainEvent(from: scope1), .rejectedRevoked)
    }

    func testRevokedThroughEpochStaysBoundedAcrossChurn() {
        var registry = GaryxGatewayScopeRegistry()
        for epoch in 1...500 {
            let scope = GaryxGatewayScope(identity: "gateway", epoch: UInt64(epoch))
            XCTAssertTrue(registry.switchActive(to: scope))
            XCTAssertTrue(registry.revoke(scope))
        }
        XCTAssertEqual(registry.revokedThroughEpoch, ["gateway": 500])
        XCTAssertTrue(registry.lifecycles.isEmpty, "revoked epochs collapse into the watermark")
        for epoch in 1...500 {
            XCTAssertEqual(
                registry.admitDomainEvent(
                    from: GaryxGatewayScope(identity: "gateway", epoch: UInt64(epoch))
                ),
                .rejectedRevoked
            )
        }
    }

    private enum SafetyKind { case logout, routeInvalidation }

    private func drainSafety(order: [SafetyKind]) -> [GaryxPreparedNavigationIntent] {
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope1)
        var coordinator = GaryxNavigationIntentCoordinator(transactionStatus: .nonTerminal)
        for kind in order {
            let intent: GaryxPreparedNavigationIntent = switch kind {
            case .logout: logout("logout")
            case .routeInvalidation: invalidation("invalidate")
            }
            enqueue(intent, into: &coordinator, scopes: scopes)
        }
        coordinator.setTransactionStatus(.terminal)
        return coordinator.drainAdmissible()
    }

    private func enqueue(
        _ intent: GaryxPreparedNavigationIntent,
        into coordinator: inout GaryxNavigationIntentCoordinator,
        scopes: GaryxGatewayScopeRegistry
    ) {
        let ticket = coordinator.beginPreparation(
            intentID: intent.id,
            key: intent.coalescingKey,
            scope: scope1
        )
        _ = coordinator.completePreparation(
            ticket,
            outcome: .ready(intent),
            scopes: scopes,
            routeState: .init()
        )
    }

    private func ordinary(_ id: String, thread: String) -> GaryxPreparedNavigationIntent {
        GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: id),
            effect: .ordinaryNavigation(.conversation(threadID: thread)),
            dependency: .absolute
        )
    }

    private func logout(_ id: String) -> GaryxPreparedNavigationIntent {
        GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: id),
            effect: .logout(scope: scope1),
            dependency: .absolute
        )
    }

    private func invalidation(_ id: String) -> GaryxPreparedNavigationIntent {
        GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: id),
            effect: .routeInvalidation(fallback: nil),
            dependency: .absolute
        )
    }

    private func scopeChange(
        _ id: String,
        to scope: GaryxGatewayScope
    ) -> GaryxPreparedNavigationIntent {
        GaryxPreparedNavigationIntent(
            id: GaryxNavigationIntentID(rawValue: id),
            effect: .gatewayScopeChange(scope),
            dependency: .absolute
        )
    }
}
