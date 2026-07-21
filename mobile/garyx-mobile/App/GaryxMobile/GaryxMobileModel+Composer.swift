import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

private struct GaryxOptimisticSendPresentation {
    let runtimeGeneration: GaryxGatewayRequestToken
    let text: String
    let attachments: [GaryxMobileComposerAttachment]
    let clientIntentId: String
    let userMessage: GaryxMobileMessage
    let assistantId: String
    let initialThreadId: String?
    let allowBusyFollowUp: Bool
    let draftOptimisticMessages: [GaryxMobileMessage]
    let shouldDispatch: Bool
    let previousMessages: [GaryxMobileMessage]
    let presentedMessages: [GaryxMobileMessage]
    let previousRuntime: GaryxThreadRuntime?
    let previousActiveAssistantId: String?
    let beganRunDispatch: Bool
}

extension GaryxMobileModel {
    func makeComposerPresentationOperationContext(
        payload: GaryxComposerPayloadCoordinator
    ) -> GaryxPresentationOperationContext? {
        let requestToken = gatewayRequestToken
        guard let capability = payload.makePresentationOperationContext(
            requestToken: requestToken
        ), let gatewayClient = try? client() else { return nil }
        return GaryxPresentationOperationContext(
            capability: capability,
            requestToken: requestToken,
            gatewayClient: gatewayClient
        )
    }

    func attachFiles(
        from urls: [URL],
        operationContext: GaryxPresentationOperationContext?
    ) async {
        guard !urls.isEmpty else { return }
        do {
            // Freeze the destination configuration before the picker result
            // crosses an await. A gateway switch may suspend this scope, but
            // this operation must still upload to and settle in its origin.
            guard let frozen = operationContext else {
                throw GaryxComposerPayloadRuntimeError.unavailable
            }
            let requestToken = frozen.requestToken
            let uploadClient = frozen.gatewayClient
            let baseContext = frozen.capability
            for (index, url) in urls.enumerated() {
                let frozenContext = index == 0
                    ? baseContext
                    : baseContext.replacingOperationID(
                        GaryxOperationID(rawValue: UUID().uuidString)
                    )
                let didAccess = url.startAccessingSecurityScopedResource()
                let metadata: GaryxComposerAttachmentMetadata
                let staged: GaryxComposerStagedUpload
                do {
                    metadata = try Self.localAttachmentMetadata(for: url)
                    staged = try await composerPayloadCoordinator.stageAttachment(
                        sourceURL: url,
                        metadata: metadata,
                        requestToken: requestToken,
                        operationContext: frozenContext
                    )
                } catch {
                    if didAccess { url.stopAccessingSecurityScopedResource() }
                    throw error
                }
                if didAccess { url.stopAccessingSecurityScopedResource() }
                do {
                    let uploaded = try await Self.upload(staged, using: uploadClient)
                    try await composerPayloadCoordinator.completeUpload(staged, uploaded: uploaded)
                } catch {
                    await composerPayloadCoordinator.failUpload(staged)
                    throw error
                }
            }
        } catch {
            guard !Task.isCancelled else { return }
            lastError = displayMessage(for: error)
        }
    }

    nonisolated private static func localAttachmentMetadata(
        for url: URL
    ) throws -> GaryxComposerAttachmentMetadata {
        let resourceValues = try? url.resourceValues(forKeys: [.contentTypeKey])
        let mediaType = resourceValues?.contentType?.preferredMIMEType
            ?? UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
            ?? "application/octet-stream"
        let kind = mediaType.hasPrefix("image/") ? "image" : "file"
        let name = url.lastPathComponent.isEmpty ? "attachment" : url.lastPathComponent
        let preview: String?
        if kind == "image" {
            let data = try Data(contentsOf: url, options: .mappedIfSafe)
            preview = dataUrl(mediaType: mediaType, base64: data.base64EncodedString())
        } else {
            preview = nil
        }
        return GaryxComposerAttachmentMetadata(
            kind: kind,
            name: name,
            mediaType: mediaType,
            previewDataURL: preview
        )
    }

    nonisolated private static func upload(
        _ staged: GaryxComposerStagedUpload,
        using client: GaryxGatewayClient
    ) async throws -> GaryxUploadedChatAttachment {
        let blob = try await Task.detached(priority: .userInitiated) {
            let data = try Data(contentsOf: staged.fileURL, options: .mappedIfSafe)
            return GaryxUploadChatAttachmentBlob(
                kind: staged.metadata.kind,
                name: staged.metadata.name,
                mediaType: staged.metadata.mediaType,
                dataBase64: data.base64EncodedString()
            )
        }.value
        let response = try await client.uploadChatAttachments(
            GaryxUploadChatAttachmentsRequest(files: [blob])
        )
        guard response.files.count == 1, let uploaded = response.files.first else {
            throw GaryxGatewayError.encodingFailed(
                "Gateway did not return the uploaded file."
            )
        }
        return uploaded
    }

