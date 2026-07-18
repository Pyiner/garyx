import Darwin
import Foundation
import GaryxMobileCore

private let scope = GaryxGatewayScope(identity: "crash-gateway", epoch: 1)
private let entryID = GaryxComposerPayloadEntryID(rawValue: "crash-entry")
private let reservationID = GaryxSendReservationID(rawValue: 9)
private let deliveryID = GaryxDeliveryRecordID(rawValue: "crash-delivery")

@main
private enum GaryxComposerDurabilityCrashHarness {
    static func main() async {
        do {
            let arguments = try HarnessArguments(CommandLine.arguments)
            try FileManager.default.createDirectory(
                at: arguments.applicationSupportURL,
                withIntermediateDirectories: true
            )
            let boundaries = BoundaryController(
                killSpecification: arguments.optional("kill"),
                killOccurrence: Int(arguments.optional("kill-occurrence") ?? "1") ?? 1,
                failureSpecification: arguments.optional("fail"),
                failureOccurrence: Int(arguments.optional("fail-occurrence") ?? "1") ?? 1,
                failureKind: arguments.optional("failure")
            )
            let store = try GaryxSQLiteComposerDurabilityStore(
                databaseURL: arguments.databaseURL,
                allocationBlockSize: 8,
                boundaryHook: { boundary in try boundaries.observeStorage(boundary) }
            )

            switch arguments.action {
            case "seed-sealed":
                try await seedSealed(store: store, includeProducerDrained: false)
            case "seed-unsettled":
                try await seedSealed(store: store, includeProducerDrained: true)
            case "commit-send":
                try await commitSealedSend(store: store)
            case "attempt":
                let gate = GaryxComposerDeliveryTransportGate(durability: store)
                guard try await gate.prepareAttempt(deliveryID: deliveryID) != nil else {
                    throw HarnessError.actionRejected("attempt")
                }
            case "attempt-then-kill":
                let gate = GaryxComposerDeliveryTransportGate(durability: store)
                guard try await gate.prepareAttempt(deliveryID: deliveryID) != nil else {
                    throw HarnessError.actionRejected("attempt-then-kill")
                }
                killNow()
            case "ambiguous":
                try await GaryxComposerDeliveryTransportGate(durability: store)
                    .recordAmbiguous(deliveryID: deliveryID)
            case "ack":
                try await GaryxComposerDeliveryTransportGate(durability: store)
                    .acknowledge(deliveryID: deliveryID)
            case "ack-delivery":
                try await GaryxComposerDeliveryTransportGate(durability: store)
                    .acknowledge(
                        deliveryID: .init(rawValue: try arguments.required("delivery"))
                    )
            case "seed-operation":
                guard let state = GaryxOperationCapabilityState(
                    rawValue: try arguments.required("state")
                ) else {
                    throw HarnessError.invalidArgument("state")
                }
                try await seedOperation(
                    store: store,
                    state: state,
                    attempted: arguments.flag("attempted"),
                    reservationOutcome: arguments.optional("reservation")
                )
            case "seed-multi-operation":
                try await seedMultipleOperations(store: store)
            case "seed-ownerless-manifest":
                try await seedOwnerlessManifest(store: store)
            case "seed-replacement":
                guard let phase = GaryxReplacementPhase(
                    rawValue: try arguments.required("phase")
                ) else {
                    throw HarnessError.invalidArgument("phase")
                }
                try await seedReplacement(
                    store: store,
                    applicationSupportURL: arguments.applicationSupportURL,
                    phase: phase,
                    includeRecordFamilies: arguments.flag("families"),
                    forceEntryErase: arguments.flag("erase-entry")
                )
            case "seed-discard-operation":
                guard let state = GaryxOperationCapabilityState(
                    rawValue: try arguments.required("state")
                ) else {
                    throw HarnessError.invalidArgument("state")
                }
                try await seedDiscardOperation(store: store, state: state)
            case "seed-discard-sessions":
                try await seedDiscardSessions(store: store)
            case "seed-discard-mixed":
                try await seedDiscardMixed(store: store)
            case "stage":
                let staging = try makeStaging(
                    arguments: arguments,
                    store: store,
                    boundaries: boundaries
                )
                let sourceURL = URL(fileURLWithPath: try arguments.required("source"))
                _ = try await staging.stage(
                    makeStagingAdmission(
                        sourceURL: sourceURL,
                        expectedRevision: try await store.load().revision
                    )
                )
            case "recover":
                let staging = try makeStaging(
                    arguments: arguments,
                    store: store,
                    boundaries: boundaries
                )
                let recovery = GaryxComposerDurabilityLaunchRecovery(
                    durability: store,
                    staging: staging,
                    scopes: scopeRegistry(arguments.optional("scope") ?? "active")
                )
                let report = try await recovery.recover()
                try printSummary(snapshot: try await store.load(), report: report)
            case "inspect":
                try printSummary(snapshot: try await store.load(), report: nil)
            case "churn-discard":
                let count = Int(arguments.optional("count") ?? "500") ?? 500
                try await churnDiscard(
                    count: count,
                    arguments: arguments,
                    store: store,
                    boundaries: boundaries
                )
                try printSummary(snapshot: try await store.load(), report: nil)
            default:
                throw HarnessError.invalidAction(arguments.action)
            }
        } catch {
            FileHandle.standardError.write(Data("\(error)\n".utf8))
            Darwin.exit(1)
        }
    }

