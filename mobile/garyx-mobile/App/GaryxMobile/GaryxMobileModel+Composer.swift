import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func attachFiles(from urls: [URL]) async {
        guard !urls.isEmpty else { return }
        do {
            let localFiles = try urls.map { url in
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
                return (
                    blob: GaryxUploadChatAttachmentBlob(
                        kind: kind,
                        name: name,
                        mediaType: mediaType,
                        dataBase64: encoded
                    ),
                    preview: GaryxPendingUploadPreview(
                        name: name,
                        mediaType: mediaType,
                        previewDataUrl: kind == "image" ? Self.dataUrl(mediaType: mediaType, base64: encoded) : nil
                    )
                )
            }
            let uploaded = try await client().uploadChatAttachments(
                GaryxUploadChatAttachmentsRequest(files: localFiles.map(\.blob))
            )
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
            lastError = displayMessage(for: error)
        }
    }

    func attachImages(_ images: [GaryxMobileSelectedImage]) async {
        guard !images.isEmpty else { return }
        for image in images {
            do {
                let encoded = image.data.base64EncodedString()
                let uploaded = try await client().uploadChatAttachments(
                    GaryxUploadChatAttachmentsRequest(
                        files: [
                            GaryxUploadChatAttachmentBlob(
                                kind: "image",
                                name: image.name,
                                mediaType: image.mediaType,
                                dataBase64: encoded
                            ),
                        ]
                    )
                )
                guard let file = uploaded.files.first else {
                    throw GaryxGatewayError.encodingFailed("Gateway did not return an uploaded image.")
                }
                let fallbackMediaType = image.mediaType.isEmpty ? "image/jpeg" : image.mediaType
                composerAttachments.append(
                    GaryxMobileComposerAttachment(
                        id: "\(file.path)-\(UUID().uuidString)",
                        kind: file.kind.isEmpty ? "image" : file.kind,
                        name: file.name,
                        mediaType: file.mediaType.isEmpty ? fallbackMediaType : file.mediaType,
                        path: file.path,
                        previewDataUrl: Self.dataUrl(mediaType: fallbackMediaType, base64: encoded)
                    )
                )
            } catch {
                lastError = displayMessage(for: error)
                return
            }
        }
    }

    func removeComposerAttachment(_ attachment: GaryxMobileComposerAttachment) {
        composerAttachments.removeAll { $0.id == attachment.id }
    }

    @discardableResult
    func sendDraft() async -> Bool {
        await sendDraft(text: draft)
    }

    @discardableResult
    func sendDraft(text rawText: String) async -> Bool {
        let text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        let attachments = composerAttachments
        guard canSendComposerPayload(text: text, attachments: attachments) else { return false }
        guard !text.isEmpty || !attachments.isEmpty else { return false }
        resetComposerDraft()
        await send(text, attachments: attachments)
        return true
    }

    func send(_ text: String, attachments: [GaryxMobileComposerAttachment] = []) async {
        if let selectedThread, activeTasksByThread[selectedThread.id] != nil {
            await queueInput(text, attachments: attachments, in: selectedThread)
            return
        }
        if let selectedThread, isThreadBusy(selectedThread.id) {
            await queueRemoteInput(text, attachments: attachments, in: selectedThread)
            return
        }

        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let clientIntentId = "mobile-\(UUID().uuidString)"
        let userMessage = GaryxMobileMessage(
            id: "local-user-\(UUID().uuidString)",
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId
        )
        let assistantId = "local-assistant-\(UUID().uuidString)"
        var optimisticThreadId = selectedThread?.id
        let draftOptimisticMessages = [userMessage]
        if let optimisticThreadId {
            finishActiveAssistantSegmentBeforeUserTurn(for: optimisticThreadId)
            mutateMessages(for: optimisticThreadId) { messages in
                messages.append(userMessage)
            }
            activeAssistantMessageIdsByThread[optimisticThreadId] = assistantId
        } else {
            messages = draftOptimisticMessages
        }

        do {
            let ensuredThread = try await ensureSelectedThreadForDraftCreation()
            let thread = ensuredThread.thread
            if optimisticThreadId == nil {
                optimisticThreadId = thread.id
                setMessages(draftOptimisticMessages, for: thread.id)
                activeAssistantMessageIdsByThread[thread.id] = assistantId
            }
            if activeTasksByThread[thread.id] != nil {
                await queueInput(text, attachments: attachments, in: thread)
                return
            }
            guard !remoteBusyThreadIds.contains(thread.id) else {
                markLatestLocalUserFailed(for: thread.id, message: "Thread is busy")
                markStreamingAssistantComplete(for: thread.id, removeEmpty: true)
                return
            }
            isSending = true
            activeRunThreadId = thread.id
            remoteBusyThreadIds.remove(thread.id)
            lastError = nil
            activeAssistantMessageIdsByThread[thread.id] = assistantId
            let workspacePath = Self.firstNonEmpty(
                thread.workspacePath,
                newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            let command = try client().encodeWebSocketCommand(
                .start(
                    threadId: thread.id,
                    message: text,
                    fromId: "garyx-mobile",
                    workspacePath: workspacePath,
                    attachments: attachments.map(\.promptAttachment),
                    metadata: [
                        "client": "garyx-mobile",
                        "client_intent_id": clientIntentId,
                        "client_timestamp_local": Self.localChatTimestamp(),
                    ]
                )
            )
            let task = try await openChatWebSocketAndSend(command: command)
            activeTask = task
            activeTasksByThread[thread.id] = task
            activeReaderTasksByThread[thread.id]?.cancel()
            let readerTask = Task { [weak self, weak task] in
                guard let self, let task else { return }
                await self.receiveEvents(from: task, threadId: thread.id, assistantMessageId: assistantId)
            }
            activeReaderTasksByThread[thread.id] = readerTask
            activeReaderTask = readerTask
        } catch {
            if let optimisticThreadId {
                markLatestLocalUserFailed(for: optimisticThreadId, message: displayMessage(for: error))
                markStreamingAssistantComplete(for: optimisticThreadId, removeEmpty: true)
            } else {
                messages.removeAll { $0.id == assistantId }
                if let index = messages.firstIndex(where: { $0.id == userMessage.id }) {
                    messages[index].statusText = displayMessage(for: error)
                }
            }
            if let optimisticThreadId {
                cancelActiveSocket(for: optimisticThreadId)
            }
            lastError = displayMessage(for: error)
        }
    }

    func queueInput(
        _ text: String,
        attachments: [GaryxMobileComposerAttachment],
        in thread: GaryxThreadSummary
    ) async {
        guard let activeTask = activeTasksByThread[thread.id] else {
            await queueRemoteInput(text, attachments: attachments, in: thread)
            return
        }
        let clientIntentId = "mobile-\(UUID().uuidString)"
        let visibleUserText = Self.visibleUserText(text: text, attachments: attachments)
        let userMessage = GaryxMobileMessage(
            id: "local-user-\(UUID().uuidString)",
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId
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
        do {
            let command = try client().encodeWebSocketCommand(
                .input(
                    threadId: thread.id,
                    message: text,
                    clientIntentId: clientIntentId,
                    attachments: attachments.map(\.promptAttachment)
                )
            )
            try await activeTask.send(.string(command))
        } catch {
            if let claimed = pendingQueuedInputsByIntentId.removeValue(forKey: clientIntentId) {
                cancelActiveSocket(for: thread.id)
                await submitQueuedInputViaGateway(claimed)
            } else {
                markLatestLocalUserFailed(for: thread.id, message: displayMessage(for: error))
                lastError = displayMessage(for: error)
            }
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
            id: "local-user-\(UUID().uuidString)",
            role: .user,
            text: visibleUserText,
            attachments: Self.messageAttachments(from: attachments),
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId
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
        await submitQueuedInputViaGateway(queued)
    }

    func submitQueuedInputViaGateway(_ queued: GaryxPendingQueuedInput) async {
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
            if Self.isSuccessfulStreamInputStatus(result.status) {
                bindLocalPendingInput(
                    threadId: queued.threadId,
                    clientIntentId: result.clientIntentId ?? queued.clientIntentId,
                    pendingInputId: result.pendingInputId
                )
                remoteBusyThreadIds.insert(queued.threadId)
            } else if Self.shouldFallbackStreamInputStatus(result.status) {
                if let claimed = pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) {
                    await dispatchQueuedInputFallback(claimed)
                }
            } else {
                pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId)
                let failureMessage = result.status.isEmpty ? "Input was not queued" : result.status
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
            if pendingQueuedInputsByIntentId.removeValue(forKey: queued.clientIntentId) != nil {
                let message = displayMessage(for: error)
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

    func dispatchQueuedInputFallback(_ queued: GaryxPendingQueuedInput) async {
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
        if activeTasksByThread[queued.threadId] != nil {
            cancelActiveSocket(for: queued.threadId)
        }
        remoteBusyThreadIds.remove(queued.threadId)
        clearLocalInputStatus(threadId: queued.threadId, clientIntentId: queued.clientIntentId)

        let assistantId = "stream-assistant-\(queued.threadId)-\(UUID().uuidString)"
        mutateMessages(for: queued.threadId) { messages in
            let assistantMessage = GaryxMobileMessage(
                id: assistantId,
                role: .assistant,
                text: "",
                timestamp: nil,
                isStreaming: true
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
            isSending = true
            activeRunThreadId = queued.threadId
            activeAssistantMessageIdsByThread[queued.threadId] = assistantId
            let workspacePath = Self.firstNonEmpty(
                thread.workspacePath,
                newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            let command = try client().encodeWebSocketCommand(
                .start(
                    threadId: queued.threadId,
                    message: queued.text,
                    fromId: "garyx-mobile",
                    workspacePath: workspacePath,
                    attachments: queued.attachments.map(\.promptAttachment),
                    metadata: [
                        "client": "garyx-mobile",
                        "client_intent_id": queued.clientIntentId,
                        "client_timestamp_local": Self.localChatTimestamp(),
                    ]
                )
            )
            let task = try await openChatWebSocketAndSend(command: command)
            activeTask = task
            activeTasksByThread[queued.threadId] = task
            activeReaderTasksByThread[queued.threadId]?.cancel()
            let readerTask = Task { [weak self, weak task] in
                guard let self, let task else { return }
                await self.receiveEvents(from: task, threadId: queued.threadId, assistantMessageId: assistantId)
            }
            activeReaderTasksByThread[queued.threadId] = readerTask
            activeReaderTask = readerTask
        } catch {
            markLocalInputFailed(
                threadId: queued.threadId,
                clientIntentId: queued.clientIntentId,
                pendingInputId: nil,
                message: displayMessage(for: error)
            )
            markStreamingAssistantComplete(for: queued.threadId, removeEmpty: true)
            cancelActiveSocket(for: queued.threadId)
            lastError = displayMessage(for: error)
        }
    }

    /// Dial the chat WebSocket and send the first command, retrying transient failures
    /// with bounded backoff before bubbling the error up to the user.
    func openChatWebSocketAndSend(command: String) async throws -> URLSessionWebSocketTask {
        let gateway = try client()
        let policy = gateway.retry
        var attempt = 0
        while true {
            attempt += 1
            try Task.checkCancellation()
            let task = try gateway.makeWebSocketTask()
            task.resume()
            do {
                try await task.send(.string(command))
                return task
            } catch {
                task.cancel(with: .goingAway, reason: nil)
                if Self.isCancellationError(error) {
                    throw error
                }
                let canRetry = attempt < policy.maxAttempts
                    && Self.isRetryableWebSocketSendError(error)
                if !canRetry {
                    throw error
                }
                let delay = policy.delay(forAttempt: attempt)
                if delay > 0 {
                    let nanoseconds = UInt64(delay * 1_000_000_000)
                    try await Task.sleep(nanoseconds: nanoseconds)
                }
            }
        }
    }

    static func isRetryableWebSocketSendError(_ error: Error) -> Bool {
        if GaryxGatewayRetryClassifier.isConnectionEstablishmentError(error) {
            return true
        }
        if GaryxGatewayRetryClassifier.isAmbiguousNetworkError(error) {
            return true
        }
        let nsError = error as NSError
        if nsError.domain == NSPOSIXErrorDomain {
            // POSIX errors raised mid-handshake (ENOTCONN, EPIPE, etc.) — safe to retry.
            return true
        }
        return false
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
        let hadLocalTask = activeTasksByThread[threadId] != nil
        var sentLocalInterrupt = false
        var sentGatewayInterrupt = false
        if let activeTask = activeTasksByThread[threadId] {
            do {
                let command = try client().encodeWebSocketCommand(.interrupt(threadId: threadId))
                try await activeTask.send(.string(command))
                sentLocalInterrupt = true
            } catch {
                // Continue to the gateway-backed interrupt below; the local socket may be stale.
            }
        }
        do {
            _ = try await client().interruptThread(threadId: threadId)
            sentGatewayInterrupt = true
        } catch {
            if !sentLocalInterrupt {
                lastError = displayMessage(for: error)
            }
        }
        if hadLocalTask {
            cancelActiveSocket(for: threadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        }
        guard sentLocalInterrupt || sentGatewayInterrupt || hadLocalTask else {
            return
        }
        remoteBusyThreadIds.remove(threadId)
        await refreshThreads()
        if selectedThread?.id == threadId {
            await loadSelectedThreadHistory()
        }
    }


    func advanceSelectedThreadDraftGeneration() {
        selectedThreadDraftGeneration = UUID()
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
        let draftGeneration = selectedThreadDraftGeneration
        let pendingWorkspace = pendingBotWorkspace?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let pendingAgentId = pendingBotAgentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let pendingBotIdForThread = pendingBotId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let workspace = pendingWorkspace.isEmpty
            ? newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            : pendingWorkspace
        let agentId = pendingAgentId.isEmpty
            ? selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
            : pendingAgentId
        let workspaceMode = pendingWorkspace.isEmpty ? workspaceModeForNewThread(workspace: workspace) : "local"
        let thread = try await client().createThread(
            GaryxCreateThreadRequest(
                workspaceDir: workspace.isEmpty ? nil : workspace,
                workspaceMode: workspaceMode,
                agentId: agentId.isEmpty ? nil : agentId,
                metadata: ["client": "garyx-mobile"]
            )
        )
        threads.insert(thread, at: 0)
        let canAdoptSelection = !adoptIfDraftStillCurrent
            || (selectedThread == nil && selectedThreadDraftGeneration == draftGeneration)
        if canAdoptSelection {
            selectedThread = thread
            draftThreadTitle = thread.title
        }
        if !pendingBotIdForThread.isEmpty {
            _ = try await client().bindBot(botId: pendingBotIdForThread, threadId: thread.id)
            if canAdoptSelection {
                clearPendingBotDraft()
            }
            await refreshRemoteState()
        }
        return GaryxEnsuredThread(thread: thread, adoptedSelection: canAdoptSelection)
    }
}