    func attachImages(
        _ images: [GaryxMobileSelectedImage],
        operationContext: GaryxPresentationOperationContext?
    ) async {
        guard !images.isEmpty else { return }
        do {
            guard let frozen = operationContext else {
                throw GaryxComposerPayloadRuntimeError.unavailable
            }
            let requestToken = frozen.requestToken
            let uploadClient = frozen.gatewayClient
            let baseContext = frozen.capability
            for (index, image) in images.enumerated() {
                let frozenContext = index == 0
                    ? baseContext
                    : baseContext.replacingOperationID(
                        GaryxOperationID(rawValue: UUID().uuidString)
                    )
                let temporaryURL = FileManager.default.temporaryDirectory
                    .appendingPathComponent("garyx-composer-\(UUID().uuidString)")
                defer { try? FileManager.default.removeItem(at: temporaryURL) }
                try image.data.write(to: temporaryURL, options: .atomic)
                let preview = Self.dataUrl(
                    mediaType: image.mediaType,
                    base64: image.data.base64EncodedString()
                )
                let staged = try await composerPayloadCoordinator.stageAttachment(
                    sourceURL: temporaryURL,
                    metadata: GaryxComposerAttachmentMetadata(
                        kind: "image",
                        name: image.name,
                        mediaType: image.mediaType,
                        previewDataURL: preview
                    ),
                    requestToken: requestToken,
                    operationContext: frozenContext
                )
                do {
                    let uploaded = try await Self.upload(staged, using: uploadClient)
                    try await composerPayloadCoordinator.completeUpload(staged, uploaded: uploaded)
                } catch {
                    await composerPayloadCoordinator.failUpload(staged)
                    throw error
                }
            }
        } catch {
            guard !Task.isCancelled else { return }
            lastError = displayMessage(for: error)
        }
    }

    func removeComposerPayloadItem(_ attachment: GaryxMobileComposerAttachment) {
        Task { [weak self] in
            guard let self else { return }
            do {
                try await composerPayloadCoordinator.removeAttachment(
                    GaryxAttachmentID(rawValue: attachment.id)
                )
            } catch {
                lastError = displayMessage(for: error)
            }
        }
    }

