import Foundation

struct GaryxComposerAttachmentMetadata: Sendable {
    let kind: String
    let name: String
    let mediaType: String
    let previewDataURL: String?
}

struct GaryxComposerStagedUpload: Sendable {
    let attachmentID: GaryxAttachmentID
    let operationKey: GaryxOperationCapabilityKey
    let fileURL: URL
    let metadata: GaryxComposerAttachmentMetadata
}

struct GaryxComposerDeliveryHandle: Equatable, Sendable {
    let deliveryID: GaryxDeliveryRecordID
    let entryID: GaryxComposerPayloadEntryID
    let scope: GaryxGatewayScope
    let reservationID: GaryxSendReservationID
    let lifecycle: GaryxPayloadLifecycleCapture
}

struct GaryxComposerReadyPayload: Sendable {
    let text: String
    let attachments: [GaryxComposerAttachment]
    let clientIntentID: String
    let delivery: GaryxComposerDeliveryHandle
}

struct GaryxComposerCreateRebuildPayload: Sendable {
    let payload: GaryxComposerReadyPayload
    let ambiguousAfter: GaryxCreateDeliveryPhase?
    let requiresThreadCreation: Bool
}

struct GaryxComposerRuntimeTestingHooks: Sendable {
    var beforePrepareSendReturns: (@Sendable () async -> Void)?
    var finalizationFailuresBeforeSuccess: Int

    init(
        beforePrepareSendReturns: (@Sendable () async -> Void)? = nil,
        finalizationFailuresBeforeSuccess: Int = 0
    ) {
        self.beforePrepareSendReturns = beforePrepareSendReturns
        self.finalizationFailuresBeforeSuccess = finalizationFailuresBeforeSuccess
    }
}

enum GaryxComposerPayloadRuntimeError: LocalizedError {
    case unavailable
    case staleActivation
    case payloadPreparing
    case attachmentNotUploaded
    case deliveryBackpressure
    case invalidTransition

    var errorDescription: String? {
        switch self {
        case .unavailable:
            "Composer payload is not available yet."
        case .staleActivation:
            "Composer context changed before the operation completed."
        case .payloadPreparing:
            "Wait for attachments to finish preparing before sending."
        case .attachmentNotUploaded:
            "An attachment is not ready to send."
        case .deliveryBackpressure:
            "Too many sends are awaiting confirmation. This draft was kept."
        case .invalidTransition:
            "Composer payload state could not advance safely."
        }
    }
}

private struct GaryxComposerDurableContext: Sendable {
    let snapshot: GaryxComposerDurabilitySnapshot
    let entry: GaryxComposerPayloadEntry
}

private struct GaryxComposerSendPreparation: Sendable {
    let entryID: GaryxComposerPayloadEntryID
    let scope: GaryxGatewayScope
    let lifecycle: GaryxPayloadLifecycleCapture
    let envelopeGeneration: UInt64
    let followupGeneration: UInt64
    let reservationID: GaryxSendReservationID
    let clientIntentID: String
    let attachments: [GaryxComposerAttachment]
}

private struct GaryxComposerSendCommitResult: Sendable {
    let context: GaryxComposerDurableContext
    let delivery: GaryxComposerDeliveryHandle
}

private struct GaryxComposerAmbiguousExitResult: Sendable {
    let context: GaryxComposerDurableContext
    let envelope: GaryxDeliveryEnvelope
    let delivery: GaryxComposerDeliveryHandle?
}

fileprivate enum GaryxCreateDeliveryUpdate: Sendable {
    case begin(entryID: GaryxComposerPayloadEntryID)
    case created(threadID: String)
    case bound
    case acknowledged
    case responseLost
}

private struct GaryxCreateDeliveryCommitResult: Sendable {
    let state: GaryxCreateDeliveryState
    let context: GaryxComposerDurableContext?
}

private struct GaryxComposerFinalizedInput: Sendable {
    let context: GaryxComposerDurableContext
    let descendantKey: GaryxSessionDescendantKey
    let aliasOrigin: GaryxComposerKey
    let aliasDestination: GaryxComposerKey
}

