import Combine
import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxProductionRouteIntentIntegrationTests: XCTestCase {
    override func tearDown() {
        GaryxRoutePreparationURLProtocolStub.requestHandler = nil
        super.tearDown()
    }

    func testPreparedConversationOccurrenceIsConsumedExactlyOnceByOpen() throws {
        let store = GaryxProductionRouteStore()
        let destination = GaryxRouteDestination.conversation(
            threadID: "thread-prepared"
        )

        let prepared = try XCTUnwrap(store.prepareConversation(destination))
        XCTAssertTrue(store.path.isEmpty)
        XCTAssertEqual(store.prepareConversation(destination)?.id, prepared.id)

        let opened = store.open(destination, source: .replace, animated: false)
        XCTAssertEqual(opened.id, prepared.id)
        XCTAssertEqual(store.path, [prepared])

        let next = store.open(destination, source: .current, animated: false)
        XCTAssertNotEqual(next.id, prepared.id)
        XCTAssertEqual(store.path.map(\.id), [prepared.id, next.id])
    }

    func testProductionPreparationPreservesAllSixTypedOutcomes() {
        let cases: [(
            GaryxPrepareOutcome<[GaryxRouteDestination]>,
            GaryxNavigationQueueResult
        )] = [
            (.ready([.panel("agents")]), .admittedImmediately),
            (.userVisibleNotFound, .userVisibleNotFound),
            (.retryableFailure(message: "retry"), .retryableFailure(message: "retry")),
            (.authenticationRequired, .authenticationRequired),
            (.cancelledOrStale, .cancelledOrStale),
            (.internalFault(code: "fixture"), .internalFault(code: "fixture")),
        ]

        for (outcome, expected) in cases {
            let store = GaryxProductionRouteStore()
            let preparation = store.beginNavigationPreparation(source: .replace)
            let submission = store.completeNavigationPreparation(
                preparation,
                outcome: outcome
            )
            XCTAssertEqual(submission.result, expected)
            XCTAssertEqual(
                store.path.map(\.destination),
                expected == .admittedImmediately ? [.panel("agents")] : []
            )
        }
    }

    func testAbsoluteDeepLinkWaitsForGestureTerminalWithoutPollutingPath() {
        let store = GaryxProductionRouteStore()
        let current = entry("current", .panel("agents"))
        store.applyCanonicalPath([current])
        store.routePhaseChanged(.preCommit)

        let preparation = store.beginNavigationPreparation(source: .deepLink)
        var visibleCount = 0
        let submission = store.completeNavigationPreparation(
            preparation,
            outcome: .ready([
                .settingsDetail("manage"),
                .settingsDetail("provider"),
            ]),
            onVisible: { visibleCount += 1 }
        )

        XCTAssertEqual(submission.result, .queued)
        XCTAssertEqual(store.path, [current])
        XCTAssertEqual(visibleCount, 0)

        store.rendererBecameIdle()

        XCTAssertEqual(
            store.path.map(\.destination),
            [.settingsDetail("manage"), .settingsDetail("provider")]
        )
        XCTAssertEqual(visibleCount, 1)
    }

    func testDeepLinkFromHomeUsesWholeChainHardSnapWithoutTransitionResidue() {
        let store = GaryxProductionRouteStore()
        let container = makeContainer(path: [], store: store)
        store.attach(container)
        let preparation = store.beginNavigationPreparation(source: .deepLink)
        var visibleCount = 0

        let submission = store.completeNavigationPreparation(
            preparation,
            outcome: .ready([
                .settingsDetail("manage"),
                .settingsDetail("provider"),
            ]),
            onVisible: { visibleCount += 1 }
        )

        XCTAssertEqual(submission.result, .admittedImmediately)
        XCTAssertEqual(
            container.path.map(\.destination),
            [.settingsDetail("manage"), .settingsDetail("provider")]
        )
        XCTAssertEqual(visibleCount, 1)
        XCTAssertFalse(container.hasTerminalResidue)
    }

    func testRelativeIntentIsDiscardedWhenItsOpenerChangesWhileQueued() {
        let store = GaryxProductionRouteStore()
        let opener = entry("opener", .panel("automations"))
        store.applyCanonicalPath([opener])
        store.routePhaseChanged(.preCommit)

        let preparation = store.beginNavigationPreparation(source: .current)
        let submission = store.completeNavigationPreparation(
            preparation,
            outcome: .ready([
                .workspaceDrilldown(.automationThreads(automationID: "automation-1"))
            ])
        )
        XCTAssertEqual(submission.result, .queued)

        let replacement = entry("replacement", .panel("agents"))
        store.applyCanonicalPath([replacement])
        store.rendererBecameIdle()

        XCTAssertEqual(store.path, [replacement])
    }

    func testQueuedDeepLinkCannotAdmitPreparedPayloadAfterGatewayScopeChanges() {
        let scopeA = GaryxGatewayScope(identity: "gateway-a", epoch: 1)
        let scopeB = GaryxGatewayScope(identity: "gateway-b", epoch: 1)
        var scopes = GaryxGatewayScopeRegistry(initialActiveScope: scopeA)
        let store = GaryxProductionRouteStore()
        store.configureNavigationScopes { scopes }
        store.routePhaseChanged(.preCommit)
        let preparation = store.beginNavigationPreparation(
            source: .replace,
            scopes: scopes
        )
        var visibleCount = 0

        let submission = store.completeNavigationPreparation(
            preparation,
            outcome: .ready([.panel("skills")]),
            scopes: scopes,
            onVisible: { visibleCount += 1 }
        )
        XCTAssertEqual(submission.result, .queued)

        XCTAssertTrue(scopes.switchActive(to: scopeB))
        store.rendererBecameIdle()

        XCTAssertTrue(store.path.isEmpty)
        XCTAssertEqual(visibleCount, 0)
    }

    func testQueuedNewChatFreezesFreshDraftIdentityAndIgnoresVisibleBotDraft() async {
        let model = routePreparationModel(session: .shared)
        model.connectionState = .ready(version: "test")
        model.effectiveDefaultAgentId = "agent-before"
        model.pendingBotId = "bot:visible"
        model.pendingBotDraftGeneration = model.selectedThreadDraftGeneration
        model.productionRouteStore.routePhaseChanged(.preCommit)

        await model.openMobileRoute(.chat, source: .deepLink)

        XCTAssertTrue(model.productionRouteStore.path.isEmpty)
        XCTAssertEqual(model.pendingBotId, "bot:visible")

        model.effectiveDefaultAgentId = "agent-after"
        model.productionRouteStore.rendererBecameIdle()

        let expected = GaryxRouteDestination.conversationDraft(
            draftID: "new-thread:agent-before"
        )
        XCTAssertEqual(model.productionRouteStore.path.map(\.destination), [expected])
        XCTAssertEqual(model.newThreadComposerPayloadKey, expected.composerKey)
        XCTAssertNil(model.pendingBotId)
        XCTAssertEqual(model.newThreadAgentTargetId(), "agent-before")
    }

    func testSameThreadReopenImmediatelyDrainsSupersededActivationWaiter() async {
        let model = routePreparationModel(session: .shared)
        let thread = GaryxMobileModel.placeholderThreadSummary(id: "thread-1")
        let first = entry("occurrence-1", .conversation(threadID: thread.id))
        let second = entry("occurrence-2", .conversation(threadID: thread.id))
        model.selectedThread = thread
        model.productionRouteStore.applyCanonicalPath([second])
        model.conversationContentActivationOccurrenceID = first.id
        model.conversationInitialHistoryRefreshTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 60_000_000_000)
        }

        let registered = expectation(description: "first occurrence waiter registered")
        let resumed = expectation(description: "superseded occurrence waiter resumed")
        let waiterTask = Task { @MainActor in
            await withCheckedContinuation { continuation in
                model.conversationContentActivationWaiters[first.id, default: []]
                    .append(continuation)
                registered.fulfill()
            }
            resumed.fulfill()
        }
        await fulfillment(of: [registered], timeout: 1)

        model.conversationRouteContentPreparationBegan(second)

        XCTAssertNil(model.conversationContentActivationWaiters[first.id])
        XCTAssertEqual(model.conversationContentActivationOccurrenceID, second.id)
        XCTAssertEqual(model.completedConversationContentActivationOccurrenceID, second.id)
        XCTAssertNotNil(
            model.conversationInitialHistoryRefreshTask,
            "local activation must complete while the independent history refresh remains in flight"
        )
        model.cancelConversationContentActivation()
        await fulfillment(of: [resumed], timeout: 1)
        waiterTask.cancel()
    }

    func testProductionLeaseBridgeJoinsNestedDismissalsAndBothResultOrders() throws {
        let store = GaryxProductionRouteStore()
        let container = makeContainer(path: [], store: store)
        store.attach(container)
        let bridge = store.presentationCoordinator
        let parent = GaryxPresentationLeaseSession()
        let child = GaryxPresentationLeaseSession(resultBearing: true)
        let context = operationContext("picker-operation")
        var factoryCalls = 0

        parent.acquireIfNeeded(
            coordinator: bridge,
            parent: nil,
            operationContext: { nil }
        )
        let parentToken = try XCTUnwrap(parent.token)
        child.acquireIfNeeded(
            coordinator: bridge,
            parent: parentToken,
            operationContext: {
                factoryCalls += 1
                return context
            }
        )
        child.acquireIfNeeded(
            coordinator: bridge,
            parent: parentToken,
            operationContext: {
                factoryCalls += 1
                return nil
            }
        )

        let childToken = try XCTUnwrap(child.token)
        XCTAssertEqual(factoryCalls, 1, "operation context freezes at lease acquisition")
        XCTAssertEqual(child.operationContext?.capability, context.capability)
        XCTAssertEqual(child.operationContext?.requestToken, context.requestToken)
        XCTAssertTrue(child.operationContext?.gatewayClient === context.gatewayClient)
        XCTAssertTrue(store.hasPresentationBarrier)

        child.markPresented()
        child.completeDismissal()
        XCTAssertEqual(
            container.presentationLeaseRecord(childToken)?.joinState,
            .dismissedAwaitingResult
        )
        child.recordResult()
        child.recordResult()
        XCTAssertEqual(container.presentationLeaseRecord(childToken)?.releaseCount, 1)
        XCTAssertTrue(store.hasPresentationBarrier, "the parent still owns the tree")

        let forcedChild = GaryxPresentationLeaseSession(resultBearing: true)
        forcedChild.acquireIfNeeded(
            coordinator: bridge,
            parent: parentToken,
            operationContext: { context }
        )
        let forcedChildToken = try XCTUnwrap(forcedChild.token)
        parent.completeDismissal()
        parent.completeDismissal()
        XCTAssertEqual(container.presentationLeaseRecord(parentToken)?.releaseCount, 1)
        XCTAssertEqual(container.presentationLeaseRecord(forcedChildToken)?.releaseCount, 1)
        XCTAssertFalse(store.hasPresentationBarrier)

        let auditReclaimer = GaryxPresentationLeaseSession()
        auditReclaimer.acquireIfNeeded(
            coordinator: bridge,
            parent: nil,
            operationContext: { nil }
        )
        XCTAssertNil(
            container.presentationLeaseRecord(forcedChildToken),
            "a later acquisition may reclaim the released audit forest"
        )
        auditReclaimer.completeDismissal()

        forcedChild.acquireIfNeeded(
            coordinator: bridge,
            parent: nil,
            operationContext: { context }
        )
        let reacquiredToken = try XCTUnwrap(forcedChild.token)
        XCTAssertNotEqual(reacquiredToken, forcedChildToken)
        forcedChild.recordNoResult()
        forcedChild.completeDismissal()
        XCTAssertEqual(container.presentationLeaseRecord(reacquiredToken)?.releaseCount, 1)

        let resultFirst = GaryxPresentationLeaseSession(resultBearing: true)
        resultFirst.acquireIfNeeded(
            coordinator: bridge,
            parent: nil,
            operationContext: { context }
        )
        let resultFirstToken = try XCTUnwrap(resultFirst.token)
        resultFirst.recordResult()
        XCTAssertEqual(
            container.presentationLeaseRecord(resultFirstToken)?.joinState,
            .resultRecordedAwaitingDismissal
        )
        resultFirst.completeDismissal()
        XCTAssertEqual(container.presentationLeaseRecord(resultFirstToken)?.releaseCount, 1)
        XCTAssertFalse(store.hasPresentationBarrier)
    }

    func testCompletedAlertDismissalObservationDoesNotRepublishBarrier() throws {
        let store = GaryxProductionRouteStore()
        let container = makeContainer(path: [], store: store)
        store.attach(container)
        let session = GaryxPresentationLeaseSession()
        session.acquireIfNeeded(
            coordinator: store.presentationCoordinator,
            parent: nil,
            operationContext: { nil }
        )
        session.markPresented()
        session.markDismissing()
        session.completeDismissal()
        XCTAssertFalse(store.hasPresentationBarrier)

        var publicationCount = 0
        let cancellable = store.objectWillChange.sink {
            publicationCount += 1
        }
        for _ in 0..<8 {
            session.bindingBecameFalse(completesDismissal: true)
        }

        XCTAssertEqual(
            publicationCount,
            0,
            "a completed alert dismissal must be observation-idempotent"
        )
        withExtendedLifetime(cancellable) {}
    }

    func testStoreDetachDefersActivePresentationBarrierPublication() async {
        let store = GaryxProductionRouteStore()
        let container = makeContainer(path: [], store: store)
        store.attach(container)
        let lease = GaryxPresentationLeaseSession()
        lease.acquireIfNeeded(
            coordinator: store.presentationCoordinator,
            parent: nil,
            operationContext: { nil }
        )
        XCTAssertTrue(store.hasPresentationBarrier)

        var isInsideDetach = false
        var publicationCount = 0
        var synchronousPublicationCount = 0
        let publication = store.objectWillChange.sink {
            publicationCount += 1
            if isInsideDetach {
                synchronousPublicationCount += 1
            }
        }

        isInsideDetach = true
        store.detach(container)
        isInsideDetach = false

        XCTAssertFalse(store.isAttached, "container ownership bookkeeping must be immediate")
        XCTAssertFalse(
            store.semanticPresentationBarrierIsActive,
            "the barrier's semantic state settles with ownership"
        )
        XCTAssertTrue(
            store.hasPresentationBarrier,
            "the observable barrier remains stable until the deferred settlement"
        )
        XCTAssertEqual(
            synchronousPublicationCount,
            0,
            "store.detach must not publish from a representable dismantle callback"
        )
        for _ in 0..<20 where store.hasPresentationBarrier {
            await Task.yield()
        }
        XCTAssertFalse(store.hasPresentationBarrier)
        XCTAssertEqual(publicationCount, 1)
        withExtendedLifetime((lease, publication)) {}
    }

    func testDeferredBarrierDetachCannotOverwriteNewerAttachment() {
        let scheduler = GaryxManualObservableSettlementScheduler()
        let store = GaryxProductionRouteStore(
            observableSettlementScheduler: scheduler
        )
        var barrierActivationWasDeferred: [Bool] = []
        store.presentationBarrierActivated = { timing in
            switch timing {
            case .immediate:
                barrierActivationWasDeferred.append(false)
            case .afterViewGraphUpdate:
                barrierActivationWasDeferred.append(true)
            }
        }
        let firstContainer = makeContainer(path: [], store: store)
        store.attach(firstContainer)
        XCTAssertTrue(store.presentationCoordinator.acquire(
            .init(rawValue: "first-barrier"),
            parent: nil,
            resultBearing: false
        ))
        XCTAssertTrue(store.hasPresentationBarrier)
        var publicationCount = 0
        let publication = store.objectWillChange.sink {
            publicationCount += 1
        }

        store.detach(firstContainer)

        XCTAssertFalse(store.semanticPresentationBarrierIsActive)
        XCTAssertTrue(store.hasPresentationBarrier)
        XCTAssertEqual(scheduler.pendingCount, 1)

        let replacementContainer = makeContainer(path: [], store: store)
        XCTAssertTrue(replacementContainer.acquirePresentationLease(
            .init(rawValue: "replacement-barrier")
        ))
        store.attach(
            replacementContainer,
            observableSettlement: .afterViewGraphUpdate
        )

        XCTAssertTrue(store.semanticPresentationBarrierIsActive)
        XCTAssertTrue(store.hasPresentationBarrier)
        XCTAssertEqual(
            barrierActivationWasDeferred,
            [false, true],
            "a lifecycle rebind must propagate graph-safe timing to barrier side effects"
        )
        scheduler.runNext()
        XCTAssertTrue(
            store.hasPresentationBarrier,
            "an older deferred detach cannot clear a replacement container's barrier"
        )
        XCTAssertEqual(scheduler.pendingCount, 0)
        XCTAssertEqual(
            publicationCount,
            0,
            "coalescing the stale detach into the latest active barrier needs no projection write"
        )
        withExtendedLifetime(publication) {}
    }

    func testProductionCanvasLifecycleReplacementDefersObservableSettlement() throws {
        let model = routePreparationModel(session: .shared)
        model.connectionState = .ready(version: "lifecycle-replacement")
        guard case .navigationShell(let firstRootOccurrenceID) =
            model.homeObservationStore.rootSurface else {
            return XCTFail("the production route canvas requires a navigation-shell occurrence")
        }
        let store = model.productionRouteStore

        func rootView(
            identity: Int,
            occurrenceID: GaryxRootSurfaceOccurrenceID
        ) -> AnyView {
            AnyView(
                GaryxProductionRouteLifecycleReplacementProbeRoot(
                    canvasIdentity: identity,
                    rootSurfaceOccurrenceID: occurrenceID,
                    store: store,
                    model: model
                )
            )
        }

        let hostingController = UIHostingController(
            rootView: rootView(identity: 0, occurrenceID: firstRootOccurrenceID)
        )
        let hostingWindow = makeRouteDismantleTestWindow()
        hostingWindow.rootViewController = hostingController
        hostingWindow.isHidden = false
        defer {
            hostingWindow.isHidden = true
            hostingWindow.rootViewController = nil
        }
        hostingController.view.frame = hostingWindow.bounds
        hostingController.view.layoutIfNeeded()
        pumpRouteDismantleRunLoop()

        let firstContainer = try XCTUnwrap(
            routeStackContainers(in: hostingController).first
        )
        model.drawerRevealInteraction.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: firstRootOccurrenceID
        )
        let firstDrawerInteraction = try XCTUnwrap(
            firstContainer.homeLeadingEdgeInteraction
        )
        XCTAssertTrue(firstDrawerInteraction.isEligible())
        XCTAssertTrue(store.presentationCoordinator.acquire(
            .init(rawValue: "lifecycle-replacement-barrier"),
            parent: nil,
            resultBearing: false
        ))
        XCTAssertTrue(firstContainer.hasPresentationBarrier)
        XCTAssertTrue(store.hasPresentationBarrier)

        var lifecycleMutation: String?
        var synchronousPublications: [(
            phase: String,
            source: String,
            stack: [String]
        )] = []
        let barrierPublication = store.objectWillChange.sink {
            guard let lifecycleMutation else { return }
            synchronousPublications.append((
                lifecycleMutation,
                "routeStore.hasPresentationBarrier",
                Thread.callStackSymbols
            ))
        }
        let revealPublication = model.drawerRevealInteraction.objectWillChange.sink {
            guard let lifecycleMutation else { return }
            synchronousPublications.append((
                lifecycleMutation,
                "drawerReveal.presentation",
                Thread.callStackSymbols
            ))
        }
        let taskTreePublication = model.taskTreeRevealInteraction.objectWillChange.sink {
            guard let lifecycleMutation else { return }
            synchronousPublications.append((
                lifecycleMutation,
                "taskTreeReveal.presentation",
                Thread.callStackSymbols
            ))
        }

        model.connectionState = .checking
        model.connectionState = .ready(version: "lifecycle-replacement-second")
        guard case .navigationShell(let secondRootOccurrenceID) =
            model.homeObservationStore.rootSurface else {
            return XCTFail("the replacement navigation shell did not begin")
        }
        XCTAssertNotEqual(secondRootOccurrenceID, firstRootOccurrenceID)
        XCTAssertFalse(firstDrawerInteraction.isEligible())

        lifecycleMutation = "update"
        hostingController.rootView = rootView(
            identity: 0,
            occurrenceID: secondRootOccurrenceID
        )
        hostingController.view.layoutIfNeeded()
        XCTAssertTrue(
            firstDrawerInteraction.isEligible(),
            "the in-place representable update must complete inside the observed window"
        )
        lifecycleMutation = nil
        XCTAssertTrue(synchronousPublications.isEmpty)

        lifecycleMutation = "active-barrier identity replacement"
        hostingController.rootView = rootView(
            identity: 1,
            occurrenceID: secondRootOccurrenceID
        )
        hostingController.view.layoutIfNeeded()
        let barrierReplacementContainer = try XCTUnwrap(
            routeStackContainers(in: hostingController).first
        )
        XCTAssertFalse(barrierReplacementContainer === firstContainer)
        lifecycleMutation = nil

        XCTAssertFalse(store.semanticPresentationBarrierIsActive)
        XCTAssertTrue(
            store.hasPresentationBarrier,
            "only the observable projection remains active until the graph update exits"
        )
        XCTAssertTrue(synchronousPublications.isEmpty, publicationFailure(
            synchronousPublications
        ))

        pumpRouteDismantleRunLoop()
        XCTAssertFalse(store.hasPresentationBarrier)
        XCTAssertTrue(store.isAttached)

        model.drawerRevealInteraction.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: secondRootOccurrenceID
        )
        model.taskTreeRevealInteraction.configure(
            extent: 300,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: secondRootOccurrenceID
        )
        let replacementDrawerInteraction = try XCTUnwrap(
            barrierReplacementContainer.homeLeadingEdgeInteraction
        )
        XCTAssertTrue(replacementDrawerInteraction.isEligible())
        replacementDrawerInteraction.began()
        replacementDrawerInteraction.changed(120, 0)
        model.taskTreeRevealInteraction.setTarget(.open, animated: true)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)
        XCTAssertEqual(
            model.taskTreeRevealInteraction.presentation.phase,
            .settling(.open)
        )

        lifecycleMutation = "active-reveal identity replacement"
        hostingController.rootView = rootView(
            identity: 2,
            occurrenceID: secondRootOccurrenceID
        )
        hostingController.view.layoutIfNeeded()
        let revealReplacementContainer = try XCTUnwrap(
            routeStackContainers(in: hostingController).first
        )
        XCTAssertFalse(revealReplacementContainer === barrierReplacementContainer)
        lifecycleMutation = nil

        XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
        XCTAssertFalse(model.taskTreeRevealInteraction.diagnostics.hasTerminalResidue)
        XCTAssertEqual(
            model.drawerRevealInteraction.presentation.phase,
            .dragging,
            "the observable reveal projection must not settle inside make or dismantle"
        )
        XCTAssertEqual(
            model.taskTreeRevealInteraction.presentation.phase,
            .settling(.open),
            "both model-lived reveal projections must stay inert during graph mutation"
        )
        XCTAssertTrue(synchronousPublications.isEmpty, publicationFailure(
            synchronousPublications
        ))

        pumpRouteDismantleRunLoop()
        XCTAssertEqual(model.drawerRevealInteraction.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
        XCTAssertTrue(store.isAttached)
        withExtendedLifetime((
            barrierPublication,
            revealPublication,
            taskTreePublication
        )) {}
    }

    func testBuild158BackgroundSceneTeardownDoesNotPublishDuringDismantle() throws {
        let barrierScheduler = GaryxManualObservableSettlementScheduler()
        let barrierProbeStore = GaryxProductionRouteStore(
            observableSettlementScheduler: barrierScheduler
        )
        let barrierProbeContainer = makeContainer(path: [], store: barrierProbeStore)
        barrierProbeStore.attach(barrierProbeContainer)
        let barrierProbeLease = GaryxPresentationLeaseSession()
        barrierProbeLease.acquireIfNeeded(
            coordinator: barrierProbeStore.presentationCoordinator,
            parent: nil,
            operationContext: { nil }
        )
        var barrierDetachPublicationCount = 0
        let barrierDetachPublication = barrierProbeStore.objectWillChange.sink {
            barrierDetachPublicationCount += 1
        }
        barrierProbeStore.detach(barrierProbeContainer)
        XCTAssertEqual(
            barrierDetachPublicationCount,
            0,
            "store.detach must not publish while a representable graph may be invalidating"
        )
        XCTAssertFalse(
            barrierProbeStore.semanticPresentationBarrierIsActive,
            "the barrier's semantic terminal decision is synchronous"
        )
        XCTAssertTrue(
            barrierProbeStore.hasPresentationBarrier,
            "the observable projection remains unchanged until the deferred settle"
        )
        XCTAssertEqual(barrierScheduler.pendingCount, 1)
        barrierScheduler.runNext()
        XCTAssertFalse(barrierProbeStore.hasPresentationBarrier)
        XCTAssertEqual(barrierDetachPublicationCount, 1)
        let model = routePreparationModel(session: .shared)
        model.connectionState = .ready(version: "build-158-dismantle-repro")
        guard case .navigationShell(let rootOccurrenceID) = model.homeObservationStore.rootSurface
        else {
            return XCTFail("the production route canvas requires a navigation-shell occurrence")
        }

        model.drawerRevealInteraction.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: rootOccurrenceID
        )

        let activeLease = GaryxPresentationLeaseSession()
        var isReleasingHostingGraph = false
        var synchronousPublications: [(source: String, stack: [String])] = []
        let revealPublication = model.drawerRevealInteraction.objectWillChange.sink {
            guard isReleasingHostingGraph else { return }
            let stack = Thread.callStackSymbols
            guard stack.contains(where: { $0.contains("dismantleUIViewController") }) else {
                return
            }
            synchronousPublications.append(("reveal.presentation", stack))
        }
        let barrierPublication = model.productionRouteStore.objectWillChange.sink {
            guard isReleasingHostingGraph else { return }
            let stack = Thread.callStackSymbols
            guard stack.contains(where: { $0.contains("dismantleUIViewController") }) else {
                return
            }
            synchronousPublications.append(("routeStore.hasPresentationBarrier", stack))
        }

        autoreleasepool {
            var hostingController: UIHostingController<AnyView>? = UIHostingController(
                rootView: AnyView(
                    GaryxProductionRouteDismantleCrashReproRoot(
                        rootSurfaceOccurrenceID: rootOccurrenceID,
                        store: model.productionRouteStore,
                        model: model
                    )
                )
            )
            var hostingWindow: UIWindow? = makeRouteDismantleTestWindow()
            hostingWindow?.rootViewController = hostingController
            hostingWindow?.isHidden = false
            hostingWindow?.layoutIfNeeded()
            pumpRouteDismantleRunLoop()

            XCTAssertTrue(model.productionRouteStore.isAttached)

            activeLease.acquireIfNeeded(
                coordinator: model.productionRouteStore.presentationCoordinator,
                parent: nil,
                operationContext: { nil }
            )
            XCTAssertTrue(model.productionRouteStore.hasPresentationBarrier)

            model.setSidebarVisible(true, animated: true)
            XCTAssertEqual(
                model.drawerRevealInteraction.presentation.phase,
                .settling(.open),
                "the dismantled owner must be active so detach takes forceTerminal -> publish"
            )

            isReleasingHostingGraph = true
            hostingWindow?.windowScene = nil
            hostingController = nil
            hostingWindow = nil
        }
        pumpRouteDismantleRunLoop(duration: 1)
        isReleasingHostingGraph = false

        XCTAssertFalse(
            model.productionRouteStore.isAttached,
            "the released hosting graph must run the production representable dismantle callback"
        )
        XCTAssertFalse(model.productionRouteStore.semanticPresentationBarrierIsActive)
        XCTAssertFalse(model.productionRouteStore.hasPresentationBarrier)
        XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
        model.drawerRevealInteraction.assertTerminalHasZeroResidue()
        XCTAssertEqual(
            model.drawerRevealInteraction.presentation,
            .init(reveal: 330, phase: .idle, target: .open),
            "the deferred reveal projection must converge to the synchronous terminal decision"
        )
        XCTAssertEqual(
            synchronousPublications.count,
            0,
            """
            UIViewControllerRepresentable.dismantle synchronously published into its invalidating \
            SwiftUI graph. Publications: \(synchronousPublications.map(\.source))
            First publication stack:
            \(synchronousPublications.first?.stack.joined(separator: "\n") ?? "<none>")
            """
        )
        withExtendedLifetime((
            barrierProbeLease,
            barrierDetachPublication,
            activeLease,
            revealPublication,
            barrierPublication
        )) {}
    }

    func testSceneInterruptionTerminatesEveryGlobalRevealInteraction() {
        let model = routePreparationModel(session: .shared)
        let revealHost = attachGlobalRevealHost(to: model)
        model.drawerRevealInteraction.configure(extent: 330, restingPosition: .closed)
        model.drawerRevealInteraction.beginGesture(in: revealHost)
        model.drawerRevealInteraction.updateGesture(
            logicalTranslation: 140,
            in: revealHost
        )
        model.taskTreeRevealInteraction.configure(extent: 300, restingPosition: .open)
        model.taskTreeRevealInteraction.setTarget(.closed, animated: true)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation.phase, .settling(.closed))

        model.handleScenePhase(.inactive)

        XCTAssertEqual(model.drawerRevealInteraction.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation, .init(
            reveal: 0,
            phase: .idle,
            target: .closed
        ))
        model.assertGlobalRevealInteractionsHaveZeroResidue()
    }

    func testSceneInterruptionStressLeavesBothLongLivedStoresIdle() {
        let model = routePreparationModel(session: .shared)
        let revealHost = attachGlobalRevealHost(to: model)
        model.drawerRevealInteraction.configure(extent: 330, restingPosition: .closed)
        model.taskTreeRevealInteraction.configure(extent: 300, restingPosition: .closed)

        for iteration in 0..<250 {
            let canonicalPosition: GaryxHorizontalRevealPosition = iteration.isMultiple(of: 2)
                ? .closed
                : .open
            model.sidebarVisible = canonicalPosition == .open
            model.isTaskTreeSidebarOpen = canonicalPosition == .open
            model.drawerRevealInteraction.invalidate(
                position: canonicalPosition,
                event: .routeInvalidated
            )
            model.taskTreeRevealInteraction.invalidate(
                position: canonicalPosition,
                event: .routeInvalidated
            )

            model.drawerRevealInteraction.beginGesture(in: revealHost)
            model.drawerRevealInteraction.updateGesture(
                logicalTranslation: canonicalPosition == .closed ? 140 : -140,
                in: revealHost
            )
            model.taskTreeRevealInteraction.setTarget(
                canonicalPosition == .closed ? .open : .closed,
                animated: true
            )
            XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)
            XCTAssertTrue(model.taskTreeRevealInteraction.isSettling)

            model.handleScenePhase(.inactive)

            XCTAssertEqual(
                model.drawerRevealInteraction.presentation.phase,
                .idle,
                "drawer iteration \(iteration)"
            )
            XCTAssertEqual(
                model.taskTreeRevealInteraction.presentation.phase,
                .idle,
                "task tree iteration \(iteration)"
            )
            XCTAssertEqual(
                model.drawerRevealInteraction.presentation.target,
                canonicalPosition,
                "drawer iteration \(iteration)"
            )
            XCTAssertEqual(
                model.taskTreeRevealInteraction.presentation.target,
                canonicalPosition,
                "task tree iteration \(iteration)"
            )
            XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
            XCTAssertFalse(model.taskTreeRevealInteraction.diagnostics.hasTerminalResidue)
        }
    }

    func testRouteAndGatewayInvalidationCannotRetainRevealOwnership() {
        let model = routePreparationModel(session: .shared)
        let revealHost = attachGlobalRevealHost(to: model)
        model.drawerRevealInteraction.configure(extent: 330, restingPosition: .closed)
        model.taskTreeRevealInteraction.configure(extent: 300, restingPosition: .closed)
        model.drawerRevealInteraction.beginGesture(in: revealHost)
        model.drawerRevealInteraction.updateGesture(
            logicalTranslation: 140,
            in: revealHost
        )
        model.taskTreeRevealInteraction.setTarget(.open, animated: true)

        model.applyCanonicalRouteProjection([])

        model.assertGlobalRevealInteractionsHaveZeroResidue()
        XCTAssertEqual(model.drawerRevealInteraction.presentation.target, .closed)
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation.target, .closed)

        model.sidebarVisible = true
        model.isTaskTreeSidebarOpen = true
        model.drawerRevealInteraction.setTarget(.open, animated: true)
        model.taskTreeRevealInteraction.setTarget(.open, animated: true)
        model.resetGatewayRuntimeState()

        XCTAssertFalse(model.sidebarVisible)
        XCTAssertFalse(model.isTaskTreeSidebarOpen)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.target, .closed)
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation.target, .closed)
        model.assertGlobalRevealInteractionsHaveZeroResidue()
    }

    func testPresentationBarrierAcquisitionTerminatesEveryGlobalRevealInteraction() {
        let model = routePreparationModel(session: .shared)
        let revealHost = attachGlobalRevealHost(to: model)
        let store = model.productionRouteStore
        let container = makeContainer(path: [], store: store)
        store.attach(container)
        model.drawerRevealInteraction.configure(extent: 330, restingPosition: .closed)
        model.taskTreeRevealInteraction.configure(extent: 300, restingPosition: .open)
        model.drawerRevealInteraction.beginGesture(in: revealHost)
        model.drawerRevealInteraction.updateGesture(
            logicalTranslation: 140,
            in: revealHost
        )
        model.taskTreeRevealInteraction.setTarget(.closed, animated: true)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation.phase, .settling(.closed))

        XCTAssertTrue(store.presentationCoordinator.acquire(
            .init(rawValue: "reveal-barrier"),
            parent: nil,
            resultBearing: false
        ))

        XCTAssertTrue(store.hasPresentationBarrier)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .idle)
        XCTAssertEqual(model.taskTreeRevealInteraction.presentation.phase, .idle)
        model.assertGlobalRevealInteractionsHaveZeroResidue()
    }

    func testModalBarrierQueuesWholeChainUntilExactlyOnceReleaseThenHardSnaps() {
        let current = entry("current-modal", .panel("agents"))
        let store = GaryxProductionRouteStore()
        store.applyCanonicalPath([current])
        let container = makeContainer(path: [current], store: store)
        store.attach(container)
        let modal = GaryxPresentationLeaseSession()
        modal.acquireIfNeeded(
            coordinator: store.presentationCoordinator,
            parent: nil,
            operationContext: { nil }
        )

        _ = store.open(
            [.settingsDetail("manage"), .settingsDetail("provider")],
            source: .replace,
            animated: false
        )
        XCTAssertEqual(store.path, [current])
        XCTAssertEqual(container.path, [current])

        modal.completeDismissal()
        modal.completeDismissal()

        let expected: [GaryxRouteDestination] = [
            .settingsDetail("manage"),
            .settingsDetail("provider"),
        ]
        XCTAssertEqual(store.path.map(\.destination), expected)
        XCTAssertEqual(container.path.map(\.destination), expected)
        XCTAssertFalse(store.hasPresentationBarrier)
    }

    func testSkillFileDeepLinkPreparesEditorAndDocumentBeforeAdmission() async throws {
        let session = routePreparationSession { request in
            switch request.url?.path {
            case "/api/skills/skill-1/tree":
                return try self.routePreparationResponse(
                    request,
                    data: self.skillEditorFixture
                )
            case "/api/skills/skill-1/file":
                XCTAssertEqual(
                    URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?
                        .queryItems?.first(where: { $0.name == "path" })?.value,
                    "docs/guide.md"
                )
                return try self.routePreparationResponse(
                    request,
                    data: self.skillDocumentFixture
                )
            default:
                XCTFail("unexpected route preparation request: \(request.url?.absoluteString ?? "nil")")
                return try self.routePreparationResponse(request, statusCode: 404, data: Data())
            }
        }
        let model = routePreparationModel(session: session)
        model.connectionState = .ready(version: "test")

        await model.openMobileRoute(
            .skillFile(skillId: "skill-1", path: "docs/guide.md")
        )

        XCTAssertEqual(model.productionRouteStore.path.map(\.destination), [.panel("skills")])
        XCTAssertEqual(model.selectedSkillEditor?.skill.id, "skill-1")
        XCTAssertEqual(model.selectedSkillDocument?.path, "docs/guide.md")
        XCTAssertNil(model.routeNotFoundStore.selection)
    }

    func testMissingSkillFileDoesNotPartiallyAdmitItsPanelRoute() async throws {
        let session = routePreparationSession { request in
            switch request.url?.path {
            case "/api/skills/skill-1/tree":
                return try self.routePreparationResponse(
                    request,
                    data: self.skillEditorFixture
                )
            case "/api/skills/skill-1/file":
                return try self.routePreparationResponse(
                    request,
                    statusCode: 404,
                    data: Data("{\"error\":\"missing\"}".utf8)
                )
            default:
                XCTFail("unexpected route preparation request: \(request.url?.absoluteString ?? "nil")")
                return try self.routePreparationResponse(request, statusCode: 404, data: Data())
            }
        }
        let model = routePreparationModel(session: session)
        model.connectionState = .ready(version: "test")

        await model.openMobileRoute(
            .skillFile(skillId: "skill-1", path: "docs/missing.md")
        )

        XCTAssertTrue(model.productionRouteStore.path.isEmpty)
        XCTAssertNil(model.selectedSkillEditor)
        XCTAssertNil(model.selectedSkillDocument)
        XCTAssertEqual(model.routeNotFoundStore.selection?.title, "Skill File Not Found")
    }

    private func entry(
        _ id: String,
        _ destination: GaryxRouteDestination
    ) -> GaryxRouteEntry {
        GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: id),
            destination: destination
        )
    }

    private func makeContainer(
        path: [GaryxRouteEntry],
        store: GaryxProductionRouteStore
    ) -> GaryxRouteStackContainer {
        var callbacks = GaryxRouteStackContainerCallbacks()
        callbacks.phaseChanged = { [weak store] phase in
            store?.routePhaseChanged(phase)
        }
        callbacks.canonicalPathChanged = { [weak store] path in
            store?.applyCanonicalPath(path)
        }
        callbacks.visibleRouteActivated = { [weak store] node in
            store?.visibleRouteActivated(node)
        }
        callbacks.rendererBecameIdle = { [weak store] in
            store?.rendererBecameIdle()
        }
        return GaryxRouteStackContainer(
            initialPath: path,
            callbacks: callbacks,
            preferencesProvider: {
                .init(reduceMotion: false, prefersCrossFadeTransitions: false)
            },
            hostBuilder: { node in AnyView(Text(String(describing: node))) }
        )
    }

    private func routeStackContainers(
        in controller: UIViewController
    ) -> [GaryxRouteStackContainer] {
        let current = (controller as? GaryxRouteStackContainer).map { [$0] } ?? []
        return current + controller.children.flatMap { routeStackContainers(in: $0) }
    }

    private func publicationFailure(
        _ publications: [(phase: String, source: String, stack: [String])]
    ) -> String {
        guard let first = publications.first else { return "" }
        return """
        representable lifecycle synchronously published observable state. \
        Publications: \(publications.map { "\($0.phase):\($0.source)" })
        First publication stack:
        \(first.stack.joined(separator: "\n"))
        """
    }

    private func attachGlobalRevealHost(
        to model: GaryxMobileModel
    ) -> GaryxHorizontalRevealHostOccurrenceID {
        let rootOccurrenceID = GaryxRootSurfaceOccurrenceID(rawValue: 1)
        let hostOccurrenceID = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: rootOccurrenceID,
            rawValue: "route-intent-test-host"
        )
        model.applyGlobalRevealRootSurfaceTransition(
            .navigationShellBegan(rootOccurrenceID)
        )
        model.attachGlobalRevealHostOccurrence(hostOccurrenceID)
        return hostOccurrenceID
    }

    private func operationContext(_ operationID: String) -> GaryxPresentationOperationContext {
        let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)
        let entryID = GaryxComposerPayloadEntryID(rawValue: "entry")
        let token = GaryxPayloadLifecycleToken(
            entryID: entryID,
            nonce: "nonce"
        )
        let requestToken = GaryxGatewayRequestToken(scope: scope, activationSequence: 1)
        let capability = GaryxScopeBoundOperationContext(
            key: GaryxOperationCapabilityKey(
                scope: scope,
                entryID: entryID,
                generation: 1,
                reservationID: nil,
                branch: .followup,
                operationID: GaryxOperationID(rawValue: operationID)
            ),
            clientIdentity: scope.identity,
            configurationFingerprint: "1",
            payloadLifecycle: GaryxPayloadLifecycleCapture(token: token, revision: 0)
        )
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: URL(string: "https://gateway.example.test")!
            )
        )
        return GaryxPresentationOperationContext(
            capability: capability,
            requestToken: requestToken,
            gatewayClient: client
        )
    }

    private func routePreparationModel(session: URLSession) -> GaryxMobileModel {
        let suiteName = "GaryxProductionRouteIntentIntegrationTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(
            "http://gateway.example.test",
            forKey: GaryxMobileSettingsKeys.gatewayUrl
        )
        return GaryxMobileModel(
            defaults: defaults,
            gatewayClientFactory: { configuration in
                GaryxGatewayClient(
                    configuration: configuration,
                    session: session,
                    retryPolicy: .disabled
                )
            }
        )
    }

    private func makeRouteDismantleTestWindow() -> UIWindow {
        guard let scene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first
        else { preconditionFailure("hosted iOS tests require an active UIWindowScene") }
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 393, height: 852)
        return window
    }

    private func pumpRouteDismantleRunLoop(duration: TimeInterval = 0.05) {
        let deadline = Date().addingTimeInterval(duration)
        repeat {
            _ = RunLoop.main.run(mode: .default, before: deadline)
        } while Date() < deadline
    }

    private func routePreparationSession(
        handler: @escaping (URLRequest) throws -> (HTTPURLResponse, Data)
    ) -> URLSession {
        GaryxRoutePreparationURLProtocolStub.requestHandler = handler
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxRoutePreparationURLProtocolStub.self]
        return URLSession(configuration: configuration)
    }

    private func routePreparationResponse(
        _ request: URLRequest,
        statusCode: Int = 200,
        data: Data
    ) throws -> (HTTPURLResponse, Data) {
        let url = try XCTUnwrap(request.url)
        let response = try XCTUnwrap(
            HTTPURLResponse(
                url: url,
                statusCode: statusCode,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )
        )
        return (response, data)
    }

    private var skillEditorFixture: Data {
        Data("""
        {
          "skill": {
            "id": "skill-1",
            "name": "Test Skill",
            "installed": true,
            "enabled": true
          },
          "entries": [
            {
              "path": "docs/guide.md",
              "name": "guide.md",
              "entry_type": "file",
              "children": []
            }
          ]
        }
        """.utf8)
    }

    private var skillDocumentFixture: Data {
        Data("""
        {
          "skill": {
            "id": "skill-1",
            "name": "Test Skill",
            "installed": true,
            "enabled": true
          },
          "path": "docs/guide.md",
          "content": "Prepared content",
          "media_type": "text/markdown",
          "preview_kind": "text",
          "editable": true
        }
        """.utf8)
    }
}