    @discardableResult
    func sendDraft() async -> Bool {
        let projectedText = activeComposerDraft
        let projectedItems = activeComposerPayloadItems
        guard composerPayloadCoordinator.canSend,
              canSendComposerPayload(text: projectedText, attachments: projectedItems),
              !projectedText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                || !projectedItems.isEmpty else {
            return false
        }
        if let threadID = selectedThread?.id {
            GaryxConversationSendJitterProbe.shared?.beginSend(
                routeIdentity: "thread:\(threadID)"
            )
        }
        let clientIntentID = "mobile-\(UUID().uuidString)"
        var optimisticPresentation: GaryxOptimisticSendPresentation?
        do {
            let payload = try await composerPayloadCoordinator.takeReadyPayload(
                clientIntentID: clientIntentID,
                presentationTransaction: GaryxComposerSendPresentationTransaction(
                    present: { prepared in
                        optimisticPresentation = self.presentOptimisticSend(
                            text: Self.normalizedComposerSendText(prepared.text),
                            attachments: Self.mobileComposerAttachments(from: prepared.attachments),
                            clientIntentId: prepared.clientIntentID
                        )
                    },
                    rollback: {
                        guard let presentation = optimisticPresentation else { return }
                        self.rollbackOptimisticSend(presentation)
                        optimisticPresentation = nil
                    }
                )
            )
            guard let optimisticPresentation else {
                throw GaryxComposerPayloadRuntimeError.invalidTransition
            }
            guard optimisticPresentation.shouldDispatch else { return true }
            GaryxMobileHaptics.shared.play(.messageSendCommitted)
            await dispatchPresentedSend(
                optimisticPresentation,
                delivery: payload.delivery
            )
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    private static func normalizedComposerSendText(_ text: String) -> String {
        text
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func mobileComposerAttachments(
        from attachments: [GaryxComposerAttachment]
    ) -> [GaryxMobileComposerAttachment] {
        attachments.map { item in
            GaryxMobileComposerAttachment(
                id: item.id.rawValue,
                kind: item.kind ?? "file",
                name: item.name ?? "attachment",
                mediaType: item.mediaType ?? "application/octet-stream",
                // Send readiness already proves every envelope attachment has
                // a non-empty uploaded path before presentation can begin.
                path: item.uploadedPath ?? "",
                previewDataUrl: item.previewDataURL
            )
        }
    }

    func performComposerDurableNoticeAction(
        _ action: GaryxComposerDurableNoticeAction
    ) async {
        do {
            switch action {
            case .restoreDelivery(let deliveryID):
                let envelope = try await composerPayloadCoordinator
                    .restoreAmbiguousDelivery(deliveryID)
                removeOptimisticDelivery(clientIntentID: envelope.clientIntentID)
            case .resendDeliveryCopy(let deliveryID):
                let result = try await composerPayloadCoordinator
                    .resendAmbiguousDelivery(deliveryID)
                removeOptimisticDelivery(clientIntentID: result.originalClientIntentID)
                try await dispatchDurablePayload(result.payload)
            case .restoreCreate(let key):
                _ = try await composerPayloadCoordinator.restoreAmbiguousCreate(key)
                removeOptimisticDelivery(clientIntentID: key.createIntentID)
            case .rebuildCreateCopy(let key):
                let rebuilt = try await composerPayloadCoordinator.rebuildAmbiguousCreate(key)
                removeOptimisticDelivery(clientIntentID: key.createIntentID)
                try await dispatchDurablePayload(rebuilt.payload)
            case .acknowledgeFeedback(let feedbackID):
                try await composerPayloadCoordinator.acknowledgeFeedback(feedbackID)
            case .retryUpload(let feedbackID):
                let staged = try await composerPayloadCoordinator.retryUpload(feedbackID)
                do {
                    let uploaded = try await Self.upload(staged, using: client())
                    try await composerPayloadCoordinator.completeUpload(staged, uploaded: uploaded)
                } catch {
                    await composerPayloadCoordinator.failUpload(staged)
                    throw error
                }
            case .removeUpload(let feedbackID):
                try await composerPayloadCoordinator.removeFailedUpload(feedbackID)
            }
            lastError = nil
        } catch {
            guard !Task.isCancelled else { return }
            lastError = displayMessage(for: error)
        }
    }

    private func dispatchDurablePayload(
        _ payload: GaryxComposerReadyPayload
    ) async throws {
        let attachments = try payload.attachments.map { attachment in
            guard let path = attachment.uploadedPath, !path.isEmpty else {
                throw GaryxComposerPayloadRuntimeError.attachmentNotUploaded
            }
            return GaryxMobileComposerAttachment(
                id: attachment.id.rawValue,
                kind: attachment.kind ?? "file",
                name: attachment.name ?? "attachment",
                mediaType: attachment.mediaType ?? "application/octet-stream",
                path: path,
                previewDataUrl: attachment.previewDataURL
            )
        }
        await send(
            payload.text,
            attachments: attachments,
            clientIntentId: payload.clientIntentID,
            delivery: payload.delivery
        )
    }

    private func removeOptimisticDelivery(clientIntentID: String) {
        let messageID = Self.userOriginMessageId(clientIntentID)
        for threadID in Array(messagesByThread.keys) {
            guard cachedMessages(for: threadID).contains(where: {
                $0.id == messageID && $0.localState == .optimistic
            }) else { continue }
            mutateMessages(for: threadID) { messages in
                messages.removeAll {
                    $0.id == messageID && $0.localState == .optimistic
                }
            }
            pendingDirectFollowUpsByThread[threadID]?.removeAll {
                $0.userId == messageID
            }
            if pendingDirectFollowUpsByThread[threadID]?.isEmpty == true {
                pendingDirectFollowUpsByThread[threadID] = nil
            }
        }
        if selectedThread == nil {
            messages.removeAll {
                $0.id == messageID && $0.localState == .optimistic
            }
        }
    }

    func send(
        _ text: String,
        attachments: [GaryxMobileComposerAttachment] = [],
        clientIntentId suppliedClientIntentId: String? = nil,
        delivery: GaryxComposerDeliveryHandle? = nil
    ) async {
        let presentation = presentOptimisticSend(
            text: text,
            attachments: attachments,
            clientIntentId: suppliedClientIntentId ?? "mobile-\(UUID().uuidString)"
        )
        guard presentation.shouldDispatch else { return }
        GaryxMobileHaptics.shared.play(.messageSendCommitted)
        await dispatchPresentedSend(presentation, delivery: delivery)
    }

    /// Installs every local surface affected by a send without suspending:
    /// transcript row, run claim, assistant ownership, and follow-up ordering.
    /// The durable composer coordinator invokes this synchronously immediately
    /// before it publishes the empty follow-up generation.
    private func presentOptimisticSend(
        text: String,
        attachments: [GaryxMobileComposerAttachment],
        clientIntentId: String
    ) -> GaryxOptimisticSendPresentation {
        let runtimeGeneration = gatewayRequestToken
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let userMessage = GaryxMobileMessage(
            id: Self.userOriginMessageId(clientIntentId),
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId,
            localState: .optimistic
        )
        let assistantId = "local-assistant-\(UUID().uuidString)"
        let initialThreadId = selectedThread?.id
        let allowBusyFollowUp = initialThreadId.map { isThreadBusy($0) } ?? false
        let draftOptimisticMessages = [userMessage]
        let previousMessages = initialThreadId.map { cachedMessages(for: $0) } ?? messages
        let previousRuntime = initialThreadId.flatMap {
            runTracker.machine.threadRuntimeByThread[$0]
        }
        let previousActiveAssistantId = initialThreadId.flatMap {
            activeAssistantMessageIdsByThread[$0]
        }
        var shouldDispatch = true
        var beganRunDispatch = false
        if let initialThreadId {
            if !allowBusyFollowUp {
                finishActiveAssistantSegmentBeforeUserTurn(for: initialThreadId)
            }
            mutateMessages(for: initialThreadId) { messages in
                messages.append(userMessage)
            }
            if allowBusyFollowUp {
                pendingDirectFollowUpsByThread[initialThreadId, default: []].append((
                    userId: userMessage.id,
                    assistantId: assistantId
                ))
            } else {
                activeAssistantMessageIdsByThread[initialThreadId] = assistantId
            }
            // The run is active the instant the user sends. Non-busy sends
            // also show the tail thinking indicator immediately; busy
            // follow-ups wait until the provider ack defines the turn boundary.
            beganRunDispatch = runTracker.beginLocalDispatch(
                threadId: initialThreadId,
                intentId: clientIntentId,
                text: visibleUserText,
                allowWhileBusy: allowBusyFollowUp
            )
            if !beganRunDispatch {
                shouldDispatch = false
                markLatestLocalUserFailed(for: initialThreadId, message: "Thread is busy")
                markStreamingAssistantComplete(for: initialThreadId, removeEmpty: true)
            }
        } else {
            messages = draftOptimisticMessages
        }
        GaryxConversationSendJitterProbe.shared?.optimisticRowAppended()
        let presentedMessages = initialThreadId.map { cachedMessages(for: $0) } ?? messages
        return GaryxOptimisticSendPresentation(
            runtimeGeneration: runtimeGeneration,
            text: text,
            attachments: attachments,
            clientIntentId: clientIntentId,
            userMessage: userMessage,
            assistantId: assistantId,
            initialThreadId: initialThreadId,
            allowBusyFollowUp: allowBusyFollowUp,
            draftOptimisticMessages: draftOptimisticMessages,
            shouldDispatch: shouldDispatch,
            previousMessages: previousMessages,
            presentedMessages: presentedMessages,
            previousRuntime: previousRuntime,
            previousActiveAssistantId: previousActiveAssistantId,
            beganRunDispatch: beganRunDispatch
        )
    }

    /// A failed durable barrier means no send existed. Remove only the local
    /// transaction we installed; if unrelated stream data arrived during the
    /// await, preserve it rather than restoring a stale whole-list snapshot.
    private func rollbackOptimisticSend(_ presentation: GaryxOptimisticSendPresentation) {
        if let threadId = presentation.initialThreadId {
            if cachedMessages(for: threadId) == presentation.presentedMessages {
                setMessages(presentation.previousMessages, for: threadId)
            } else {
                mutateMessages(for: threadId) { messages in
                    messages.removeAll { $0.id == presentation.userMessage.id }
                }
            }
            forgetPendingDirectFollowUp(
                threadId: threadId,
                userId: presentation.userMessage.id,
                assistantId: presentation.assistantId
            )
            if activeAssistantMessageIdsByThread[threadId] == presentation.assistantId {
                activeAssistantMessageIdsByThread[threadId] = presentation.previousActiveAssistantId
            }
            if presentation.beganRunDispatch {
                runTracker.rollbackLocalDispatch(
                    threadId: threadId,
                    intentId: presentation.clientIntentId,
                    previousRuntime: presentation.previousRuntime
                )
            }
        } else if messages == presentation.presentedMessages {
            messages = presentation.previousMessages
        } else {
            messages.removeAll { $0.id == presentation.userMessage.id }
        }
    }

    private func dispatchPresentedSend(
        _ presentation: GaryxOptimisticSendPresentation,
        delivery: GaryxComposerDeliveryHandle?
    ) async {
        let runtimeGeneration = presentation.runtimeGeneration
        let text = presentation.text
        let attachments = presentation.attachments
        let clientIntentId = presentation.clientIntentId
        let userMessage = presentation.userMessage
        let visibleUserText = userMessage.text
        let assistantId = presentation.assistantId
        var optimisticThreadId = presentation.initialThreadId
        let allowBusyFollowUp = presentation.allowBusyFollowUp
        let draftOptimisticMessages = presentation.draftOptimisticMessages

        #if DEBUG
        if let probe = GaryxConversationSendJitterProbe.shared,
           probe.usesCapturedMaterializationFixture,
           let optimisticThreadId {
            Task { @MainActor [weak self] in
                try? await Task.sleep(nanoseconds: 90_000_000)
                guard let self,
                      selectedThread?.id == optimisticThreadId else { return }
                probe.committedRowMaterialized()
                mutateMessages(for: optimisticThreadId) { messages in
                    guard let index = messages.firstIndex(where: { $0.id == userMessage.id }) else {
                        return
                    }
                    messages[index].localState = .remoteFinal
                    messages[index].historyIndex = 371
                    messages[index].timestamp = "22:14"
                }
                if let snapshot = renderSnapshot(for: optimisticThreadId) {
                    let committedRowID = "user_turn:\(userMessage.id)"
                    let alreadyContainsCommittedRow = snapshot.rows.contains { row in
                        guard case .userTurn(let turn) = row else { return false }
                        return turn.id == committedRowID
                    }
                    let rows = alreadyContainsCommittedRow
                        ? snapshot.rows
                        : snapshot.rows + [
                            .userTurn(GaryxRenderUserTurnRow(
                                id: committedRowID,
                                user: GaryxRenderMessageRef(
                                    id: userMessage.id,
                                    seq: 372,
                                    role: "user"
                                ),
                                activity: []
                            )),
                        ]
                    setRenderSnapshot(
                        GaryxRenderSnapshot(
                            basedOnSeq: 372,
                            rows: rows,
                            tailActivity: .thinking,
                            activeToolGroupId: snapshot.activeToolGroupId,
                            progressLocus: snapshot.progressLocus,
                            filteredPlaceholders: snapshot.filteredPlaceholders,
                            rateLimit: snapshot.rateLimit,
                            window: snapshot.window,
                            rowsHash: snapshot.rowsHash
                        ),
                        for: optimisticThreadId
                    )
                }
                if let delivery {
                    try? await composerPayloadCoordinator.acknowledgeDelivery(delivery)
                }
            }
            return
        }
        #endif

        do {
            // A create-delivery record extends the already committed message
            // delivery. Legacy low-level sends have no durable envelope and
            // must retain the gateway's existing request-token semantics.
            let createIntentID = delivery == nil ? nil : clientIntentId
            let ensuredThread = try await ensureSelectedThreadForDraftCreation(
                createIntentID: createIntentID
            )
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else { return }
            let thread = ensuredThread.thread
            if optimisticThreadId == nil {
                // Promotion changes only the destination index. The stable
                // Entry/token continues to own follow-up input that arrived
                // while thread creation was in flight.
                try await promoteActiveComposerPayload(to: thread.id)
                optimisticThreadId = thread.id
                setMessages(draftOptimisticMessages, for: thread.id)
                activeAssistantMessageIdsByThread[thread.id] = assistantId
            }
            guard runTracker.beginLocalDispatch(
                threadId: thread.id,
                intentId: clientIntentId,
                text: visibleUserText,
                allowWhileBusy: allowBusyFollowUp && thread.id == optimisticThreadId
            ) else {
                markLatestLocalUserFailed(for: thread.id, message: "Thread is busy")
                markStreamingAssistantComplete(for: thread.id, removeEmpty: true)
                runTracker.failLocalDispatch(
                    threadId: thread.id,
                    intentId: clientIntentId,
                    error: "Thread is busy"
                )
                refreshHomeThreadsAfterLocalRunStateChange()
                return
            }
            refreshHomeThreadsAfterLocalRunStart()
            lastError = nil
            if !hasPendingDirectFollowUpAssistant(threadId: thread.id, assistantId: assistantId) {
                activeAssistantMessageIdsByThread[thread.id] = assistantId
            }
            let workspacePath = Self.firstNonEmpty(
                thread.workspacePath,
                newThreadWorkspaceSelection.workspacePath ?? ""
            )
            try await startChatRunViaGateway(
                threadId: thread.id,
                message: text,
                attachments: attachments,
                clientIntentId: clientIntentId,
                workspacePath: workspacePath,
                assistantMessageId: assistantId,
                delivery: delivery,
                createDeliveryKey: ensuredThread.createDeliveryKey
            )
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            if let optimisticThreadId {
                markLatestLocalUserFailed(for: optimisticThreadId, message: displayMessage(for: error))
                forgetPendingDirectFollowUp(
                    threadId: optimisticThreadId,
                    userId: userMessage.id,
                    assistantId: assistantId
                )
                if !allowBusyFollowUp {
                    markStreamingAssistantComplete(for: optimisticThreadId, removeEmpty: true)
                }
            } else {
                messages.removeAll { $0.id == assistantId }
                if let index = messages.firstIndex(where: { $0.id == userMessage.id }) {
                    messages[index].statusText = displayMessage(for: error)
                }
            }
            if let optimisticThreadId {
                if !allowBusyFollowUp {
                    activeAssistantMessageIdsByThread[optimisticThreadId] = nil
                }
                runTracker.failLocalDispatch(
                    threadId: optimisticThreadId,
                    intentId: clientIntentId,
                    error: displayMessage(for: error)
                )
                refreshHomeThreadsAfterLocalRunStateChange()
            }
            lastError = displayMessage(for: error)
        }
    }

    func queueRemoteInput(
        _ text: String,
        attachments: [GaryxMobileComposerAttachment],
        in thread: GaryxThreadSummary
    ) async {
        let clientIntentId = "mobile-\(UUID().uuidString)"
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let userMessage = GaryxMobileMessage(
            id: Self.userOriginMessageId(clientIntentId),
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId,
            localState: .optimistic
        )
        mutateMessages(for: thread.id) { messages in
            messages.append(userMessage)
        }
        let queued = GaryxPendingQueuedInput(
            threadId: thread.id,
            text: text,
            attachments: attachments,
            clientIntentId: clientIntentId
        )
        pendingQueuedInputsByIntentId[clientIntentId] = queued
        threadSummaryLeaseOwner.replaceComposerReferences(
            ownerId: clientIntentId,
            threadIds: [thread.id],
            summaries: cachedThreadSummary(for: thread.id).map { [$0] } ?? []
        )
        runTracker.beginQueuedSteer(threadId: thread.id, intentId: clientIntentId, text: visibleUserText)
        await submitQueuedInputViaGateway(queued)
    }

    func submitQueuedInputViaGateway(_ queued: GaryxPendingQueuedInput) async {
        let runtimeGeneration = gatewayRequestToken
        pendingQueuedInputsByIntentId[queued.clientIntentId] = queued
        threadSummaryLeaseOwner.replaceComposerReferences(
            ownerId: queued.clientIntentId,
            threadIds: [queued.threadId],
            summaries: cachedThreadSummary(for: queued.threadId).map { [$0] } ?? []
        )
        do {
            let result = try await client().streamInput(
                GaryxStreamInputRequest(
                    threadId: queued.threadId,
                    clientIntentId: queued.clientIntentId,
                    message: queued.text,
                    attachments: queued.attachments.map(\.promptAttachment)
                )
            )
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else { return }
            if Self.isSuccessfulStreamInputStatus(result.status) {
                bindLocalPendingInput(
                    threadId: queued.threadId,
                    clientIntentId: result.clientIntentId ?? queued.clientIntentId,
                    pendingInputId: result.pendingInputId
                )
                runTracker.confirmQueuedSteerAccepted(
                    threadId: queued.threadId,
                    intentId: queued.clientIntentId,
                    pendingInputId: result.pendingInputId
                )
            } else if Self.shouldFallbackStreamInputStatus(result.status) {
                if let claimed = pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) {
                    await dispatchQueuedInputFallback(claimed, runtimeGeneration: runtimeGeneration)
                }
            } else {
                pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId)
                threadSummaryLeaseOwner.settleComposer(ownerId: queued.clientIntentId)
                let failureMessage = result.status.isEmpty ? "Input was not queued" : result.status
                runTracker.failQueuedSteer(
                    threadId: queued.threadId,
                    intentId: queued.clientIntentId,
                    error: failureMessage
                )
                let markedInput = markLocalInputFailed(
                    threadId: queued.threadId,
                    clientIntentId: result.clientIntentId ?? queued.clientIntentId,
                    pendingInputId: result.pendingInputId,
                    message: failureMessage
                )
                if !markedInput {
                    lastError = failureMessage
                }
            }
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            if pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) != nil {
                threadSummaryLeaseOwner.cancelComposer(ownerId: queued.clientIntentId)
                let message = displayMessage(for: error)
                runTracker.failQueuedSteer(
                    threadId: queued.threadId,
                    intentId: queued.clientIntentId,
                    error: message
                )
                markLocalInputFailed(
                    threadId: queued.threadId,
                    clientIntentId: queued.clientIntentId,
                    pendingInputId: nil,
                    message: message
                )
                lastError = message
            }
        }
    }