private actor GaryxComposerPayloadPersistenceQueue {
    private let durability: GaryxSQLiteComposerDurabilityStore
    private let staging: GaryxComposerStagedAssetStore
    private let beforePrepareSendReturns: (@Sendable () async -> Void)?
    private var finalizationFailuresRemaining: Int
    private var acceptedInputSequences: [GaryxComposerInputSessionID: UInt64] = [:]
    private var launchRecoveryCompleted = false
    private var transactionGateHeld = false
    private var transactionGateWaiters: [CheckedContinuation<Void, Never>] = []

    init(
        applicationSupportDirectory: URL,
        quotaLimitBytes: Int,
        testingHooks: GaryxComposerRuntimeTestingHooks
    ) throws {
        let databaseURL = applicationSupportDirectory
            .appendingPathComponent("Garyx", isDirectory: true)
            .appendingPathComponent("ComposerPayload", isDirectory: true)
            .appendingPathComponent("composer.sqlite", isDirectory: false)
        let durability = try GaryxSQLiteComposerDurabilityStore(databaseURL: databaseURL)
        self.durability = durability
        staging = try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: applicationSupportDirectory,
            durability: durability,
            quotaLimitBytes: quotaLimitBytes
        )
        beforePrepareSendReturns = testingHooks.beforePrepareSendReturns
        finalizationFailuresRemaining = testingHooks.finalizationFailuresBeforeSuccess
    }

    func activate(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        try await runLaunchRecoveryIfNeeded(activeScope: scope)
        var snapshot = try await durability.load()
        let supersededEpochs = snapshot.scopeRegistry.lifecycles.keys
            .filter {
                $0.identity == scope.identity
                    && $0.epoch < scope.epoch
                    && snapshot.scopeRegistry.lifecycle(of: $0) != .revoked
            }
            .sorted { $0.epoch < $1.epoch }
        for superseded in supersededEpochs {
            guard let settlement = GaryxGatewayScopeSettlementPlanner.revoke(
                snapshot: snapshot,
                scope: superseded
            ) else { continue }
            snapshot = try await durability.commit(settlement.transaction)
            let recovery = GaryxComposerDurabilityLaunchRecovery(
                durability: durability,
                staging: staging,
                scopes: snapshot.scopeRegistry
            )
            _ = try await recovery.recover()
            snapshot = try await durability.load()
        }
        var scopeRegistry = snapshot.scopeRegistry
        guard scopeRegistry.switchActive(to: scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        if scopeRegistry != snapshot.scopeRegistry {
            snapshot = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "activate durable gateway scope partition",
                    mutations: [.replaceScopeRegistry(scopeRegistry)]
                )
            )
        }
        let matches = snapshot.payloadStore.entriesByScope[scope]?.values.filter {
            $0.destination == key && $0.lifecycle.phase == .active
        } ?? []
        if matches.count == 1, let entry = matches.first {
            return GaryxComposerDurableContext(snapshot: snapshot, entry: entry)
        }
        guard matches.isEmpty else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }

        let generation = try await durability.allocatePayloadGeneration()
        snapshot = try await durability.load()
        if let concurrent = snapshot.payloadStore.entriesByScope[scope]?.values.first(where: {
            $0.destination == key && $0.lifecycle.phase == .active
        }) {
            return GaryxComposerDurableContext(snapshot: snapshot, entry: concurrent)
        }
        let entryID = GaryxComposerPayloadEntryID(rawValue: UUID().uuidString)
        let entry = GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: key,
            lifecycleToken: GaryxPayloadLifecycleToken(
                entryID: entryID,
                nonce: UUID().uuidString
            ),
            currentGeneration: generation
        )
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "activate route-owned composer payload",
                mutations: [.claimGeneration(generation), .upsertEntry(entry)]
            )
        )
        return GaryxComposerDurableContext(snapshot: committed, entry: entry)
    }

    func persistText(
        context: GaryxComposerDurableContext,
        sessionID: GaryxComposerInputSessionID,
        sequence: UInt64,
        generation: UInt64,
        text: String
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        guard sequence > (acceptedInputSequences[sessionID] ?? 0) else {
            let snapshot = try await durability.load()
            guard let entry = snapshot.payloadStore.entry(
                context.entry.id,
                scope: context.entry.scope
            ) else {
                throw GaryxComposerPayloadRuntimeError.staleActivation
            }
            return GaryxComposerDurableContext(snapshot: snapshot, entry: entry)
        }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
        ), entry.lifecycle.token == context.entry.lifecycle.token,
           entry.lifecycle.phase == .active,
           generation >= entry.currentGeneration else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        entry.setText(text, generation: generation)
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist ordered composer input",
                mutations: [.upsertEntry(entry)]
            )
        )
        acceptedInputSequences[sessionID] = sequence
        return GaryxComposerDurableContext(snapshot: committed, entry: entry)
    }

    func stageAttachment(
        sourceURL: URL,
        metadata: GaryxComposerAttachmentMetadata,
        requestToken: GaryxGatewayRequestToken,
        operationContext: GaryxScopeBoundOperationContext
    ) async throws -> (GaryxComposerDurableContext, GaryxComposerStagedUpload) {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let entry = snapshot.payloadStore.entry(
            operationContext.key.entryID,
            scope: operationContext.key.scope
        ), entry.lifecycle.phase == .active,
           requestToken.scope == entry.scope else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }

        let operationKey = operationContext.key
        guard operationKey.scope == entry.scope,
              operationKey.entryID == entry.id,
              operationKey.generation == entry.currentGeneration,
              operationKey.reservationID == nil,
              operationKey.branch == .followup,
              operationContext.clientIdentity == requestToken.scope.identity,
              operationContext.configurationFingerprint == String(requestToken.activationSequence),
              operationContext.payloadLifecycle == GaryxPayloadLifecycleCapture(
                  token: entry.lifecycle.token,
                  revision: entry.lifecycle.revision
              ) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let assetID = GaryxStagedAssetID(rawValue: UUID().uuidString)
        let staged = try await staging.stage(
            .init(
                expectedRevision: snapshot.revision,
                sourceURL: sourceURL,
                assetID: assetID,
                entry: entry,
                context: operationContext
            )
        )
        guard var stagedEntry = staged.snapshot.payloadStore.entry(entry.id, scope: entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        var operation = staged.operation
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: entry.scope)
        guard operation.transition(
            expectedKey: operationKey,
            to: .uploading,
            lifecycle: stagedEntry.lifecycle.snapshot,
            scopes: scopes
        ) == .applied,
        operation.markUploadAttempted(
            expectedKey: operationKey,
            authoritativeEntry: stagedEntry,
            lifecycle: stagedEntry.lifecycle.snapshot,
            scopes: scopes
        ) == .applied else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let attachmentID = GaryxAttachmentID(rawValue: UUID().uuidString)
        stagedEntry.addAttachment(
            GaryxComposerAttachment(
                id: attachmentID,
                stagedAssetID: assetID,
                generation: stagedEntry.currentGeneration,
                byteCount: operation.reservedBytes,
                kind: metadata.kind,
                name: metadata.name,
                mediaType: metadata.mediaType,
                previewDataURL: metadata.previewDataURL
            )
        )
        let manifest = GaryxOperationManifest(
            key: operationKey,
            stagedPath: staged.manifest.stagedPath,
            state: operation.state,
            uploadAttempted: operation.uploadAttempted
        )
        let committed = try await durability.commit(
            .init(
                expectedRevision: staged.snapshot.revision,
                label: "publish staged composer attachment before upload",
                mutations: [
                    .upsertEntry(stagedEntry),
                    .upsertOperation(operation),
                    .upsertManifest(manifest),
                ]
            )
        )
        return (
            GaryxComposerDurableContext(snapshot: committed, entry: stagedEntry),
            GaryxComposerStagedUpload(
                attachmentID: attachmentID,
                operationKey: operationKey,
                fileURL: staged.fileURL,
                metadata: metadata
            )
        )
    }

    func completeUpload(
        staged: GaryxComposerStagedUpload,
        uploaded: GaryxUploadedChatAttachment
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            staged.operationKey.entryID,
            scope: staged.operationKey.scope
        ), var operation = snapshot.operations[staged.operationKey],
           let originalAttachment = entry.attachments[staged.attachmentID],
           operation.stagedAssetID == originalAttachment.stagedAssetID else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: entry.scope)
        guard operation.complete(
            expectedKey: staged.operationKey,
            authoritativeEntry: entry,
            lifecycle: entry.lifecycle.snapshot,
            scopes: scopes
        ) == .applied else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let kind = uploaded.kind.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = uploaded.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let mediaType = uploaded.mediaType.trimmingCharacters(in: .whitespacesAndNewlines)
        let path = uploaded.path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !path.isEmpty else { throw GaryxComposerPayloadRuntimeError.invalidTransition }
        entry.addAttachment(
            originalAttachment.recordingUpload(
                kind: kind.isEmpty ? staged.metadata.kind : kind,
                name: name.isEmpty ? staged.metadata.name : name,
                mediaType: mediaType.isEmpty ? staged.metadata.mediaType : mediaType,
                path: path
            )
        )
        entry.removeOperation(staged.operationKey)
        let assetID = originalAttachment.stagedAssetID
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "complete attachment upload and condemn staged file",
                mutations: [
                    .upsertEntry(entry),
                    .registerFileCleanup(assetID: assetID, owner: staged.operationKey),
                    .releaseStagedAsset(assetID),
                    .removeManifest(staged.operationKey),
                    .removeOperation(staged.operationKey),
                ]
            )
        )
        _ = try await staging.settleCondemnedFiles()
        let settled = try await durability.load()
        return GaryxComposerDurableContext(snapshot: settled, entry: entry)
    }

    func failUpload(
        staged: GaryxComposerStagedUpload
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            staged.operationKey.entryID,
            scope: staged.operationKey.scope
        ), var operation = snapshot.operations[staged.operationKey],
           let manifest = snapshot.manifests[staged.operationKey] else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: entry.scope)
        guard operation.transition(
            expectedKey: staged.operationKey,
            to: .failedRetryable,
            lifecycle: entry.lifecycle.snapshot,
            scopes: scopes
        ) == .applied else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let feedback = GaryxOperationFeedback(
            id: GaryxFeedbackID(rawValue: UUID().uuidString),
            scope: entry.scope,
            entryID: entry.id,
            operationID: staged.operationKey.operationID,
            kind: .uploadRetryable
        )
        entry.addFeedbackReference(feedback.id)
        let nextManifest = GaryxOperationManifest(
            key: staged.operationKey,
            stagedPath: manifest.stagedPath,
            state: operation.state,
            uploadAttempted: operation.uploadAttempted
        )
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "retain retryable attachment upload",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(nextManifest),
                    .upsertFeedback(feedback),
                ]
            )
        )
        return GaryxComposerDurableContext(snapshot: committed, entry: entry)
    }

    func removeAttachment(
        context: GaryxComposerDurableContext,
        attachmentID: GaryxAttachmentID
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
        ), let attachment = entry.attachments[attachmentID] else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        entry.removeAttachment(attachmentID)
        var mutations: [GaryxComposerDurabilityMutation] = [.upsertEntry(entry)]
        if let owner = snapshot.stagedAssetOwners[attachment.stagedAssetID],
           owner.entryID == entry.id {
            entry.removeOperation(owner)
            mutations[0] = .upsertEntry(entry)
            mutations.append(.registerFileCleanup(assetID: attachment.stagedAssetID, owner: owner))
            mutations.append(.releaseStagedAsset(attachment.stagedAssetID))
            mutations.append(.removeManifest(owner))
            mutations.append(.removeOperation(owner))
        }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "remove attachment from active payload",
                mutations: mutations
            )
        )
        if snapshot.stagedAssetOwners[attachment.stagedAssetID] != nil {
            _ = try await staging.settleCondemnedFiles()
        }
        let settled = try await durability.load()
        return GaryxComposerDurableContext(snapshot: settled, entry: entry)
    }

    func prepareSend(
        context: GaryxComposerDurableContext,
        clientIntentID: String
    ) async throws -> GaryxComposerSendPreparation {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        var snapshot = try await durability.load()
        guard let entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
        ), entry.lifecycle.token == context.entry.lifecycle.token else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let operations = entry.operationKeys.compactMap { snapshot.operations[$0] }
        guard GaryxComposerSendReadinessPolicy.evaluate(operations) == .ready else {
            throw GaryxComposerPayloadRuntimeError.payloadPreparing
        }
        let attachments = entry.attachments.values.sorted { $0.id.rawValue < $1.id.rawValue }
        guard attachments.allSatisfy({ $0.uploadedPath?.isEmpty == false }) else {
            throw GaryxComposerPayloadRuntimeError.attachmentNotUploaded
        }
        let envelopeBytes = GaryxDeliveryEnvelope(
            text: entry.currentText,
            attachmentIDs: attachments.map(\.id),
            attachments: attachments,
            generation: entry.currentGeneration,
            clientIntentID: clientIntentID
        ).estimatedBytes
        let quota = GaryxDeliveryQuota(rebuilding: Array(snapshot.deliveries.values))
        guard quota.canSeal(scope: entry.scope, envelopeBytes: envelopeBytes) else {
            if let feedback = GaryxDeliveryBackpressurePlanner.plan(
                snapshot: snapshot,
                entryID: entry.id,
                envelopeBytes: envelopeBytes,
                feedbackID: GaryxFeedbackID(rawValue: UUID().uuidString)
            ) {
                _ = try await durability.commit(feedback)
            }
            throw GaryxComposerPayloadRuntimeError.deliveryBackpressure
        }
        let followupGeneration = try await durability.allocatePayloadGeneration()
        let reservationID = try await durability.allocateSendReservationID()
        snapshot = try await durability.load()
        guard let current = snapshot.payloadStore.entry(entry.id, scope: entry.scope),
              current.currentGeneration == entry.currentGeneration,
              current.lifecycle.token == entry.lifecycle.token else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        if let beforePrepareSendReturns {
            await beforePrepareSendReturns()
        }
        return GaryxComposerSendPreparation(
            entryID: entry.id,
            scope: entry.scope,
            lifecycle: .init(token: entry.lifecycle.token, revision: entry.lifecycle.revision),
            envelopeGeneration: entry.currentGeneration,
            followupGeneration: followupGeneration,
            reservationID: reservationID,
            clientIntentID: clientIntentID,
            attachments: attachments
        )
    }

    func commitSend(
        _ preparation: GaryxComposerSendPreparation,
        envelopeText: String,
        provisionalText: String
    ) async throws -> GaryxComposerSendCommitResult {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            preparation.entryID,
            scope: preparation.scope
        ), entry.lifecycle.token == preparation.lifecycle.token,
           entry.lifecycle.phase == .active,
           entry.currentGeneration == preparation.envelopeGeneration else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        entry.setText(provisionalText, generation: preparation.followupGeneration)
        let lifecycle = entry.lifecycle.snapshot
        let envelope = GaryxDeliveryEnvelope(
            text: envelopeText,
            attachmentIDs: preparation.attachments.map(\.id),
            attachments: preparation.attachments,
            generation: preparation.envelopeGeneration,
            clientIntentID: preparation.clientIntentID
        )
        let quota = GaryxDeliveryQuota(rebuilding: Array(snapshot.deliveries.values))
        guard quota.canSeal(scope: preparation.scope, envelopeBytes: envelope.estimatedBytes) else {
            if let feedback = GaryxDeliveryBackpressurePlanner.plan(
                snapshot: snapshot,
                entryID: preparation.entryID,
                envelopeBytes: envelope.estimatedBytes,
                feedbackID: GaryxFeedbackID(rawValue: UUID().uuidString)
            ) {
                _ = try await durability.commit(feedback)
            }
            throw GaryxComposerPayloadRuntimeError.deliveryBackpressure
        }
        var barrier = GaryxSendCommitBarrier(
            entryID: preparation.entryID,
            scope: preparation.scope,
            payloadLifecycle: preparation.lifecycle
        )
        let deliveryID = GaryxDeliveryRecordID(rawValue: UUID().uuidString)
        guard barrier.seal(
            reservationID: preparation.reservationID,
            envelope: envelope,
            followupGeneration: preparation.followupGeneration,
            readiness: .ready,
            quota: quota,
            producerPhase: .live,
            lifecycle: lifecycle
        ) == .sealed,
        barrier.replaceProvisionalText(provisionalText, lifecycle: lifecycle),
        let settlement = barrier.durableCommit(
            deliveryID: deliveryID,
            correlationID: preparation.clientIntentID,
            clientIntentID: preparation.clientIntentID,
            lifecycle: lifecycle
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        var ledger = GaryxProvisionalReservationLedger(
            key: .init(
                scope: preparation.scope,
                entryID: preparation.entryID,
                reservationID: preparation.reservationID
            ),
            envelopeGeneration: preparation.envelopeGeneration,
            followupGeneration: preparation.followupGeneration
        )
        guard ledger.settle(.committed, targetGeneration: preparation.followupGeneration) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let send = try GaryxComposerCommitSend(
            expectedRevision: snapshot.revision,
            ledger: ledger,
            sealedPayloadEntry: entry,
            barrier: barrier,
            settlement: settlement
        )
        let committed = try await durability.commitSend(send)
        barrier.returnToIdle()
        let released = try await durability.commit(
            .init(
                expectedRevision: committed.revision,
                label: "release short-lived send barrier",
                mutations: [.upsertBarrier(barrier)]
            )
        )
        guard let updated = released.payloadStore.entry(
            preparation.entryID,
            scope: preparation.scope
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        return GaryxComposerSendCommitResult(
            context: GaryxComposerDurableContext(snapshot: released, entry: updated),
            delivery: GaryxComposerDeliveryHandle(
                deliveryID: deliveryID,
                entryID: preparation.entryID,
                scope: preparation.scope,
                reservationID: preparation.reservationID,
                lifecycle: preparation.lifecycle
            )
        )
    }

    /// Publishes the producer-drained boundary and the N+1 snapshot together.
    /// The source adapter cannot be acknowledged or released until this
    /// transaction succeeds, so process death never observes a half-close.
    func persistFinalizedInput(
        context: GaryxComposerDurableContext,
        state: GaryxComposerInputReducerState,
        reservationID: GaryxSendReservationID?
    ) async throws -> GaryxComposerFinalizedInput {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        if finalizationFailuresRemaining > 0 {
            finalizationFailuresRemaining -= 1
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        guard let drained = state.producerDrained,
              let nextEpoch = state.nextEpochSnapshot,
              state.producerPhase == .terminal,
              state.reservationPhase != .sealed,
              state.closePublicationCount == 1 else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
        ), state.session.payloadLifecycle.isAdmitted(by: entry.lifecycle.snapshot) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        entry.setText(nextEpoch.text, generation: nextEpoch.payloadGeneration)
        let key = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: state.session.sessionID,
            epoch: state.session.epoch
        )
        let durableDrained = GaryxDurableProducerDrainedRecord(
            scope: entry.scope,
            entryID: entry.id,
            reservationID: reservationID,
            record: drained
        )
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "materialize composer dual-terminal input",
                mutations: [
                    .upsertEntry(entry),
                    .upsertProducerDrained(key, durableDrained),
                ]
            )
        )
        return GaryxComposerFinalizedInput(
            context: GaryxComposerDurableContext(snapshot: committed, entry: entry),
            descendantKey: key,
            aliasOrigin: state.session.composerKey,
            aliasDestination: entry.destination
        )
    }

    /// The adapter close acknowledgement retires the durable finalizer lease.
    /// Keeping this as a second transaction makes the crash window recoverable:
    /// a retained producer-drained row means close still needs acknowledgement.
    func acknowledgeFinalizedInput(
        _ finalized: GaryxComposerFinalizedInput
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            finalized.context.entry.id,
            scope: finalized.context.entry.scope
        ) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        guard snapshot.producerDrained[finalized.descendantKey] != nil else {
            return GaryxComposerDurableContext(snapshot: snapshot, entry: entry)
        }
        var aliases = snapshot.aliases
        let retiredAliasCount = aliases.retireLineage(
            releasing: [
                GaryxComposerAliasRelease(
                    origin: finalized.aliasOrigin,
                    activeOrClosingSessions: 1
                ),
            ],
            endingAt: finalized.aliasDestination,
            scope: entry.scope
        )
        if retiredAliasCount > 0 {
            entry.setAliasReferenceCount(max(0, entry.aliasReferenceCount - retiredAliasCount))
        }
        var mutations: [GaryxComposerDurabilityMutation] = [
            .removeProducerDrained(finalized.descendantKey),
        ]
        if aliases != snapshot.aliases {
            mutations.append(.replaceAliases(aliases))
        }
        if entry != finalized.context.entry {
            mutations.append(.upsertEntry(entry))
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "acknowledge composer input close",
                mutations: mutations
            )
        )
        return GaryxComposerDurableContext(snapshot: committed, entry: entry)
    }

    func promote(
        context: GaryxComposerDurableContext,
        to destination: GaryxComposerKey
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        var store = snapshot.payloadStore
        let source = context.entry.destination
        guard store.promote(
                entryID: context.entry.id,
                scope: context.entry.scope,
                to: destination
              ),
              let entry = store.entry(context.entry.id, scope: context.entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        var promotedEntry = entry
        var aliases = snapshot.aliases
        if source != destination {
            guard aliases.establishPromotion(
                scope: context.entry.scope,
                source: source,
                target: destination,
                activeOrClosingSessions: 1
            ) == .established else {
                throw GaryxComposerPayloadRuntimeError.invalidTransition
            }
            promotedEntry.setAliasReferenceCount(promotedEntry.aliasReferenceCount + 1)
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "promote route-owned composer entry",
                mutations: [
                    .upsertEntry(promotedEntry),
                    .replaceAliases(aliases),
                ]
            )
        )
        return GaryxComposerDurableContext(snapshot: committed, entry: promotedEntry)
    }

    func markTransportAttempted(
        _ handle: GaryxComposerDeliveryHandle,
        createDeliveryKey: GaryxCreateDeliveryKey? = nil
    ) async throws -> GaryxComposerDurableContext? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var delivery = matchingDelivery(handle, in: snapshot),
              delivery.markTransportAttempted() else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        var mutations: [GaryxComposerDurabilityMutation] = [
            .upsertDelivery(delivery),
        ]
        if let createDeliveryKey {
            guard createDeliveryKey.scope == handle.scope,
                  var create = snapshot.createDeliveries[createDeliveryKey],
                  create.entryID == handle.entryID else {
                throw GaryxComposerPayloadRuntimeError.staleActivation
            }
            create.chatStartAttempted()
            guard create.phase == .chatStartAttempted else {
                throw GaryxComposerPayloadRuntimeError.invalidTransition
            }
            mutations.append(.upsertCreateDelivery(create))
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist message and create attempt before gateway dispatch",
                mutations: mutations
            )
        )
        return context(for: handle, in: committed)
    }

    func markDeliveryAmbiguous(
        _ handle: GaryxComposerDeliveryHandle
    ) async throws -> GaryxComposerDurableContext? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var delivery = matchingDelivery(handle, in: snapshot) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        if delivery.phase != .ambiguous {
            guard delivery.markAmbiguous() else {
                throw GaryxComposerPayloadRuntimeError.invalidTransition
            }
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist ambiguous gateway delivery",
                mutations: [.upsertDelivery(delivery)]
            )
        )
        return context(for: handle, in: committed)
    }

    func acknowledgeDelivery(
        _ handle: GaryxComposerDeliveryHandle
    ) async throws -> GaryxComposerDurableContext? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard var delivery = matchingDelivery(handle, in: snapshot),
              delivery.phase == .transportAttempted
                || delivery.phase == .ambiguous
                || delivery.phase == .acknowledged else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        delivery.recordServerAcknowledgement()
        var mutations: [GaryxComposerDurabilityMutation] = [.upsertDelivery(delivery)]
        if var entry = snapshot.payloadStore.entry(handle.entryID, scope: handle.scope) {
            entry.removeDeliveryReference(handle.deliveryID)
            mutations.append(.upsertEntry(entry))
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist gateway delivery acknowledgement",
                mutations: mutations
            )
        )
        return context(for: handle, in: committed)
    }

    func deliveryPhase(
        _ handle: GaryxComposerDeliveryHandle
    ) async throws -> GaryxDeliveryRecordPhase? {
        let snapshot = try await durability.load()
        return matchingDelivery(handle, in: snapshot)?.phase
    }

    func restoreAmbiguousDelivery(
        _ handle: GaryxComposerDeliveryHandle
    ) async throws -> GaryxComposerAmbiguousExitResult {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let recoveredGeneration = try await durability.allocatePayloadGeneration()
        let snapshot = try await durability.load()
        let recoveredEntryID = GaryxComposerPayloadEntryID(rawValue: UUID().uuidString)
        let conflictID = GaryxPayloadConflictSetID(
            rawValue: "delivery-\(handle.deliveryID.rawValue)"
        )
        guard let plan = GaryxDeliveryDraftRecoveryPlanner.plan(
            snapshot: snapshot,
            deliveryID: handle.deliveryID,
            recoveredEntryID: recoveredEntryID,
            recoveredLifecycleNonce: UUID().uuidString,
            recoveredGeneration: recoveredGeneration,
            conflictSetID: conflictID
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let committed = try await durability.commit(plan.transaction)
        guard let context = context(for: handle, in: committed) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return GaryxComposerAmbiguousExitResult(
            context: context,
            envelope: plan.envelope,
            delivery: nil
        )
    }

    func resendAmbiguousDelivery(
        _ handle: GaryxComposerDeliveryHandle,
        newClientIntentID: String
    ) async throws -> GaryxComposerAmbiguousExitResult {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        let duplicateID = GaryxDeliveryRecordID(rawValue: UUID().uuidString)
        guard let plan = GaryxDeliveryDuplicateResendPlanner.plan(
            snapshot: snapshot,
            deliveryID: handle.deliveryID,
            newDeliveryID: duplicateID,
            newClientIntentID: newClientIntentID
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let committed = try await durability.commit(plan.transaction)
        guard let entry = committed.payloadStore.entry(handle.entryID, scope: handle.scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let lifecycle = GaryxPayloadLifecycleCapture(
            token: entry.lifecycle.token,
            revision: entry.lifecycle.revision
        )
        return GaryxComposerAmbiguousExitResult(
            context: .init(snapshot: committed, entry: entry),
            envelope: plan.envelope,
            delivery: GaryxComposerDeliveryHandle(
                deliveryID: duplicateID,
                entryID: entry.id,
                scope: entry.scope,
                reservationID: handle.reservationID,
                lifecycle: lifecycle
            )
        )
    }

    func restoreAmbiguousCreate(
        _ key: GaryxCreateDeliveryKey
    ) async throws -> GaryxComposerAmbiguousExitResult {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let recoveredGeneration = try await durability.allocatePayloadGeneration()
        let snapshot = try await durability.load()
        let recoveredEntryID = GaryxComposerPayloadEntryID(rawValue: UUID().uuidString)
        let conflictID = GaryxPayloadConflictSetID(
            rawValue: "create-\(key.createIntentID)"
        )
        guard let plan = GaryxCreateDraftRecoveryPlanner.plan(
            snapshot: snapshot,
            key: key,
            recoveredEntryID: recoveredEntryID,
            recoveredLifecycleNonce: UUID().uuidString,
            recoveredGeneration: recoveredGeneration,
            conflictSetID: conflictID
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let committed = try await durability.commit(plan.transaction)
        guard let create = committed.createDeliveries[key],
              let entryID = create.entryID,
              let entry = committed.payloadStore.entry(entryID, scope: key.scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return GaryxComposerAmbiguousExitResult(
            context: .init(snapshot: committed, entry: entry),
            envelope: plan.envelope,
            delivery: nil
        )
    }

    func rebuildAmbiguousCreate(
        _ key: GaryxCreateDeliveryKey,
        newClientIntentID: String
    ) async throws -> (GaryxComposerAmbiguousExitResult, GaryxCreateDeliveryPhase?) {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        let duplicateID = GaryxDeliveryRecordID(rawValue: UUID().uuidString)
        guard let plan = GaryxCreateDuplicateRebuildPlanner.plan(
            snapshot: snapshot,
            key: key,
            newCreateIntentID: newClientIntentID,
            newDeliveryID: duplicateID
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let committed = try await durability.commit(plan.transaction)
        let destinationCreate = plan.newCreateKey.flatMap {
            committed.createDeliveries[$0]
        } ?? committed.createDeliveries[key]
        guard let entryID = destinationCreate?.entryID,
              let entry = committed.payloadStore.entry(entryID, scope: key.scope),
              let duplicate = committed.deliveries[duplicateID] else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let lifecycle = GaryxPayloadLifecycleCapture(
            token: entry.lifecycle.token,
            revision: entry.lifecycle.revision
        )
        return (
            GaryxComposerAmbiguousExitResult(
                context: .init(snapshot: committed, entry: entry),
                envelope: plan.envelope,
                delivery: GaryxComposerDeliveryHandle(
                    deliveryID: duplicateID,
                    entryID: entry.id,
                    scope: entry.scope,
                    reservationID: duplicate.reservationID,
                    lifecycle: lifecycle
                )
            ),
            plan.ambiguousAfter
        )
    }

    func resolveRecoveredDraft(
        conflictSetID: GaryxPayloadConflictSetID,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        host: GaryxComposerDurableContext,
        useRecovered: Bool
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let replacementGeneration: UInt64?
        if useRecovered {
            replacementGeneration = try await durability.allocatePayloadGeneration()
        } else {
            replacementGeneration = nil
        }
        let snapshot = try await durability.load()
        let plan: GaryxRecoveredDraftResolutionPlan?
        if let replacementGeneration {
            plan = GaryxRecoveredDraftResolutionPlanner.useRecoveredDraft(
                snapshot: snapshot,
                conflictSetID: conflictSetID,
                hostEntryID: host.entry.id,
                recoveredEntryID: recoveredEntryID,
                replacementGeneration: replacementGeneration
            )
        } else {
            plan = GaryxRecoveredDraftResolutionPlanner.keepCurrentDraft(
                snapshot: snapshot,
                conflictSetID: conflictSetID,
                hostEntryID: host.entry.id,
                recoveredEntryID: recoveredEntryID
            )
        }
        guard let plan else { throw GaryxComposerPayloadRuntimeError.invalidTransition }
        let committed = try await durability.commit(plan.transaction)
        guard let entry = committed.payloadStore.entry(host.entry.id, scope: host.entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return .init(snapshot: committed, entry: entry)
    }

    func ingestDeliveryEvidence(
        correlationID: String,
        authenticatedScope: GaryxGatewayScope
    ) async throws -> GaryxComposerDurableContext? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        let plan = GaryxDeliveryEvidencePlanner.plan(
            snapshot: snapshot,
            correlationID: correlationID,
            authenticatedScope: authenticatedScope
        )
        guard let transaction = plan.transaction else { return nil }
        let committed = try await durability.commit(transaction)
        guard case .updated(let deliveryID) = plan.disposition,
              let record = committed.deliveries[deliveryID],
              let entry = committed.payloadStore.entry(record.entryID, scope: record.scope) else {
            return nil
        }
        return .init(snapshot: committed, entry: entry)
    }

    func presentFeedback(
        hostEntryID: GaryxComposerPayloadEntryID,
        hasInteractionOwner: Bool
    ) async throws -> GaryxComposerDurableContext? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let transaction = GaryxFeedbackPresentationPlanner.plan(
            snapshot: snapshot,
            hostEntryID: hostEntryID,
            hasInteractionOwner: hasInteractionOwner
        ) else { return nil }
        let committed = try await durability.commit(transaction)
        guard let scoped = committed.payloadStore.entriesByScope.first(where: {
            $0.value[hostEntryID] != nil
        }), let entry = scoped.value[hostEntryID] else { return nil }
        return .init(snapshot: committed, entry: entry)
    }

    func acknowledgeFeedback(
        feedbackID: GaryxFeedbackID,
        host: GaryxComposerDurableContext
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let transaction = GaryxFeedbackAcknowledgementPlanner.plan(
            snapshot: snapshot,
            feedbackID: feedbackID,
            hostEntryID: host.entry.id
        ) else { throw GaryxComposerPayloadRuntimeError.invalidTransition }
        let committed = try await durability.commit(transaction)
        guard let entry = committed.payloadStore.entry(host.entry.id, scope: host.entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return .init(snapshot: committed, entry: entry)
    }

    func retryUpload(
        feedbackID: GaryxFeedbackID,
        host: GaryxComposerDurableContext
    ) async throws -> (GaryxComposerDurableContext, GaryxComposerStagedUpload) {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let feedback = snapshot.feedback[feedbackID],
              feedback.entryID == host.entry.id,
              feedback.scope == host.entry.scope,
              feedback.kind == .uploadRetryable,
              let operationID = feedback.operationID else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let matches = snapshot.operations.filter {
            $0.key.scope == feedback.scope
                && $0.key.entryID == feedback.entryID
                && $0.key.operationID == operationID
        }
        guard matches.count == 1,
              let (oldKey, oldOperation) = matches.first,
              let assetID = oldOperation.stagedAssetID,
              let attachment = host.entry.attachments.values.first(where: {
                  $0.stagedAssetID == assetID
              }) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let successorContext = oldOperation.context.replacingOperationID(
            GaryxOperationID(rawValue: UUID().uuidString)
        )
        let successor = GaryxOperationCapability(
            context: successorContext,
            state: .requested
        )
        guard let plan = GaryxReplacementFeedbackSwapPlanner.planRetry(
            snapshot: snapshot,
            oldOperationKey: oldKey,
            successor: successor,
            replacementID: GaryxReplacementID(rawValue: UUID().uuidString),
            feedbackID: feedbackID,
            scopes: snapshot.scopeRegistry,
            beginUpload: true
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let committed = try await durability.commit(plan.transaction)
        guard let entry = committed.payloadStore.entry(host.entry.id, scope: host.entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return (
            .init(snapshot: committed, entry: entry),
            GaryxComposerStagedUpload(
                attachmentID: attachment.id,
                operationKey: plan.successorOperation.context.key,
                fileURL: staging.rootURL.appendingPathComponent(
                    plan.successorManifest.stagedPath,
                    isDirectory: false
                ),
                metadata: GaryxComposerAttachmentMetadata(
                    kind: attachment.kind ?? "file",
                    name: attachment.name ?? "attachment",
                    mediaType: attachment.mediaType ?? "application/octet-stream",
                    previewDataURL: attachment.previewDataURL
                )
            )
        )
    }

    func removeFailedUpload(
        feedbackID: GaryxFeedbackID,
        host: GaryxComposerDurableContext
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let feedback = snapshot.feedback[feedbackID],
              feedback.entryID == host.entry.id,
              feedback.scope == host.entry.scope,
              let operationID = feedback.operationID else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let matchingKeys = snapshot.operations.keys.filter {
            $0.scope == feedback.scope
                && $0.entryID == feedback.entryID
                && $0.operationID == operationID
        }
        guard matchingKeys.count == 1,
              let operationKey = matchingKeys.first,
              let plan = GaryxOperationRemovalFeedbackPlanner.plan(
                snapshot: snapshot,
                operationKey: operationKey,
                feedbackID: feedbackID,
                scopes: snapshot.scopeRegistry
              ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        _ = try await durability.commit(plan.transaction)
        let settled = try await staging.settleCondemnedFiles()
        guard let entry = settled.payloadStore.entry(host.entry.id, scope: host.entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return .init(snapshot: settled, entry: entry)
    }

    func suspendScope(_ scope: GaryxGatewayScope) async throws {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        var registry = snapshot.scopeRegistry
        guard registry.activeScope == scope else { return }
        _ = registry.suspendActive()
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "suspend durable gateway scope partition",
                mutations: [.replaceScopeRegistry(registry)]
            )
        )
    }

    func revokeScope(_ scope: GaryxGatewayScope) async throws {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let plan = GaryxGatewayScopeSettlementPlanner.revoke(
            snapshot: snapshot,
            scope: scope
        ) else { return }
        let committed = try await durability.commit(plan.transaction)
        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: durability,
            staging: staging,
            scopes: committed.scopeRegistry
        )
        _ = try await recovery.recover()
    }

    func updateCreateDelivery(
        key: GaryxCreateDeliveryKey,
        update: GaryxCreateDeliveryUpdate
    ) async throws -> GaryxCreateDeliveryCommitResult {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        var state: GaryxCreateDeliveryState
        switch update {
        case .begin(let entryID):
            if let existing = snapshot.createDeliveries[key] {
                let context = existing.entryID.flatMap {
                    snapshot.payloadStore.entry($0, scope: key.scope)
                }.map { GaryxComposerDurableContext(snapshot: snapshot, entry: $0) }
                return .init(state: existing, context: context)
            }
            let scoped = snapshot.createDeliveries.values.filter {
                $0.scope == key.scope && !$0.isTerminalCorrelation
            }
            let global = snapshot.createDeliveries.values.filter { !$0.isTerminalCorrelation }
            guard scoped.count < GaryxCreateDeliveryState.nonTerminalPerScopeLimit,
                  global.count < GaryxCreateDeliveryState.nonTerminalGlobalLimit else {
                throw GaryxComposerPayloadRuntimeError.deliveryBackpressure
            }
            state = GaryxCreateDeliveryState(
                scope: key.scope,
                createIntentID: key.createIntentID,
                entryID: entryID
            )
        case .created(let threadID):
            guard var current = snapshot.createDeliveries[key] else {
                throw GaryxComposerPayloadRuntimeError.staleActivation
            }
            current.created(threadID: threadID)
            state = current
        case .bound:
            guard var current = snapshot.createDeliveries[key] else {
                throw GaryxComposerPayloadRuntimeError.staleActivation
            }
            current.bound()
            state = current
        case .acknowledged:
            guard var current = snapshot.createDeliveries[key] else {
                throw GaryxComposerPayloadRuntimeError.staleActivation
            }
            current.acknowledged()
            state = current
        case .responseLost:
            guard var current = snapshot.createDeliveries[key] else {
                throw GaryxComposerPayloadRuntimeError.staleActivation
            }
            current.responseLost()
            state = current
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "advance durable multi-stage conversation creation",
                mutations: [.upsertCreateDelivery(state)]
            )
        )
        let persisted = committed.createDeliveries[key] ?? state
        let context = persisted.entryID.flatMap {
            committed.payloadStore.entry($0, scope: key.scope)
        }.map { GaryxComposerDurableContext(snapshot: committed, entry: $0) }
        return .init(state: persisted, context: context)
    }

    func createDeliveryPhase(
        for key: GaryxCreateDeliveryKey
    ) async throws -> GaryxCreateDeliveryPhase? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        return try await durability.load().createDeliveries[key]?.phase
    }

    func discardReclaimable(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey
    ) async throws -> GaryxComposerPayloadEntryID? {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        let matches = snapshot.payloadStore.entriesByScope[scope]?.values.filter {
            $0.destination == key && $0.lifecycle.phase == .active
        } ?? []
        guard matches.count <= 1 else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        guard let entry = matches.first, entry.isReclaimable else { return nil }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "reclaim empty composer entry",
                mutations: [.removeEntry(scope: scope, entryID: entry.id)]
            )
        )
        return entry.id
    }

    private func runLaunchRecoveryIfNeeded(activeScope: GaryxGatewayScope) async throws {
        guard !launchRecoveryCompleted else { return }
        let snapshot = try await durability.load()
        var knownScopes = Set(snapshot.payloadStore.entriesByScope.keys)
        knownScopes.formUnion(snapshot.operations.keys.map(\.scope))
        knownScopes.formUnion(snapshot.manifests.keys.map(\.scope))
        knownScopes.formUnion(snapshot.ledgers.keys.map(\.scope))
        knownScopes.formUnion(snapshot.deliveries.values.map(\.scope))
        knownScopes.insert(activeScope)

        var recoveryScopes = snapshot.scopeRegistry
        for scope in knownScopes.sorted(by: Self.scopeSort) {
            _ = recoveryScopes.switchActive(to: scope)
        }
        _ = recoveryScopes.switchActive(to: activeScope)
        let recovery = GaryxComposerDurabilityLaunchRecovery(
            durability: durability,
            staging: staging,
            scopes: recoveryScopes
        )
        _ = try await recovery.recover()
        try await settleInputResidueAfterProcessDeath()
        launchRecoveryCompleted = true
    }

    /// UIKit sessions cannot survive a process boundary. Once Core has
    /// converged every durable reservation/discard, acknowledge the remaining
    /// materialized close rows and release their promotion aliases in one
    /// transaction. The payload text was committed with producerDrained, so
    /// this removes only dead host/session ownership.
    private func settleInputResidueAfterProcessDeath() async throws {
        let snapshot = try await durability.load()
        var aliases = snapshot.aliases
        for scope in aliases.partitions.keys.sorted(by: Self.scopeSort) {
            let sources = aliases.partitions[scope]?.keys.sorted(by: Self.composerKeySort) ?? []
            for source in sources {
                _ = aliases.markDrained(source: source, scope: scope)
            }
        }

        var mutations: [GaryxComposerDurabilityMutation] = []
        if aliases != snapshot.aliases {
            mutations.append(.replaceAliases(aliases))
        }
        for scope in snapshot.payloadStore.entriesByScope.keys.sorted(by: Self.scopeSort) {
            let entries = snapshot.payloadStore.entriesByScope[scope]?.values.sorted {
                $0.id.rawValue < $1.id.rawValue
            } ?? []
            for var entry in entries where entry.aliasReferenceCount > 0 {
                entry.setAliasReferenceCount(0)
                mutations.append(.upsertEntry(entry))
            }
        }
        for key in snapshot.producerDrained.keys.sorted(by: Self.sessionKeySort) {
            mutations.append(.removeProducerDrained(key))
        }
        guard !mutations.isEmpty else { return }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "settle process-death composer input sessions",
                mutations: mutations
            )
        )
    }

    private static func scopeSort(_ lhs: GaryxGatewayScope, _ rhs: GaryxGatewayScope) -> Bool {
        lhs.identity == rhs.identity
            ? lhs.epoch < rhs.epoch
            : lhs.identity < rhs.identity
    }

    private static func composerKeySort(_ lhs: GaryxComposerKey, _ rhs: GaryxComposerKey) -> Bool {
        func value(_ key: GaryxComposerKey) -> String {
            switch key {
            case .draft(let id): "draft:\(id)"
            case .thread(let id): "thread:\(id)"
            }
        }
        return value(lhs) < value(rhs)
    }

    private static func sessionKeySort(
        _ lhs: GaryxSessionDescendantKey,
        _ rhs: GaryxSessionDescendantKey
    ) -> Bool {
        if lhs.token.entryID != rhs.token.entryID {
            return lhs.token.entryID.rawValue < rhs.token.entryID.rawValue
        }
        if lhs.sessionID != rhs.sessionID {
            return lhs.sessionID.rawValue < rhs.sessionID.rawValue
        }
        return lhs.epoch < rhs.epoch
    }

    private func matchingDelivery(
        _ handle: GaryxComposerDeliveryHandle,
        in snapshot: GaryxComposerDurabilitySnapshot
    ) -> GaryxDeliveryRecord? {
        guard let delivery = snapshot.deliveries[handle.deliveryID],
              delivery.entryID == handle.entryID,
              delivery.scope == handle.scope,
              delivery.reservationID == handle.reservationID else {
            return nil
        }
        return delivery
    }

    private func context(
        for handle: GaryxComposerDeliveryHandle,
        in snapshot: GaryxComposerDurabilitySnapshot
    ) -> GaryxComposerDurableContext? {
        guard let entry = snapshot.payloadStore.entry(handle.entryID, scope: handle.scope),
              entry.lifecycle.token == handle.lifecycle.token else { return nil }
        return GaryxComposerDurableContext(snapshot: snapshot, entry: entry)
    }

    private func acquireTransactionGate() async {
        if !transactionGateHeld {
            transactionGateHeld = true
            return
        }
        await withCheckedContinuation { continuation in
            transactionGateWaiters.append(continuation)
        }
    }

    private func releaseTransactionGate() {
        guard !transactionGateWaiters.isEmpty else {
            transactionGateHeld = false
            return
        }
        transactionGateWaiters.removeFirst().resume()
    }
}

