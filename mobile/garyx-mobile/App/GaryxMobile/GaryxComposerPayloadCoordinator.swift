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

private actor GaryxComposerPayloadPersistenceQueue {
    private let durability: GaryxSQLiteComposerDurabilityStore
    private let staging: GaryxComposerStagedAssetStore
    private var acceptedInputSequences: [GaryxComposerInputSessionID: UInt64] = [:]

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
        text: String
    ) async throws -> GaryxComposerDurableContext {
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
        acceptedInputSequences[sessionID] = sequence

        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
        ), entry.lifecycle.token == context.entry.lifecycle.token,
           entry.lifecycle.phase == .active,
           entry.currentGeneration == context.entry.currentGeneration else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        entry.setText(text, generation: entry.currentGeneration)
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist ordered composer input",
                mutations: [.upsertEntry(entry)]
            )
        )
        return GaryxComposerDurableContext(snapshot: committed, entry: entry)
    }

    func stageAttachment(
        context: GaryxComposerDurableContext,
        sourceURL: URL,
        metadata: GaryxComposerAttachmentMetadata,
        requestToken: GaryxGatewayRequestToken
    ) async throws -> (GaryxComposerDurableContext, GaryxComposerStagedUpload) {
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
        context: GaryxComposerDurableContext,
        staged: GaryxComposerStagedUpload,
        uploaded: GaryxUploadedChatAttachment
    ) async throws -> GaryxComposerDurableContext {
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
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
        let committed = try await durability.commit(
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
        context: GaryxComposerDurableContext,
        staged: GaryxComposerStagedUpload
    ) async throws -> GaryxComposerDurableContext {
        let snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
            context.entry.id,
            scope: context.entry.scope
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
        let committed = try await durability.commit(
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

    func takeReadyPayload(
        context: GaryxComposerDurableContext
    ) async throws -> (GaryxComposerDurableContext, String, [GaryxComposerAttachment]) {
        var snapshot = try await durability.load()
        guard var entry = snapshot.payloadStore.entry(
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
        let text = entry.currentText
        let nextGeneration = try await durability.allocatePayloadGeneration()
        snapshot = try await durability.load()
        guard var current = snapshot.payloadStore.entry(entry.id, scope: entry.scope),
              current.currentGeneration == entry.currentGeneration,
              current.beginFreshGeneration(nextGeneration) else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "advance composer after payload handoff",
                mutations: [.claimGeneration(nextGeneration), .upsertEntry(current)]
            )
        )
        return (GaryxComposerDurableContext(snapshot: committed, entry: current), text, attachments)
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

    private let persistence: GaryxComposerPayloadPersistenceQueue
    private var durableContext: GaryxComposerDurableContext?
    private var activationTicket: UInt64 = 0
    private var inputState: GaryxComposerInputReducerState?
    private var inputEpochByEntry: [GaryxComposerPayloadEntryID: UInt64] = [:]
    private var scopes = GaryxGatewayScopeRegistry()
    private var adapters: [GaryxRouteInstanceID: WeakAdapter] = [:]
    private var liveOccurrenceID: GaryxRouteInstanceID?
    private var routeActivation: RouteActivation?

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
        _ = scopes.switchActive(to: scope)
        snapshot.isReadOnly = true
        do {
            let context = try await persistence.activate(scope: scope, key: key)
            guard ticket == activationTicket else { return }
            durableContext = context
            installInputState(for: context.entry)
            publish(context: context, readOnly: false)
            grantCurrentConfigurationToLiveAdapter()
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
        durableContext = nil
        inputState = nil
        snapshot = .unavailable
    }

    func revokeScope(_ scope: GaryxGatewayScope) {
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
        guard case .applied = disposition else { return }
        inputState = state
        var entry = context.entry
        entry.setText(text, generation: state.currentGeneration)
        let localContext = GaryxComposerDurableContext(snapshot: context.snapshot, entry: entry)
        durableContext = localContext
        publish(context: localContext, readOnly: snapshot.isReadOnly)
        Task {
            do {
                let committed = try await persistence.persistText(
                    context: context,
                    sessionID: state.session.sessionID,
                    sequence: identity.inputSequence,
                    text: text
                )
                guard self.inputState?.session.sessionID == state.session.sessionID else { return }
                self.durableContext = committed
                self.publish(context: committed, readOnly: self.snapshot.isReadOnly)
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
            liveOccurrenceID = adapter.occurrenceID
            if let configuration = inputConfiguration(),
               configuration.composerKey == adapter.composerKey {
                adapter.grantLive(configuration)
            } else {
                adapter.makeReadOnly()
            }
        } else {
            adapter.makeReadOnly()
        }
        pruneAdapters()
    }

    func unregister(_ adapter: GaryxComposerInputAdapter) {
        if adapters[adapter.occurrenceID]?.value === adapter {
            adapters.removeValue(forKey: adapter.occurrenceID)
        }
        if liveOccurrenceID == adapter.occurrenceID {
            liveOccurrenceID = nil
        }
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
              let adapter = adapters[sourceOccurrenceID]?.value,
              var state = inputState,
              let context = durableContext,
              state.session.composerKey == sourceKey else {
            routeActivation = RouteActivation(
                sourceOccurrenceID: sourceOccurrenceID,
                destinationOccurrenceID: destinationOccurrenceID,
                activation: activation,
                terminal: nil
            )
            return
        }
        let close = adapter.finalizeInput()
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
            terminal: nil
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
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let updated = try await persistence.completeUpload(
            context: context,
            staged: staged,
            uploaded: uploaded
        )
        guard durableContext?.entry.id == updated.entry.id else { return }
        durableContext = updated
        publish(context: updated, readOnly: snapshot.isReadOnly)
    }

    func failUpload(_ staged: GaryxComposerStagedUpload) async {
        guard let context = durableContext else { return }
        guard let updated = try? await persistence.failUpload(context: context, staged: staged),
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

    func takeReadyPayload() async throws -> (String, [GaryxComposerAttachment]) {
        guard let context = durableContext else {
            throw GaryxComposerPayloadRuntimeError.unavailable
        }
        let (updated, text, attachments) = try await persistence.takeReadyPayload(context: context)
        guard durableContext?.entry.id == updated.entry.id else {
            throw GaryxComposerPayloadRuntimeError.staleActivation
        }
        durableContext = updated
        installInputState(for: updated.entry)
        publish(context: updated, readOnly: false)
        return (text, attachments)
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
            payloadGeneration: state.currentGeneration,
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
        guard var routeActivation,
              let terminal = routeActivation.terminal,
              terminal.outcome == .committed else { return }
        if routeActivation.activation.sourceKey != nil {
            guard var state = inputState,
                  state.producerPhase == .terminal,
                  state.reservationPhase != .sealed,
                  state.closePublicationCount == 1,
                  let context = durableContext else { return }
            routeActivation.activation.producerAndReservationReachedTerminal()
            state.acknowledgeClose(
                lifecycle: context.entry.lifecycle.snapshot,
                scopes: scopes
            )
            routeActivation.activation.closeAcknowledged()
            inputState = state
        }
        let destinationOccurrenceID = routeActivation.destinationOccurrenceID
        let destinationKey = routeActivation.activation.destinationKey
        self.routeActivation = nil
        guard terminal.visibility == .visible,
              let destinationOccurrenceID,
              let destinationKey,
              let scope = scopes.activeScope else {
            durableContext = nil
            inputState = nil
            snapshot = .unavailable
            return
        }
        liveOccurrenceID = destinationOccurrenceID
        Task {
            await activate(scope: scope, key: destinationKey)
        }
    }

    private func grantCurrentConfigurationToLiveAdapter() {
        guard let liveOccurrenceID,
              let adapter = adapters[liveOccurrenceID]?.value,
              let configuration = inputConfiguration(),
              configuration.composerKey == adapter.composerKey else { return }
        adapter.grantLive(configuration)
    }

    private func pruneAdapters() {
        adapters = adapters.filter { $0.value.value != nil }
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
