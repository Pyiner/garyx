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

    func testRevokedScopeCleansEveryOperationBeforeRemovingSharedEntry() async throws {
        let fixture = try makeFixture()
        let firstSource = fixture.directory.appendingPathComponent("first.bin")
        let secondSource = fixture.directory.appendingPathComponent("second.bin")
        try Data("first".utf8).write(to: firstSource, options: .atomic)
        try Data("second".utf8).write(to: secondSource, options: .atomic)
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: store,
            quotaLimitBytes: 1_024
        )

        let first = try await staging.stage(
            stagedAssetAdmission(
                sourceURL: firstSource,
                assetID: .init(rawValue: "first.bin"),
                operationID: .init(rawValue: "shared-entry-first"),
                entry: makeEntry(),
                expectedRevision: 0
            )
        )
        let afterFirst = try await store.load()
        let sharedEntry = try XCTUnwrap(afterFirst.payloadStore.entry(entryID, scope: scope))
        let second = try await staging.stage(
            stagedAssetAdmission(
                sourceURL: secondSource,
                assetID: .init(rawValue: "second.bin"),
                operationID: .init(rawValue: "shared-entry-second"),
                entry: sharedEntry,
                expectedRevision: afterFirst.revision
            )
        )
        XCTAssertEqual(second.snapshot.operations.count, 2)

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            staging: staging,
            scopes: scopeRegistry(.revoked)
        )
        _ = try await recovery.recover()

        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let restored = try await relaunched.load()
        XCTAssertNil(restored.payloadStore.entry(entryID, scope: scope))
        XCTAssertNil(restored.operations[first.operation.context.key])
        XCTAssertNil(restored.operations[second.operation.context.key])
        XCTAssertTrue(restored.manifests.isEmpty)
        XCTAssertTrue(restored.stagedAssetOwners.isEmpty)
        XCTAssertTrue(restored.pendingFileCleanup.isEmpty)
        XCTAssertEqual(restored.reservedBytes, 0)

        let secondRelaunch = GaryxComposerDurabilityLaunchRecovery(
            durability: relaunched,
            staging: staging,
            scopes: scopeRegistry(.revoked)
        )
        _ = try await secondRelaunch.recover()
    }

    func testCancelledOperationInRevokedScopePreservesSiblingPayload() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        var entry = makeEntry(text: "keep sibling text")
        let siblingAttachmentID = GaryxAttachmentID(rawValue: "keep-sibling")
        entry.addAttachment(
            .init(
                id: siblingAttachmentID,
                stagedAssetID: .init(rawValue: "keep-sibling.bin"),
                generation: entry.currentGeneration,
                byteCount: 5
            )
        )
        let key = operationKey("cancelled-revoked-child")
        let cancelledAssetID = GaryxStagedAssetID(rawValue: "cancelled-revoked.bin")
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: .cancelled,
            assetID: cancelledAssetID,
            reservedBytes: 7,
            attempted: false
        )
        entry.addOperation(key)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed cancelled revoked child",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(
                        .init(
                            key: key,
                            stagedPath: cancelledAssetID.rawValue,
                            state: .cancelled,
                            uploadAttempted: false
                        )
                    ),
                    .reserveStagedAsset(assetID: cancelledAssetID, owner: key, bytes: 7),
                ]
            )
        )

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: scopeRegistry(.revoked)
        )
        _ = try await recovery.recover()
        let restored = try await store.load()
        let survivingEntry = try XCTUnwrap(restored.payloadStore.entry(entryID, scope: scope))
        XCTAssertEqual(survivingEntry.currentText, "keep sibling text")
        XCTAssertNotNil(survivingEntry.attachments[siblingAttachmentID])
        XCTAssertFalse(survivingEntry.operationKeys.contains(key))
        XCTAssertNil(restored.operations[key])
        XCTAssertNil(restored.manifests[key])
        XCTAssertEqual(restored.reservedBytes, 0)
    }

    func testPendingReplacementAbortSettlesLiveAssetOwnerAcrossRelaunch() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: store,
            quotaLimitBytes: 1_024
        )
        var entry = makeEntry(text: "keep pending replacement siblings")
        let key = operationKey("pending-replacement-owner")
        let assetID = GaryxStagedAssetID(rawValue: "pending-replacement.bin")
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: .failedRetryable,
            assetID: assetID,
            reservedBytes: 31,
            attempted: true
        )
        entry.addOperation(key)
        let replacement = GaryxReplacementRecord(
            id: .init(rawValue: "pending-replacement"),
            scope: scope,
            entryID: entryID,
            oldKey: key,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: assetID,
            reservedBytes: 31
        )
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed pending replacement with live owner",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(
                        .init(
                            key: key,
                            stagedPath: assetID.rawValue,
                            state: .failedRetryable,
                            uploadAttempted: true
                        )
                    ),
                    .upsertReplacement(replacement),
                    .reserveStagedAsset(assetID: assetID, owner: key, bytes: 31),
                ]
            )
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            staging: staging,
            scopes: scopeRegistry(.active)
        ).recover()
        let relaunched = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL
        )
        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: relaunched,
            staging: staging,
            scopes: scopeRegistry(.active)
        ).recover()
        let restored = try await relaunched.load()
        XCTAssertEqual(
            restored.payloadStore.entry(entryID, scope: scope)?.currentText,
            "keep pending replacement siblings"
        )
        XCTAssertFalse(
            try XCTUnwrap(restored.payloadStore.entry(entryID, scope: scope))
                .operationKeys.contains(key)
        )
        XCTAssertNil(restored.operations[key])
        XCTAssertNil(restored.manifests[key])
        XCTAssertNil(restored.replacements[replacement.id])
        XCTAssertTrue(restored.stagedAssetOwners.isEmpty)
        XCTAssertTrue(restored.pendingFileCleanup.isEmpty)
        XCTAssertEqual(restored.reservedBytes, 0)
    }

    func testPendingReplacementAbortPreservesOldOperationWhenProvisionalAssetIsUnowned() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: store,
            quotaLimitBytes: 1_024
        )
        var entry = makeEntry(text: "keep original retry")
        let key = operationKey("pending-replacement-original")
        let originalAssetID = GaryxStagedAssetID(rawValue: "original-retry.bin")
        let provisionalAssetID = GaryxStagedAssetID(rawValue: "new-provisional.bin")
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: .failedRetryable,
            assetID: originalAssetID,
            reservedBytes: 29,
            attempted: true
        )
        entry.addOperation(key)
        let replacement = GaryxReplacementRecord(
            id: .init(rawValue: "unowned-pending-replacement"),
            scope: scope,
            entryID: entryID,
            oldKey: key,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: provisionalAssetID,
            reservedBytes: 31
        )
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed unowned pending replacement provisional",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(
                        .init(
                            key: key,
                            stagedPath: originalAssetID.rawValue,
                            state: .failedRetryable,
                            uploadAttempted: true
                        )
                    ),
                    .upsertReplacement(replacement),
                    .reserveStagedAsset(assetID: originalAssetID, owner: key, bytes: 29),
                ]
            )
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            staging: staging,
            scopes: scopeRegistry(.active)
        ).recover()
        let restored = try await store.load()
        XCTAssertEqual(restored.operations[key], operation)
        XCTAssertNotNil(restored.manifests[key])
        XCTAssertEqual(restored.stagedAssetOwners[originalAssetID], key)
        XCTAssertEqual(restored.reservedBytes, 29)
        XCTAssertNil(restored.replacements[replacement.id])
        XCTAssertTrue(restored.pendingFileCleanup.isEmpty)
    }

    func testCommittedReplacementRevocationDelegatesToRetryableSuccessorAndPreservesSiblings() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: store,
            quotaLimitBytes: 1_024
        )
        var entry = makeEntry(text: "keep mixed replacement siblings")
        let siblingAttachmentID = GaryxAttachmentID(rawValue: "replacement-sibling")
        entry.addAttachment(
            .init(
                id: siblingAttachmentID,
                stagedAssetID: .init(rawValue: "replacement-sibling.bin"),
                generation: entry.currentGeneration,
                byteCount: 9
            )
        )
        let oldKey = operationKey("replacement-old")
        let successorKey = operationKey("replacement-successor")
        let assetID = GaryxStagedAssetID(rawValue: "replacement-successor.bin")
        let old = makeOperation(
            key: oldKey,
            entry: entry,
            state: .superseded,
            assetID: nil,
            reservedBytes: 0,
            attempted: true
        )
        let successor = makeOperation(
            key: successorKey,
            entry: entry,
            state: .failedRetryable,
            assetID: assetID,
            reservedBytes: 37,
            attempted: true
        )
        entry.addOperation(oldKey)
        entry.addOperation(successorKey)
        let feedbackID = GaryxFeedbackID(rawValue: "replacement-feedback")
        let lineageID = GaryxAttachmentLineageID(rawValue: "replacement-lineage")
        entry.addFeedbackReference(feedbackID)
        let feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entryID,
            operationID: successorKey.operationID,
            lineageID: lineageID,
            kind: .uploadRetryable
        )
        let lineage = GaryxAttachmentLineageTombstone(
            id: lineageID,
            scope: scope,
            entryID: entryID,
            attachmentSlotID: siblingAttachmentID,
            failedOperationID: successorKey.operationID,
            feedbackID: feedbackID,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        var replacement = GaryxReplacementRecord(
            id: .init(rawValue: "committed-replacement"),
            scope: scope,
            entryID: entryID,
            oldKey: oldKey,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: assetID,
            reservedBytes: 37
        )
        replacement.commit(newKey: successorKey)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed committed replacement for revoked mixed state",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(old),
                    .upsertOperation(successor),
                    .upsertManifest(
                        .init(
                            key: oldKey,
                            stagedPath: "transferred",
                            state: .superseded,
                            uploadAttempted: true
                        )
                    ),
                    .upsertManifest(
                        .init(
                            key: successorKey,
                            stagedPath: assetID.rawValue,
                            state: .failedRetryable,
                            uploadAttempted: true
                        )
                    ),
                    .upsertReplacement(replacement),
                    .upsertFeedback(feedback),
                    .upsertAttachmentLineage(lineage),
                    .reserveStagedAsset(assetID: assetID, owner: successorKey, bytes: 37),
                ]
            )
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            staging: staging,
            scopes: scopeRegistry(.revoked)
        ).recover()
        let relaunched = try GaryxSQLiteComposerDurabilityStore(
            databaseURL: fixture.databaseURL
        )
        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: relaunched,
            staging: staging,
            scopes: scopeRegistry(.revoked)
        ).recover()
        let restored = try await relaunched.load()
        let survivingEntry = try XCTUnwrap(restored.payloadStore.entry(entryID, scope: scope))
        XCTAssertEqual(survivingEntry.currentText, "keep mixed replacement siblings")
        XCTAssertNotNil(survivingEntry.attachments[siblingAttachmentID])
        XCTAssertTrue(survivingEntry.operationKeys.isEmpty)
        XCTAssertTrue(survivingEntry.feedbackReferences.isEmpty)
        XCTAssertTrue(restored.operations.isEmpty)
        XCTAssertTrue(restored.manifests.isEmpty)
        XCTAssertTrue(restored.replacements.isEmpty)
        XCTAssertTrue(restored.feedback.isEmpty)
        XCTAssertTrue(restored.attachmentLineages.isEmpty)
        XCTAssertTrue(restored.stagedAssetOwners.isEmpty)
        XCTAssertTrue(restored.pendingFileCleanup.isEmpty)
        XCTAssertEqual(restored.reservedBytes, 0)
    }

    func testRevokedEntryAtomicallyClearsReplacementFeedbackAndLineageFamilies() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: fixture.applicationSupport,
            durability: store,
            quotaLimitBytes: 1_024
        )
        var entry = makeEntry(text: "erase revoked payload")
        let key = operationKey("revoked-family-owner")
        let assetID = GaryxStagedAssetID(rawValue: "revoked-family.bin")
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: .requested,
            assetID: assetID,
            reservedBytes: 41,
            attempted: false
        )
        entry.addOperation(key)
        let feedbackID = GaryxFeedbackID(rawValue: "revoked-family-feedback")
        let lineageID = GaryxAttachmentLineageID(rawValue: "revoked-family-lineage")
        entry.addFeedbackReference(feedbackID)
        let feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entryID,
            operationID: key.operationID,
            lineageID: lineageID,
            kind: .uploadRetryable
        )
        let lineage = GaryxAttachmentLineageTombstone(
            id: lineageID,
            scope: scope,
            entryID: entryID,
            attachmentSlotID: .init(rawValue: "revoked-family-slot"),
            failedOperationID: key.operationID,
            feedbackID: feedbackID,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        let replacement = GaryxReplacementRecord(
            id: .init(rawValue: "revoked-family-replacement"),
            scope: scope,
            entryID: entryID,
            oldKey: key,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: assetID,
            reservedBytes: 41
        )
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed every revoked Entry record family",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(
                        .init(
                            key: key,
                            stagedPath: assetID.rawValue,
                            state: .requested,
                            uploadAttempted: false
                        )
                    ),
                    .upsertReplacement(replacement),
                    .upsertFeedback(feedback),
                    .upsertAttachmentLineage(lineage),
                    .reserveStagedAsset(assetID: assetID, owner: key, bytes: 41),
                ]
            )
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            staging: staging,
            scopes: scopeRegistry(.revoked)
        ).recover()
        let restored = try await store.load()
        XCTAssertNil(restored.payloadStore.entry(entryID, scope: scope))
        XCTAssertTrue(restored.operations.isEmpty)
        XCTAssertTrue(restored.manifests.isEmpty)
        XCTAssertTrue(restored.replacements.isEmpty)
        XCTAssertTrue(restored.feedback.isEmpty)
        XCTAssertTrue(restored.attachmentLineages.isEmpty)
        XCTAssertTrue(restored.stagedAssetOwners.isEmpty)
        XCTAssertTrue(restored.pendingFileCleanup.isEmpty)
        XCTAssertEqual(restored.reservedBytes, 0)
    }

    func testDiscardSettlementUsesCurrentAcknowledgementInsteadOfCapturedDelivery() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let send = try makeCommitSend()
        _ = try await store.commitSend(send)
        let gate = GaryxComposerDeliveryTransportGate(durability: store)
        try await gate.performAttempt(deliveryID: send.delivery.id) { _ in }

        var attemptedSnapshot = try await store.load()
        var entry = try XCTUnwrap(attemptedSnapshot.payloadStore.entry(entryID, scope: scope))
        XCTAssertTrue(entry.beginDiscard(revision: attemptedSnapshot.revision + 1))
        let capturedAttempt = try XCTUnwrap(attemptedSnapshot.deliveries[send.delivery.id])
        XCTAssertEqual(capturedAttempt.phase, .transportAttempted)
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: try XCTUnwrap(attemptedSnapshot.barriers[entryID]),
            deliveries: [capturedAttempt.id: capturedAttempt]
        )
        attemptedSnapshot = try await store.commit(
            .init(
                expectedRevision: attemptedSnapshot.revision,
                label: "capture attempted delivery for discard",
                mutations: [
                    .upsertEntry(entry),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )

        try await gate.acknowledge(deliveryID: send.delivery.id)
        let acknowledged = try await store.load()
        XCTAssertEqual(acknowledged.deliveries[send.delivery.id]?.phase, .acknowledged)
        XCTAssertEqual(acknowledged.deliveries[send.delivery.id]?.evidence, .serverAcknowledged)

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
        _ = try await recovery.recover()

        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored.deliveries[send.delivery.id]?.phase, .terminalEvidence)
        XCTAssertEqual(restored.deliveries[send.delivery.id]?.evidence, .serverAcknowledged)
        XCTAssertEqual(
            restored.deliveries[send.delivery.id]?.userDisposition,
            GaryxDeliveryUserDisposition.none
        )
    }

    func testFullCorrelationPoolCannotBrickDiscardRecoveryAcrossRelaunches() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let historicalEntryID = GaryxComposerPayloadEntryID(rawValue: "correlation-history-entry")
        let historicalReservationID = GaryxSendReservationID(rawValue: 8)
        var historicalLedger = GaryxProvisionalReservationLedger(
            key: .init(
                scope: scope,
                entryID: historicalEntryID,
                reservationID: historicalReservationID
            ),
            envelopeGeneration: 1,
            followupGeneration: 2
        )
        XCTAssertTrue(historicalLedger.settle(.committed, targetGeneration: 2))
        var targetLedger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        XCTAssertTrue(targetLedger.settle(.committed, targetGeneration: 11))

        var entry = makeEntry(text: "discard-at-capacity")
        XCTAssertTrue(entry.beginDiscard(revision: 2))
        let barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        let targetEnvelope = GaryxDeliveryEnvelope(
            text: "target",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "target-intent"
        )
        var target = GaryxDeliveryRecord(
            id: .init(rawValue: "zz-discard-target"),
            scope: scope,
            entryID: entryID,
            reservationID: reservationID,
            correlationID: "target-correlation",
            envelope: targetEnvelope
        )
        XCTAssertTrue(target.markTransportAttempted())
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            deliveries: [target.id: target]
        )
        var mutations: [GaryxComposerDurabilityMutation] = [
            .upsertLedger(historicalLedger),
            .upsertLedger(targetLedger),
            .upsertEntry(entry),
        ]
        let historicalEnvelope = GaryxDeliveryEnvelope(
            text: "historical",
            attachmentIDs: [],
            generation: 1,
            clientIntentID: "historical-intent"
        )
        for index in 0..<GaryxPersistentTombstoneBudget().countLimit {
            var record = GaryxDeliveryRecord(
                id: .init(rawValue: String(format: "historical-%04d", index)),
                scope: scope,
                entryID: historicalEntryID,
                reservationID: historicalReservationID,
                correlationID: String(format: "historical-correlation-%04d", index),
                envelope: historicalEnvelope
            )
            record.recordServerAcknowledgement()
            mutations.append(.upsertDelivery(record))
        }
        mutations.append(.upsertDelivery(target))
        mutations.append(.upsertDiscardConvergence(convergence))
        let admitted = try await store.commit(
            .init(expectedRevision: 0, label: "fill correlation pool", mutations: mutations)
        )
        XCTAssertEqual(
            admitted.persistentTombstoneUsage.correlationCount,
            GaryxPersistentTombstoneBudget().countLimit
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: scopeRegistry(.active)
        ).recover()
        for relaunch in 1...2 {
            let relaunched = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL
            )
            _ = try await GaryxComposerDurabilityLaunchRecovery(
                durability: relaunched,
                scopes: scopeRegistry(.active)
            ).recover()
            let restored = try await relaunched.load()
            XCTAssertNil(
                restored.deliveries[.init(rawValue: "historical-0000")],
                "oldest correlation tombstone survived relaunch \(relaunch)"
            )
            XCTAssertEqual(restored.deliveries[target.id]?.phase, .evidence)
            XCTAssertEqual(
                restored.persistentTombstoneUsage.correlationCount,
                restored.tombstoneBudget.countLimit
            )
            XCTAssertNil(restored.discardConvergence[entryID])
        }
    }

    func testDiscardRemovesProducerAndRecoveredClosePayloadAcrossRelaunches() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let send = try makeCommitSend(includeProducerDrained: true)
        var snapshot = try await store.commitSend(send)
        let producerKey = try XCTUnwrap(snapshot.producerDrained.keys.first)
        var entry = try XCTUnwrap(snapshot.payloadStore.entry(entryID, scope: scope))
        XCTAssertTrue(entry.beginDiscard(revision: 2))

        let closeReservationID = GaryxSendReservationID(rawValue: 10)
        var closeLedger = GaryxProvisionalReservationLedger(
            key: .init(
                scope: scope,
                entryID: entryID,
                reservationID: closeReservationID
            ),
            envelopeGeneration: 11,
            followupGeneration: 12
        )
        XCTAssertTrue(closeLedger.settle(.revoked, targetGeneration: 13))
        let closeKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: .init(rawValue: "discarded-recovered-close"),
            epoch: 2
        )
        let close = GaryxRecoveredInputCloseRecord(
            key: closeKey,
            scope: scope,
            entryID: entryID,
            reservationID: closeReservationID,
            targetGeneration: 13,
            finalSequence: 5,
            finalText: "discarded-final-text"
        )
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: send.barrier,
            deliveries: [send.delivery.id: send.delivery]
        )
        snapshot = try await store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "admit payload-bearing discard descendants",
                mutations: [
                    .upsertLedger(closeLedger),
                    .upsertRecoveredInputClose(close),
                    .upsertEntry(entry),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
        XCTAssertNotNil(snapshot.producerDrained[producerKey])
        XCTAssertNotNil(snapshot.recoveredInputClosures[closeKey])

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: scopeRegistry(.active)
        ).recover()
        for _ in 0..<2 {
            let relaunched = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL
            )
            _ = try await GaryxComposerDurabilityLaunchRecovery(
                durability: relaunched,
                scopes: scopeRegistry(.active)
            ).recover()
            let restored = try await relaunched.load()
            XCTAssertTrue(restored.producerDrained.isEmpty)
            XCTAssertTrue(restored.recoveredInputClosures.isEmpty)
            let restoredBarrier = try XCTUnwrap(restored.barriers[entryID])
            XCTAssertEqual(restoredBarrier.phase, .idle)
            XCTAssertNil(restoredBarrier.envelopeText)
            XCTAssertTrue(restoredBarrier.envelopeAttachmentIDs.isEmpty)
            XCTAssertNil(restoredBarrier.envelopeClientIntentID)
            XCTAssertTrue(restoredBarrier.provisionalFollowupText.isEmpty)
            XCTAssertTrue(restoredBarrier.provisionalFollowupAttachmentIDs.isEmpty)
            XCTAssertNil(restored.payloadStore.entry(entryID, scope: scope))
            XCTAssertNil(restored.discardConvergence[entryID])
        }
    }

    func testDiscardAdmissionWithOnlyAcknowledgedDeliveryStillPublishesTerminalEvidence() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let send = try makeCommitSend()
        _ = try await store.commitSend(send)
        let gate = GaryxComposerDeliveryTransportGate(durability: store)
        try await gate.performAttempt(deliveryID: send.delivery.id) { _ in }
        try await gate.acknowledge(deliveryID: send.delivery.id)

        let acknowledgedSnapshot = try await store.load()
        var entry = try XCTUnwrap(
            acknowledgedSnapshot.payloadStore.entry(entryID, scope: scope)
        )
        XCTAssertTrue(entry.beginDiscard(revision: acknowledgedSnapshot.revision + 1))
        let acknowledged = try XCTUnwrap(acknowledgedSnapshot.deliveries[send.delivery.id])
        XCTAssertEqual(acknowledged.phase, .acknowledged)
        _ = try await store.commit(
            .init(
                expectedRevision: acknowledgedSnapshot.revision,
                label: "capture acknowledged-only discard",
                mutations: [
                    .upsertEntry(entry),
                    .upsertDiscardConvergence(
                        .init(
                            lifecycle: entry.lifecycle,
                            barrier: try XCTUnwrap(acknowledgedSnapshot.barriers[entryID]),
                            deliveries: [acknowledged.id: acknowledged]
                        )
                    ),
                ]
            )
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        ).recover()
        let restored = try await store.load()
        XCTAssertEqual(restored.deliveries[send.delivery.id]?.phase, .terminalEvidence)
        XCTAssertEqual(restored.deliveries[send.delivery.id]?.evidence, .serverAcknowledged)

        try await gate.acknowledge(deliveryID: send.delivery.id)
        let idempotentLateFrame = try await store.load()
        XCTAssertEqual(idempotentLateFrame.deliveries[send.delivery.id]?.phase, .terminalEvidence)
        XCTAssertEqual(
            idempotentLateFrame.deliveries[send.delivery.id]?.evidence,
            .serverAcknowledged
        )
    }

    func testRecoveryFeedbackIdentityIncludesEntryIdentity() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let otherEntryID = GaryxComposerPayloadEntryID(rawValue: "recovery-entry-other")
        var firstEntry = makeEntry()
        var secondEntry = GaryxComposerPayloadEntry(
            id: otherEntryID,
            scope: scope,
            destination: .draft("recovery-draft-other"),
            lifecycleToken: .init(entryID: otherEntryID, nonce: "recovery-token-other"),
            currentGeneration: 10
        )
        let sharedOperationID = GaryxOperationID(rawValue: "same-local-operation")
        let firstKey = operationKey(sharedOperationID.rawValue)
        let secondKey = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: otherEntryID,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: sharedOperationID
        )
        let firstOperation = makeOperation(
            key: firstKey,
            entry: firstEntry,
            state: .uploading,
            assetID: nil,
            reservedBytes: 0,
            attempted: true
        )
        let secondOperation = makeOperation(
            key: secondKey,
            entry: secondEntry,
            state: .uploading,
            assetID: nil,
            reservedBytes: 0,
            attempted: true
        )
        firstEntry.addOperation(firstKey)
        secondEntry.addOperation(secondKey)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed same operation identity in two entries",
                mutations: [
                    .upsertEntry(firstEntry),
                    .upsertOperation(firstOperation),
                    .upsertManifest(
                        .init(
                            key: firstKey,
                            stagedPath: "first",
                            state: .uploading,
                            uploadAttempted: true
                        )
                    ),
                    .upsertEntry(secondEntry),
                    .upsertOperation(secondOperation),
                    .upsertManifest(
                        .init(
                            key: secondKey,
                            stagedPath: "second",
                            state: .uploading,
                            uploadAttempted: true
                        )
                    ),
                ]
            )
        )

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
        _ = try await recovery.recover()
        let restored = try await store.load()
        XCTAssertEqual(restored.feedback.count, 2)
        let firstReferences = try XCTUnwrap(
            restored.payloadStore.entry(entryID, scope: scope)?.feedbackReferences
        )
        let secondReferences = try XCTUnwrap(
            restored.payloadStore.entry(otherEntryID, scope: scope)?.feedbackReferences
        )
        XCTAssertEqual(firstReferences.count, 1)
        XCTAssertEqual(secondReferences.count, 1)
        XCTAssertTrue(firstReferences.isDisjoint(with: secondReferences))
    }

    func testDiscardRetiresEntireMultiHopAliasLineage() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let source = GaryxComposerKey.draft("alias-source")
        let intermediate = GaryxComposerKey.thread("alias-intermediate")
        let destination = GaryxComposerKey.thread("alias-destination")
        var entry = makeEntry()
        entry.promote(to: destination)
        XCTAssertTrue(entry.beginDiscard(revision: 2))
        let barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(token: entry.lifecycle.token, revision: entry.lifecycle.revision)
        )
        let liveSessionKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: .init(rawValue: "multi-hop-live-session"),
            epoch: 1
        )
        let pendingAckSessionKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: .init(rawValue: "multi-hop-pending-ack-session"),
            epoch: 2
        )
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            sessions: [
                liveSessionKey: .init(
                    key: liveSessionKey,
                    composerKey: source,
                    phase: .live,
                    finalSequence: nil
                ),
                pendingAckSessionKey: .init(
                    key: pendingAckSessionKey,
                    composerKey: intermediate,
                    phase: .closePendingAck,
                    finalSequence: 7
                ),
            ]
        )
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: source,
                target: intermediate,
                activeOrClosingSessions: 1
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: intermediate,
                target: destination,
                activeOrClosingSessions: 2,
                pendingCloseAcknowledgements: 1
            ),
            .established
        )
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed multi-hop alias discard",
                mutations: [
                    .upsertEntry(entry),
                    .replaceAliases(aliases),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
        _ = try await recovery.recover()
        let restored = try await store.load()
        XCTAssertEqual(restored.aliases.aliasCount, 0)
        XCTAssertTrue(restored.aliases.partitions[scope]?.isEmpty ?? true)
    }

    func testDiscardRetiresOnlyCapturedBranchAtAliasFanIn() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        let discardedSource = GaryxComposerKey.draft("discarded-source")
        let liveSource = GaryxComposerKey.draft("live-source")
        let destination = GaryxComposerKey.thread("shared-destination")
        var entry = makeEntry()
        entry.promote(to: destination)
        XCTAssertTrue(entry.beginDiscard(revision: 2))
        let barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(token: entry.lifecycle.token, revision: entry.lifecycle.revision)
        )
        let sessionKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: .init(rawValue: "discarded-source-session"),
            epoch: 1
        )
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            sessions: [
                sessionKey: .init(
                    key: sessionKey,
                    composerKey: discardedSource,
                    phase: .live,
                    finalSequence: nil
                ),
            ]
        )
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: discardedSource,
                target: destination,
                activeOrClosingSessions: 1
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: liveSource,
                target: destination,
                activeOrClosingSessions: 1
            ),
            .established
        )
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed alias fan-in discard",
                mutations: [
                    .upsertEntry(entry),
                    .replaceAliases(aliases),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )

        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: scopeRegistry(.active)
        ).recover()
        let restored = try await store.load()
        XCTAssertNil(restored.aliases.partitions[scope]?[discardedSource])
        XCTAssertNotNil(restored.aliases.partitions[scope]?[liveSource])
        XCTAssertEqual(restored.aliases.aliasCount, 1)
        XCTAssertEqual(
            restored.aliases.resolve(
                liveSource,
                scope: scope,
                scopes: scopeRegistry(.active)
            ),
            .resolved(destination)
        )
    }

    func testDiscardDecrementsSharedAliasSuffixWithoutBreakingSiblingAcrossRelaunches() async throws {
        let fixture = try makeFixture()
        let discardedSource = GaryxComposerKey.draft("discarded-shared-suffix-source")
        let liveSource = GaryxComposerKey.draft("live-shared-suffix-source")
        let sharedIntermediate = GaryxComposerKey.thread("shared-suffix-intermediate")
        let destination = GaryxComposerKey.thread("shared-suffix-destination")
        var entry = makeEntry()
        entry.promote(to: destination)
        XCTAssertTrue(entry.beginDiscard(revision: 2))
        let barrier = GaryxSendCommitBarrier(
            entryID: entryID,
            scope: scope,
            payloadLifecycle: .init(token: entry.lifecycle.token, revision: entry.lifecycle.revision)
        )
        let sessionKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: .init(rawValue: "discarded-shared-suffix-session"),
            epoch: 1
        )
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            sessions: [
                sessionKey: .init(
                    key: sessionKey,
                    composerKey: discardedSource,
                    phase: .live,
                    finalSequence: nil
                ),
            ]
        )
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: discardedSource,
                target: sharedIntermediate,
                activeOrClosingSessions: 1
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: liveSource,
                target: sharedIntermediate,
                activeOrClosingSessions: 1
            ),
            .established
        )
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: sharedIntermediate,
                target: destination,
                activeOrClosingSessions: 2
            ),
            .established
        )
        do {
            let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
            _ = try await store.commit(
                .init(
                    expectedRevision: 0,
                    label: "seed alias shared-suffix discard",
                    mutations: [
                        .upsertEntry(entry),
                        .replaceAliases(aliases),
                        .upsertDiscardConvergence(convergence),
                    ]
                )
            )
        }

        for relaunch in 1...2 {
            let relaunchedStore = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: fixture.databaseURL
            )
            _ = try await GaryxComposerDurabilityLaunchRecovery(
                durability: relaunchedStore,
                scopes: scopeRegistry(.active)
            ).recover()
            let restored = try await relaunchedStore.load()
            XCTAssertNil(
                restored.aliases.partitions[scope]?[discardedSource],
                "discarded source survived relaunch \(relaunch)"
            )
            XCTAssertEqual(
                restored.aliases.partitions[scope]?[liveSource]?.activeOrClosingSessions,
                1
            )
            XCTAssertEqual(
                restored.aliases.partitions[scope]?[sharedIntermediate]?
                    .activeOrClosingSessions,
                1,
                "shared suffix lost the live sibling reference on relaunch \(relaunch)"
            )
            XCTAssertEqual(
                restored.aliases.resolve(
                    liveSource,
                    scope: scope,
                    scopes: scopeRegistry(.active)
                ),
                .resolved(destination)
            )
        }
    }

    func testDiscardSubtractsSessionContributionWithoutTopologyOwnershipAcrossRelaunches() async throws {
        struct PromotionSeed {
            let source: GaryxComposerKey
            let target: GaryxComposerKey
            let activeOrClosingSessions: Int
        }
        struct Shape {
            let name: String
            let origin: GaryxComposerKey
            let residualSource: GaryxComposerKey
            let destination: GaryxComposerKey
            let promotions: [PromotionSeed]
            let discardedSessionCount: Int
            let preRetiredSessionCount: Int
        }
        let firstOrigin = GaryxComposerKey.draft("occupancy-only-origin")
        let intermediate = GaryxComposerKey.thread("occupancy-only-intermediate")
        let firstDestination = GaryxComposerKey.thread("occupancy-only-destination")
        let followUpSource = GaryxComposerKey.draft("same-source-follow-up")
        let followUpDestination = GaryxComposerKey.thread("same-source-destination")
        let shapes = [
            Shape(
                name: "shared suffix without a predecessor edge",
                origin: firstOrigin,
                residualSource: intermediate,
                destination: firstDestination,
                promotions: [
                    .init(
                        source: firstOrigin,
                        target: intermediate,
                        activeOrClosingSessions: 1
                    ),
                    .init(
                        source: intermediate,
                        target: firstDestination,
                        activeOrClosingSessions: 2
                    ),
                ],
                discardedSessionCount: 1,
                preRetiredSessionCount: 0
            ),
            Shape(
                name: "same-source follow-up occupancy",
                origin: followUpSource,
                residualSource: followUpSource,
                destination: followUpDestination,
                promotions: [
                    .init(
                        source: followUpSource,
                        target: followUpDestination,
                        activeOrClosingSessions: 2
                    ),
                ],
                discardedSessionCount: 1,
                preRetiredSessionCount: 1
            ),
            Shape(
                name: "same-origin session multiplicity",
                origin: .draft("duplicate-session-origin"),
                residualSource: .draft("duplicate-session-origin"),
                destination: .thread("duplicate-session-destination"),
                promotions: [
                    .init(
                        source: .draft("duplicate-session-origin"),
                        target: .thread("duplicate-session-destination"),
                        activeOrClosingSessions: 3
                    ),
                ],
                discardedSessionCount: 2,
                preRetiredSessionCount: 0
            ),
        ]

        for shape in shapes {
            let fixture = try makeFixture()
            var entry = makeEntry()
            entry.promote(to: shape.destination)
            XCTAssertTrue(entry.beginDiscard(revision: 2), shape.name)
            let barrier = GaryxSendCommitBarrier(
                entryID: entryID,
                scope: scope,
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            )
            var sessions = Dictionary(uniqueKeysWithValues: (0..<shape.discardedSessionCount).map {
                index in
                let key = GaryxSessionDescendantKey(
                    token: entry.lifecycle.token,
                    sessionID: .init(rawValue: "occupancy-session-\(shape.name)-\(index)"),
                    epoch: UInt64(index + 1)
                )
                return (
                    key,
                    GaryxSessionDescendant(
                        key: key,
                        composerKey: shape.origin,
                        phase: .live,
                        finalSequence: nil
                    )
                )
            })
            for index in 0..<shape.preRetiredSessionCount {
                let key = GaryxSessionDescendantKey(
                    token: entry.lifecycle.token,
                    sessionID: .init(rawValue: "already-retired-\(shape.name)-\(index)"),
                    epoch: UInt64(shape.discardedSessionCount + index + 1)
                )
                sessions[key] = GaryxSessionDescendant(
                    key: key,
                    composerKey: shape.origin,
                    phase: .retired,
                    finalSequence: nil
                )
            }
            let convergence = GaryxPayloadDiscardConvergence(
                lifecycle: entry.lifecycle,
                barrier: barrier,
                sessions: sessions
            )
            var aliases = GaryxComposerAliasTable()
            for promotion in shape.promotions {
                XCTAssertEqual(
                    aliases.establishPromotion(
                        scope: scope,
                        source: promotion.source,
                        target: promotion.target,
                        activeOrClosingSessions: promotion.activeOrClosingSessions
                    ),
                    .established,
                    shape.name
                )
            }
            do {
                let store = try GaryxSQLiteComposerDurabilityStore(
                    databaseURL: fixture.databaseURL
                )
                _ = try await store.commit(
                    .init(
                        expectedRevision: 0,
                        label: "seed occupancy-only alias discard",
                        mutations: [
                            .upsertEntry(entry),
                            .replaceAliases(aliases),
                            .upsertDiscardConvergence(convergence),
                        ]
                    )
                )
            }

            for relaunch in 1...2 {
                let relaunchedStore = try GaryxSQLiteComposerDurabilityStore(
                    databaseURL: fixture.databaseURL
                )
                _ = try await GaryxComposerDurabilityLaunchRecovery(
                    durability: relaunchedStore,
                    scopes: scopeRegistry(.active)
                ).recover()
                let restored = try await relaunchedStore.load()
                XCTAssertEqual(
                    restored.aliases.partitions[scope]?[shape.residualSource]?
                        .activeOrClosingSessions,
                    1,
                    "\(shape.name), relaunch \(relaunch)"
                )
                XCTAssertEqual(
                    restored.aliases.resolve(
                        shape.residualSource,
                        scope: scope,
                        scopes: scopeRegistry(.active)
                    ),
                    .resolved(shape.destination),
                    "\(shape.name), relaunch \(relaunch)"
                )
            }
        }
    }

    func testOwnerlessManifestRecoveryClearsEntryMembershipAtomically() async throws {
        let fixture = try makeFixture()
        let store = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        var entry = makeEntry()
        let key = operationKey("ownerless-manifest")
        entry.addOperation(key)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed manifest without capability",
                mutations: [
                    .upsertEntry(entry),
                    .upsertManifest(
                        .init(
                            key: key,
                            stagedPath: "ownerless.bin",
                            state: .preparing,
                            uploadAttempted: false
                        )
                    ),
                ]
            )
        )

        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: store,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
        _ = try await recovery.recover()
        let relaunched = try GaryxSQLiteComposerDurabilityStore(databaseURL: fixture.databaseURL)
        _ = try await GaryxComposerDurabilityLaunchRecovery(
            durability: relaunched,
            scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
        ).recover()
        let restored = try await relaunched.load()
        XCTAssertNil(restored.manifests[key])
        XCTAssertFalse(
            try XCTUnwrap(restored.payloadStore.entry(entryID, scope: scope))
                .operationKeys.contains(key)
        )
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

    private func stagedAssetAdmission(
        sourceURL: URL,
        assetID: GaryxStagedAssetID,
        operationID: GaryxOperationID,
        entry: GaryxComposerPayloadEntry,
        expectedRevision: UInt64
    ) -> GaryxComposerStagedAssetAdmission {
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: entry.currentGeneration,
            reservationID: nil,
            branch: .followup,
            operationID: operationID
        )
        return GaryxComposerStagedAssetAdmission(
            expectedRevision: expectedRevision,
            sourceURL: sourceURL,
            assetID: assetID,
            entry: entry,
            context: .init(
                key: key,
                clientIdentity: "recovery-client",
                configurationFingerprint: "recovery-config",
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            )
        )
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

    private func makeCommitSend(
        includeProducerDrained: Bool = false
    ) throws -> GaryxComposerCommitSend {
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
        var producerDrained: [
            GaryxSessionDescendantKey: GaryxDurableProducerDrainedRecord
        ] = [:]
        if includeProducerDrained {
            let key = GaryxSessionDescendantKey(
                token: entry.lifecycle.token,
                sessionID: .init(rawValue: "committed-producer-drained"),
                epoch: 1
            )
            producerDrained[key] = GaryxDurableProducerDrainedRecord(
                scope: scope,
                entryID: entryID,
                reservationID: reservationID,
                record: .init(
                    sessionID: key.sessionID,
                    epoch: key.epoch,
                    finalSequence: 4,
                    bufferedText: "discarded-buffered-text"
                )
            )
        }
        return try GaryxComposerCommitSend(
            expectedRevision: 0,
            ledger: ledger,
            sealedPayloadEntry: entry,
            barrier: barrier,
            settlement: settlement,
            producerDrained: producerDrained
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