@MainActor
final class GaryxComposerPayloadCoordinator: ObservableObject {
    struct Snapshot: Equatable {
        var projection: GaryxComposerPayloadProjection?
        var notices: [GaryxComposerDurableNotice] = []
        var isReadOnly = true
        var revision: UInt64 = 0

        static let unavailable = Self()
    }

    @Published private(set) var snapshot = Snapshot.unavailable
    private(set) var finalizationFailureDescription: String?

    private let persistence: GaryxComposerPayloadPersistenceQueue
    private var durableContext: GaryxComposerDurableContext?
    private var activationTicket: UInt64 = 0
    private var inputState: GaryxComposerInputReducerState?
    private var inputEpochByEntry: [GaryxComposerPayloadEntryID: UInt64] = [:]
    private var routeProjections: [ScopedComposerKey: GaryxComposerPayloadProjection] = [:]
    private var scopes = GaryxGatewayScopeRegistry()
    private var adapters: [GaryxRouteInstanceID: WeakAdapter] = [:]
    private var liveOccurrenceID: GaryxRouteInstanceID?
    private var routeActivation: RouteActivation?
    private var finalizationTask: Task<Void, Never>?
    private var sendCommitInFlightSessionID: GaryxComposerInputSessionID?
    private var feedbackPresentationTask: Task<Void, Never>?
    private var pendingActivation: (scope: GaryxGatewayScope, key: GaryxComposerKey, ticket: UInt64)?
    private var deferredScopeRevocation: GaryxGatewayScope?
    private var deferredRouteActivation: (
        scope: GaryxGatewayScope,
        key: GaryxComposerKey,
        occurrenceID: GaryxRouteInstanceID,
        restoresFocus: Bool
    )?
    private let focusCoordinator = GaryxRouteFocusCoordinator()
    private var sceneIsActive = true

