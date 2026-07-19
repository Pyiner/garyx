import XCTest
@testable import GaryxMobileCore

final class GaryxPresentationTransactionTests: XCTestCase {
    func testPhaseByFiveOwnerTable() {
        var coordinator = GaryxPresentationTransactionCoordinator()
        XCTAssertEqual(
            coordinator.owners,
            owners(.source, .source, .source, .source, .edgePanEligible)
        )

        XCTAssertTrue(coordinator.begin())
        XCTAssertEqual(
            coordinator.owners,
            owners(.source, .source, .frozen, .source, .tracking)
        )

        XCTAssertTrue(coordinator.release(commit: false))
        XCTAssertEqual(
            coordinator.owners,
            owners(.source, .source, .frozen, .source, .coordinatorRegrabOnly)
        )
        XCTAssertTrue(coordinator.regrabCancelSettle())
        XCTAssertEqual(coordinator.phase, .preCommit)

        XCTAssertTrue(coordinator.release(commit: true))
        XCTAssertEqual(
            coordinator.owners,
            owners(.destination, .destination, .frozen, .sourceFrozen, .locked)
        )
        XCTAssertTrue(coordinator.finish(visibility: .visible))
        XCTAssertEqual(
            coordinator.owners,
            owners(.destination, .destination, .destination, .destination, .edgePanEligible)
        )
    }

    func testTerminalOutcomeByVisibilitySixCombinationDispositionTable() {
        let cases: [(GaryxPresentationTerminalState, GaryxPresentationTerminalDisposition)] = [
            (
                .init(outcome: .committed, visibility: .visible),
                .init(
                    focus: .activateDestinationWhenInputReady,
                    screenChanged: .emitExactlyOnce,
                    modal: .destinationEligible
                )
            ),
            (
                .init(outcome: .committed, visibility: .inactive),
                .init(
                    focus: .deferDestinationUntilActive,
                    screenChanged: .deferUntilActive,
                    modal: .remainFrozenUntilActive
                )
            ),
            (
                .init(outcome: .committed, visibility: .superseded),
                .init(focus: .none, screenChanged: .none, modal: .handoffToNextTransaction)
            ),
            (
                .init(outcome: .cancelled, visibility: .visible),
                .init(focus: .none, screenChanged: .none, modal: .restoreSourceEligibility)
            ),
            (
                .init(outcome: .cancelled, visibility: .inactive),
                .init(focus: .none, screenChanged: .none, modal: .remainFrozenUntilActive)
            ),
            (
                .init(outcome: .cancelled, visibility: .superseded),
                .init(focus: .none, screenChanged: .none, modal: .handoffToNextTransaction)
            ),
        ]

        for (state, expected) in cases {
            XCTAssertEqual(GaryxPresentationTerminalDisposition.resolve(state), expected)
        }
    }

    func testTerminalOwnerTableForAllSixCombinations() {
        for outcome in GaryxPresentationTerminalOutcome.allCases {
            for visibility in GaryxPresentationVisibility.allCases {
                var coordinator = GaryxPresentationTransactionCoordinator()
                XCTAssertTrue(coordinator.begin())
                XCTAssertTrue(coordinator.release(commit: outcome == .committed))
                XCTAssertTrue(coordinator.finish(visibility: visibility))

                XCTAssertEqual(
                    coordinator.owners.canonical,
                    outcome == .committed ? .destination : .source
                )
                XCTAssertEqual(coordinator.owners.data, coordinator.owners.canonical)
                switch visibility {
                case .visible:
                    XCTAssertEqual(
                        coordinator.owners.pageInteraction,
                        outcome == .committed ? .destination : .source
                    )
                case .inactive:
                    XCTAssertEqual(coordinator.owners.pageInteraction, .frozen)
                case .superseded:
                    XCTAssertEqual(coordinator.owners.pageInteraction, .nextTransaction)
                }
            }
        }
    }

    func testEventByPhaseDecisionTable() {
        for startingPhase in [
            GaryxPresentationTransactionPhase.preCommit,
            .cancelSettle,
            .commitSettle,
        ] {
            var geometry = GaryxPresentationTransactionCoordinator(phase: startingPhase)
            XCTAssertEqual(geometry.handle(.geometryChanged), .rederiveGeometry)
            XCTAssertEqual(geometry.phase, startingPhase)
            XCTAssertEqual(geometry.handle(.keyboardGeometryChanged), .ignored)

            var inactive = GaryxPresentationTransactionCoordinator(phase: startingPhase)
            let inactiveTerminal = GaryxPresentationTerminalState(
                outcome: startingPhase == .commitSettle ? .committed : .cancelled,
                visibility: .inactive
            )
            XCTAssertEqual(
                inactive.handle(.sceneInactive),
                .reachedTerminal(inactiveTerminal)
            )

            for event in [
                GaryxPresentationCoordinatorEvent.routeInvalidated,
                .gatewayForced,
            ] {
                var forced = GaryxPresentationTransactionCoordinator(phase: startingPhase)
                let forcedTerminal = GaryxPresentationTerminalState(
                    outcome: startingPhase == .commitSettle ? .committed : .cancelled,
                    visibility: .superseded
                )
                XCTAssertEqual(forced.handle(event), .reachedTerminal(forcedTerminal))
            }

            var cancelled = GaryxPresentationTransactionCoordinator(phase: startingPhase)
            let cancelEffect = cancelled.handle(.recognizerCancelled)
            if startingPhase == .preCommit {
                XCTAssertEqual(cancelEffect, .transitioned(.cancelSettle))
            } else {
                XCTAssertEqual(cancelEffect, .ignored)
                XCTAssertEqual(cancelled.phase, startingPhase)
            }
        }
    }