    private static func makeStaging(
        arguments: HarnessArguments,
        store: GaryxSQLiteComposerDurabilityStore,
        boundaries: BoundaryController
    ) throws -> GaryxComposerStagedAssetStore {
        try GaryxComposerStagedAssetStore(
            applicationSupportDirectory: arguments.applicationSupportURL,
            durability: store,
            quotaLimitBytes: 16 * 1024 * 1024,
            boundaryHook: { boundary in try boundaries.observeStaging(boundary) }
        )
    }

    private static func seedSealed(
        store: GaryxSQLiteComposerDurabilityStore,
        includeProducerDrained: Bool
    ) async throws {
        var entry = makeEntry(text: "T")
        entry.setText("U", generation: 11)
        var barrier = makeBarrier(entry: entry)
        let envelope = GaryxDeliveryEnvelope(
            text: "T",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "crash-intent"
        )
        guard barrier.seal(
            reservationID: reservationID,
            envelope: envelope,
            followupGeneration: 11,
            readiness: .ready,
            quota: .init(),
            producerPhase: .live,
            lifecycle: entry.lifecycle.snapshot
        ) == .sealed,
        barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot) else {
            throw HarnessError.actionRejected("seed sealed barrier")
        }
        let ledger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        var mutations: [GaryxComposerDurabilityMutation] = [
            .setGenerationHighWatermark(32),
            .setReservationHighWatermark(32),
            .upsertLedger(ledger),
            .upsertEntry(entry),
            .upsertBarrier(barrier),
        ]
        if includeProducerDrained {
            let drained = makeProducerDrained(entry: entry)
            mutations.append(.upsertProducerDrained(drained.key, drained.value))
        }
        _ = try await store.commit(
            .init(expectedRevision: 0, label: "harness seed sealed", mutations: mutations)
        )
    }

    private static func commitSealedSend(
        store: GaryxSQLiteComposerDurabilityStore
    ) async throws {
        let snapshot = try await store.load()
        guard var entry = snapshot.payloadStore.entry(entryID, scope: scope),
              var barrier = snapshot.barriers[entryID],
              var ledger = snapshot.ledgers[
                .init(scope: scope, entryID: entryID, reservationID: reservationID)
              ],
              ledger.settle(.committed, targetGeneration: 11),
              let settlement = barrier.durableCommit(
                deliveryID: deliveryID,
                correlationID: "crash-correlation",
                clientIntentID: "crash-intent",
                lifecycle: entry.lifecycle.snapshot
              ) else {
            throw HarnessError.actionRejected("commit sealed send")
        }
        entry.setText("U", generation: 11)
        let send = try GaryxComposerCommitSend(
            expectedRevision: snapshot.revision,
            ledger: ledger,
            sealedPayloadEntry: entry,
            barrier: barrier,
            settlement: settlement
        )
        _ = try await store.commitSend(send)
    }

    private static func seedOperation(
        store: GaryxSQLiteComposerDurabilityStore,
        state: GaryxOperationCapabilityState,
        attempted: Bool,
        reservationOutcome: String?
    ) async throws {
        var entry = makeEntry()
        let useReservation = reservationOutcome != nil && reservationOutcome != "nil"
        let key = operationKey(
            "manifest-\(state.rawValue)",
            reservationID: useReservation ? reservationID : nil
        )
        let ownsAsset = state != .superseded
        let assetID = ownsAsset
            ? GaryxStagedAssetID(rawValue: "manifest-\(state.rawValue).bin")
            : nil
        let bytes = ownsAsset ? 31 : 0
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: state,
            assetID: assetID,
            bytes: bytes,
            attempted: attempted
        )
        entry.addOperation(key)
        var mutations: [GaryxComposerDurabilityMutation] = []
        if useReservation {
            var ledger = GaryxProvisionalReservationLedger(
                key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
                envelopeGeneration: 10,
                followupGeneration: 11
            )
            switch reservationOutcome {
            case "committed":
                _ = ledger.settle(.committed, targetGeneration: 11)
            case "revoked":
                _ = ledger.settle(.revoked, targetGeneration: 12)
            case "none":
                entry.setText("U", generation: 11)
                var barrier = makeBarrier(entry: entry)
                let envelope = GaryxDeliveryEnvelope(
                    text: "T",
                    attachmentIDs: [],
                    generation: 10,
                    clientIntentID: "manifest-intent"
                )
                guard barrier.seal(
                    reservationID: reservationID,
                    envelope: envelope,
                    followupGeneration: 11,
                    readiness: .ready,
                    quota: .init(),
                    producerPhase: .live,
                    lifecycle: entry.lifecycle.snapshot
                ) == .sealed,
                barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot) else {
                    throw HarnessError.actionRejected("seed manifest reservation")
                }
                mutations.append(.setGenerationHighWatermark(32))
                mutations.append(.upsertLedger(ledger))
                mutations.append(.upsertBarrier(barrier))
            default:
                break
            }
            if reservationOutcome != "none" {
                mutations.append(.upsertLedger(ledger))
            }
        }
        mutations.append(contentsOf: [
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
        ])
        if let assetID {
            mutations.append(.reserveStagedAsset(assetID: assetID, owner: key, bytes: bytes))
        }
        _ = try await store.commit(
            .init(expectedRevision: 0, label: "harness seed operation", mutations: mutations)
        )
    }

    private static func seedDiscardOperation(
        store: GaryxSQLiteComposerDurabilityStore,
        state: GaryxOperationCapabilityState
    ) async throws {
        var entry = makeEntry()
        let key = operationKey("discard-\(state.rawValue)")
        let assetID = state == .superseded
            ? nil
            : GaryxStagedAssetID(rawValue: "discard-\(state.rawValue).bin")
        let bytes = assetID == nil ? 0 : 37
        let operation = makeOperation(
            key: key,
            entry: entry,
            state: state,
            assetID: assetID,
            bytes: bytes,
            attempted: state == .uploading
        )
        entry.addOperation(key)
        guard entry.beginDiscard(revision: 2) else {
            throw HarnessError.actionRejected("begin discard")
        }
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: makeBarrier(entry: entry),
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
            .init(expectedRevision: 0, label: "harness seed discard operation", mutations: mutations)
        )
    }

    private static func seedMultipleOperations(
        store: GaryxSQLiteComposerDurabilityStore
    ) async throws {
        var entry = makeEntry(text: "multi-operation")
        let firstKey = operationKey("multi-first")
        let secondKey = operationKey("multi-second")
        let firstAssetID = GaryxStagedAssetID(rawValue: "multi-first.bin")
        let secondAssetID = GaryxStagedAssetID(rawValue: "multi-second.bin")
        let first = makeOperation(
            key: firstKey,
            entry: entry,
            state: .preparing,
            assetID: firstAssetID,
            bytes: 17,
            attempted: false
        )
        let second = makeOperation(
            key: secondKey,
            entry: entry,
            state: .preparing,
            assetID: secondAssetID,
            bytes: 19,
            attempted: false
        )
        entry.addOperation(firstKey)
        entry.addOperation(secondKey)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "harness seed shared Entry operations",
                mutations: [
                    .upsertEntry(entry),
                    .upsertOperation(first),
                    .upsertManifest(
                        .init(
                            key: firstKey,
                            stagedPath: firstAssetID.rawValue,
                            state: .preparing,
                            uploadAttempted: false
                        )
                    ),
                    .reserveStagedAsset(assetID: firstAssetID, owner: firstKey, bytes: 17),
                    .upsertOperation(second),
                    .upsertManifest(
                        .init(
                            key: secondKey,
                            stagedPath: secondAssetID.rawValue,
                            state: .preparing,
                            uploadAttempted: false
                        )
                    ),
                    .reserveStagedAsset(assetID: secondAssetID, owner: secondKey, bytes: 19),
                ]
            )
        )
    }

    private static func seedOwnerlessManifest(
        store: GaryxSQLiteComposerDurabilityStore
    ) async throws {
        var entry = makeEntry(text: "ownerless")
        let key = operationKey("ownerless-manifest")
        entry.addOperation(key)
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "harness seed ownerless manifest",
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
    }

    private static func seedReplacement(
        store: GaryxSQLiteComposerDurabilityStore,
        applicationSupportURL: URL,
        phase: GaryxReplacementPhase,
        includeRecordFamilies: Bool,
        forceEntryErase: Bool
    ) async throws {
        var entry = makeEntry(text: "replacement-sibling-text")
        let siblingAttachmentID = GaryxAttachmentID(rawValue: "replacement-sibling")
        entry.addAttachment(
            .init(
                id: siblingAttachmentID,
                stagedAssetID: .init(rawValue: "replacement-sibling.bin"),
                generation: entry.currentGeneration,
                byteCount: 7
            )
        )
        let oldKey = operationKey("replacement-old")
        let successorKey = operationKey("replacement-successor")
        let assetID = GaryxStagedAssetID(rawValue: "replacement-provisional.bin")
        var replacement = GaryxReplacementRecord(
            id: .init(rawValue: "replacement-record"),
            scope: scope,
            entryID: entryID,
            oldKey: oldKey,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: assetID,
            reservedBytes: 43
        )
        var mutations: [GaryxComposerDurabilityMutation] = [.upsertEntry(entry)]
        let feedbackOperationID: GaryxOperationID

        switch phase {
        case .pendingReplacement, .aborted:
            let state: GaryxOperationCapabilityState = forceEntryErase
                ? .requested
                : .failedRetryable
            let owner = makeOperation(
                key: oldKey,
                entry: entry,
                state: state,
                assetID: assetID,
                bytes: 43,
                attempted: state == .failedRetryable
            )
            entry.addOperation(oldKey)
            if phase == .aborted { replacement.abort() }
            mutations = [
                .upsertEntry(entry),
                .upsertOperation(owner),
                .upsertManifest(
                    .init(
                        key: oldKey,
                        stagedPath: assetID.rawValue,
                        state: state,
                        uploadAttempted: state == .failedRetryable
                    )
                ),
            ]
            feedbackOperationID = oldKey.operationID
        case .committed:
            let old = makeOperation(
                key: oldKey,
                entry: entry,
                state: .superseded,
                assetID: nil,
                bytes: 0,
                attempted: true
            )
            let successor = makeOperation(
                key: successorKey,
                entry: entry,
                state: .failedRetryable,
                assetID: assetID,
                bytes: 43,
                attempted: true
            )
            entry.addOperation(oldKey)
            entry.addOperation(successorKey)
            replacement.commit(newKey: successorKey)
            mutations = [
                .upsertEntry(entry),
                .upsertOperation(old),
                .upsertManifest(
                    .init(
                        key: oldKey,
                        stagedPath: "transferred",
                        state: .superseded,
                        uploadAttempted: true
                    )
                ),
                .upsertOperation(successor),
                .upsertManifest(
                    .init(
                        key: successorKey,
                        stagedPath: assetID.rawValue,
                        state: .failedRetryable,
                        uploadAttempted: true
                    )
                ),
            ]
            feedbackOperationID = successorKey.operationID
        case .settled:
            throw HarnessError.invalidArgument("settled replacement seed")
        }

        mutations.append(.upsertReplacement(replacement))
        if includeRecordFamilies {
            let feedbackID = GaryxFeedbackID(rawValue: "replacement-feedback")
            let lineageID = GaryxAttachmentLineageID(rawValue: "replacement-lineage")
            entry.addFeedbackReference(feedbackID)
            let feedback = GaryxOperationFeedback(
                id: feedbackID,
                scope: scope,
                entryID: entryID,
                operationID: feedbackOperationID,
                lineageID: lineageID,
                kind: .uploadRetryable
            )
            let lineage = GaryxAttachmentLineageTombstone(
                id: lineageID,
                scope: scope,
                entryID: entryID,
                attachmentSlotID: siblingAttachmentID,
                failedOperationID: feedbackOperationID,
                feedbackID: feedbackID,
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            )
            mutations[0] = .upsertEntry(entry)
            mutations.append(.upsertFeedback(feedback))
            mutations.append(.upsertAttachmentLineage(lineage))
        }
        let owner = phase == .committed ? successorKey : oldKey
        mutations.append(.reserveStagedAsset(assetID: assetID, owner: owner, bytes: 43))

        let stagedRoot = applicationSupportURL
            .appendingPathComponent("Garyx", isDirectory: true)
            .appendingPathComponent("ComposerPayload", isDirectory: true)
        try FileManager.default.createDirectory(
            at: stagedRoot,
            withIntermediateDirectories: true
        )
        try Data("replacement provisional".utf8).write(
            to: stagedRoot.appendingPathComponent(assetID.rawValue)
        )
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "harness seed replacement recovery",
                mutations: mutations
            )
        )
    }

    private static func seedDiscardSessions(
        store: GaryxSQLiteComposerDurabilityStore,
        suffix: String = ""
    ) async throws {
        let localEntryID = suffix.isEmpty
            ? entryID
            : GaryxComposerPayloadEntryID(rawValue: "crash-entry-\(suffix)")
        var entry = GaryxComposerPayloadEntry(
            id: localEntryID,
            scope: scope,
            destination: .draft("D-\(suffix)"),
            lifecycleToken: .init(entryID: localEntryID, nonce: "crash-token-\(suffix)"),
            currentGeneration: 10,
            text: "discard"
        )
        let first = GaryxSessionDescendant(
            key: .init(
                token: entry.lifecycle.token,
                sessionID: .init(rawValue: "S1-\(suffix)"),
                epoch: 1
            ),
            composerKey: .draft("D-\(suffix)"),
            phase: .closePendingAck,
            finalSequence: 4
        )
        let intermediate = GaryxComposerKey.thread("T1-\(suffix)")
        let destination = GaryxComposerKey.thread("T2-\(suffix)")
        entry.promote(to: intermediate)
        entry.promote(to: destination)
        let second = GaryxSessionDescendant(
            key: .init(
                token: entry.lifecycle.token,
                sessionID: .init(rawValue: "S2-\(suffix)"),
                epoch: 2
            ),
            composerKey: destination,
            phase: .live,
            finalSequence: nil
        )
        guard entry.beginDiscard(revision: 2) else {
            throw HarnessError.actionRejected("begin session discard")
        }
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: GaryxSendCommitBarrier(
                entryID: localEntryID,
                scope: scope,
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            sessions: [first.key: first, second.key: second]
        )
        let snapshot = try await store.load()
        var aliases = snapshot.aliases
        guard aliases.establishPromotion(
            scope: scope,
            source: .draft("D-\(suffix)"),
            target: intermediate,
            activeOrClosingSessions: 2,
            pendingCloseAcknowledgements: 1
        ) == .established,
        aliases.establishPromotion(
            scope: scope,
            source: intermediate,
            target: destination,
            activeOrClosingSessions: 1
        ) == .established else {
            throw HarnessError.actionRejected("seed session alias")
        }
        _ = try await store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "harness seed cross-promotion discard sessions",
                mutations: [
                    .replaceAliases(aliases),
                    .upsertEntry(entry),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
    }

    private static func seedDiscardMixed(
        store: GaryxSQLiteComposerDurabilityStore
    ) async throws {
        let historicalReservationID = GaryxSendReservationID(rawValue: 8)
        var entry = GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("D-mixed"),
            lifecycleToken: .init(entryID: entryID, nonce: "crash-mixed-token"),
            currentGeneration: 10,
            text: "T"
        )
        entry.setText("U", generation: 11)
        let first = GaryxSessionDescendant(
            key: .init(
                token: entry.lifecycle.token,
                sessionID: .init(rawValue: "S1-mixed"),
                epoch: 1
            ),
            composerKey: .draft("D-mixed"),
            phase: .closePendingAck,
            finalSequence: 4
        )
        let intermediate = GaryxComposerKey.thread("T1-mixed")
        let destination = GaryxComposerKey.thread("T2-mixed")
        entry.promote(to: intermediate)
        entry.promote(to: destination)
        let second = GaryxSessionDescendant(
            key: .init(
                token: entry.lifecycle.token,
                sessionID: .init(rawValue: "S2-mixed"),
                epoch: 2
            ),
            composerKey: destination,
            phase: .finalizing,
            finalSequence: nil
        )

        var barrier = makeBarrier(entry: entry)
        let activeEnvelope = GaryxDeliveryEnvelope(
            text: "T",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "mixed-active-intent"
        )
        guard barrier.seal(
            reservationID: reservationID,
            envelope: activeEnvelope,
            followupGeneration: 11,
            readiness: .ready,
            quota: .init(),
            producerPhase: .live,
            lifecycle: entry.lifecycle.snapshot
        ) == .sealed,
        barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot) else {
            throw HarnessError.actionRejected("seed mixed barrier")
        }

        var historicalLedger = GaryxProvisionalReservationLedger(
            key: .init(
                scope: scope,
                entryID: entryID,
                reservationID: historicalReservationID
            ),
            envelopeGeneration: 9,
            followupGeneration: 10
        )
        guard historicalLedger.settle(.committed, targetGeneration: 10) else {
            throw HarnessError.actionRejected("seed historical ledger")
        }
        let activeLedger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        let historicalEnvelope = GaryxDeliveryEnvelope(
            text: "historical",
            attachmentIDs: [],
            generation: 9,
            clientIntentID: "mixed-historical-intent"
        )
        let notDispatched = GaryxDeliveryRecord(
            id: .init(rawValue: "mixed-not-dispatched"),
            scope: scope,
            entryID: entryID,
            reservationID: historicalReservationID,
            correlationID: "mixed-not-correlation",
            envelope: historicalEnvelope
        )
        var attempted = GaryxDeliveryRecord(
            id: .init(rawValue: "mixed-attempted"),
            scope: scope,
            entryID: entryID,
            reservationID: historicalReservationID,
            correlationID: "mixed-attempted-correlation",
            envelope: historicalEnvelope
        )
        var acknowledged = GaryxDeliveryRecord(
            id: .init(rawValue: "mixed-acknowledged"),
            scope: scope,
            entryID: entryID,
            reservationID: historicalReservationID,
            correlationID: "mixed-acknowledged-correlation",
            envelope: historicalEnvelope
        )
        guard attempted.markTransportAttempted(),
              acknowledged.markTransportAttempted() else {
            throw HarnessError.actionRejected("seed mixed delivery attempt")
        }
        acknowledged.recordServerAcknowledgement()

        guard entry.beginDiscard(revision: 2) else {
            throw HarnessError.actionRejected("seed mixed discard")
        }
        let deliveries = [
            notDispatched.id: notDispatched,
            attempted.id: attempted,
            acknowledged.id: acknowledged,
        ]
        let convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            sessions: [first.key: first, second.key: second],
            deliveries: deliveries
        )
        var aliases = GaryxComposerAliasTable()
        guard aliases.establishPromotion(
            scope: scope,
            source: .draft("D-mixed"),
            target: intermediate,
            activeOrClosingSessions: 2,
            pendingCloseAcknowledgements: 1
        ) == .established,
        aliases.establishPromotion(
            scope: scope,
            source: intermediate,
            target: destination,
            activeOrClosingSessions: 1
        ) == .established else {
            throw HarnessError.actionRejected("seed mixed alias")
        }
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "harness seed mixed discard convergence",
                mutations: [
                    .setGenerationHighWatermark(32),
                    .upsertLedger(historicalLedger),
                    .upsertLedger(activeLedger),
                    .replaceAliases(aliases),
                    .upsertEntry(entry),
                    .upsertBarrier(barrier),
                ] + deliveries.values.map(GaryxComposerDurabilityMutation.upsertDelivery) + [
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
    }

    private static func churnDiscard(
        count: Int,
        arguments: HarnessArguments,
        store: GaryxSQLiteComposerDurabilityStore,
        boundaries: BoundaryController
    ) async throws {
        let staging = try makeStaging(
            arguments: arguments,
            store: store,
            boundaries: boundaries
        )
        for index in 0..<count {
            try await seedDiscardSessions(store: store, suffix: String(index))
            let recovery = GaryxComposerDurabilityLaunchRecovery(
                durability: store,
                staging: staging,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            )
            _ = try await recovery.recover()
        }
    }

    private static func makeStagingAdmission(
        sourceURL: URL,
        expectedRevision: UInt64
    ) -> GaryxComposerStagedAssetAdmission {
        let entry = makeEntry()
        return GaryxComposerStagedAssetAdmission(
            expectedRevision: expectedRevision,
            sourceURL: sourceURL,
            assetID: GaryxStagedAssetID(rawValue: "crash-stage.bin"),
            entry: entry,
            context: .init(
                key: operationKey("crash-stage"),
                clientIdentity: "crash-client",
                configurationFingerprint: "crash-config",
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            )
        )
    }

    private static func makeEntry(text: String = "") -> GaryxComposerPayloadEntry {
        GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("D"),
            lifecycleToken: .init(entryID: entryID, nonce: "crash-token"),
            currentGeneration: 10,
            text: text
        )
    }

    private static func makeBarrier(
        entry: GaryxComposerPayloadEntry
    ) -> GaryxSendCommitBarrier {
        GaryxSendCommitBarrier(
            entryID: entry.id,
            scope: scope,
            payloadLifecycle: .init(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
    }

    private static func makeProducerDrained(
        entry: GaryxComposerPayloadEntry
    ) -> (key: GaryxSessionDescendantKey, value: GaryxDurableProducerDrainedRecord) {
        let key = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: .init(rawValue: "crash-session"),
            epoch: 1
        )
        return (
            key,
            .init(
                scope: scope,
                entryID: entry.id,
                reservationID: reservationID,
                record: .init(
                    sessionID: key.sessionID,
                    epoch: key.epoch,
                    finalSequence: 4,
                    bufferedText: "U"
                )
            )
        )
    }

    private static func operationKey(
        _ rawValue: String,
        reservationID: GaryxSendReservationID? = nil
    ) -> GaryxOperationCapabilityKey {
        .init(
            scope: scope,
            entryID: entryID,
            generation: reservationID == nil ? 10 : 11,
            reservationID: reservationID,
            branch: .followup,
            operationID: .init(rawValue: rawValue)
        )
    }

    private static func makeOperation(
        key: GaryxOperationCapabilityKey,
        entry: GaryxComposerPayloadEntry,
        state: GaryxOperationCapabilityState,
        assetID: GaryxStagedAssetID?,
        bytes: Int,
        attempted: Bool
    ) -> GaryxOperationCapability {
        .init(
            context: .init(
                key: key,
                clientIdentity: "crash-client",
                configurationFingerprint: "crash-config",
                payloadLifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: state,
            stagedAssetID: assetID,
            reservedBytes: bytes,
            uploadAttempted: attempted
        )
    }

    private static func scopeRegistry(_ rawValue: String) -> GaryxGatewayScopeRegistry {
        var registry = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        switch rawValue {
        case "suspended":
            _ = registry.suspendActive()
        case "revoked":
            _ = registry.revoke(scope)
        default:
            break
        }
        return registry
    }

    private static func printSummary(
        snapshot: GaryxComposerDurabilitySnapshot,
        report: GaryxComposerDurabilityRecoveryReport?
    ) throws {
        let summary = HarnessSummary(snapshot: snapshot, report: report)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        FileHandle.standardOutput.write(try encoder.encode(summary))
        FileHandle.standardOutput.write(Data("\n".utf8))
    }
}

private struct HarnessSummary: Codable {
    let revision: UInt64
    let currentText: String?
    let currentGeneration: UInt64?
    let aliasCount: Int
    let deliveryPhases: [String: String]
    let deliveryEvidence: [String: String]
    let deliveryUserDispositions: [String: String]
    let deliveryDispositions: [String: String]
    let ledgerOutcomes: [String: String]
    let targetGenerations: [String: UInt64]
    let operationStates: [String: String]
    let entryOperationMembershipCount: Int
    let entryAttachmentCount: Int
    let manifestCount: Int
    let replacementCount: Int
    let feedbackCount: Int
    let attachmentLineageCount: Int
    let discardCount: Int
    let discardTombstoneCount: Int
    let reservedBytes: Int
    let stagedOwnerCount: Int
    let pendingCleanupCount: Int
    let recoveredCloseCount: Int
    let closePublicationTotal: Int

    init(
        snapshot: GaryxComposerDurabilitySnapshot,
        report: GaryxComposerDurabilityRecoveryReport?
    ) {
        revision = snapshot.revision
        let entry = snapshot.payloadStore.entry(entryID, scope: scope)
        currentText = entry?.currentText
        currentGeneration = entry?.currentGeneration
        aliasCount = snapshot.aliases.aliasCount
        deliveryPhases = Dictionary(uniqueKeysWithValues: snapshot.deliveries.map {
            ($0.key.rawValue, $0.value.phase.rawValue)
        })
        deliveryEvidence = Dictionary(uniqueKeysWithValues: snapshot.deliveries.map {
            ($0.key.rawValue, $0.value.evidence.rawValue)
        })
        deliveryUserDispositions = Dictionary(uniqueKeysWithValues: snapshot.deliveries.map {
            ($0.key.rawValue, $0.value.userDisposition.rawValue)
        })
        deliveryDispositions = Dictionary(uniqueKeysWithValues: (report?.deliveryDispositions ?? [:]).map {
            ($0.key.rawValue, $0.value.rawValue)
        })
        ledgerOutcomes = Dictionary(uniqueKeysWithValues: snapshot.ledgers.map {
            (String($0.key.reservationID.rawValue), $0.value.terminalOutcome?.rawValue ?? "none")
        })
        targetGenerations = Dictionary(uniqueKeysWithValues: snapshot.ledgers.compactMap {
            guard let generation = $0.value.targetMapping?.generation else { return nil }
            return (String($0.key.reservationID.rawValue), generation)
        })
        operationStates = Dictionary(uniqueKeysWithValues: snapshot.operations.map {
            ($0.key.operationID.rawValue, $0.value.state.rawValue)
        })
        entryOperationMembershipCount = entry?.operationKeys.count ?? 0
        entryAttachmentCount = entry?.attachments.count ?? 0
        manifestCount = snapshot.manifests.count
        replacementCount = snapshot.replacements.count
        feedbackCount = snapshot.feedback.count
        attachmentLineageCount = snapshot.attachmentLineages.count
        discardCount = snapshot.discardConvergence.count
        discardTombstoneCount = snapshot.discardConvergence.values.reduce(0) {
            $0 + $1.persistentTombstoneCount
        }
        reservedBytes = snapshot.reservedBytes
        stagedOwnerCount = snapshot.stagedAssetOwners.count
        pendingCleanupCount = snapshot.pendingFileCleanup.count
        recoveredCloseCount = snapshot.recoveredInputClosures.count
        closePublicationTotal = snapshot.recoveredInputClosures.values.reduce(0) {
            $0 + $1.closePublicationCount
        }
    }
}

private final class BoundaryController: @unchecked Sendable {
    private let killSpecification: String?
    private let killOccurrence: Int
    private let failureSpecification: String?
    private let failureOccurrence: Int
    private let failureKind: String?
    private var killMatches = 0
    private var failureMatches = 0
    private let lock = NSLock()

    init(
        killSpecification: String?,
        killOccurrence: Int,
        failureSpecification: String?,
        failureOccurrence: Int,
        failureKind: String?
    ) {
        self.killSpecification = killSpecification
        self.killOccurrence = max(1, killOccurrence)
        self.failureSpecification = failureSpecification
        self.failureOccurrence = max(1, failureOccurrence)
        self.failureKind = failureKind
    }

    func observeStorage(_ boundary: GaryxComposerDurabilityStorageBoundary) throws {
        let name = Self.storageName(boundary)
        try observeFailure(name) {
            switch failureKind {
            case "enospc":
                GaryxSQLiteComposerDurabilityError.injectedNoSpace(boundary)
            default:
                GaryxSQLiteComposerDurabilityError.injectedFsyncFailure(boundary)
            }
        }
        observeKill(name)
    }

    func observeStaging(_ boundary: GaryxComposerStagingBoundary) throws {
        let name = "staging:\(Self.stagingName(boundary))"
        try observeFailure(name) {
            switch failureKind {
            case "enospc": GaryxComposerStagingError.injectedNoSpace(boundary)
            default: GaryxComposerStagingError.injectedFsyncFailure(boundary)
            }
        }
        observeKill(name)
    }

    private func observeKill(_ name: String) {
        guard killSpecification == name else { return }
        lock.lock()
        killMatches += 1
        let shouldKill = killMatches == killOccurrence
        lock.unlock()
        if shouldKill { killNow() }
    }

    private func observeFailure(
        _ name: String,
        makeError: () -> any Error
    ) throws {
        guard failureSpecification == name else { return }
        lock.lock()
        failureMatches += 1
        let shouldFail = failureMatches == failureOccurrence
        lock.unlock()
        if shouldFail { throw makeError() }
    }

    private static func storageName(_ boundary: GaryxComposerDurabilityStorageBoundary) -> String {
        switch boundary {
        case .transactionBegan: "transactionBegan"
        case .mutationApplied(let index): "mutation:\(index)"
        case .familyPersisted(let family): "family:\(family.rawValue)"
        case .metadataPersisted: "metadata"
        case .beforeCommit: "beforeCommit"
        case .afterCommit: "afterCommit"
        }
    }

    private static func stagingName(_ boundary: GaryxComposerStagingBoundary) -> String {
        switch boundary {
        case .quotaReserved: "quotaReserved"
        case .beforeCopy: "beforeCopy"
        case .copiedToTemporaryFile: "copiedToTemporaryFile"
        case .beforeFileSync: "beforeFileSync"
        case .fileSynced: "fileSynced"
        case .atomicallyRenamed: "atomicallyRenamed"
        case .directorySynced: "directorySynced"
        }
    }
}

private struct HarnessArguments {
    let values: [String: String]
    let flags: Set<String>

    init(_ arguments: [String]) throws {
        var values: [String: String] = [:]
        var flags: Set<String> = []
        var index = 1
        while index < arguments.count {
            let argument = arguments[index]
            guard argument.hasPrefix("--") else {
                throw HarnessError.invalidArgument(argument)
            }
            let key = String(argument.dropFirst(2))
            if index + 1 < arguments.count, !arguments[index + 1].hasPrefix("--") {
                values[key] = arguments[index + 1]
                index += 2
            } else {
                flags.insert(key)
                index += 1
            }
        }
        self.values = values
        self.flags = flags
    }

    var action: String { values["action"] ?? "" }
    var databaseURL: URL { URL(fileURLWithPath: values["db"] ?? "") }
    var applicationSupportURL: URL {
        URL(fileURLWithPath: values["app-support"] ?? "")
    }

    func required(_ key: String) throws -> String {
        guard let value = values[key], !value.isEmpty else {
            throw HarnessError.invalidArgument(key)
        }
        return value
    }

    func optional(_ key: String) -> String? { values[key] }
    func flag(_ key: String) -> Bool { flags.contains(key) }
}

private enum HarnessError: Error {
    case invalidAction(String)
    case invalidArgument(String)
    case actionRejected(String)
}

private func killNow() -> Never {
    _ = Darwin.kill(Darwin.getpid(), SIGKILL)
    while true {
        _ = Darwin.pause()
    }
}