    init(
        applicationSupportDirectory: URL,
        quotaLimitBytes: Int = 100 * 1_024 * 1_024,
        testingHooks: GaryxComposerRuntimeTestingHooks = .init()
    ) throws {
        persistence = try GaryxComposerPayloadPersistenceQueue(
            applicationSupportDirectory: applicationSupportDirectory,
            quotaLimitBytes: quotaLimitBytes,
            testingHooks: testingHooks
        )
    }

    static func production() -> GaryxComposerPayloadCoordinator {
        do {
            let root = try FileManager.default.url(
                for: .applicationSupportDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: true
            )
            return try GaryxComposerPayloadCoordinator(applicationSupportDirectory: root)
        } catch {
            preconditionFailure("composer durability initialization failed: \(error)")
        }
    }

    var currentText: String { snapshot.projection?.text ?? "" }
    var currentAttachments: [GaryxComposerAttachment] { snapshot.projection?.attachments ?? [] }
    var durableNotices: [GaryxComposerDurableNotice] { snapshot.notices }
    var canSend: Bool { snapshot.projection?.readiness == .ready && !snapshot.isReadOnly }
    var activeKey: GaryxComposerKey? { snapshot.projection?.key }

    func projection(forRouteKey key: GaryxComposerKey) -> GaryxComposerPayloadProjection? {
        if adapterKeyMatchesActiveSession(key) {
            return snapshot.projection
        }
        guard let scope = scopes.activeScope else { return nil }
        return routeProjections[ScopedComposerKey(scope: scope, key: key)]
    }