    func testHostLifecycleAllowsOnlyMountedAppearedActiveInactiveDisappearedSequence() {
        var lifecycle = GaryxRouteHostLifecycle()
        XCTAssertFalse(lifecycle.transition(to: .active))
        XCTAssertTrue(lifecycle.transition(to: .appeared))
        XCTAssertTrue(lifecycle.transition(to: .active))
        XCTAssertTrue(lifecycle.transition(to: .inactive))
        XCTAssertTrue(lifecycle.transition(to: .active))
        XCTAssertTrue(lifecycle.transition(to: .inactive))
        XCTAssertTrue(lifecycle.transition(to: .disappeared))
        XCTAssertFalse(lifecycle.transition(to: .active))
    }

    func testLeaseIsAcquiredSynchronouslyAndNestedChildKeepsBarrier() {
        var tree = GaryxPresentationLeaseTree()
        let parent = token("parent")
        let child = token("child")
        XCTAssertTrue(tree.acquire(parent))
        XCTAssertTrue(tree.hasBarrier, "barrier exists before UIKit presentation starts")
        XCTAssertTrue(tree.acquire(child, parent: parent))
        tree.markPresented(parent)
        tree.markPresented(child)

        tree.dismissalCompleted(child)
        XCTAssertEqual(tree.records[child]?.joinState, .released)
        XCTAssertTrue(tree.hasBarrier, "parent still owns the barrier")
        XCTAssertEqual(tree.records[parent]?.releaseCount, 0)

        tree.dismissalCompleted(parent)
        XCTAssertFalse(tree.hasBarrier)
        XCTAssertEqual(tree.records[parent]?.releaseCount, 1)
    }

    func testParentForcedDismissReleasesWholeTreeExactlyOnce() {
        var tree = GaryxPresentationLeaseTree()
        let parent = token("parent")
        let child = token("child")
        let grandchild = token("grandchild")
        XCTAssertTrue(tree.acquire(parent))
        XCTAssertTrue(tree.acquire(child, parent: parent, resultBearing: true))
        XCTAssertTrue(tree.acquire(grandchild, parent: child))

        tree.forceDismissSubtree(parent)
        tree.dismissalCompleted(parent)
        tree.forceDismissSubtree(parent)

        XCTAssertFalse(tree.hasBarrier)
        for member in [parent, child, grandchild] {
            XCTAssertEqual(tree.records[member]?.joinState, .released)
            XCTAssertEqual(tree.records[member]?.releaseCount, 1)
        }
        XCTAssertEqual(tree.records[child]?.result, .explicitNoResult)
    }

    func testResultBearingLeaseJoinsDismissalAndResultInBothOrders() {
        for resultFirst in [false, true] {
            var tree = GaryxPresentationLeaseTree()
            let picker = token(resultFirst ? "result-first" : "dismiss-first")
            XCTAssertTrue(tree.acquire(picker, resultBearing: true))
            tree.markPresented(picker)

            if resultFirst {
                tree.recordResult(picker)
                XCTAssertEqual(tree.records[picker]?.joinState, .resultRecordedAwaitingDismissal)
                tree.dismissalCompleted(picker)
            } else {
                tree.dismissalCompleted(picker)
                XCTAssertEqual(tree.records[picker]?.joinState, .dismissedAwaitingResult)
                XCTAssertTrue(tree.hasBarrier)
                tree.recordResult(picker)
            }

            XCTAssertEqual(tree.records[picker]?.joinState, .released)
            XCTAssertEqual(tree.records[picker]?.releaseCount, 1)
            XCTAssertFalse(tree.hasBarrier)
        }
    }

    func testResultCancellationAndPresentationFailureAreExplicitTerminals() {
        var cancelled = GaryxPresentationLeaseTree()
        let picker = token("picker")
        XCTAssertTrue(cancelled.acquire(picker, resultBearing: true))
        cancelled.recordNoResult(picker)
        cancelled.dismissalCompleted(picker)
        XCTAssertEqual(cancelled.records[picker]?.joinState, .released)

        var failed = GaryxPresentationLeaseTree()
        let sheet = token("failed")
        XCTAssertTrue(failed.acquire(sheet, resultBearing: true))
        failed.presentationFailed(sheet)
        XCTAssertEqual(failed.records[sheet]?.result, .explicitNoResult)
        XCTAssertFalse(failed.hasBarrier)
    }