    func dispatchQueuedInputFallback(
        _ queued: GaryxPendingQueuedInput,
        runtimeGeneration: GaryxGatewayRequestToken
    ) async {
        defer {
            threadSummaryLeaseOwner.settleComposer(ownerId: queued.clientIntentId)
        }
        guard runtimeGeneration == gatewayRequestToken else { return }
        let fallbackSelectedThread = selectedThread?.id == queued.threadId ? selectedThread : nil
        guard let thread = cachedThreadSummary(for: queued.threadId) ?? fallbackSelectedThread else {
            markLocalInputFailed(
                threadId: queued.threadId,
                clientIntentId: queued.clientIntentId,
                pendingInputId: nil,
                message: "Input was not queued"
            )
            return
        }
        runTracker.releaseQueuedSteer(threadId: queued.threadId, intentId: queued.clientIntentId)
        clearLocalInputStatus(threadId: queued.threadId, clientIntentId: queued.clientIntentId)

        let assistantId = "stream-assistant-\(queued.threadId)-\(UUID().uuidString)"
        mutateMessages(for: queued.threadId) { messages in
            let assistantMessage = GaryxMobileMessage(
                id: assistantId,
                role: .assistant,
                text: "",
                timestamp: nil,
                isStreaming: true,
                localState: .remotePartial
            )
            if let userIndex = messages.indices.last(where: { index in
                messages[index].role == .user && messages[index].clientIntentId == queued.clientIntentId
            }) {
                let insertIndex = messages.index(after: userIndex)
                messages.insert(assistantMessage, at: insertIndex)
            } else {
                messages.append(assistantMessage)
            }
        }

        do {
            // The fallback is a fresh chat dispatch; claiming the chat-start
            // window here keeps a racing transcript reload from clearing the
            // sending state mid-dispatch (the legacy flags missed this).
            runTracker.beginLocalDispatch(
                threadId: queued.threadId,
                intentId: queued.clientIntentId,
                text: queued.text
            )
            refreshHomeThreadsAfterLocalRunStart()
            activeAssistantMessageIdsByThread[queued.threadId] = assistantId
            let workspacePath = Self.firstNonEmpty(
                thread.workspacePath,
                newThreadWorkspaceSelection.workspacePath ?? ""
            )
            try await startChatRunViaGateway(
                threadId: queued.threadId,
                message: queued.text,
                attachments: queued.attachments,
                clientIntentId: queued.clientIntentId,
                workspacePath: workspacePath,
                assistantMessageId: assistantId
            )
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            markLocalInputFailed(
                threadId: queued.threadId,
                clientIntentId: queued.clientIntentId,
                pendingInputId: nil,
                message: displayMessage(for: error)
            )
            markStreamingAssistantComplete(for: queued.threadId, removeEmpty: true)
            activeAssistantMessageIdsByThread[queued.threadId] = nil
            runTracker.failLocalDispatch(
                threadId: queued.threadId,
                intentId: queued.clientIntentId,
                error: displayMessage(for: error)
            )
            refreshHomeThreadsAfterLocalRunStateChange()
            lastError = displayMessage(for: error)
        }
    }