    func activate(scope: GaryxGatewayScope, key: GaryxComposerKey) async {
        activationTicket &+= 1
        let ticket = activationTicket
        guard routeActivation == nil, finalizationTask == nil else {
            pendingActivation = (scope, key, ticket)
            snapshot.isReadOnly = true
            return
        }
        await performActivation(scope: scope, key: key, ticket: ticket)
    }

    private func performActivation(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey,
        ticket: UInt64
    ) async {
        _ = scopes.switchActive(to: scope)
        snapshot.isReadOnly = true
        do {
            let context = try await persistence.activate(scope: scope, key: key)
            guard ticket == activationTicket else { return }
            durableContext = context
            installInputState(for: context.entry)
            publish(context: context, readOnly: !sceneIsActive)
            grantCurrentConfigurationToLiveAdapter()
            grantPendingFocusIfReady()
        } catch {
            guard ticket == activationTicket else { return }
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
        }
    }

    func suspendScope(_ scope: GaryxGatewayScope) {
        guard scopes.activeScope == scope else { return }
        _ = scopes.suspendActive()
        Task { try? await persistence.suspendScope(scope) }
        activationTicket &+= 1
        pendingActivation = nil
        if routeActivation == nil, finalizationTask == nil {
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
        } else {
            snapshot.isReadOnly = true
        }
    }

    func revokeScope(_ scope: GaryxGatewayScope) {
        if routeActivation != nil || finalizationTask != nil {
            deferredScopeRevocation = scope
            activationTicket &+= 1
            pendingActivation = nil
            snapshot.isReadOnly = true
            return
        }
        completeScopeRevocation(scope)
    }

    private func completeScopeRevocation(_ scope: GaryxGatewayScope) {
        _ = scopes.revoke(scope)
        Task { try? await persistence.revokeScope(scope) }
        if durableContext?.entry.scope == scope {
            activationTicket &+= 1
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
        }
    }

    func acceptText(_ text: String, identity: GaryxComposerInputEventIdentity) {
        guard var state = inputState,
              let context = durableContext else { return }
        let disposition = state.applyText(
            text,
            identity: identity,
            aliases: context.snapshot.aliases,
            lifecycle: context.entry.lifecycle.snapshot,
            scopes: scopes
        )
        guard case .applied(_, let generation) = disposition else { return }
        inputState = state
        var entry = context.entry
        entry.setText(text, generation: generation)
        let localContext = GaryxComposerDurableContext(snapshot: context.snapshot, entry: entry)
        durableContext = localContext
        publish(context: localContext, readOnly: snapshot.isReadOnly)
        if sendCommitInFlightSessionID == state.session.sessionID {
            return
        }
        Task {
            do {
                let committed = try await persistence.persistText(
                    context: context,
                    sessionID: state.session.sessionID,
                    sequence: identity.inputSequence,
                    generation: generation,
                    text: text
                )
                guard let currentState = self.inputState,
                      currentState.session.sessionID == state.session.sessionID else { return }
                if currentState.lastAppliedSequence == identity.inputSequence {
                    self.durableContext = committed
                    self.publish(context: committed, readOnly: self.snapshot.isReadOnly)
                } else if currentState.lastAppliedSequence > identity.inputSequence,
                          let current = self.durableContext,
                          current.entry.id == committed.entry.id,
                          current.entry.lifecycle.token == committed.entry.lifecycle.token {
                    // The persistence actor can return while UIKit has already
                    // admitted a newer sequence. Advance the durable revision
                    // without projecting the older text back into the editor.
                    self.durableContext = GaryxComposerDurableContext(
                        snapshot: committed.snapshot,
                        entry: current.entry
                    )
                }
            } catch {
                // The input remains visible; the next ordered event retries from
                // the authoritative Entry. A stale activation is ignored.
            }
        }
    }

