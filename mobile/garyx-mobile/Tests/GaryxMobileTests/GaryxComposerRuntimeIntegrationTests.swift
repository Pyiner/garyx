import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxComposerRuntimeIntegrationTests: XCTestCase {
    func testRealUIKitHostUnmarksCJKBeforeCapturingExactFinalSequence() throws {
        let view = makeTextView()
        var ordered: [(String, GaryxComposerInputEventIdentity)] = []
        view.onOrderedText = { ordered.append(($0, $1)) }
        view.grantLive(configuration(key: .draft("cjk")))

        view.setMarkedText("你好", selectedRange: NSRange(location: 2, length: 0))
        view.observedTextDidChange()
        XCTAssertNotNil(view.markedTextRange)

        let close = view.finalizeInput()

        XCTAssertNil(view.markedTextRange)
        XCTAssertEqual(close.text, "你好")
        XCTAssertEqual(close.finalSequence, ordered.last?.1.inputSequence)
        XCTAssertEqual(close.finalSequence, UInt64(ordered.count))
        XCTAssertFalse(close.pendingProducers.contains(.markedText))
        XCTAssertFalse(view.isLive)
    }

    func testDictationPendingResultAndFailureBothReachOneTerminalBoundary() {
        let resultView = makeTextView()
        var resultTerminals: [GaryxInputProducerKind] = []
        var resultTexts: [String] = []
        resultView.onProducerTerminal = { resultTerminals.append($0) }
        resultView.onOrderedText = { text, _ in resultTexts.append(text) }
        resultView.grantLive(configuration(key: .draft("dictation-result")))
        resultView.beginDictationRecognitionForTesting()

        let resultClose = resultView.finalizeInput()
        XCTAssertEqual(resultClose.pendingProducers, [.dictation])
        resultView.acceptRecognizedDictationTextForTesting("dictated text")
        XCTAssertEqual(resultTexts.last, "dictated text")
        XCTAssertEqual(resultTerminals, [.dictation])

        let failureView = makeTextView()
        var failureTerminals: [GaryxInputProducerKind] = []
        var failureTexts: [String] = []
        failureView.onProducerTerminal = { failureTerminals.append($0) }
        failureView.onOrderedText = { text, _ in failureTexts.append(text) }
        failureView.grantLive(configuration(key: .draft("dictation-failure")))
        failureView.beginDictationRecognitionForTesting()

        let failureClose = failureView.finalizeInput()
        XCTAssertEqual(failureClose.pendingProducers, [.dictation])
        failureView.failDictationRecognitionForTesting()
        XCTAssertEqual(failureTerminals, [.dictation])
        let countAtTerminal = failureTexts.count
        failureView.acceptRecognizedDictationTextForTesting("too late")
        XCTAssertEqual(failureTexts.count, countAtTerminal)
    }

    func testPayloadTextAndAttachmentRestorePerKeyAndGatewayScope() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            quotaLimitBytes: 1_024 * 1_024
        )
        let g1 = GaryxGatewayScope(identity: "gateway-one", epoch: 1)
        let g2 = GaryxGatewayScope(identity: "gateway-two", epoch: 1)
        let keyA = GaryxComposerKey.draft("A")
        let keyB = GaryxComposerKey.draft("B")

        await coordinator.activate(scope: g1, key: keyA)
        try await persistText("alpha", in: coordinator)
        let sourceURL = directory.appendingPathComponent("attachment.txt")
        try Data("attachment".utf8).write(to: sourceURL)
        let staged = try await coordinator.stageAttachment(
            sourceURL: sourceURL,
            metadata: .init(
                kind: "file",
                name: "attachment.txt",
                mediaType: "text/plain",
                previewDataURL: nil
            ),
            requestToken: .init(scope: g1, activationSequence: 1)
        )
        try await coordinator.completeUpload(
            staged,
            uploaded: try uploadedAttachment(path: "/remote/attachment.txt")
        )
        XCTAssertEqual(coordinator.currentAttachments.count, 1)

        await coordinator.activate(scope: g1, key: keyB)
        XCTAssertEqual(coordinator.currentText, "")
        XCTAssertTrue(coordinator.currentAttachments.isEmpty)
        try await persistText("bravo", in: coordinator)

        await coordinator.activate(scope: g1, key: keyA)
        XCTAssertEqual(coordinator.currentText, "alpha")
        XCTAssertEqual(coordinator.currentAttachments.map(\.name), ["attachment.txt"])

        await coordinator.activate(scope: g2, key: keyA)
        XCTAssertEqual(coordinator.currentText, "")
        XCTAssertTrue(coordinator.currentAttachments.isEmpty)
        try await persistText("gateway two", in: coordinator)

        await coordinator.activate(scope: g1, key: keyA)
        XCTAssertEqual(coordinator.currentText, "alpha")
        XCTAssertEqual(coordinator.currentAttachments.count, 1)
        await coordinator.activate(scope: g2, key: keyA)
        XCTAssertEqual(coordinator.currentText, "gateway two")
    }

    func testPresentationFrozenOperationContextPersistsToItsOriginAfterRouteSwap() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            quotaLimitBytes: 1_024 * 1_024
        )
        let scope = GaryxGatewayScope(identity: "route-swap-gateway", epoch: 1)
        let request = GaryxGatewayRequestToken(scope: scope, activationSequence: 7)
        let origin = GaryxComposerKey.draft("origin")
        let replacementRoute = GaryxComposerKey.draft("replacement-route")
        await coordinator.activate(scope: scope, key: origin)
        let frozen = try XCTUnwrap(
            coordinator.makePresentationOperationContext(requestToken: request)
        )
        let sourceURL = directory.appendingPathComponent("route-swap.txt")
        try Data("route-owned".utf8).write(to: sourceURL)

        // The picker lease was accepted on origin, then a queued whole-chain
        // route replacement became visible before asynchronous staging ran.
        await coordinator.activate(scope: scope, key: replacementRoute)
        let staged = try await coordinator.stageAttachment(
            sourceURL: sourceURL,
            metadata: .init(
                kind: "file",
                name: "route-swap.txt",
                mediaType: "text/plain",
                previewDataURL: nil
            ),
            requestToken: request,
            operationContext: frozen
        )

        XCTAssertEqual(staged.operationKey, frozen.key)
        XCTAssertEqual(staged.operationKey.entryID, frozen.key.entryID)
        XCTAssertTrue(coordinator.currentAttachments.isEmpty)

        await coordinator.activate(scope: scope, key: origin)
        XCTAssertEqual(coordinator.currentAttachments.map(\.name), ["route-swap.txt"])
        XCTAssertEqual(coordinator.currentAttachments.first?.id, staged.attachmentID)
    }

    func testRemainingRoutesPreserveActualRetryableOwnerForSwapPlanner() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            quotaLimitBytes: 1_024 * 1_024
        )
        let scope = GaryxGatewayScope(identity: "route-planner-gateway", epoch: 1)
        let request = GaryxGatewayRequestToken(scope: scope, activationSequence: 1)
        let originKey = GaryxComposerKey.draft("route-planner-origin")
        await coordinator.activate(scope: scope, key: originKey)
        let frozen = try XCTUnwrap(
            coordinator.makePresentationOperationContext(requestToken: request)
        )
        let sourceURL = directory.appendingPathComponent("retryable-route-owner.txt")
        try Data("retryable owner".utf8).write(to: sourceURL)
        let staged = try await coordinator.stageAttachment(
            sourceURL: sourceURL,
            metadata: .init(
                kind: "file",
                name: "retryable-route-owner.txt",
                mediaType: "text/plain",
                previewDataURL: nil
            ),
            requestToken: request,
            operationContext: frozen
        )
        await coordinator.failUpload(staged)

        let databaseURL = directory
            .appendingPathComponent("Garyx", isDirectory: true)
            .appendingPathComponent("ComposerPayload", isDirectory: true)
            .appendingPathComponent("composer.sqlite", isDirectory: false)
        let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: databaseURL)
        let actualRetryable = try await durability.load()
        let old = try XCTUnwrap(actualRetryable.operations[staged.operationKey])
        let feedback = try XCTUnwrap(
            actualRetryable.feedback.values.first(where: {
                $0.operationID == staged.operationKey.operationID
            })
        )
        let assetID = try XCTUnwrap(old.stagedAssetID)
        XCTAssertEqual(old.state, .failedRetryable)
        XCTAssertEqual(actualRetryable.manifests[old.context.key]?.state, .failedRetryable)
        XCTAssertEqual(actualRetryable.stagedAssetOwners[assetID], old.context.key)
        XCTAssertEqual(actualRetryable.stagedAssetReservedBytes[assetID], old.reservedBytes)

        let routeStore = GaryxProductionRouteStore()
        let originEntry = entry(
            "route-planner-origin-occurrence",
            destination: .conversationDraft(draftID: "route-planner-origin")
        )
        routeStore.applyCanonicalPath([originEntry])
        var callbacks = GaryxRouteStackContainerCallbacks()
        callbacks.phaseChanged = { [weak routeStore] phase in
            routeStore?.routePhaseChanged(phase)
        }
        callbacks.canonicalPathChanged = { [weak routeStore] path in
            routeStore?.applyCanonicalPath(path)
        }
        callbacks.visibleRouteActivated = { [weak routeStore] node in
            routeStore?.visibleRouteActivated(node)
        }
        callbacks.rendererBecameIdle = { [weak routeStore] in
            routeStore?.rendererBecameIdle()
        }
        let container = GaryxRouteStackContainer(
            initialPath: [originEntry],
            callbacks: callbacks,
            preferencesProvider: {
                .init(reduceMotion: false, prefersCrossFadeTransitions: false)
            },
            hostBuilder: { node in AnyView(Text(String(describing: node))) }
        )
        container.loadViewIfNeeded()
        routeStore.attach(container)

        let remainingRoutes = GaryxMobileSettingsTab.allCases.map(GaryxMobileRoute.settings)
            + GaryxMobilePanel.allCases
                .filter { $0 != .chat }
                .map(GaryxMobileRoute.panel)
            + [
                .automation("automation-1"),
                .automationThreads("automation-1"),
                .capsule("capsule-1"),
                .agent("agent-1"),
                .skill("skill-1"),
                .skillFile(skillId: "skill-1", path: "SKILL.md"),
                .workspace("/workspace/test"),
                .bot(channel: "api", accountId: "1000000001"),
                .workspaceFile(workspaceDir: "/workspace/test", path: "README.md"),
            ]
        for route in remainingRoutes {
            let destinations = GaryxMobileRoutePlan.destinations(
                for: route,
                draftID: "unused-draft",
                resolvedBotGroupID: "configured-group"
            )
            _ = routeStore.open(destinations, source: .deepLink, animated: false)
            XCTAssertEqual(
                container.path.map(\.destination),
                destinations,
                "wrong visible occurrence chain for \(route)"
            )
            XCTAssertFalse(container.hasTerminalResidue)

            var expected = destinations
            while !expected.isEmpty {
                expected.removeLast()
                routeStore.popOne(animated: false)
                XCTAssertEqual(
                    container.path.map(\.destination),
                    expected,
                    "pop did not expose the predecessor for \(route)"
                )
            }
        }

        let afterRoutes = try await durability.load()
        XCTAssertEqual(afterRoutes.operations[old.context.key], old)
        XCTAssertEqual(afterRoutes.manifests[old.context.key], actualRetryable.manifests[old.context.key])
        XCTAssertEqual(afterRoutes.stagedAssetOwners[assetID], old.context.key)
        XCTAssertEqual(afterRoutes.stagedAssetReservedBytes[assetID], old.reservedBytes)

        let successorContext = old.context.replacingOperationID(
            GaryxOperationID(rawValue: "route-planner-successor")
        )
        let successor = GaryxOperationCapability(context: successorContext)
        let replacement = GaryxReplacementRecord(
            id: GaryxReplacementID(rawValue: "route-planner-replacement"),
            scope: scope,
            entryID: old.context.key.entryID,
            oldKey: old.context.key,
            reservationID: old.context.key.reservationID,
            branch: old.context.key.branch,
            stagedAssetID: assetID,
            reservedBytes: old.reservedBytes
        )
        let withReplacement = try await durability.commit(
            .init(
                expectedRevision: afterRoutes.revision,
                label: "admit route-side retry replacement",
                mutations: [.upsertReplacement(replacement)]
            )
        )
        let plan = try XCTUnwrap(
            GaryxReplacementFeedbackSwapPlanner.plan(
                snapshot: withReplacement,
                successor: successor,
                replacementID: replacement.id,
                feedbackID: feedback.id,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            )
        )
        let committed = try await durability.commit(plan.transaction)

        XCTAssertEqual(committed.stagedAssetOwners, [assetID: successorContext.key])
        XCTAssertEqual(committed.stagedAssetReservedBytes[assetID], old.reservedBytes)
        XCTAssertNil(committed.manifests[old.context.key])
        XCTAssertEqual(committed.manifests[successorContext.key], plan.successorManifest)
        XCTAssertNil(committed.operations[old.context.key]?.stagedAssetID)
        XCTAssertEqual(committed.operations[old.context.key]?.reservedBytes, 0)
        XCTAssertEqual(committed.feedback[feedback.id]?.phase, .acknowledged)
        XCTAssertEqual(committed.replacements[replacement.id]?.phase, .committed)
    }

    func testFeedbackRetryAndRemoveOwnTheirAcknowledgementsAndAttachmentAtomically() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            quotaLimitBytes: 1_024 * 1_024
        )
        let scope = GaryxGatewayScope(identity: "feedback-action-gateway", epoch: 1)
        let request = GaryxGatewayRequestToken(scope: scope, activationSequence: 1)
        await coordinator.activate(scope: scope, key: .draft("feedback-action"))
        let sourceURL = directory.appendingPathComponent("feedback-action.txt")
        try Data("retry then remove".utf8).write(to: sourceURL)
        let staged = try await coordinator.stageAttachment(
            sourceURL: sourceURL,
            metadata: .init(
                kind: "file",
                name: "feedback-action.txt",
                mediaType: "text/plain",
                previewDataURL: nil
            ),
            requestToken: request
        )
        await coordinator.failUpload(staged)

        let databaseURL = directory
            .appendingPathComponent("Garyx", isDirectory: true)
            .appendingPathComponent("ComposerPayload", isDirectory: true)
            .appendingPathComponent("composer.sqlite", isDirectory: false)
        let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: databaseURL)
        var snapshot = try await durability.load()
        let firstFeedback = try XCTUnwrap(snapshot.feedback.values.first(where: {
            $0.operationID == staged.operationKey.operationID
        }))

        let retry = try await coordinator.retryUpload(firstFeedback.id)
        snapshot = try await durability.load()
        XCTAssertNotEqual(retry.operationKey, staged.operationKey)
        XCTAssertEqual(snapshot.operations[staged.operationKey]?.state, .superseded)
        XCTAssertEqual(snapshot.operations[retry.operationKey]?.state, .uploading)
        XCTAssertEqual(snapshot.operations[retry.operationKey]?.uploadAttempted, true)
        XCTAssertEqual(snapshot.feedback[firstFeedback.id]?.phase, .acknowledged)
        XCTAssertEqual(
            snapshot.stagedAssetOwners.values.first,
            retry.operationKey,
            "retry must transfer the single physical-file owner"
        )

        await coordinator.failUpload(retry)
        snapshot = try await durability.load()
        let retryFeedback = try XCTUnwrap(snapshot.feedback.values.first(where: {
            $0.operationID == retry.operationKey.operationID && !$0.isTerminal
        }))
        try await coordinator.removeFailedUpload(retryFeedback.id)
        snapshot = try await durability.load()

        XCTAssertNil(snapshot.operations[retry.operationKey])
        XCTAssertNil(snapshot.manifests[retry.operationKey])
        XCTAssertEqual(snapshot.feedback[retryFeedback.id]?.phase, .acknowledged)
        XCTAssertEqual(snapshot.reservedBytes, 0)
        XCTAssertTrue(
            snapshot.payloadStore.entry(retry.operationKey.entryID, scope: scope)?
                .attachments.isEmpty == true
        )
        XCTAssertFalse(FileManager.default.fileExists(atPath: retry.fileURL.path))
    }

    func testPresentationContextKeepsUploadOnOriginGatewayAfterScopeSwitch() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            quotaLimitBytes: 1_024 * 1_024
        )
        let requestSeen = expectation(description: "origin gateway received upload")
        let capturedHost = GaryxLockedStringBox()
        GaryxComposerDeliveryURLProtocolStub.requestHandler = { request in
            capturedHost.set(request.url?.host)
            requestSeen.fulfill()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: request.url ?? URL(string: "http://gateway-one.example.test")!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {"files":[{"kind":"image","path":"/remote/origin.jpg","name":"origin.jpg","mediaType":"image/jpeg"}]}
                    """.utf8
                )
            )
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxComposerDeliveryURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxComposerDeliveryURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let suiteName = "GaryxPresentationOriginGateway-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.set(
            "http://gateway-one.example.test",
            forKey: GaryxMobileSettingsKeys.gatewayUrl
        )
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let model = GaryxMobileModel(
            defaults: defaults,
            gatewayClientFactory: { gatewayConfiguration in
                GaryxGatewayClient(
                    configuration: gatewayConfiguration,
                    session: session,
                    retryPolicy: .disabled
                )
            },
            composerPayloadCoordinator: coordinator
        )
        try await waitUntil { coordinator.inputConfiguration() != nil }
        let originScope = model.gatewayRequestToken.scope
        let originKey = GaryxComposerKey.draft("presentation-origin")
        await coordinator.activate(scope: originScope, key: originKey)
        let frozen = try XCTUnwrap(
            model.makeComposerPresentationOperationContext(payload: coordinator)
        )

        model.exitCurrentGatewayScope(.suspend)
        model.gatewayURL = "http://gateway-two.example.test"
        model.activateCurrentGatewayScope()
        await model.attachImages(
            [
                GaryxMobileSelectedImage(
                    name: "origin.jpg",
                    mediaType: "image/jpeg",
                    data: Data([0xFF, 0xD8, 0xFF, 0xD9])
                )
            ],
            operationContext: frozen
        )

        await fulfillment(of: [requestSeen], timeout: 2)
        XCTAssertEqual(capturedHost.get(), "gateway-one.example.test")
        XCTAssertNil(model.lastError)

        await coordinator.activate(scope: originScope, key: originKey)
        XCTAssertEqual(coordinator.currentAttachments.map(\.name), ["origin.jpg"])
    }

    func testPendingUploadHoldsPayloadPreparingSendLockWithoutAdvancingText() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            quotaLimitBytes: 1_024 * 1_024
        )
        let scope = GaryxGatewayScope(identity: "preparing-gateway", epoch: 1)
        await coordinator.activate(scope: scope, key: .draft("preparing"))
        try await persistText("still visible", in: coordinator)
        let sourceURL = directory.appendingPathComponent("pending.bin")
        try Data("pending".utf8).write(to: sourceURL)
        _ = try await coordinator.stageAttachment(
            sourceURL: sourceURL,
            metadata: .init(
                kind: "file",
                name: "pending.bin",
                mediaType: "application/octet-stream",
                previewDataURL: nil
            ),
            requestToken: .init(scope: scope, activationSequence: 1)
        )

        do {
            _ = try await coordinator.takeReadyPayload(clientIntentID: "blocked-send")
            XCTFail("pending upload must lock send")
        } catch GaryxComposerPayloadRuntimeError.payloadPreparing {
            XCTAssertEqual(coordinator.currentText, "still visible")
        }
    }

    func testRegisteringNewCanonicalOccurrenceRevokesPreviousLiveAdapterFirst() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        await coordinator.activate(
            scope: .init(identity: "activation-gateway", epoch: 1),
            key: .draft("shared")
        )
        let first = GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: "first-occurrence"),
            composerKey: .draft("shared")
        )
        let second = GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: "second-occurrence"),
            composerKey: .draft("shared")
        )

        coordinator.register(first, isCanonicalTop: true)
        XCTAssertTrue(first.isLive)
        coordinator.register(second, isCanonicalTop: true)

        XCTAssertFalse(first.isLive)
        XCTAssertTrue(second.isLive)
        XCTAssertEqual([first, second].filter(\.isLive).count, 1)
    }

    func testPromotionAndConcurrentOrderedInputKeepStablePayloadText() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        let scope = GaryxGatewayScope(identity: "promotion-gateway", epoch: 1)
        await coordinator.activate(scope: scope, key: .draft("draft"))
        let configuration = try XCTUnwrap(coordinator.inputConfiguration())

        let promotion = Task {
            try await coordinator.promoteActive(to: .thread("thread"))
        }
        coordinator.acceptText(
            "typed while promoting",
            identity: .init(
                composerKey: configuration.composerKey,
                sessionID: configuration.sessionID,
                inputSessionEpoch: configuration.epoch,
                payloadGeneration: configuration.payloadGeneration,
                reservationID: nil,
                inputSequence: 1
            )
        )
        try await promotion.value
        try await waitUntil {
            coordinator.currentText == "typed while promoting"
                && coordinator.activeKey == .thread("thread")
        }
        XCTAssertEqual(coordinator.currentText, "typed while promoting")
        XCTAssertEqual(coordinator.activeKey, .thread("thread"))
        let promotedConfiguration = try XCTUnwrap(coordinator.inputConfiguration())
        XCTAssertEqual(promotedConfiguration.composerKey, .thread("thread"))
        XCTAssertEqual(promotedConfiguration.sessionID, configuration.sessionID)
        XCTAssertEqual(promotedConfiguration.epoch, configuration.epoch)

        // Reproduce the production seam that used to wedge when promotion had
        // unmounted the adapter: a committed pop must virtually release the
        // still-valid reducer and permit the next activation.
        coordinator.routeCommitReleased(
            sourceOccurrenceID: .init(rawValue: "promoted-source"),
            sourceKey: .thread("thread"),
            destinationOccurrenceID: nil,
            destinationKey: nil
        )
        coordinator.routeReachedTerminal(.init(outcome: .committed, visibility: .visible))
        try await waitUntil { coordinator.inputConfiguration() == nil }
        await coordinator.activate(scope: scope, key: .draft("next"))
        try await waitUntil { coordinator.activeKey == .draft("next") }
    }

    func testAdapterRebuildWithinOneSessionResumesOrderedSequenceSpace() throws {
        let firstView = makeTextView()
        var events: [GaryxComposerInputEventIdentity] = []
        firstView.onOrderedText = { _, identity in events.append(identity) }
        let draft = configuration(key: .draft("promoted-sequence"))
        firstView.grantLive(draft)
        firstView.replaceLiveText("one")

        let rebuiltView = makeTextView()
        rebuiltView.onOrderedText = { _, identity in events.append(identity) }
        rebuiltView.grantLive(
            .init(
                composerKey: .thread("promoted-sequence"),
                sessionID: draft.sessionID,
                epoch: draft.epoch,
                payloadGeneration: draft.payloadGeneration,
                reservationID: nil,
                nextInputSequence: 2,
                initialText: "one",
                isReadOnly: false
            )
        )
        rebuiltView.replaceLiveText("two")

        let close = rebuiltView.finalizeInput()
        XCTAssertEqual(events.map(\.inputSequence), [1, 2, 3])
        XCTAssertEqual(close.finalSequence, 3)
    }

    func testRelaunchReleasesDeadPromotionAliasBeforeReclaim() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let scope = GaryxGatewayScope(identity: "alias-relaunch-gateway", epoch: 1)

        let firstProcess = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        await firstProcess.activate(scope: scope, key: .draft("alias-relaunch"))
        try await firstProcess.promoteActive(to: .thread("alias-relaunch"))
        XCTAssertEqual(firstProcess.activeKey, .thread("alias-relaunch"))

        let relaunched = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        await relaunched.activate(scope: scope, key: .thread("alias-relaunch"))
        try await relaunched.discard(key: .thread("alias-relaunch"))

        XCTAssertNil(relaunched.activeKey)
    }

    func testCommittedTerminalCancelsPendingDictationWithoutExternalCallback() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        let scope = GaryxGatewayScope(identity: "terminal-cancel-gateway", epoch: 1)
        let key = GaryxComposerKey.draft("terminal-cancel")
        await coordinator.activate(scope: scope, key: key)
        let source = GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: "terminal-cancel-source"),
            composerKey: key
        )
        let destination = GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: "terminal-cancel-destination"),
            composerKey: key
        )
        source.onOrderedText = coordinator.acceptText
        source.onProducerTerminal = { producer in
            coordinator.producerReachedTerminal(producer, occurrenceID: source.occurrenceID)
        }
        coordinator.register(source, isCanonicalTop: true)
        coordinator.register(destination, isCanonicalTop: false)
        source.beginDictationRecognitionForTesting()

        coordinator.routeCommitReleased(
            sourceOccurrenceID: source.occurrenceID,
            sourceKey: key,
            destinationOccurrenceID: destination.occurrenceID,
            destinationKey: key
        )
        coordinator.routeReachedTerminal(.init(outcome: .committed, visibility: .visible))

        try await waitUntil { destination.isLive }
        XCTAssertNil(coordinator.finalizationFailureDescription)
    }

    func testTransientFinalizationFailureRetriesWithoutAnotherLifecycleEvent() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            testingHooks: .init(finalizationFailuresBeforeSuccess: 1)
        )
        let scope = GaryxGatewayScope(identity: "finalization-retry-gateway", epoch: 1)
        let key = GaryxComposerKey.draft("finalization-retry")
        await coordinator.activate(scope: scope, key: key)
        let source = GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: "finalization-retry-source"),
            composerKey: key
        )
        let destination = GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: "finalization-retry-destination"),
            composerKey: key
        )
        source.onOrderedText = coordinator.acceptText
        source.onProducerTerminal = { producer in
            coordinator.producerReachedTerminal(producer, occurrenceID: source.occurrenceID)
        }
        coordinator.register(source, isCanonicalTop: true)
        coordinator.register(destination, isCanonicalTop: false)

        coordinator.routeCommitReleased(
            sourceOccurrenceID: source.occurrenceID,
            sourceKey: key,
            destinationOccurrenceID: destination.occurrenceID,
            destinationKey: key
        )
        coordinator.routeReachedTerminal(.init(outcome: .committed, visibility: .visible))

        try await waitUntil { destination.isLive }
        XCTAssertNil(coordinator.finalizationFailureDescription)
    }

    func testSendSealUsesLatestReducerStateAcrossPrepareSuspension() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let gate = ComposerAsyncGate()
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory,
            testingHooks: .init(beforePrepareSendReturns: { await gate.suspend() })
        )
        let scope = GaryxGatewayScope(identity: "send-race-gateway", epoch: 1)
        await coordinator.activate(scope: scope, key: .draft("send-race"))
        try await persistText("before suspension", in: coordinator)

        let send = Task {
            try await coordinator.takeReadyPayload(clientIntentID: "send-race-intent")
        }
        await gate.waitUntilSuspended()
        let configuration = try XCTUnwrap(coordinator.inputConfiguration())
        coordinator.acceptText(
            "typed during suspension",
            identity: .init(
                composerKey: configuration.composerKey,
                sessionID: configuration.sessionID,
                inputSessionEpoch: configuration.epoch,
                payloadGeneration: configuration.payloadGeneration,
                reservationID: nil,
                inputSequence: 2
            )
        )
        await gate.resume()

        let payload = try await send.value
        XCTAssertEqual(payload.text, "typed during suspension")
        try await coordinator.markTransportAttempted(payload.delivery)
        try await coordinator.acknowledgeDelivery(payload.delivery)
    }

    func testAcknowledgedDeliverySettlementDoesNotBrickSixtyFifthSend() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        let scope = GaryxGatewayScope(identity: "delivery-quota-gateway", epoch: 1)
        await coordinator.activate(scope: scope, key: .draft("delivery-quota"))

        for sequence in 1...65 {
            let configuration = try XCTUnwrap(coordinator.inputConfiguration())
            coordinator.acceptText(
                "message \(sequence)",
                identity: .init(
                    composerKey: configuration.composerKey,
                    sessionID: configuration.sessionID,
                    inputSessionEpoch: configuration.epoch,
                    payloadGeneration: configuration.payloadGeneration,
                    reservationID: configuration.reservationID,
                    inputSequence: UInt64(sequence)
                )
            )
            let payload = try await coordinator.takeReadyPayload(
                clientIntentID: "delivery-intent-\(sequence)"
            )
            XCTAssertEqual(payload.text, "message \(sequence)")
            try await coordinator.markTransportAttempted(payload.delivery)
            try await coordinator.acknowledgeDelivery(payload.delivery)
            let phase = try await coordinator.deliveryPhase(for: payload.delivery)
            XCTAssertEqual(phase, .acknowledged)
        }
    }

    func testProductionGatewayDispatchMarksAttemptBeforeRequestThenAcknowledges() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let requestStarted = expectation(description: "chat request reached URL loading")
        let responseGate = DispatchSemaphore(value: 0)
        GaryxComposerDeliveryURLProtocolStub.requestHandler = { request in
            guard request.url?.path == "/api/chat/start" else {
                throw URLError(.badURL)
            }
            requestStarted.fulfill()
            guard responseGate.wait(timeout: .now() + 5) == .success else {
                throw URLError(.timedOut)
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: request.url ?? URL(string: "http://gateway.example.test")!,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    #"{"status":"accepted","run_id":"delivery-run","thread_id":"delivery-thread"}"#.utf8
                )
            )
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxComposerDeliveryURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            responseGate.signal()
            GaryxComposerDeliveryURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        let suiteName = "GaryxComposerDeliveryRuntime-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.set(
            "http://gateway.example.test",
            forKey: GaryxMobileSettingsKeys.gatewayUrl
        )
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let model = GaryxMobileModel(
            defaults: defaults,
            gatewayClientFactory: { gatewayConfiguration in
                GaryxGatewayClient(
                    configuration: gatewayConfiguration,
                    session: session,
                    retryPolicy: .disabled
                )
            },
            composerPayloadCoordinator: coordinator
        )
        let scope = model.gatewayRequestToken.scope
        // Model initialization schedules the gateway scope's default draft
        // activation. Let that ticket settle before selecting this test's
        // thread entry so it cannot supersede the explicit activation.
        try await waitUntil { coordinator.inputConfiguration() != nil }
        await coordinator.activate(scope: scope, key: .thread("delivery-thread"))
        try await waitUntil { coordinator.activeKey == .thread("delivery-thread") }
        try await persistText("delivery body", in: coordinator)
        let payload = try await coordinator.takeReadyPayload(
            clientIntentID: "delivery-model-intent"
        )
        let createKey = try coordinator.makeCreateDeliveryKey(
            createIntentID: "delivery-model-intent"
        )
        try await coordinator.beginCreateDelivery(createKey)
        try await coordinator.recordCreatedThread("delivery-thread", for: createKey)
        XCTAssertTrue(
            model.runTracker.beginLocalDispatch(
                threadId: "delivery-thread",
                intentId: "delivery-model-intent",
                text: payload.text
            )
        )

        let dispatch = Task { @MainActor in
            try await model.startChatRunViaGateway(
                threadId: "delivery-thread",
                message: payload.text,
                attachments: [],
                clientIntentId: "delivery-model-intent",
                workspacePath: nil,
                assistantMessageId: "delivery-assistant",
                delivery: payload.delivery,
                createDeliveryKey: createKey
            )
        }
        await fulfillment(of: [requestStarted], timeout: 2)
        let attemptedPhase = try await coordinator.deliveryPhase(for: payload.delivery)
        let attemptedCreatePhase = await coordinator.createDeliveryPhase(for: createKey)
        XCTAssertEqual(attemptedPhase, .transportAttempted)
        XCTAssertEqual(
            attemptedCreatePhase,
            .chatStartAttempted,
            "the message and multi-stage create attempt cross one durable boundary"
        )

        responseGate.signal()
        try await dispatch.value
        let acknowledgedPhase = try await coordinator.deliveryPhase(for: payload.delivery)
        let acknowledgedCreatePhase = await coordinator.createDeliveryPhase(for: createKey)
        XCTAssertEqual(acknowledgedPhase, .acknowledged)
        XCTAssertEqual(acknowledgedCreatePhase, .acknowledged)
    }

    func testRapidOrderedUIKitInputNeverRegressesToAnOlderDurableCompletion() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        await coordinator.activate(
            scope: .init(identity: "rapid-input-gateway", epoch: 1),
            key: .draft("rapid-input")
        )
        let configuration = try XCTUnwrap(coordinator.inputConfiguration())
        let text = "route focus"
        for sequence in 1...text.count {
            let end = text.index(text.startIndex, offsetBy: sequence)
            coordinator.acceptText(
                String(text[..<end]),
                identity: .init(
                    composerKey: configuration.composerKey,
                    sessionID: configuration.sessionID,
                    inputSessionEpoch: configuration.epoch,
                    payloadGeneration: configuration.payloadGeneration,
                    reservationID: nil,
                    inputSequence: UInt64(sequence)
                )
            )
        }

        try await waitUntil { coordinator.currentText == text }
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertEqual(coordinator.currentText, text)
    }

    func testHostActivationWaitsForDurableCloseAndEveryCancelEventDrains() async throws {
        for reason in GaryxInputProducerCancellation.allCases {
            let directory = try temporaryDirectory()
            defer { try? FileManager.default.removeItem(at: directory) }
            let coordinator = try GaryxComposerPayloadCoordinator(
                applicationSupportDirectory: directory
            )
            let key = GaryxComposerKey.draft("cancel-\(reason.rawValue)")
            await coordinator.activate(
                scope: .init(identity: "cancel-gateway-\(reason.rawValue)", epoch: 1),
                key: key
            )
            let source = GaryxComposerOrderedTextView(
                occurrenceID: .init(rawValue: "source-\(reason.rawValue)"),
                composerKey: key
            )
            let destination = GaryxComposerOrderedTextView(
                occurrenceID: .init(rawValue: "destination-\(reason.rawValue)"),
                composerKey: key
            )
            source.onOrderedText = coordinator.acceptText
            source.onProducerTerminal = { producer in
                coordinator.producerReachedTerminal(
                    producer,
                    occurrenceID: source.occurrenceID
                )
            }
            coordinator.register(source, isCanonicalTop: true)
            coordinator.register(destination, isCanonicalTop: false)
            source.replaceLiveText("survives \(reason.rawValue)")
            source.beginDictationRecognitionForTesting()

            coordinator.routeCommitReleased(
                sourceOccurrenceID: source.occurrenceID,
                sourceKey: key,
                destinationOccurrenceID: destination.occurrenceID,
                destinationKey: key
            )
            XCTAssertFalse(source.isLive)
            XCTAssertFalse(destination.isLive)
            coordinator.cancelPendingInput(reason)
            coordinator.routeReachedTerminal(.init(outcome: .committed, visibility: .visible))

            try await waitUntil {
                destination.isLive
                    && coordinator.currentText == "survives \(reason.rawValue)"
            }
            XCTAssertNil(
                coordinator.finalizationFailureDescription,
                "\(reason.rawValue): \(coordinator.finalizationFailureDescription ?? "")"
            )
            XCTAssertFalse(source.isLive)
            XCTAssertEqual([source, destination].filter(\.isLive).count, 1)
        }
    }

    func testProductionRouteStorePreservesOccurrencesAndFocusesExistingDraft() {
        let store = GaryxProductionRouteStore()
        let firstA = store.open(.conversation(threadID: "A"), source: .current, animated: false)
        _ = store.open(.panel("agents"), source: .current, animated: false)
        let secondA = store.open(.conversation(threadID: "A"), source: .current, animated: false)
        XCTAssertNotEqual(firstA.id, secondA.id)
        XCTAssertEqual(store.path.map(\.destination), [
            .conversation(threadID: "A"),
            .panel("agents"),
            .conversation(threadID: "A"),
        ])

        let draft = store.open(.conversationDraft(draftID: "D"), source: .current, animated: false)
        _ = store.open(.panel("skills"), source: .current, animated: false)
        let focused = store.open(.conversationDraft(draftID: "D"), source: .current, animated: false)
        XCTAssertEqual(focused.id, draft.id)
        XCTAssertEqual(store.path.last?.id, draft.id)
        XCTAssertEqual(store.path.filter { $0.destination == .conversationDraft(draftID: "D") }.count, 1)
    }

    func testConversationLiveStoreIsRebuiltFromPromotedRouteValue() {
        var entry = GaryxRouteEntry(
            id: .init(rawValue: "promoted-live-store"),
            destination: .conversationDraft(draftID: "draft-live-store")
        )
        XCTAssertNil(GaryxConversationLiveStore(destination: entry.destination).threadID)

        entry.replacePayload(with: .conversation(threadID: "thread-live-store"))

        let promoted = GaryxConversationLiveStore(destination: entry.destination)
        XCTAssertEqual(promoted.threadID, "thread-live-store")
        XCTAssertEqual(promoted.routeIdentity, "thread:thread-live-store")
    }

    func testTypedDrilldownAndWorkspaceFileRoutesPreserveTheirCanonicalBackTargets() throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let coordinator = try GaryxComposerPayloadCoordinator(
            applicationSupportDirectory: directory
        )
        let suiteName = "GaryxNavigationRuntime-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let model = GaryxMobileModel(
            defaults: defaults,
            composerPayloadCoordinator: coordinator
        )

        model.openPanel(.automations, source: .replace)
        model.openWorkspaceBotsDrilldown(.automationThreads("automation-a"), source: .current)
        XCTAssertEqual(model.productionRouteStore.path.map(\.destination), [
            .panel(GaryxMobilePanel.automations.rawValue),
            .workspaceDrilldown(.automationThreads(automationID: "automation-a")),
        ])
        XCTAssertEqual(model.workspaceBotsDrilldown, .automationThreads("automation-a"))

        _ = model.productionRouteStore.open(
            .conversation(threadID: "thread-a"),
            source: .current,
            animated: false
        )
        model.applyCanonicalRouteProjection(model.productionRouteStore.path)
        model.productionRouteStore.popOne(animated: false)
        model.applyCanonicalRouteProjection(model.productionRouteStore.path)
        XCTAssertEqual(model.activePanel, .workspaceBots)
        XCTAssertEqual(model.workspaceBotsDrilldown, .automationThreads("automation-a"))

        model.openWorkspaceFilesPanel(source: .replace)
        XCTAssertEqual(
            model.productionRouteStore.path.last?.destination,
            .panel(GaryxMobilePanel.workspaces.rawValue)
        )
        XCTAssertEqual(model.activePanel, .workspaces)

        model.openSettings(tab: .gateway, source: .replace)
        XCTAssertEqual(model.productionRouteStore.path.map(\.destination), [
            .settingsDetail(GaryxMobileSettingsTab.manage.rawValue),
            .settingsDetail(GaryxMobileSettingsTab.gateway.rawValue),
        ])
        model.dismissCurrentRoute()
        model.applyCanonicalRouteProjection(model.productionRouteStore.path)
        XCTAssertEqual(model.activeSettingsTab, .manage)
    }

    func testHardSnapUsesReleaseCanonicalTerminalOrder() {
        let first = entry("first", destination: .conversation(threadID: "A"))
        let second = entry("second", destination: .conversation(threadID: "B"))
        var events: [String] = []
        var callbacks = GaryxRouteStackContainerCallbacks()
        callbacks.commitReleased = { _, _ in events.append("release") }
        callbacks.canonicalPathChanged = { _ in events.append("canonical") }
        callbacks.terminalReached = { _ in events.append("terminal") }
        let container = GaryxRouteStackContainer(
            initialPath: [first],
            callbacks: callbacks,
            preferencesProvider: { .init(reduceMotion: false, prefersCrossFadeTransitions: false) },
            hostBuilder: { node in AnyView(Text(String(describing: node))) }
        )
        container.loadViewIfNeeded()

        XCTAssertTrue(container.requestHardSnap(to: [second]))
        XCTAssertEqual(events, ["release", "canonical", "terminal"])
        XCTAssertEqual(container.path, [second])
        XCTAssertFalse(container.hasTerminalResidue)
    }

    private func makeTextView() -> GaryxComposerOrderedTextView {
        GaryxComposerOrderedTextView(
            occurrenceID: .init(rawValue: UUID().uuidString),
            composerKey: .draft("test")
        )
    }

    private func configuration(key: GaryxComposerKey) -> GaryxComposerInputConfiguration {
        .init(
            composerKey: key,
            sessionID: .init(rawValue: UUID().uuidString),
            epoch: 1,
            payloadGeneration: 1,
            reservationID: nil,
            nextInputSequence: 1,
            initialText: "",
            isReadOnly: false
        )
    }

    private func persistText(
        _ text: String,
        in coordinator: GaryxComposerPayloadCoordinator
    ) async throws {
        let configuration = try XCTUnwrap(coordinator.inputConfiguration())
        let revision = coordinator.snapshot.revision
        coordinator.acceptText(
            text,
            identity: .init(
                composerKey: configuration.composerKey,
                sessionID: configuration.sessionID,
                inputSessionEpoch: configuration.epoch,
                payloadGeneration: configuration.payloadGeneration,
                reservationID: configuration.reservationID,
                inputSequence: 1
            )
        )
        for _ in 0..<100 where coordinator.snapshot.revision < revision + 2 {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertGreaterThanOrEqual(coordinator.snapshot.revision, revision + 2)
    }

    private func uploadedAttachment(path: String) throws -> GaryxUploadedChatAttachment {
        try JSONDecoder().decode(
            GaryxUploadedChatAttachment.self,
            from: Data(
                "{\"kind\":\"file\",\"path\":\"\(path)\",\"name\":\"attachment.txt\",\"media_type\":\"text/plain\"}".utf8
            )
        )
    }

    private func temporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("GaryxComposerRuntime-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func waitUntil(
        _ predicate: @MainActor () -> Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) async throws {
        for _ in 0..<200 {
            if predicate() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("condition did not become true", file: file, line: line)
    }

    private func entry(
        _ id: String,
        destination: GaryxRouteDestination
    ) -> GaryxRouteEntry {
        .init(id: .init(rawValue: id), destination: destination)
    }
}

private actor ComposerAsyncGate {
    private var isSuspended = false
    private var continuation: CheckedContinuation<Void, Never>?

    func suspend() async {
        isSuspended = true
        await withCheckedContinuation { continuation in
            self.continuation = continuation
        }
    }

    func waitUntilSuspended() async {
        while !isSuspended {
            await Task.yield()
        }
    }

    func resume() {
        continuation?.resume()
        continuation = nil
    }
}

private final class GaryxComposerDeliveryURLProtocolStub: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let requestHandler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        let request = request
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            do {
                let (response, data) = try requestHandler(request)
                client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
                client?.urlProtocol(self, didLoad: data)
                client?.urlProtocolDidFinishLoading(self)
            } catch {
                client?.urlProtocol(self, didFailWithError: error)
            }
        }
    }

    override func stopLoading() {}
}

private final class GaryxLockedStringBox: @unchecked Sendable {
    private let lock = NSLock()
    private var value: String?

    func set(_ value: String?) {
        lock.lock()
        self.value = value
        lock.unlock()
    }

    func get() -> String? {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}
