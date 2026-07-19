import SwiftUI
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxProductionRouteIntentIntegrationTests: XCTestCase {
    override func tearDown() {
        GaryxRoutePreparationURLProtocolStub.requestHandler = nil
        super.tearDown()
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

    func testSceneInterruptionTerminatesEveryGlobalRevealInteraction() {
        let model = routePreparationModel(session: .shared)
        model.drawerRevealInteraction.configure(extent: 330, restingPosition: .closed)
        model.drawerRevealInteraction.beginGesture()
        model.drawerRevealInteraction.updateGesture(logicalTranslation: 140)
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

            model.drawerRevealInteraction.beginGesture()
            model.drawerRevealInteraction.updateGesture(
                logicalTranslation: canonicalPosition == .closed ? 140 : -140
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
        model.drawerRevealInteraction.configure(extent: 330, restingPosition: .closed)
        model.taskTreeRevealInteraction.configure(extent: 300, restingPosition: .closed)
        model.drawerRevealInteraction.beginGesture()
        model.drawerRevealInteraction.updateGesture(logicalTranslation: 140)
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