    func register(
        _ adapter: GaryxComposerInputAdapter,
        isCanonicalTop: Bool
    ) {
        adapters[adapter.occurrenceID] = WeakAdapter(adapter)
        if isCanonicalTop {
            if let previousOccurrenceID = liveOccurrenceID,
               previousOccurrenceID != adapter.occurrenceID {
                adapters[previousOccurrenceID]?.value?.makeReadOnly()
            }
            liveOccurrenceID = adapter.occurrenceID
            if let configuration = inputConfiguration(),
               adapterKeyMatchesActiveSession(adapter.composerKey) {
                adapter.grantLive(configuration)
            } else {
                adapter.makeReadOnly()
            }
            grantPendingFocusIfReady()
            scheduleFeedbackPresentationIfNeeded()
        } else {
            adapter.makeReadOnly()
        }
        pruneAdapters()
    }

    func unregister(_ adapter: GaryxComposerInputAdapter) {
        if routeActivation?.sourceOccurrenceID == adapter.occurrenceID {
            cancelPendingInput(.hostTeardown)
        }
        if adapters[adapter.occurrenceID]?.value === adapter {
            adapters.removeValue(forKey: adapter.occurrenceID)
        }
        if liveOccurrenceID == adapter.occurrenceID {
            liveOccurrenceID = nil
        }
    }

    func replaceLiveText(_ text: String) {
        guard let liveOccurrenceID,
              let adapter = adapters[liveOccurrenceID]?.value else { return }
        adapter.replaceLiveText(text)
    }

    /// Same-main-actor seam called by the route container immediately before
    /// its canonical path mutation. The active UIKit adapter commits marked
    /// text first and returns the exact final sequence; Core then freezes the
    /// session under its producer lease in this call stack.
    func routeCommitReleased(
        sourceOccurrenceID: GaryxRouteInstanceID?,
        sourceKey: GaryxComposerKey?,
        destinationOccurrenceID: GaryxRouteInstanceID?,
        destinationKey: GaryxComposerKey?
    ) {
        var activation = GaryxComposerHostActivation(
            sourceKey: sourceKey,
            destinationKey: destinationKey
        )
        guard let sourceKey else {
            routeActivation = RouteActivation(
                sourceOccurrenceID: sourceOccurrenceID,
                destinationOccurrenceID: destinationOccurrenceID,
                activation: activation,
                terminal: nil,
                reservationIDAtRelease: nil,
                sourceWasFocused: false,
                sourceRequiresFinalization: false
            )
            return
        }
        guard activation.commitReleased() else { return }

        let close = sourceOccurrenceID
            .flatMap { adapters[$0]?.value }
            .map { $0.finalizeInput() }
        // finalizeInput synchronously publishes unmarked text. Re-read only
        // after that callback; a temporarily absent host uses the reducer's
        // exact current sequence and an empty producer set as a virtual close.
        guard var state = inputState,
              let context = durableContext,
              activeSession(state, context: context, resolvesTo: sourceKey) else {
            assertionFailure("live adapter lost its durable input session at release")
            activation.producerAndReservationReachedTerminal()
            activation.closeAcknowledged()
            routeActivation = RouteActivation(
                sourceOccurrenceID: sourceOccurrenceID,
                destinationOccurrenceID: destinationOccurrenceID,
                activation: activation,
                terminal: nil,
                reservationIDAtRelease: nil,
                sourceWasFocused: false,
                sourceRequiresFinalization: false
            )
            return
        }
        if let close, close.finalSequence != state.lastAppliedSequence {
            assertionFailure("UIKit close must carry the exact final input sequence")
            finalizationFailureDescription = "UIKit final sequence mismatch"
        }
        let release = state.releaseForCommittedNavigation(
            pendingProducers: close?.pendingProducers ?? [],
            lifecycle: context.entry.lifecycle.snapshot,
            scopes: scopes
        )
        guard release == .released else {
            assertionFailure("committed route release must freeze the live adapter")
            return
        }
        inputState = state
        snapshot.isReadOnly = true
        liveOccurrenceID = nil
        routeActivation = RouteActivation(
            sourceOccurrenceID: sourceOccurrenceID,
            destinationOccurrenceID: destinationOccurrenceID,
            activation: activation,
            terminal: nil,
            reservationIDAtRelease: state.activeReservationID,
            sourceWasFocused: close?.wasFocused ?? false,
            sourceRequiresFinalization: true
        )
        advanceRouteActivationIfReady()
    }

    func routeReachedTerminal(_ terminal: GaryxPresentationTerminalState) {
        guard var routeActivation else { return }
        if terminal.outcome == .cancelled {
            routeActivation.activation.cancelled()
            self.routeActivation = nil
            if let sourceOccurrenceID = routeActivation.sourceOccurrenceID {
                liveOccurrenceID = sourceOccurrenceID
            }
            snapshot.isReadOnly = false
            grantCurrentConfigurationToLiveAdapter()
            return
        }
        routeActivation.terminal = terminal
        self.routeActivation = routeActivation
        cancelPendingInput(
            terminal.visibility == .superseded
                ? .superseded
                : .transactionSettleTerminal
        )
        advanceRouteActivationIfReady()
    }

    func sceneDidBecomeInactive() {
        sceneIsActive = false
        snapshot.isReadOnly = true
        if routeActivation == nil,
           let liveOccurrenceID,
           let adapter = adapters[liveOccurrenceID]?.value {
            adapter.makeReadOnly()
        }
    }

    func sceneDidBecomeActive() {
        sceneIsActive = true
        if let deferred = deferredRouteActivation {
            deferredRouteActivation = nil
            liveOccurrenceID = deferred.occurrenceID
            if deferred.restoresFocus {
                focusCoordinator.deferFocus(to: deferred.occurrenceID)
            }
            Task {
                await activate(scope: deferred.scope, key: deferred.key)
            }
            return
        }
        if routeActivation == nil, durableContext != nil {
            snapshot.isReadOnly = false
            grantCurrentConfigurationToLiveAdapter()
            grantPendingFocusIfReady()
        }
    }

    func producerReachedTerminal(
        _ producer: GaryxInputProducerKind,
        occurrenceID: GaryxRouteInstanceID
    ) {
        guard routeActivation?.sourceOccurrenceID == occurrenceID,
              var state = inputState,
              let context = durableContext else { return }
        _ = state.producerReachedTerminal(
            producer,
            lifecycle: context.entry.lifecycle.snapshot,
            scopes: scopes
        )
        inputState = state
        advanceRouteActivationIfReady()
    }

    func cancelPendingInput(_ reason: GaryxInputProducerCancellation) {
        guard var state = inputState,
              let context = durableContext else { return }
        _ = state.cancelPendingProducers(
            reason,
            lifecycle: context.entry.lifecycle.snapshot,
            scopes: scopes
        )
        inputState = state
        advanceRouteActivationIfReady()
    }

    /// Scope replacement must retire the live UIKit session even though no
    /// route topology changes. It uses the same ordered release seam as a
    /// navigation commit, then deterministically terminates every producer.
    func terminateActiveInputForScopeExit(
        _ reason: GaryxInputProducerCancellation,
        visibility: GaryxPresentationVisibility = .superseded
    ) {
        guard let occurrenceID = liveOccurrenceID,
              let key = inputState?.session.composerKey else {
            cancelPendingInput(reason)
            return
        }
        routeCommitReleased(
            sourceOccurrenceID: occurrenceID,
            sourceKey: key,
            destinationOccurrenceID: nil,
            destinationKey: nil
        )
        cancelPendingInput(reason)
        routeReachedTerminal(
            GaryxPresentationTerminalState(outcome: .committed, visibility: visibility)
        )
    }

    func stageAttachment(
        sourceURL: URL,
        metadata: GaryxComposerAttachmentMetadata,
        requestToken: GaryxGatewayRequestToken
    ) async throws -> GaryxComposerStagedUpload {
        guard let operationContext = makePresentationOperationContext(
            requestToken: requestToken
        ) else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        return try await stageAttachment(
            sourceURL: sourceURL,
            metadata: metadata,
            requestToken: requestToken,
            operationContext: operationContext
        )
    }

    func stageAttachment(
        sourceURL: URL,
        metadata: GaryxComposerAttachmentMetadata,
        requestToken: GaryxGatewayRequestToken,
        operationContext: GaryxScopeBoundOperationContext
    ) async throws -> GaryxComposerStagedUpload {
        let (updated, staged) = try await persistence.stageAttachment(
            sourceURL: sourceURL,
            metadata: metadata,
            requestToken: requestToken,
            operationContext: operationContext
        )
        if durableContext?.entry.id == updated.entry.id {
            durableContext = updated
            publish(context: updated, readOnly: snapshot.isReadOnly)
            grantCurrentConfigurationToLiveAdapter()
        }
        return staged
    }

