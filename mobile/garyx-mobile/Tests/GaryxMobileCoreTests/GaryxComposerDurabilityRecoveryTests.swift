import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxComposerDurabilityRecoveryTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "recovery-gateway", epoch: 1)
    private let entryID = GaryxComposerPayloadEntryID(rawValue: "recovery-entry")
    private let reservationID = GaryxSendReservationID(rawValue: 9)

    func testTransportGatePersistsAttemptBeforeNetworkAndNeverRunsNetworkOnCommitFailure() async throws {
        let failedFixture = try makeFixture()
        let setupStore = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: failedFixture.databaseURL
        )
        let send = try makeCommitSend()
        _ = try await setupStore.commitSend(send)

        let failedStore = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: failedFixture.databaseURL,
            boundaryHook: { boundary in
                if boundary == .beforeCommit {
                    throw GaryxSQLiteComposerDurabilityError.injectedFsyncFailure(boundary)
                }
            }
        )
        let failedProbe = NetworkProbe()
        let failedGate = GaryxComposerDeliveryTransportGate(durability: failedStore)
        do {
            try await failedGate.performAttempt(deliveryID: send.delivery.id) { _ in
                failedProbe.count += 1
            }
            XCTFail("attempt publication must fail")
        } catch {
            XCTAssertEqual(
                error as? GaryxSQLiteComposerDurabilityError,
                .injectedFsyncFailure(.beforeCommit)
            )
        }
        XCTAssertEqual(failedProbe.count, 0)
        let failedRelaunch = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: failedFixture.databaseURL
        )
        let failedSnapshot = try await failedRelaunch.load()
        XCTAssertEqual(failedSnapshot.deliveries[send.delivery.id]?.phase, .notDispatched)

        let successfulFixture = try makeFixture()
        let successfulStore = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: successfulFixture.databaseURL
        )
        let successfulSend = try makeCommitSend()
        _ = try await successfulStore.commitSend(successfulSend)
        let successfulProbe = NetworkProbe()
        let gate = GaryxComposerDeliveryTransportGate(durability: successfulStore)
        try await gate.performAttempt(deliveryID: successfulSend.delivery.id) { _ in
            let snapshot = try await successfulStore.load()
            XCTAssertEqual(snapshot.deliveries[successfulSend.delivery.id]?.phase, .transportAttempted)
            successfulProbe.count += 1
        }
        XCTAssertEqual(successfulProbe.count, 1)

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: successfulStore,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
        var report = try await recovery.recover()
        XCTAssertEqual(report.deliveryDispositions[successfulSend.delivery.id], .userTerminable)
        try await gate.acknowledge(deliveryID: successfulSend.delivery.id)
        report = try await recovery.recover()
        XCTAssertEqual(report.deliveryDispositions[successfulSend.delivery.id], .acknowledged)
    }

    func testLaunchSynthesizesRevokedFiveStepFinalAsTUAtGPlusTwoAndClosesOnce() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let initial = makeUnsettledReservationState()
        _ = try await store.commit(initial.transaction)

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
        let report = try await recovery.recover()
        XCTAssertEqual(report.syntheticReservationRecoveries, 1)
        let restored = try await store.load()
        let ledger = try XCTUnwrap(restored.ledgers[initial.ledger.key])
        XCTAssertEqual(ledger.terminalOutcome, .revoked)
        let mergeGeneration = try XCTUnwrap(ledger.targetMapping?.generation)
        XCTAssertGreaterThan(mergeGeneration, 11)
        XCTAssertEqual(restored.payloadStore.entry(entryID, scope: scope)?.currentGeneration, mergeGeneration)
        XCTAssertEqual(restored.payloadStore.entry(entryID, scope: scope)?.currentText, "TU")
        XCTAssertEqual(restored.recoveredInputClosures[initial.drained.key]?.finalText, "TU")
        XCTAssertEqual(restored.recoveredInputClosures[initial.drained.key]?.closePublicationCount, 1)

        let secondReport = try await recovery.recover()
        XCTAssertEqual(secondReport.syntheticReservationRecoveries, 0)
        let secondSnapshot = try await store.load()
        XCTAssertEqual(secondSnapshot.recoveredInputClosures[initial.drained.key]?.closePublicationCount, 1)
    }

    func testOperationManifestRecoveryMatrixPersistsExpectedOutcomeAcrossRelaunch() async throws {
        let cases: [(GaryxOperationCapabilityState, Bool)] = [
            (.requested, false),
            (.preparing, false),
            (.uploading, false),
            (.uploading, true),
            (.completed, true),
            (.failedRetryable, true),
            (.failedTerminal, true),
            (.cancelled, false),
            (.superseded, false),
        ]
        for lifecycle in [
            GaryxGatewayScopeLifecycle.active,
            .suspended,
            .revoked,
        ] {
            for (state, attempted) in cases {
                let fixture = try makeFixture()
                let store = try GaryxSQLiteComposerDurabilityStore(
                    databaseURL: fixture.databaseURL
                )
                let seeded = try await seedOperation(
                    state: state,
                    attempted: attempted,
                    store: store
                )
                let staging = try GaryxComposerStagedAssetStore(
                    applicationSupportDirectory: fixture.applicationSupport,
                    durability: store,
                    quotaLimitBytes: 1_024
                )
                let recovery = GaryxComposerDurabilityLaunchRecovery(
                    durability: store,
                    staging: staging,
                    scopes: scopeRegistry(lifecycle)
                )
                _ = try await recovery.recover()

                let relaunched = try GaryxSQLiteComposerDurabilityStore(
                    databaseURL: fixture.databaseURL
                )
                let restored = try await relaunched.load()
                let recovered = restored.operations[seeded.key]
                switch (state, attempted, lifecycle) {
                case (.uploading, false, .active), (.uploading, false, .suspended):
                    XCTAssertEqual(recovered?.state, .uploading)
                    XCTAssertEqual(restored.reservedBytes, seeded.reservedBytes)
                case (.uploading, true, .active), (.uploading, true, .suspended):
                    XCTAssertEqual(recovered?.state, .failedRetryable)
                    XCTAssertEqual(restored.reservedBytes, seeded.reservedBytes)
                    XCTAssertTrue(restored.feedback.values.contains { $0.kind == .uploadRetryable })
                case (.failedRetryable, _, .active), (.failedRetryable, _, .suspended):
                    XCTAssertEqual(recovered?.state, .failedRetryable)
                    XCTAssertEqual(restored.reservedBytes, seeded.reservedBytes)
                case (.failedTerminal, _, .active), (.failedTerminal, _, .suspended):
                    XCTAssertEqual(recovered?.state, .failedTerminal)
                    XCTAssertEqual(restored.reservedBytes, 0)
                    XCTAssertTrue(restored.feedback.values.contains { $0.kind == .uploadTerminal })
                default:
                    XCTAssertNil(recovered, "state=\(state) scope=\(lifecycle)")
                    XCTAssertEqual(restored.reservedBytes, 0)
                    XCTAssertNil(restored.manifests[seeded.key])
                }
            }
        }
    }

    func testEveryOperationStateDestinationDiscardConvergesAllResourcesToZero() async throws {
        for state in GaryxOperationCapabilityState.allCases {
            let fixture = try makeFixture()
            let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
            let seeded = try await seedDiscardingOperation(state: state, store: store)
            let staging = try GaryxComposerStagedAssetStore(
                applicationSupportDirectory: fixture.applicationSupport,
                durability: store,
                quotaLimitBytes: 1_024
            )
            let recovery = GaryxComposerDurabilityLaunchRecovery(
                durability: store,
                staging: staging,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            )
            _ = try await recovery.recover()

            let relaunched = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL
            )
            let restored = try await relaunched.load()
            XCTAssertNil(restored.payloadStore.entry(entryID, scope: scope), "state \(state)")
            XCTAssertNil(restored.operations[seeded.key], "state \(state)")
            XCTAssertNil(restored.manifests[seeded.key], "state \(state)")
            XCTAssertTrue(restored.replacements.isEmpty, "state \(state)")
            XCTAssertTrue(restored.feedback.isEmpty, "state \(state)")
            XCTAssertTrue(restored.attachmentLineages.isEmpty, "state \(state)")
            XCTAssertTrue(restored.stagedAssetOwners.isEmpty, "state \(state)")
            XCTAssertTrue(restored.pendingFileCleanup.isEmpty, "state \(state)")
            XCTAssertEqual(restored.reservedBytes, 0, "state \(state)")
            XCTAssertTrue(restored.discardConvergence.isEmpty, "state \(state)")
        }
    }

    private func seedOperation(
        state: GaryxOperationCapabilityState,
        attempted: Bool,
        store: GaryxSQLiteComposerDurabilityStore
    ) async throws -> (key: GaryxOperationCapabilityKey, reservedBytes: Int) {
        var entry = makeEntry()
        let key = operationKey("matrix-\(state.rawValue)")
        let ownsAsset = state != .superseded
        let assetID = ownsAsset ? GaryxStagedAssetID(rawValue: "matrix-\(state.rawValue).bin") : nil
        let reservedBytes = ownsAsset ? 17 : 0
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: state,
            assetID: assetID,
            reservedBytes: reservedBytes,
            attempted: attempted
        )
        entry.addOperation(key)
        var mutations: [GaryxComposerDurabilityMutation] = [
            .upsertEntry(entry),
            .upsertOperation(operation),
            .upsertManifest(
                .init(
                    key: key,
                    stagedPath: assetID?.rawValue ?? "transferred",
                    state: state,
                    uploadAttempted: attempted
                )
            ),
        ]
        if let assetID {
            mutations.append(.reserveStagedAsset(assetID: assetID, owner: key, bytes: reservedBytes))
        }
        _ = try await store.commit(
            .init(expectedRevision: 0, label: "seed operation recovery", mutations: mutations)
        )
        return (key, reservedBytes)
    }

    private func seedDiscardingOperation(
        state: GaryxOperationCapabilityState,
        store: GaryxSQLiteComposerDurabilityStore
    ) async throws -> (key: GaryxOperationCapabilityKey, assetID: GaryxStagedAssetID?) {
        var entry = makeEntry()
        let key = operationKey("discard-\(state.rawValue)")
        let assetID = state == .superseded
            ? nil
            : GaryxStagedAssetID(rawValue: "discard-\(state.rawValue).bin")
        let bytes = assetID == nil ? 0 : 23
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: state,
            assetID: assetID,
            reservedBytes: bytes,
            attempted: state == .uploading
        )
        entry.addOperation(key)
        XCTAssertTrue(entry.beginDiscard(revision: 2))
        let barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            operations: [key: operation],
            stagedAssetIDs: assetID.map { [$0] } ?? [],
            reservedBytes: bytes
        )
        var mutations: [GaryxComposerDurabilityMutation] = [
            .upsertEntry(entry),
            .upsertOperation(operation),
            .upsertManifest(
                .init(
                    key: key,
                    stagedPath: assetID?.rawValue ?? "transferred",
                    state: state,
                    uploadAttempted: state == .uploading
                )
            ),
            .upsertDiscardConvergence(convergence),
        ]
        if let assetID {
            mutations.append(.reserveStagedAsset(assetID: assetID, owner: key, bytes: bytes))
        }
        _ = try await store.commit(
            .init(expectedRevision: 0, label: "seed destination discard", mutations: mutations)
        )
        return (key, assetID)
    }

    private func makeOperation(
        key: GaryxOperationCapabilityKey,
        entry: GaryxComposerPayloadEntry,
        state: GaryxOperationCapabilityState,
        assetID: GaryxStagedAssetID?,
        reservedBytes: Int,
        attempted: Bool
    ) -> GaryxOperationCapability {
        GaryxOperationCapability(
            context: .init(
                key: key,
                clientIdentity: "recovery-client",
                configurationFingerprint: "recovery-config",
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: state,
            stagedAssetID: assetID,
            reservedBytes: reservedBytes,
            uploadAttempted: attempted
        )
    }

    private func makeUnsettledReservationState() -> (
        transaction: GaryxComposerDurabilityTransaction,
        ledger: GaryxProvisionalReservationLedger,
        drained: (key: GaryxSessionDescendantKey, value: GaryxDurableProducerDrainedRecord)
    ) {
        var entry = makeEntry(text: "T")
        entry.setText("U", generation: 11)
        var barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        let envelope = GaryxDeliveryEnvelope(
            text: "T",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "unsettled-intent"
        )
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservationID,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot))
        let ledger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        let descendantKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: GaryxComposerInputSessionID(rawValue: "unsettled-session"),
            epoch: 1
        )
        let drained = (
            descendantKey,
            GaryxDurableProducerDrainedRecord(
                scope: scope,
                entryID: entryID,
                reservationID: reservationID,
                record: .init(
                    sessionID: descendantKey.sessionID,
                    epoch: descendantKey.epoch,
                    finalSequence: 4,
                    bufferedText: "U"
                )
            )
        )
        return (
            .init(
                expectedRevision: 0,
                label: "seed unsettled reservation",
                mutations: [
                    .setGenerationHighWatermark(32),
                    .upsertLedger(ledger),
                    .upsertEntry(entry),
                    .upsertBarrier(barrier),
                    .upsertProducerDrained(drained.0, drained.1),
                ]
            ),
            ledger,
            (drained.0, drained.1)
        )
    }

    private func makeCommitSend() throws -> GaryxComposerCommitSend {
        var entry = makeEntry(text: "message")
        entry.setText("next", generation: 11)
        var barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        let envelope = GaryxDeliveryEnvelope(
            text: "message",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "transport-intent"
        )
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservationID,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("next", lifecycle: entry.lifecycle.snapshot))
        let settlement = try XCTUnwrap(
            barrier.durableCommit(
                deliveryID: GaryxDeliveryRecordID(rawValue: "transport-delivery"),
                correlationID: "transport-correlation",
                clientIntentID: "transport-intent",
                lifecycle: entry.lifecycle.snapshot
            )
        )
        var ledger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        XCTAssertTrue(ledger.settle(.committed, targetGeneration: 11))
        return try GaryxComposerCommitSend(
            expectedRevision: 0,
            ledger: ledger,
            sealedPayloadEntry: entry,
            barrier: barrier,
            settlement: settlement
        )
    }

    private func makeEntry(text: String = "") -> GaryxComposerPayloadEntry {
        GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("recovery-draft"),
            lifecycleToken: .init(entryID: entryID, nonce: "recovery-token"),
            currentGeneration: 10,
            text: text
        )
    }

    private func operationKey(_ rawValue: String) -> GaryxOperationCapabilityKey {
        .init(
            scope: scope,
            entryID: entryID,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: rawValue)
        )
    }

    private func scopeRegistry(
        _ lifecycle: GaryxGatewayScopeLifecycle
    ) -> GaryxGatewayScopeRegistry {
        var registry = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        switch lifecycle {
        case .active:
            break
        case .suspended:
            _ = registry.suspendActive()
        case .revoked:
            _ = registry.revoke(scope)
        }
        return registry
    }

    private func makeFixture() throws -> (
        directory: URL,
        applicationSupport: URL,
        databaseURL: URL
    ) {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("garyx-recovery-tests-\(UUID().uuidString)", isDirectory: true)
        let applicationSupport = directory.appendingPathComponent("ApplicationSupport")
        try FileManager.default.createDirectory(
            at: applicationSupport,
            withIntermediateDirectories: true
        )
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return (
            directory,
            applicationSupport,
            applicationSupport.appendingPathComponent("composer.sqlite3")
        )
    }
}

private final class NetworkProbe: @unchecked Sendable {
    var count = 0
}