    func startChatRunViaGateway(
        threadId: String,
        message: String,
        attachments: [GaryxMobileComposerAttachment],
        clientIntentId: String,
        workspacePath: String?,
        assistantMessageId: String,
        delivery: GaryxComposerDeliveryHandle? = nil,
        createDeliveryKey: GaryxCreateDeliveryKey? = nil
    ) async throws {
        let runtimeGeneration = gatewayRequestToken
        let result: GaryxStartChatResult
        do {
            let request = GaryxStartChatRequest(
                threadId: threadId,
                message: message,
                attachments: attachments.map(\.promptAttachment),
                workspacePath: workspacePath,
                metadata: [
                    "client": "garyx-mobile",
                    "client_intent_id": clientIntentId,
                    "client_timestamp_local": Self.localChatTimestamp(),
                ]
            )
            if let delivery {
                result = try await client().startChat(
                    request,
                    beforeDispatch: { [composerPayloadCoordinator] _ in
                        try await composerPayloadCoordinator.markTransportAttempted(
                            delivery,
                            createDeliveryKey: createDeliveryKey
                        )
                    }
                )
            } else {
                result = try await client().startChat(request)
            }
            guard Self.isSuccessfulStreamInputStatus(result.status) else {
                throw GaryxGatewayError.encodingFailed(
                    result.status.isEmpty ? "Chat start was not accepted." : result.status
                )
            }
            if let delivery {
                try await composerPayloadCoordinator.acknowledgeDelivery(delivery)
            }
            if let createDeliveryKey {
                try await composerPayloadCoordinator.acknowledgeCreateDelivery(createDeliveryKey)
            }
        } catch {
            let requestError = error
            if let delivery,
               let phase = try? await composerPayloadCoordinator.deliveryPhase(for: delivery) {
                switch phase {
                case .transportAttempted:
                    try? await composerPayloadCoordinator.markDeliveryAmbiguous(delivery)
                case .notDispatched where createDeliveryKey == nil:
                    // The before-dispatch durability gate failed, so transport
                    // provably did not run. Reclaim the outbox quota and
                    // return the envelope to automatic composer placement
                    // before returning the request failure to the caller.
                    try await composerPayloadCoordinator.recoverUndispatchedDelivery(delivery)
                case .notDispatched:
                    if let createDeliveryKey {
                        await composerPayloadCoordinator.markCreateDeliveryAmbiguous(
                            createDeliveryKey
                        )
                    }
                default:
                    break
                }
            }
            if let createDeliveryKey,
               await composerPayloadCoordinator.createDeliveryPhase(
                   for: createDeliveryKey
            ) == .chatStartAttempted {
                await composerPayloadCoordinator.markCreateDeliveryAmbiguous(createDeliveryKey)
            }
            throw requestError
        }
        try Task.checkCancellation()
        guard runtimeGeneration == gatewayRequestToken else {
            throw CancellationError()
        }
        let acceptedThreadId = result.threadId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? threadId
            : result.threadId
        runTracker.confirmChatStartAccepted(
            requestedThreadId: threadId,
            acceptedThreadId: acceptedThreadId,
            intentId: clientIntentId,
            runId: result.runId
        )
        refreshHomeThreadsAfterLocalRunStateChange()
        if !hasPendingDirectFollowUpAssistant(threadId: acceptedThreadId, assistantId: assistantMessageId) {
            activeAssistantMessageIdsByThread[acceptedThreadId] = assistantMessageId
        }
    }