    func testProgrammaticAndInteractiveDismissCallbacksDeduplicateRelease() {
        var tree = GaryxPresentationLeaseTree()
        let sheet = token("sheet")
        XCTAssertTrue(tree.acquire(sheet))
        tree.markDismissing(sheet)
        tree.dismissalCompleted(sheet) // programmatic completion
        tree.dismissalCompleted(sheet) // presentationControllerDidDismiss
        XCTAssertEqual(tree.records[sheet]?.releaseCount, 1)
        XCTAssertEqual(tree.garbageCollectReleased(), 1)
        XCTAssertTrue(tree.records.isEmpty)
        tree.dismissalCompleted(sheet)
        XCTAssertTrue(tree.records.isEmpty, "late callbacks cannot recreate a reclaimed lease")
    }

    func testReleasedLeaseForestChurnGarbageCollectsToZero() {
        var tree = GaryxPresentationLeaseTree()
        for index in 0..<500 {
            let parent = token("parent-\(index)")
            let child = token("child-\(index)")
            XCTAssertTrue(tree.acquire(parent))
            XCTAssertTrue(tree.acquire(child, parent: parent, resultBearing: true))
            tree.forceDismissSubtree(parent)
            XCTAssertFalse(tree.hasBarrier)
            XCTAssertEqual(tree.records[parent]?.releaseCount, 1)
            XCTAssertEqual(tree.records[child]?.releaseCount, 1)
            XCTAssertEqual(tree.garbageCollectReleased(), 2)
            XCTAssertTrue(tree.records.isEmpty)
            tree.dismissalCompleted(child)
            XCTAssertTrue(tree.records.isEmpty)
        }
    }

    func testHardSnapRemainsBlockedUntilForcedDismissCompletion() {
        var tree = GaryxPresentationLeaseTree()
        let modal = token("modal")
        XCTAssertTrue(tree.acquire(modal))
        tree.markDismissing(modal)
        XCTAssertTrue(tree.hasBarrier)
        tree.forceDismissSubtree(modal)
        XCTAssertFalse(tree.hasBarrier, "hard snap may occur only after the release event")
    }

    func testPathDiffDecisionTableAllRows() {
        let a = entry("a", .conversation(threadID: "a"))
        let b = entry("b", .panel("agents"))
        let c = entry("c", .settingsDetail("provider"))
        let a2 = entry("a2", .conversation(threadID: "a"))
        var promoted = entry("draft", .conversationDraft(draftID: "draft"))
        let draft = promoted
        promoted.replacePayload(with: .conversation(threadID: "thread"))

        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [a], to: [a]), .noChange)
        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [a], to: [a, b]), .push)
        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [a, b], to: [a]), .pop)
        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [a, b, c], to: [a]), .popMultiple(2))
        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [a, b], to: [a, c]), .replaceTop)
        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [draft], to: [promoted]), .promoteInPlace)
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(from: [a, draft, c], to: [a, promoted, c]),
            .promoteInPlace,
            "mid-stack promotion changes payload identity without changing topology"
        )
        XCTAssertEqual(GaryxPathDiffPlanner.decide(from: [a], to: []), .resetToHome)
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(
                from: [a, b],
                to: [c],
                source: .declaredWholeChainReplacement
            ),
            .wholeChainReplacement
        )
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(from: [a, b, c], to: [a, a2, c]),
            .normalizeIllegalMutationAndLogFault
        )
        var mutatedMiddle = b
        mutatedMiddle.replacePayload(with: .panel("skills"))
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(from: [a, b, c], to: [a, mutatedMiddle, c]),
            .normalizeIllegalMutationAndLogFault,
            "same-instance middle mutations are still illegal path diffs"
        )
        var illegalTopDestination = b
        illegalTopDestination.replacePayload(with: .panel("skills"))
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(from: [a, b], to: [a, illegalTopDestination]),
            .normalizeIllegalMutationAndLogFault,
            "same-instance top destination changes require a declared promotion"
        )
        let refreshedTop = GaryxRouteEntry(
            id: b.id,
            destination: b.destination,
            payloadRevision: b.payloadRevision + 1
        )
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(from: [a, b], to: [a, refreshedTop]),
            .inPlacePayloadUpdate
        )
        XCTAssertEqual(
            GaryxPathDiffPlanner.decide(from: [a, b], to: [a, b, a2]),
            .push,
            "same composer key opens as a new occurrence"
        )
    }

    private func owners(
        _ canonical: GaryxPresentationRouteOwner,
        _ data: GaryxPresentationRouteOwner,
        _ interaction: GaryxPresentationInteractionOwner,
        _ focus: GaryxPresentationFocusOwner,
        _ control: GaryxPresentationTransitionControl
    ) -> GaryxPresentationOwnerSnapshot {
        .init(
            canonical: canonical,
            data: data,
            pageInteraction: interaction,
            focusAndAccessibility: focus,
            transitionControl: control
        )
    }

    private func token(_ value: String) -> GaryxPresentationLeaseToken {
        GaryxPresentationLeaseToken(rawValue: value)
    }

    private func entry(_ id: String, _ destination: GaryxRouteDestination) -> GaryxRouteEntry {
        GaryxRouteEntry(id: GaryxRouteInstanceID(rawValue: id), destination: destination)
    }
}