private struct GaryxProductionRouteDismantleCrashReproRoot: View {
    let rootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID
    let store: GaryxProductionRouteStore
    let model: GaryxMobileModel
    @ObservedObject private var revealInteraction: GaryxHorizontalRevealInteractionStore

    init(
        rootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID,
        store: GaryxProductionRouteStore,
        model: GaryxMobileModel
    ) {
        self.rootSurfaceOccurrenceID = rootSurfaceOccurrenceID
        self.store = store
        self.model = model
        _revealInteraction = ObservedObject(wrappedValue: model.drawerRevealInteraction)
    }

    var body: some View {
        GaryxProductionRouteCanvas(
            rootSurfaceOccurrenceID: rootSurfaceOccurrenceID,
            store: store,
            model: model,
            homeContent: AnyView(Color.clear),
            routeContent: { _ in AnyView(Color.clear) },
            onOpenDrawer: {}
        )
        .offset(x: revealInteraction.presentation.reveal)
    }
}

private struct GaryxProductionRouteLifecycleReplacementProbeRoot: View {
    let canvasIdentity: Int
    let rootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID
    let store: GaryxProductionRouteStore
    let model: GaryxMobileModel

    var body: some View {
        GaryxProductionRouteCanvas(
            rootSurfaceOccurrenceID: rootSurfaceOccurrenceID,
            store: store,
            model: model,
            homeContent: AnyView(Color.clear),
            routeContent: { _ in AnyView(Color.clear) },
            onOpenDrawer: {}
        )
        .id(canvasIdentity)
    }
}

private final class GaryxRoutePreparationURLProtocolStub: URLProtocol {
    nonisolated(unsafe) static var requestHandler: (
        (URLRequest) throws -> (HTTPURLResponse, Data)
    )?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}