    /// Re-send a user message that previously failed. Removes the failed user bubble +
    /// any trailing failed assistant placeholder and runs the normal send pipeline.
    @discardableResult
    func retryFailedUserMessage(_ messageId: String) async -> Bool {
        guard let threadId = selectedThread?.id else { return false }
        var capturedText: String?
        var capturedAttachments: [GaryxMobileMessageAttachment] = []
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { message in
                message.id == messageId
                    && message.role == .user
                    && (message.statusText?.isEmpty == false)
            }) else {
                return
            }
            capturedText = messages[index].text
            capturedAttachments = messages[index].attachments
            // Drop the failed user bubble plus any trailing local assistant placeholder
            // so the resend path can rebuild the optimistic state cleanly.
            messages.removeSubrange(index..<messages.endIndex)
        }
        guard let text = capturedText else { return false }
        let composerPayloadItems = capturedAttachments.compactMap(Self.composerAttachment(from:))
        lastError = nil
        await send(text, attachments: composerPayloadItems)
        return true
    }

    static func composerAttachment(
        from messageAttachment: GaryxMobileMessageAttachment
    ) -> GaryxMobileComposerAttachment? {
        guard let path = messageAttachment.path?.trimmingCharacters(in: .whitespacesAndNewlines),
              !path.isEmpty
        else { return nil }
        return GaryxMobileComposerAttachment(
            id: messageAttachment.id,
            kind: messageAttachment.kind,
            name: messageAttachment.name,
            mediaType: messageAttachment.mediaType,
            path: path,
            previewDataUrl: messageAttachment.dataUrl
        )
    }

    func interruptActiveRun() async {
        guard let threadId = selectedThread?.id ?? activeRunThreadId else { return }
        var sentGatewayInterrupt = false
        do {
            _ = try await client().interruptThread(threadId: threadId)
            sentGatewayInterrupt = true
        } catch {
            lastError = displayMessage(for: error)
        }
        guard sentGatewayInterrupt else {
            return
        }
        runTracker.interruptConfirmed(threadId: threadId)
        clearActiveRun(threadId: threadId)
        markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        await refreshThreads(source: .userAction)
        if selectedThread?.id == threadId {
            await loadSelectedThreadHistory()
        }
    }

    func advanceSelectedThreadDraftGeneration() {
        selectedThreadDraftGeneration = UUID()
        pendingBotDraftGeneration = nil
        frozenNewThreadAgentTargetGeneration = nil
        clearNewThreadModelOverride()
    }

    func ensureSelectedThread() async throws -> GaryxThreadSummary {
        try await ensureThreadForCurrentDraft(adoptIfDraftStillCurrent: false).thread
    }

    func ensureSelectedThreadForDraftCreation(
        createIntentID: String?
    ) async throws -> GaryxEnsuredThread {
        try await ensureThreadForCurrentDraft(
            adoptIfDraftStillCurrent: true,
            createIntentID: createIntentID
        )
    }

    func ensureThreadForCurrentDraft(
        adoptIfDraftStillCurrent: Bool,
        createIntentID: String? = nil
    ) async throws -> GaryxEnsuredThread {
        if let selectedThread {
            return GaryxEnsuredThread(thread: selectedThread, adoptedSelection: true)
        }
        let runtimeGeneration = gatewayRequestToken
        let draftGeneration = selectedThreadDraftGeneration
        let pendingBotDraft = currentPendingBotDraft()
        let pendingWorkspace = pendingBotDraft?.workspace ?? ""
        let pendingAgentId = pendingBotDraft?.agentId ?? ""
        let pendingBotIdForThread = pendingBotDraft?.botId ?? ""
        let workspace = pendingWorkspace.isEmpty
            ? (newThreadWorkspaceSelection.createPayloadWorkspaceDir ?? "")
            : pendingWorkspace
        let agentId = pendingAgentId.isEmpty
            ? newThreadAgentTargetId()
            : pendingAgentId
        let workspaceMode = pendingWorkspace.isEmpty ? workspaceModeForNewThread(workspace: workspace) : "local"
        let modelOverride = newThreadModelOverride.trimmingCharacters(in: .whitespacesAndNewlines)
        let reasoningEffortOverride = newThreadReasoningEffortOverride
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let serviceTierOverride = newThreadServiceTierOverride
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let createDeliveryKey: GaryxCreateDeliveryKey?
        if let createIntentID {
            createDeliveryKey = try composerPayloadCoordinator.makeCreateDeliveryKey(
                createIntentID: createIntentID
            )
        } else {
            createDeliveryKey = nil
        }
        let createRequest = GaryxCreateThreadRequest(
                workspaceDir: workspace.isEmpty ? nil : workspace,
                workspaceMode: workspaceMode,
                agentId: agentId.isEmpty ? nil : agentId,
                model: modelOverride.isEmpty ? nil : modelOverride,
                modelReasoningEffort: reasoningEffortOverride.isEmpty ? nil : reasoningEffortOverride,
                modelServiceTier: serviceTierOverride.isEmpty ? nil : serviceTierOverride,
                metadata: ["client": "garyx-mobile"]
            )
        let thread: GaryxThreadSummary
        do {
            if createDeliveryKey != nil {
                thread = try await client().createThread(
                    createRequest,
                    beforeDispatch: { [composerPayloadCoordinator] _ in
                        if let createDeliveryKey {
                            try await composerPayloadCoordinator.beginCreateDelivery(
                                createDeliveryKey
                            )
                        }
                    }
                )
            } else {
                thread = try await client().createThread(createRequest)
            }
            if let createDeliveryKey {
                try await composerPayloadCoordinator.recordCreatedThread(
                    thread.id,
                    for: createDeliveryKey
                )
            }
        } catch {
            if let createDeliveryKey {
                await composerPayloadCoordinator.markCreateDeliveryAmbiguous(createDeliveryKey)
            }
            throw error
        }
        try Task.checkCancellation()
        guard runtimeGeneration == gatewayRequestToken else {
            throw CancellationError()
        }
        let mutationId = nextThreadMutationId(kind: "insert", threadId: thread.id)
        let affectedStoreIds = threadMutationHubStore.value
            .residentStoreIdsAffectedByInsert(workspacePath: thread.workspacePath)
        _ = threadMutationHubStore.value.began(
            mutationId: mutationId,
            kind: .insert(threadId: thread.id),
            gatewayRuntimeEpoch: threadMutationHubStore.value.gatewayRuntimeEpoch,
            affectedStoreIds: affectedStoreIds
        )
        let authority = GaryxThreadMutationAuthority(
            membership: .upsertAtHead(threadId: thread.id),
            summary: thread
        )
        _ = threadMutationHubStore.value.committed(
            mutationId: mutationId,
            gatewayRuntimeEpoch: threadMutationHubStore.value.gatewayRuntimeEpoch,
            authority: authority
        )
        applyThreadMutationAuthorityToResidentProviders(authority)
        threadHistoryLoadedIds.insert(thread.id)
        let canAdoptSelection = !adoptIfDraftStillCurrent
            || (selectedThread == nil && selectedThreadDraftGeneration == draftGeneration)
        if canAdoptSelection {
            adoptsDraftConversationToken = true
            if !productionRouteStore.isAttached {
                selectedThread = thread
            }
            draftThreadTitle = thread.title
            clearPendingNewThreadAgentTarget()
            clearNewThreadModelOverride()
        }
        if !pendingBotIdForThread.isEmpty {
            do {
                _ = try await client().bindBot(botId: pendingBotIdForThread, threadId: thread.id)
                if let createDeliveryKey {
                    try await composerPayloadCoordinator.recordCreateBindingCompleted(
                        for: createDeliveryKey
                    )
                }
            } catch {
                if let createDeliveryKey {
                    await composerPayloadCoordinator.markCreateDeliveryAmbiguous(createDeliveryKey)
                }
                throw error
            }
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else {
                throw CancellationError()
            }
            clearPendingBotDraftIfCurrent(
                botId: pendingBotIdForThread,
                workspace: pendingWorkspace,
                agentId: pendingAgentId,
                draftGeneration: draftGeneration
            )
            await refreshRemoteState()
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else {
                throw CancellationError()
            }
        }
        return GaryxEnsuredThread(
            thread: thread,
            adoptedSelection: canAdoptSelection,
            createDeliveryKey: createDeliveryKey
        )
    }

    func currentPendingBotDraft() -> (botId: String, workspace: String, agentId: String)? {
        guard pendingBotDraftGeneration == selectedThreadDraftGeneration else { return nil }
        let botId = pendingBotId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !botId.isEmpty else { return nil }
        let workspace = pendingBotWorkspace?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let agentId = pendingBotAgentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return (botId: botId, workspace: workspace, agentId: agentId)
    }

    static func userOriginMessageId(_ clientIntentId: String) -> String {
        "origin:\(clientIntentId)"
    }

    func clearPendingBotDraftIfCurrent(
        botId: String,
        workspace: String,
        agentId: String,
        draftGeneration: UUID
    ) {
        guard pendingBotDraftGeneration == draftGeneration,
              pendingBotId?.trimmingCharacters(in: .whitespacesAndNewlines) == botId,
              (pendingBotWorkspace?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "") == workspace,
              (pendingBotAgentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "") == agentId else {
            return
        }
        clearPendingBotDraft()
    }
}
