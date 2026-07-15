import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

private struct GaryxPreparedLocalChatFile: Sendable {
    let blob: GaryxUploadChatAttachmentBlob
    let preview: GaryxPendingUploadPreview
}

extension GaryxMobileModel {
    func attachFiles(from urls: [URL]) async {
        guard !urls.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let localFiles = try await Task.detached(priority: .userInitiated) {
                try Self.prepareLocalChatFiles(from: urls)
            }.value
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let uploaded = try await client().uploadChatAttachments(
                GaryxUploadChatAttachmentsRequest(files: localFiles.map(\.blob))
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            guard uploaded.files.count == localFiles.count else {
                throw GaryxGatewayError.encodingFailed(
                    "Gateway did not return every uploaded file."
                )
            }
            var previews = localFiles.map(\.preview)
            composerAttachments.append(
                contentsOf: uploaded.files.map { file in
                    let preview = Self.matchedUploadPreview(for: file, from: &previews)
                    return GaryxMobileComposerAttachment(
                        id: "\(file.path)-\(UUID().uuidString)",
                        kind: file.kind,
                        name: file.name,
                        mediaType: file.mediaType,
                        path: file.path,
                        previewDataUrl: preview?.previewDataUrl
                    )
                }
            )
        } catch {
            guard !Task.isCancelled, runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    nonisolated private static func prepareLocalChatFiles(
        from urls: [URL]
    ) throws -> [GaryxPreparedLocalChatFile] {
        try urls.map { url in
            let didAccess = url.startAccessingSecurityScopedResource()
            defer {
                if didAccess {
                    url.stopAccessingSecurityScopedResource()
                }
            }
            let data = try Data(contentsOf: url)
            let resourceValues = try? url.resourceValues(forKeys: [.contentTypeKey])
            let mediaType = resourceValues?.contentType?.preferredMIMEType
                ?? UTType(filenameExtension: url.pathExtension)?.preferredMIMEType
                ?? "application/octet-stream"
            let kind = mediaType.hasPrefix("image/") ? "image" : "file"
            let encoded = data.base64EncodedString()
            let name = url.lastPathComponent.isEmpty ? "attachment" : url.lastPathComponent
            return GaryxPreparedLocalChatFile(
                blob: GaryxUploadChatAttachmentBlob(
                    kind: kind,
                    name: name,
                    mediaType: mediaType,
                    dataBase64: encoded
                ),
                preview: GaryxPendingUploadPreview(
                    name: name,
                    mediaType: mediaType,
                    previewDataUrl: kind == "image"
                        ? Self.dataUrl(mediaType: mediaType, base64: encoded)
                        : nil
                )
            )
        }
    }

    func attachImages(_ images: [GaryxMobileSelectedImage]) async {
        guard !images.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        let batch = await Task.detached(priority: .userInitiated) {
            GaryxChatImageUploadBatch.prepare(images)
        }.value
        guard runtimeGeneration == gatewayRuntimeGeneration else { return }
        do {
            let uploaded = try await client().uploadChatAttachments(batch.request)
            let attachments = await Task.detached(priority: .userInitiated) {
                batch.composerAttachments(from: uploaded.files)
            }.value
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            guard let attachments else {
                throw GaryxGatewayError.encodingFailed(
                    "Gateway did not return every uploaded image."
                )
            }
            composerAttachments.append(contentsOf: attachments)
        } catch {
            guard !Task.isCancelled, runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func removeComposerAttachment(_ attachment: GaryxMobileComposerAttachment) {
        composerAttachments.removeAll { $0.id == attachment.id }
    }

    @discardableResult
    func sendDraft() async -> Bool {
        await sendDraft(text: activeComposerDraft)
    }

    @discardableResult
    func sendDraft(text rawText: String) async -> Bool {
        // The iOS composer emits carriage returns (`\r`) for line breaks, and
        // pasted text can carry `\r\n`. Downstream storage, the provider prompt,
        // and the `\n`-based transcript renderer only understand `\n`, so collapse
        // line endings before trimming the surrounding whitespace.
        let text = rawText
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let attachments = composerAttachments
        guard canSendComposerPayload(text: text, attachments: attachments) else { return false }
        guard !text.isEmpty || !attachments.isEmpty else { return false }
        resetComposerDraft()
        await send(text, attachments: attachments)
        return true
    }

    func send(_ text: String, attachments: [GaryxMobileComposerAttachment] = []) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let clientIntentId = "mobile-\(UUID().uuidString)"
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
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let thread = ensuredThread.thread
            if optimisticThreadId == nil {
                optimisticThreadId = thread.id
                setMessages(draftOptimisticMessages, for: thread.id)
                activeAssistantMessageIdsByThread[thread.id] = assistantId
                // The thread was just created from the new-thread composer; bind
                // the draft context to it so any follow-up text is saved against
                // this thread instead of the new-thread buffer.
                switchComposerDraft(to: thread.id)
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
                assistantMessageId: assistantId
            )
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
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
        runTracker.beginQueuedSteer(threadId: thread.id, intentId: clientIntentId, text: visibleUserText)
        await submitQueuedInputViaGateway(queued)
    }

    func submitQueuedInputViaGateway(_ queued: GaryxPendingQueuedInput) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        pendingQueuedInputsByIntentId[queued.clientIntentId] = queued
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
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
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
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) != nil {
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
        runtimeGeneration: UUID
    ) async {
        guard runtimeGeneration == gatewayRuntimeGeneration else { return }
        let fallbackSelectedThread = selectedThread?.id == queued.threadId ? selectedThread : nil
        guard let thread = threads.first(where: { $0.id == queued.threadId }) ?? fallbackSelectedThread else {
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
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
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
        assistantMessageId: String
    ) async throws {
        let runtimeGeneration = gatewayRuntimeGeneration
        let result = try await client().startChat(
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
        try Task.checkCancellation()
        guard runtimeGeneration == gatewayRuntimeGeneration else {
            throw CancellationError()
        }
        guard Self.isSuccessfulStreamInputStatus(result.status) else {
            throw GaryxGatewayError.encodingFailed(
                result.status.isEmpty ? "Chat start was not accepted." : result.status
            )
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
        let composerAttachments = capturedAttachments.compactMap(Self.composerAttachment(from:))
        lastError = nil
        await send(text, attachments: composerAttachments)
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
        let runtimeGeneration = gatewayRuntimeGeneration
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
        guard runtimeGeneration == gatewayRuntimeGeneration else {
            throw CancellationError()
        }
        threads.insert(thread, at: 0)
        recentThreadFeeds.upsertChat(threadId: thread.id)
        threadHistoryLoadedIds.insert(thread.id)
        let canAdoptSelection = !adoptIfDraftStillCurrent
            || (selectedThread == nil && selectedThreadDraftGeneration == draftGeneration)
        if canAdoptSelection {
            adoptsDraftConversationToken = true
            selectedThread = thread
            draftThreadTitle = thread.title
            clearPendingNewThreadAgentTarget()
            clearNewThreadModelOverride()
        }
        if !pendingBotIdForThread.isEmpty {
            _ = try await client().bindBot(botId: pendingBotIdForThread, threadId: thread.id)
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRuntimeGeneration else {
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
            guard runtimeGeneration == gatewayRuntimeGeneration else {
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