    /// Frozen synchronously with a result-bearing presentation lease, before
    /// SwiftUI observes the picker/camera request. The eventual file operation
    /// must present this exact capability back to the durability boundary.
    func makePresentationOperationContext(
        requestToken: GaryxGatewayRequestToken
    ) -> GaryxScopeBoundOperationContext? {
        guard let context = durableContext,
              context.entry.scope == requestToken.scope,
              context.entry.lifecycle.phase == .active else { return nil }
        return GaryxScopeBoundOperationContext(
            key: GaryxOperationCapabilityKey(
                scope: context.entry.scope,
                entryID: context.entry.id,
                generation: context.entry.currentGeneration,
                reservationID: nil,
                branch: .followup,
                operationID: GaryxOperationID(rawValue: UUID().uuidString)
            ),
            clientIdentity: requestToken.scope.identity,
            configurationFingerprint: String(requestToken.activationSequence),
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: context.entry.lifecycle.token,
                revision: context.entry.lifecycle.revision
            )
        )
    }

    func completeUpload(
        _ staged: GaryxComposerStagedUpload,
        uploaded: GaryxUploadedChatAttachment
    ) async throws {
        guard scopes.lifecycle(of: staged.operationKey.scope) != .revoked else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let updated = try await persistence.completeUpload(
            staged: staged,
            uploaded: uploaded
        )
        guard durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func failUpload(_ staged: GaryxComposerStagedUpload) async {
        guard scopes.lifecycle(of: staged.operationKey.scope) != .revoked,
              let updated = try? await persistence.failUpload(staged: staged),
              durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func removeAttachment(_ id: GaryxAttachmentID) async throws {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let updated = try await persistence.removeAttachment(context: context, attachmentID: id)
        guard durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func takeReadyPayload(clientIntentID: String) async throws -> GaryxComposerReadyPayload {
        guard let initialContext = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let preparation: GaryxComposerSendPreparation
        do {
            preparation = try await persistence.prepareSend(
                context: initialContext,
                clientIntentID: clientIntentID
            )
        } catch {
            let restored = try? await persistence.activate(
                scope: initialContext.entry.scope,
                key: initialContext.entry.destination
            )
            if let restored {
                durableContext = restored
                installInputState(for: restored.entry)
                publish(context: restored, readOnly: false)
                grantCurrentConfigurationToLiveAdapter()
            }
            throw error
        }
        // `prepareSend` allocates durable identities off-main-actor. Re-read
        // the reducer afterwards so ordered UIKit events admitted during that
        // suspension are part of the envelope linearized by `beginSend`.
        guard let context = durableContext,
              context.entry.id == preparation.entryID,
              var state = inputState,
              state.session.payloadLifecycle == preparation.lifecycle else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let envelopeText = state.currentText
        let sequenceAtSeal = state.lastAppliedSequence
        guard state.beginSend(
            reservationID: preparation.reservationID,
            followupGeneration: preparation.followupGeneration,
            lifecycle: context.entry.lifecycle.snapshot,
            scopes: scopes
        ) == .sealed(
            envelope: envelopeText,
            followupGeneration: preparation.followupGeneration
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let sessionID = state.session.sessionID
        inputState = state
        sendCommitInFlightSessionID = sessionID
        snapshot.isReadOnly = false
        grantCurrentConfigurationToLiveAdapter()
        let committed: GaryxComposerSendCommitResult
        do {
            committed = try await persistence.commitSend(
                preparation,
                envelopeText: envelopeText,
                provisionalText: state.textByGeneration[preparation.followupGeneration] ?? ""
            )
        } catch {
            sendCommitInFlightSessionID = nil
            // Storage did not publish the barrier. Restore from the authoritative
            // Entry rather than leaving the adapter stranded in a sealed window.
            let restorationKey = durableContext?.entry.destination ?? context.entry.destination
            let restored = try await persistence.activate(
                scope: context.entry.scope,
                key: restorationKey
            )
            durableContext = restored
            installInputState(for: restored.entry)
            publish(context: restored, readOnly: false)
            grantCurrentConfigurationToLiveAdapter()
            throw error
        }
        let updated = committed.context
        guard durableContext?.entry.id == updated.entry.id,
              var settledState = inputState,
              settledState.session.sessionID == sessionID,
              settledState.commitReservation(
                lifecycle: updated.entry.lifecycle.snapshot,
                scopes: scopes
              ), settledState.returnReservationToIdle(
                lifecycle: updated.entry.lifecycle.snapshot,
                scopes: scopes
              ) else {
            sendCommitInFlightSessionID = nil
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }

        let followupSequence = settledState.lastAppliedSequence
        let followupGeneration = settledState.currentGeneration
        let followupText = settledState.currentText
        var locallyProjectedEntry = updated.entry
        locallyProjectedEntry.setText(followupText, generation: followupGeneration)
        let localContext = GaryxComposerDurableContext(
            snapshot: updated.snapshot,
            entry: locallyProjectedEntry
        )
        durableContext = localContext
        inputState = settledState
        sendCommitInFlightSessionID = nil
        publish(context: localContext, readOnly: false)
        grantCurrentConfigurationToLiveAdapter()

        if followupSequence > sequenceAtSeal {
            do {
                let flushed = try await persistence.persistText(
                    context: updated,
                    sessionID: sessionID,
                    sequence: followupSequence,
                    generation: followupGeneration,
                    text: followupText
                )
                if inputState?.session.sessionID == sessionID,
                   inputState?.lastAppliedSequence == followupSequence {
                    durableContext = flushed
                    publish(context: flushed, readOnly: false)
                }
            } catch {
                // The follow-up remains visible. Its next ordered event retries
                // the same generation while the sealed envelope stays durable.
            }
        }
        advanceRouteActivationIfReady()
        return GaryxComposerReadyPayload(
            text: envelopeText,
            attachments: preparation.attachments,
            clientIntentID: preparation.clientIntentID,
            delivery: committed.delivery
        )
    }

    func markTransportAttempted(
        _ delivery: GaryxComposerDeliveryHandle,
        createDeliveryKey: GaryxCreateDeliveryKey? = nil
    ) async throws {
        if let updated = try await persistence.markTransportAttempted(
            delivery,
            createDeliveryKey: createDeliveryKey
        ),
           durableContext?.entry.id == updated.entry.id {
            durableContext = updated
            publish(context: updated, readOnly: snapshot.isReadOnly)
        }
    }

    func markDeliveryAmbiguous(_ delivery: GaryxComposerDeliveryHandle) async throws {
        if let updated = try await persistence.markDeliveryAmbiguous(delivery),
           durableContext?.entry.id == updated.entry.id {
            durableContext = updated
            publish(context: updated, readOnly: snapshot.isReadOnly)
        }
    }

    func acknowledgeDelivery(_ delivery: GaryxComposerDeliveryHandle) async throws {
        if let updated = try await persistence.acknowledgeDelivery(delivery),
           durableContext?.entry.id == updated.entry.id {
            durableContext = updated
            publish(context: updated, readOnly: snapshot.isReadOnly)
        }
    }

    func ingestDeliveryEvidence(
        correlationID: String,
        authenticatedScope: GaryxGatewayScope
    ) async {
        guard !correlationID.isEmpty else { return }
        guard let updated = try? await persistence.ingestDeliveryEvidence(
            correlationID: correlationID,
            authenticatedScope: authenticatedScope
        ), durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func restoreAmbiguousDelivery(
        _ deliveryID: GaryxDeliveryRecordID
    ) async throws -> GaryxDeliveryEnvelope {
        guard let context = durableContext,
              let record = context.snapshot.deliveries[deliveryID] else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let handle = GaryxComposerDeliveryHandle(
            deliveryID: deliveryID,
            entryID: record.entryID,
            scope: record.scope,
            reservationID: record.reservationID,
            lifecycle: .init(
                token: context.entry.lifecycle.token,
                revision: context.entry.lifecycle.revision
            )
        )
        let result = try await persistence.restoreAmbiguousDelivery(handle)
        guard durableContext?.entry.id == result.context.entry.id else {
            return result.envelope
        }
        durableContext = result.context
        publish(context: result.context, readOnly: snapshot.isReadOnly)
        return result.envelope
    }

    func resendAmbiguousDelivery(
        _ deliveryID: GaryxDeliveryRecordID
    ) async throws -> (payload: GaryxComposerReadyPayload, originalClientIntentID: String) {
        guard let context = durableContext,
              let record = context.snapshot.deliveries[deliveryID] else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let handle = GaryxComposerDeliveryHandle(
            deliveryID: deliveryID,
            entryID: record.entryID,
            scope: record.scope,
            reservationID: record.reservationID,
            lifecycle: .init(
                token: context.entry.lifecycle.token,
                revision: context.entry.lifecycle.revision
            )
        )
        let newIntentID = "mobile-\(UUID().uuidString)"
        let result = try await persistence.resendAmbiguousDelivery(
            handle,
            newClientIntentID: newIntentID
        )
        guard let duplicate = result.delivery else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        if durableContext?.entry.id == result.context.entry.id {
            durableContext = result.context
            publish(context: result.context, readOnly: snapshot.isReadOnly)
        }
        return (
            GaryxComposerReadyPayload(
                text: result.envelope.text,
                attachments: result.envelope.attachments,
                clientIntentID: newIntentID,
                delivery: duplicate
            ),
            record.correlationID
        )
    }

    func restoreAmbiguousCreate(
        _ key: GaryxCreateDeliveryKey
    ) async throws -> GaryxDeliveryEnvelope {
        let result = try await persistence.restoreAmbiguousCreate(key)
        guard durableContext?.entry.id == result.context.entry.id else {
            return result.envelope
        }
        durableContext = result.context
        publish(context: result.context, readOnly: snapshot.isReadOnly)
        return result.envelope
    }

    func rebuildAmbiguousCreate(
        _ key: GaryxCreateDeliveryKey
    ) async throws -> GaryxComposerCreateRebuildPayload {
        let newIntentID = "mobile-\(UUID().uuidString)"
        let (result, ambiguousAfter) = try await persistence.rebuildAmbiguousCreate(
            key,
            newClientIntentID: newIntentID
        )
        guard let duplicate = result.delivery else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        if durableContext?.entry.id == result.context.entry.id {
            durableContext = result.context
            publish(context: result.context, readOnly: snapshot.isReadOnly)
        }
        return GaryxComposerCreateRebuildPayload(
            payload: GaryxComposerReadyPayload(
                text: result.envelope.text,
                attachments: result.envelope.attachments,
                clientIntentID: newIntentID,
                delivery: duplicate
            ),
            ambiguousAfter: ambiguousAfter,
            requiresThreadCreation: result.context.snapshot.createDeliveries.values.contains {
                $0.createIntentID == newIntentID && $0.phase == .createPending
            }
        )
    }

    func resolveRecoveredDraft(
        conflictSetID: GaryxPayloadConflictSetID,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        useRecovered: Bool
    ) async throws {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let updated = try await persistence.resolveRecoveredDraft(
            conflictSetID: conflictSetID,
            recoveredEntryID: recoveredEntryID,
            host: context,
            useRecovered: useRecovered
        )
        guard durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        installInputState(for: updated.entry)
        publish(context: updated, readOnly: snapshot.isReadOnly)
        replaceLiveText(updated.entry.currentText)
        grantCurrentConfigurationToLiveAdapter()
    }

    func acknowledgeFeedback(_ feedbackID: GaryxFeedbackID) async throws {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let updated = try await persistence.acknowledgeFeedback(
            feedbackID: feedbackID,
            host: context
        )
        guard durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func retryUpload(_ feedbackID: GaryxFeedbackID) async throws -> GaryxComposerStagedUpload {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let (updated, staged) = try await persistence.retryUpload(
            feedbackID: feedbackID,
            host: context
        )
        guard durableContext?.entry.id == updated.entry.id else { return staged }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
        return staged
    }

    func removeFailedUpload(_ feedbackID: GaryxFeedbackID) async throws {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let updated = try await persistence.removeFailedUpload(
            feedbackID: feedbackID,
            host: context
        )
        guard durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func makeCreateDeliveryKey(createIntentID: String) throws -> GaryxCreateDeliveryKey {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        return GaryxCreateDeliveryKey(
            scope: context.entry.scope,
            createIntentID: createIntentID
        )
    }

    func beginCreateDelivery(_ key: GaryxCreateDeliveryKey) async throws {
        guard let context = durableContext,
              context.entry.scope == key.scope else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let result = try await persistence.updateCreateDelivery(
            key: key,
            update: .begin(entryID: context.entry.id)
        )
        adoptCreateDeliveryResult(result)
    }

    func recordCreatedThread(
        _ threadID: String,
        for key: GaryxCreateDeliveryKey
    ) async throws {
        adoptCreateDeliveryResult(
            try await persistence.updateCreateDelivery(
                key: key,
                update: .created(threadID: threadID)
            )
        )
    }

    func recordCreateBindingCompleted(for key: GaryxCreateDeliveryKey) async throws {
        adoptCreateDeliveryResult(
            try await persistence.updateCreateDelivery(key: key, update: .bound)
        )
    }

    func createDeliveryPhase(
        for key: GaryxCreateDeliveryKey
    ) async -> GaryxCreateDeliveryPhase? {
        try? await persistence.createDeliveryPhase(for: key)
    }

    func acknowledgeCreateDelivery(_ key: GaryxCreateDeliveryKey) async throws {
        adoptCreateDeliveryResult(
            try await persistence.updateCreateDelivery(key: key, update: .acknowledged)
        )
    }

    func markCreateDeliveryAmbiguous(_ key: GaryxCreateDeliveryKey) async {
        guard let result = try? await persistence.updateCreateDelivery(
            key: key,
            update: .responseLost
        ) else { return }
        adoptCreateDeliveryResult(result)
    }

    private func adoptCreateDeliveryResult(_ result: GaryxCreateDeliveryCommitResult) {
        guard let updated = result.context,
              durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func deliveryPhase(
        for delivery: GaryxComposerDeliveryHandle
    ) async throws -> GaryxDeliveryRecordPhase? {
        try await persistence.deliveryPhase(delivery)
    }

    func promoteActive(to destination: GaryxComposerKey) async throws {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let updated = try await persistence.promote(context: context, to: destination)
        guard durableContext?.entry.id == updated.entry.id else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
        grantCurrentConfigurationToLiveAdapter()
    }

    func discard(key: GaryxComposerKey) async throws {
        guard let scope = scopes.activeScope else { return }
        let removed = try await persistence.discardReclaimable(scope: scope, key: key)
        guard let removed, durableContext?.entry.id == removed else { return }
        durableContext = nil
        inputState = nil
        snapshot = .unavailable
    }

    func setReadOnly(_ readOnly: Bool) {
        snapshot.isReadOnly = readOnly
    }

    func inputConfiguration() -> GaryxComposerInputConfiguration? {
        guard let state = inputState, let context = durableContext else { return nil }
        return GaryxComposerInputConfiguration(
            composerKey: context.entry.destination,
            sessionID: state.session.sessionID,
            epoch: state.session.epoch,
            payloadGeneration: state.reservedGeneration ?? state.currentGeneration,
            reservationID: state.activeReservationID,
            nextInputSequence: state.lastAppliedSequence &+ 1,
            initialText: state.currentText,
            isReadOnly: snapshot.isReadOnly
        )
    }

    func routeKeyMatchesActiveSession(_ key: GaryxComposerKey) -> Bool {
        adapterKeyMatchesActiveSession(key)
    }

    private func activeSession(
        _ state: GaryxComposerInputReducerState,
        context: GaryxComposerDurableContext,
        resolvesTo sourceKey: GaryxComposerKey
    ) -> Bool {
        guard case .resolved(let sessionDestination) = context.snapshot.aliases.resolve(
            state.session.composerKey,
            scope: state.session.scope,
            scopes: scopes
        ), case .resolved(let sourceDestination) = context.snapshot.aliases.resolve(
            sourceKey,
            scope: state.session.scope,
            scopes: scopes
        ) else { return false }
        return sessionDestination == sourceDestination
    }

    private func adapterKeyMatchesActiveSession(_ key: GaryxComposerKey) -> Bool {
        guard let state = inputState, let context = durableContext else { return false }
        return activeSession(state, context: context, resolvesTo: key)
    }

    private func installInputState(for entry: GaryxComposerPayloadEntry) {
        let epoch = (inputEpochByEntry[entry.id] ?? 0) + 1
        inputEpochByEntry[entry.id] = epoch
        let session = GaryxComposerInputSession(
            composerKey: entry.destination,
            sessionID: GaryxComposerInputSessionID(rawValue: UUID().uuidString),
            epoch: epoch,
            scope: entry.scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        inputState = GaryxComposerInputReducerState(
            session: session,
            payloadGeneration: entry.currentGeneration,
            initialText: entry.currentText
        )
    }

    private func publish(context: GaryxComposerDurableContext, readOnly: Bool) {
        let entry = context.entry
        let operations = context.snapshot.operations
        var projectionStore = context.snapshot.payloadStore
        if projectionStore.entry(entry.id, scope: entry.scope) != entry {
            projectionStore.update(entry)
        }
        let projection = GaryxComposerPayloadDirectory(store: projectionStore)
            .projection(scope: entry.scope, key: entry.destination, operations: operations)
        if let projection {
            routeProjections[ScopedComposerKey(scope: entry.scope, key: entry.destination)] = projection
        }
        snapshot = Snapshot(
            projection: projection,
            notices: GaryxComposerDurableNoticeProjector.project(
                snapshot: context.snapshot,
                hostEntryID: entry.id,
                hasInteractionOwner: liveOccurrenceID != nil && !readOnly
            ),
            isReadOnly: readOnly,
            revision: snapshot.revision &+ 1
        )
        scheduleFeedbackPresentationIfNeeded()
    }

    private func scheduleFeedbackPresentationIfNeeded() {
        guard feedbackPresentationTask == nil,
              let context = durableContext,
              liveOccurrenceID != nil,
              !snapshot.isReadOnly,
              context.snapshot.feedback.values.contains(where: {
                  $0.entryID == context.entry.id && $0.phase == .pending
              }) else { return }
        let entryID = context.entry.id
        feedbackPresentationTask = Task { [weak self] in
            guard let self else { return }
            defer { feedbackPresentationTask = nil }
            guard let updated = try? await persistence.presentFeedback(
                hostEntryID: entryID,
                hasInteractionOwner: true
            ), durableContext?.entry.id == updated.entry.id else { return }
            durableContext = updated
            publish(context: updated, readOnly: snapshot.isReadOnly)
        }
    }

    private func advanceRouteActivationIfReady() {
        guard let routeActivation,
              let terminal = routeActivation.terminal,
              terminal.outcome == .committed else { return }
        if routeActivation.sourceRequiresFinalization {
            guard let state = inputState,
                  state.producerPhase == .terminal,
                  state.reservationPhase != .sealed,
                  state.closePublicationCount == 1,
                  durableContext != nil else { return }
            guard state.closeAcknowledged else {
                scheduleFinalizationIfNeeded(routeActivation: routeActivation)
                return
            }
        }
        let destinationOccurrenceID = routeActivation.destinationOccurrenceID
        let destinationKey = routeActivation.activation.destinationKey
        let sourceWasFocused = routeActivation.sourceWasFocused
        self.routeActivation = nil
        guard let destinationOccurrenceID,
              let destinationKey,
              let scope = scopes.activeScope else {
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
            finishDeferredScopeExitIfNeeded()
            resumePendingActivationIfNeeded()
            return
        }
        if terminal.visibility == .inactive {
            deferredRouteActivation = (
                scope,
                destinationKey,
                destinationOccurrenceID,
                sourceWasFocused
            )
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
            return
        }
        guard terminal.visibility == .visible else {
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
            return
        }
        liveOccurrenceID = destinationOccurrenceID
        if sourceWasFocused {
            focusCoordinator.deferFocus(to: destinationOccurrenceID)
        }
        Task {
            await activate(scope: scope, key: destinationKey)
        }
    }

    private func scheduleFinalizationIfNeeded(routeActivation: RouteActivation) {
        guard finalizationTask == nil,
              let sessionID = inputState?.session.sessionID else { return }
        let reservationID = routeActivation.reservationIDAtRelease
        finalizationFailureDescription = nil
        finalizationTask = Task { [weak self] in
            guard let self else { return }
            var finalized: GaryxComposerFinalizedInput?
            var retryDelay = Duration.milliseconds(20)
            while !Task.isCancelled {
                guard let state = inputState,
                      state.session.sessionID == sessionID,
                      let context = durableContext,
                      var activation = self.routeActivation else {
                    finalizationTask = nil
                    return
                }
                do {
                    if finalized == nil {
                        finalized = try await persistence.persistFinalizedInput(
                            context: context,
                            state: state,
                            reservationID: reservationID
                        )
                        activation.activation.producerAndReservationReachedTerminal()
                        self.routeActivation = activation
                    }
                    guard let finalized else { continue }
                    let acknowledged = try await persistence.acknowledgeFinalizedInput(finalized)
                    guard var currentState = inputState,
                          currentState.session.sessionID == sessionID,
                          var currentActivation = self.routeActivation else {
                        finalizationTask = nil
                        return
                    }
                    currentState.acknowledgeClose(
                        lifecycle: acknowledged.entry.lifecycle.snapshot,
                        scopes: scopes
                    )
                    currentActivation.activation.closeAcknowledged()
                    durableContext = acknowledged
                    inputState = currentState
                    self.routeActivation = currentActivation
                    finalizationFailureDescription = nil
                    finalizationTask = nil
                    advanceRouteActivationIfReady()
                    return
                } catch {
                    // Keep the old host pinned and retry the same idempotent
                    // boundary without waiting for an unrelated lifecycle event.
                    finalizationFailureDescription = String(describing: error)
                    try? await Task.sleep(for: retryDelay)
                    retryDelay = min(retryDelay * 2, .seconds(1))
                }
            }
            finalizationTask = nil
        }
    }

    private func grantCurrentConfigurationToLiveAdapter() {
        guard let liveOccurrenceID,
              let adapter = adapters[liveOccurrenceID]?.value,
              let configuration = inputConfiguration(),
              adapterKeyMatchesActiveSession(adapter.composerKey) else { return }
        adapter.grantLive(configuration)
    }

    private func grantPendingFocusIfReady() {
        guard sceneIsActive,
              let liveOccurrenceID,
              let adapter = adapters[liveOccurrenceID]?.value,
              !snapshot.isReadOnly else { return }
        focusCoordinator.grantIfReady(to: adapter, sceneIsActive: sceneIsActive)
    }

    private func pruneAdapters() {
        adapters = adapters.filter { $0.value.value != nil }
    }

    private func finishDeferredScopeExitIfNeeded() {
        guard let scope = deferredScopeRevocation else { return }
        deferredScopeRevocation = nil
        completeScopeRevocation(scope)
    }

    private func resumePendingActivationIfNeeded() {
        guard routeActivation == nil,
              finalizationTask == nil,
              let pending = pendingActivation else { return }
        pendingActivation = nil
        Task { [weak self] in
            guard let self, pending.ticket == activationTicket else { return }
            await performActivation(
                scope: pending.scope,
                key: pending.key,
                ticket: pending.ticket
            )
        }
    }

    private final class WeakAdapter {
        weak var value: GaryxComposerInputAdapter?

        init(_ value: GaryxComposerInputAdapter) {
            self.value = value
        }
    }

    private struct ScopedComposerKey: Hashable {
        let scope: GaryxGatewayScope
        let key: GaryxComposerKey
    }

    private struct RouteActivation {
        let sourceOccurrenceID: GaryxRouteInstanceID?
        let destinationOccurrenceID: GaryxRouteInstanceID?
        var activation: GaryxComposerHostActivation
        var terminal: GaryxPresentationTerminalState?
        let reservationIDAtRelease: GaryxSendReservationID?
        let sourceWasFocused: Bool
        let sourceRequiresFinalization: Bool
    }
}

/// Single focus token owner for route-to-route composer transfer. The token is
/// captured at commit release and consumed only after the destination adapter
/// is live, input-ready, visible, and scene-active.
@MainActor
private final class GaryxRouteFocusCoordinator {
    private var pendingOccurrenceID: GaryxRouteInstanceID?

    func deferFocus(to occurrenceID: GaryxRouteInstanceID) {
        pendingOccurrenceID = occurrenceID
    }

    func grantIfReady(
        to adapter: GaryxComposerInputAdapter,
        sceneIsActive: Bool
    ) {
        guard pendingOccurrenceID == adapter.occurrenceID,
              GaryxRouteAccessibilityGate.allowsComposerFocus(
                  inputReady: adapter.isInputReady,
                  isVisibleRoute: adapter.isLive,
                  sceneIsActive: sceneIsActive
              ) else { return }
        pendingOccurrenceID = nil
        adapter.requestFocus()
    }
}

struct GaryxComposerInputConfiguration: Equatable {
    let composerKey: GaryxComposerKey
    let sessionID: GaryxComposerInputSessionID
    let epoch: UInt64
    let payloadGeneration: UInt64
    let reservationID: GaryxSendReservationID?
    let nextInputSequence: UInt64
    let initialText: String
    let isReadOnly: Bool
}
