import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func attachFiles(from urls: [URL]) async {
        guard !urls.isEmpty else { return }
        let requestToken = gatewayRequestToken
        do {
            // Freeze the destination configuration before the picker result
            // crosses an await. A gateway switch may suspend this scope, but
            // this operation must still upload to and settle in its origin.
            let uploadClient = try client()
            for url in urls {
                let didAccess = url.startAccessingSecurityScopedResource()
                let metadata: GaryxComposerAttachmentMetadata
                let staged: GaryxComposerStagedUpload
                do {
                    metadata = try Self.localAttachmentMetadata(for: url)
                    staged = try await composerPayloadCoordinator.stageAttachment(
                        sourceURL: url,
                        metadata: metadata,
                        requestToken: requestToken
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

    func attachImages(_ images: [GaryxMobileSelectedImage]) async {
        guard !images.isEmpty else { return }
        let requestToken = gatewayRequestToken
        do {
            let uploadClient = try client()
            for image in images {
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
                    requestToken: requestToken
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
        let clientIntentID = "mobile-\(UUID().uuidString)"
        do {
            let payload = try await composerPayloadCoordinator.takeReadyPayload(
                clientIntentID: clientIntentID
            )
            let text = payload.text
                .replacingOccurrences(of: "\r\n", with: "\n")
                .replacingOccurrences(of: "\r", with: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            let attachments = payload.attachments.compactMap { item -> GaryxMobileComposerAttachment? in
                guard let path = item.uploadedPath, !path.isEmpty else { return nil }
                return GaryxMobileComposerAttachment(
                    id: item.id.rawValue,
                    kind: item.kind ?? "file",
                    name: item.name ?? "attachment",
                    mediaType: item.mediaType ?? "application/octet-stream",
                    path: path,
                    previewDataUrl: item.previewDataURL
                )
            }
            guard attachments.count == payload.attachments.count else {
                throw GaryxComposerPayloadRuntimeError.attachmentNotUploaded
            }
            await send(
                text,
                attachments: attachments,
                clientIntentId: clientIntentID,
                delivery: payload.delivery
            )
            return true
        } catch {
            lastError = displayMessage(for: error)
            return false
        }
    }

    func send(
        _ text: String,
        attachments: [GaryxMobileComposerAttachment] = [],
        clientIntentId suppliedClientIntentId: String? = nil,
        delivery: GaryxComposerDeliveryHandle? = nil
    ) async {
        let runtimeGeneration = gatewayRequestToken
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let clientIntentId = suppliedClientIntentId ?? "mobile-\(UUID().uuidString)"
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
        var optimisticThreadId = selectedThread?.id
        let allowBusyFollowUp = optimisticThreadId.map { isThreadBusy($0) } ?? false
        let draftOptimisticMessages = [userMessage]
        if let optimisticThreadId {
            if !allowBusyFollowUp {
                finishActiveAssistantSegmentBeforeUserTurn(for: optimisticThreadId)
            }
            mutateMessages(for: optimisticThreadId) { messages in
                messages.append(userMessage)
            }
            if allowBusyFollowUp {
                pendingDirectFollowUpsByThread[optimisticThreadId, default: []].append((
                    userId: userMessage.id,
                    assistantId: assistantId
                ))
            } else {
                activeAssistantMessageIdsByThread[optimisticThreadId] = assistantId
            }
            // The run is active the instant the user sends. Non-busy sends
            // also show the tail thinking indicator immediately; busy
            // follow-ups wait until the provider ack defines the turn boundary.
            guard runTracker.beginLocalDispatch(
                threadId: optimisticThreadId,
                intentId: clientIntentId,
                text: visibleUserText,
                allowWhileBusy: allowBusyFollowUp
            ) else {
                markLatestLocalUserFailed(for: optimisticThreadId, message: "Thread is busy")
                markStreamingAssistantComplete(for: optimisticThreadId, removeEmpty: true)
                return
            }
        } else {
            messages = draftOptimisticMessages
        }

        do {
            let ensuredThread = try await ensureSelectedThreadForDraftCreation()
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
                newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            try await startChatRunViaGateway(
                threadId: thread.id,
                message: text,
                attachments: attachments,
                clientIntentId: clientIntentId,
                workspacePath: workspacePath,
                assistantMessageId: assistantId,
                delivery: delivery
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
                newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
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
        delivery: GaryxComposerDeliveryHandle? = nil
    ) async throws {
        let runtimeGeneration = gatewayRequestToken
        var crossedTransportBoundary = false
        if let delivery {
            try await composerPayloadCoordinator.markTransportAttempted(delivery)
            crossedTransportBoundary = true
        }
        let result: GaryxStartChatResult
        do {
            result = try await client().startChat(
                GaryxStartChatRequest(
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
            )
            guard Self.isSuccessfulStreamInputStatus(result.status) else {
                throw GaryxGatewayError.encodingFailed(
                    result.status.isEmpty ? "Chat start was not accepted." : result.status
                )
            }
            if let delivery {
                try await composerPayloadCoordinator.acknowledgeDelivery(delivery)
            }
        } catch {
            if crossedTransportBoundary, let delivery {
                try? await composerPayloadCoordinator.markDeliveryAmbiguous(delivery)
            }
            throw error
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
        clearNewThreadModelOverride()
    }

    func ensureSelectedThread() async throws -> GaryxThreadSummary {
        try await ensureThreadForCurrentDraft(adoptIfDraftStillCurrent: false).thread
    }

    func ensureSelectedThreadForDraftCreation() async throws -> GaryxEnsuredThread {
        try await ensureThreadForCurrentDraft(adoptIfDraftStillCurrent: true)
    }

    func ensureThreadForCurrentDraft(adoptIfDraftStillCurrent: Bool) async throws -> GaryxEnsuredThread {
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
            ? newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
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
        let thread = try await client().createThread(
            GaryxCreateThreadRequest(
                workspaceDir: workspace.isEmpty ? nil : workspace,
                workspaceMode: workspaceMode,
                agentId: agentId.isEmpty ? nil : agentId,
                model: modelOverride.isEmpty ? nil : modelOverride,
                modelReasoningEffort: reasoningEffortOverride.isEmpty ? nil : reasoningEffortOverride,
                modelServiceTier: serviceTierOverride.isEmpty ? nil : serviceTierOverride,
                metadata: ["client": "garyx-mobile"]
            )
        )
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
            _ = try await client().bindBot(botId: pendingBotIdForThread, threadId: thread.id)
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
        return GaryxEnsuredThread(thread: thread, adoptedSelection: canAdoptSelection)
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
