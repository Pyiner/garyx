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

enum GaryxComposerPayloadRuntimeError: LocalizedError {
    case unavailable
    case staleActivation
    case payloadPreparing
    case attachmentNotUploaded
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
    let text: String
    let attachments: [GaryxComposerAttachment]
}

private struct GaryxComposerFinalizedInput: Sendable {
    let context: GaryxComposerDurableContext
    let descendantKey: GaryxSessionDescendantKey
}

private actor GaryxComposerPayloadPersistenceQueue {
    private let durability: GaryxSQLiteComposerDurabilityStore
    private let staging: GaryxComposerStagedAssetStore
    private var acceptedInputSequences: [GaryxComposerInputSessionID: UInt64] = [:]
    private var transactionGateHeld = false
    private var transactionGateWaiters: [CheckedContinuation<Void, Never>] = []

    init(applicationSupportDirectory: URL, quotaLimitBytes: Int) throws {
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
    }

    func activate(
        scope: GaryxGatewayScope,
        key: GaryxComposerKey
    ) async throws -> GaryxComposerDurableContext {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        var snapshot = try await durability.load()
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
        context: GaryxComposerDurableContext,
        sourceURL: URL,
        metadata: GaryxComposerAttachmentMetadata,
        requestToken: GaryxGatewayRequestToken
    ) async throws -> (GaryxComposerDurableContext, GaryxComposerStagedUpload) {
        await acquireTransactionGate()
        defer { releaseTransactionGate() }
        let snapshot = try await durability.load()
        guard let entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
        ), entry.lifecycle.token == context.entry.lifecycle.token,
           entry.lifecycle.phase == .active,
           entry.currentGeneration == context.entry.currentGeneration,
           requestToken.scope == entry.scope else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }

        let operationKey = GaryxOperationCapabilityKey(
            scope: entry.scope,
            entryID: entry.id,
            generation: entry.currentGeneration,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: UUID().uuidString)
        )
        let operationContext = GaryxScopeBoundOperationContext(
            key: operationKey,
            clientIdentity: requestToken.scope.identity,
            configurationFingerprint: String(requestToken.activationSequence),
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
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
        let followupGeneration = try await durability.allocatePayloadGeneration()
        let reservationID = try await durability.allocateSendReservationID()
        snapshot = try await durability.load()
        guard let current = snapshot.payloadStore.entry(entry.id, scope: entry.scope),
              current.currentGeneration == entry.currentGeneration,
              current.lifecycle.token == entry.lifecycle.token else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        return GaryxComposerSendPreparation(
            entryID: entry.id,
            scope: entry.scope,
            lifecycle: .init(token: entry.lifecycle.token, revision: entry.lifecycle.revision),
            envelopeGeneration: entry.currentGeneration,
            followupGeneration: followupGeneration,
            reservationID: reservationID,
            clientIntentID: clientIntentID,
            text: entry.currentText,
            attachments: attachments
        )
    }

    func commitSend(
        _ preparation: GaryxComposerSendPreparation,
        provisionalText: String
    ) async throws -> GaryxComposerDurableContext {
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
            text: preparation.text,
            attachmentIDs: preparation.attachments.map(\.id),
            generation: preparation.envelopeGeneration,
            clientIntentID: preparation.clientIntentID
        )
        var barrier = GaryxSendCommitBarrier(
            entryID: preparation.entryID,
            scope: preparation.scope,
            payloadLifecycle: preparation.lifecycle
        )
        guard barrier.seal(
            reservationID: preparation.reservationID,
            envelope: envelope,
            followupGeneration: preparation.followupGeneration,
            readiness: .ready,
            quota: GaryxDeliveryQuota(rebuilding: Array(snapshot.deliveries.values)),
            producerPhase: .live,
            lifecycle: lifecycle
        ) == .sealed,
        barrier.replaceProvisionalText(provisionalText, lifecycle: lifecycle),
        let settlement = barrier.durableCommit(
            deliveryID: GaryxDeliveryRecordID(rawValue: UUID().uuidString),
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
        return GaryxComposerDurableContext(snapshot: released, entry: updated)
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
            descendantKey: key
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
        guard let entry = snapshot.payloadStore.entry(
            finalized.context.entry.id,
            scope: finalized.context.entry.scope
        ), snapshot.producerDrained[finalized.descendantKey] != nil else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "acknowledge composer input close",
                mutations: [.removeProducerDrained(finalized.descendantKey)]
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
        guard store.promote(
                entryID: context.entry.id,
                scope: context.entry.scope,
                to: destination
              ),
              let entry = store.entry(context.entry.id, scope: context.entry.scope) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "promote route-owned composer entry",
                mutations: [.upsertEntry(entry)]
            )
        )
        return GaryxComposerDurableContext(snapshot: committed, entry: entry)
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
    private var scopes = GaryxGatewayScopeRegistry()
    private var adapters: [GaryxRouteInstanceID: WeakAdapter] = [:]
    private var liveOccurrenceID: GaryxRouteInstanceID?
    private var routeActivation: RouteActivation?
    private var finalizationTask: Task<Void, Never>?
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

    init(applicationSupportDirectory: URL, quotaLimitBytes: Int = 100 * 1_024 * 1_024) throws {
        persistence = try GaryxComposerPayloadPersistenceQueue(
            applicationSupportDirectory: applicationSupportDirectory,
            quotaLimitBytes: quotaLimitBytes
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
    var canSend: Bool { snapshot.projection?.readiness == .ready && !snapshot.isReadOnly }
    var activeKey: GaryxComposerKey? { snapshot.projection?.key }

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
               configuration.composerKey == adapter.composerKey {
                adapter.grantLive(configuration)
            } else {
                adapter.makeReadOnly()
            }
            grantPendingFocusIfReady()
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
        guard let sourceOccurrenceID,
              sourceKey != nil,
              activation.commitReleased(),
              let adapter = adapters[sourceOccurrenceID]?.value else {
            routeActivation = RouteActivation(
                sourceOccurrenceID: sourceOccurrenceID,
                destinationOccurrenceID: destinationOccurrenceID,
                activation: activation,
                terminal: nil,
                reservationIDAtRelease: nil,
                sourceWasFocused: false
            )
            return
        }
        let close = adapter.finalizeInput()
        // finalizeInput synchronously publishes the unmarked text through the
        // ordered callback. Re-read the reducer only after that callback so
        // finalSequence is compared with the state containing that event.
        guard var state = inputState,
              let context = durableContext,
              state.session.composerKey == sourceKey else {
            assertionFailure("live adapter lost its durable input session at release")
            return
        }
        precondition(
            close.finalSequence == state.lastAppliedSequence,
            "UIKit close must carry the exact final input sequence"
        )
        let release = state.releaseForCommittedNavigation(
            pendingProducers: close.pendingProducers,
            lifecycle: context.entry.lifecycle.snapshot,
            scopes: scopes
        )
        precondition(release == .released, "committed route release must freeze the live adapter")
        inputState = state
        snapshot.isReadOnly = true
        liveOccurrenceID = nil
        routeActivation = RouteActivation(
            sourceOccurrenceID: sourceOccurrenceID,
            destinationOccurrenceID: destinationOccurrenceID,
            activation: activation,
            terminal: nil,
            reservationIDAtRelease: state.activeReservationID,
            sourceWasFocused: close.wasFocused
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
        guard let context = durableContext,
              context.entry.scope == requestToken.scope else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let (updated, staged) = try await persistence.stageAttachment(
            context: context,
            sourceURL: sourceURL,
            metadata: metadata,
            requestToken: requestToken
        )
        guard durableContext?.entry.id == updated.entry.id else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
        return staged
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

    func takeReadyPayload(clientIntentID: String) async throws -> (String, [GaryxComposerAttachment]) {
        guard let context = durableContext, var state = inputState else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let preparation = try await persistence.prepareSend(
            context: context,
            clientIntentID: clientIntentID
        )
        guard state.session.payloadLifecycle == preparation.lifecycle,
              state.beginSend(
                reservationID: preparation.reservationID,
                followupGeneration: preparation.followupGeneration,
                lifecycle: context.entry.lifecycle.snapshot,
                scopes: scopes
              ) == .sealed(
                envelope: preparation.text,
                followupGeneration: preparation.followupGeneration
              ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        inputState = state
        snapshot.isReadOnly = false
        grantCurrentConfigurationToLiveAdapter()
        let updated: GaryxComposerDurableContext
        do {
            updated = try await persistence.commitSend(
                preparation,
                provisionalText: state.textByGeneration[preparation.followupGeneration] ?? ""
            )
        } catch {
            // Storage did not publish the barrier. Restore from the authoritative
            // Entry rather than leaving the adapter stranded in a sealed window.
            let restored = try await persistence.activate(
                scope: context.entry.scope,
                key: context.entry.destination
            )
            durableContext = restored
            installInputState(for: restored.entry)
            publish(context: restored, readOnly: false)
            grantCurrentConfigurationToLiveAdapter()
            throw error
        }
        guard durableContext?.entry.id == updated.entry.id else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        guard var settledState = inputState,
              settledState.session.sessionID == state.session.sessionID,
              settledState.commitReservation(
            lifecycle: updated.entry.lifecycle.snapshot,
            scopes: scopes
        ), settledState.returnReservationToIdle(
            lifecycle: updated.entry.lifecycle.snapshot,
            scopes: scopes
        ) else {
            throw GaryxComposerPayloadRuntimeError.invalidTransition
        }
        durableContext = updated
        inputState = settledState
        publish(context: updated, readOnly: false)
        grantCurrentConfigurationToLiveAdapter()
        advanceRouteActivationIfReady()
        return (preparation.text, preparation.attachments)
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
        guard let state = inputState else { return nil }
        return GaryxComposerInputConfiguration(
            composerKey: state.session.composerKey,
            sessionID: state.session.sessionID,
            epoch: state.session.epoch,
            payloadGeneration: state.reservedGeneration ?? state.currentGeneration,
            reservationID: state.activeReservationID,
            initialText: state.currentText,
            isReadOnly: snapshot.isReadOnly
        )
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
        let projection = GaryxComposerPayloadDirectory(store: context.snapshot.payloadStore)
            .projection(scope: entry.scope, key: entry.destination, operations: operations)
        snapshot = Snapshot(
            projection: projection,
            isReadOnly: readOnly,
            revision: snapshot.revision &+ 1
        )
    }

    private func advanceRouteActivationIfReady() {
        guard let routeActivation,
              let terminal = routeActivation.terminal,
              terminal.outcome == .committed else { return }
        if routeActivation.activation.sourceKey != nil {
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
              let state = inputState,
              let context = durableContext else { return }
        let sessionID = state.session.sessionID
        let reservationID = routeActivation.reservationIDAtRelease
        finalizationFailureDescription = nil
        finalizationTask = Task { [weak self] in
            guard let self else { return }
            do {
                let finalized = try await persistence.persistFinalizedInput(
                    context: context,
                    state: state,
                    reservationID: reservationID
                )
                guard inputState?.session.sessionID == sessionID,
                      var activation = self.routeActivation else {
                    finalizationTask = nil
                    return
                }
                activation.activation.producerAndReservationReachedTerminal()
                self.routeActivation = activation

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
            } catch {
                // The old host stays pinned and read-only. A later producer,
                // reservation, scene, or terminal event retries the same
                // idempotent durability boundary.
                finalizationTask = nil
                finalizationFailureDescription = String(describing: error)
            }
        }
    }

    private func grantCurrentConfigurationToLiveAdapter() {
        guard let liveOccurrenceID,
              let adapter = adapters[liveOccurrenceID]?.value,
              let configuration = inputConfiguration(),
              configuration.composerKey == adapter.composerKey else { return }
        adapter.grantLive(configuration)
    }

    private func grantPendingFocusIfReady() {
        guard sceneIsActive,
              let liveOccurrenceID,
              let adapter = adapters[liveOccurrenceID]?.value,
              !snapshot.isReadOnly else { return }
        focusCoordinator.grantIfReady(to: adapter)
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

    private struct RouteActivation {
        let sourceOccurrenceID: GaryxRouteInstanceID?
        let destinationOccurrenceID: GaryxRouteInstanceID?
        var activation: GaryxComposerHostActivation
        var terminal: GaryxPresentationTerminalState?
        let reservationIDAtRelease: GaryxSendReservationID?
        let sourceWasFocused: Bool
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

    func grantIfReady(to adapter: GaryxComposerInputAdapter) {
        guard pendingOccurrenceID == adapter.occurrenceID, adapter.isLive else { return }
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
    let initialText: String
    let isReadOnly: Bool
}
