import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func updateRemoteBusyState(from event: GaryxChatStreamEvent) {
        runTracker.apply(streamEvent: event)
    }

    func handle(_ event: GaryxChatStreamEvent, threadId: String, assistantMessageId: String, affectsActiveRun: Bool) {
        let eventThreadId = Self.threadId(from: event)
        updateRemoteBusyState(from: event)
        if !Self.isAssistantDeltaEvent(event) {
            flushPendingAssistantDelta(for: threadId)
        }
        switch event {
        case .userMessage(let runId, _, let text, let imageCount):
            appendRemoteUserMessage(
                runId: runId,
                threadId: threadId,
                text: text,
                imageCount: imageCount
            )
        case .assistantDelta(_, _, let delta, _):
            appendAssistantDelta(delta, threadId: threadId, assistantMessageId: assistantMessageId)
        case .assistantBoundary:
            appendAssistantBoundary(threadId: threadId, assistantMessageId: assistantMessageId)
        case .toolUse(_, _, let message):
            appendToolTraceEvent(.toolUse, threadId: threadId, message: message)
        case .toolResult(_, _, let message):
            appendToolTraceEvent(.toolResult, threadId: threadId, message: message)
        case .userAck where affectsActiveRun:
            let nextAssistantId = moveNextPendingDirectFollowUpToAckBoundary(threadId: threadId)
            markActiveAssistantSegmentComplete(for: threadId)
            activeAssistantMessageIdsByThread[threadId] = nextAssistantId
            if selectedThread?.id == eventThreadId {
                Task { await loadSelectedThreadHistory() }
            }
        case .streamInput(let status, _, let clientIntentId, let pendingInputId):
            if Self.isSuccessfulStreamInputStatus(status) {
                bindLocalPendingInput(threadId: threadId, clientIntentId: clientIntentId, pendingInputId: pendingInputId)
            } else {
                let normalizedClientIntentId = clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                if Self.shouldFallbackStreamInputStatus(status),
                   !normalizedClientIntentId.isEmpty,
                   let queued = pendingQueuedInputsByIntentId.removeValue(forKey: normalizedClientIntentId) {
                    Task { await dispatchQueuedInputFallback(queued) }
                    break
                }
                let failureMessage = status.isEmpty ? "Input was not queued" : status
                let markedInput = markLocalInputFailed(
                    threadId: threadId,
                    clientIntentId: clientIntentId,
                    pendingInputId: pendingInputId,
                    message: failureMessage
                )
                if !markedInput {
                    lastError = failureMessage
                }
            }
        case .threadTitleUpdated(_, let threadId, let title):
            applyThreadTitleUpdate(threadId: threadId, title: title)
        case .done where affectsActiveRun:
            pendingDirectFollowUpsByThread[threadId] = nil
            clearActiveRun(threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .runComplete where affectsActiveRun:
            pendingDirectFollowUpsByThread[threadId] = nil
            clearActiveRun(threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .error(_, _, let error) where affectsActiveRun:
            if Self.isTransientGatewayErrorMessage(error) {
                // The tracker keeps the run busy through transient gateway
                // noise; only the status banner and stream UI react here.
                gatewaySettingsStatus = "Waiting to sync with gateway"
                markStreamingAssistantComplete(for: threadId, removeEmpty: true)
            } else {
                lastError = error
                markLatestLocalUserFailed(for: threadId, message: error)
                pendingDirectFollowUpsByThread[threadId] = nil
                markStreamingAssistantComplete(for: threadId, removeEmpty: true)
                clearActiveRun(threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            }
        case .interrupt where affectsActiveRun:
            pendingDirectFollowUpsByThread[threadId] = nil
            clearActiveRun(threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .snapshot(let threadId, let payload):
            guard selectedThread?.id == threadId,
                  let transcript = try? transcript(fromSnapshotPayload: payload) else {
                return
            }
            selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
            let threadRunActive = remoteBusyThreadIds.contains(threadId)
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: threadRunActive)
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript),
                    threadRunActive: threadRunActive
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
        default:
            break
        }
    }

    func moveNextPendingDirectFollowUpToAckBoundary(threadId: String) -> String? {
        guard var pendingFollowUps = pendingDirectFollowUpsByThread[threadId],
              !pendingFollowUps.isEmpty else {
            return nil
        }
        let followUp = pendingFollowUps.removeFirst()
        pendingDirectFollowUpsByThread[threadId] = pendingFollowUps.isEmpty ? nil : pendingFollowUps
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == followUp.userId || $0.remoteId == followUp.userId }) else {
                return
            }
            let message = messages.remove(at: index)
            messages.append(message)
        }
        return followUp.assistantId
    }

    func forgetPendingDirectFollowUp(threadId: String, userId: String, assistantId: String) {
        guard var pendingFollowUps = pendingDirectFollowUpsByThread[threadId] else { return }
        pendingFollowUps.removeAll { $0.userId == userId || $0.assistantId == assistantId }
        pendingDirectFollowUpsByThread[threadId] = pendingFollowUps.isEmpty ? nil : pendingFollowUps
    }

    func hasPendingDirectFollowUpAssistant(threadId: String, assistantId: String) -> Bool {
        pendingDirectFollowUpsByThread[threadId]?.contains { $0.assistantId == assistantId } ?? false
    }

    func hasPendingDirectFollowUpUser(threadId: String, userId: String) -> Bool {
        pendingDirectFollowUpsByThread[threadId]?.contains { $0.userId == userId } ?? false
    }

    func appendAssistantDelta(_ delta: String, threadId: String, assistantMessageId: String) {
        guard !delta.isEmpty else { return }
        let targetId = activeAssistantMessageIdsByThread[threadId]
            ?? "stream-assistant-\(threadId)-\(UUID().uuidString)"
        activeAssistantMessageIdsByThread[threadId] = targetId
        if var pending = pendingAssistantDeltasByThread[threadId],
           pending.targetId == targetId {
            pending.text += delta
            pendingAssistantDeltasByThread[threadId] = pending
        } else {
            pendingAssistantDeltasByThread[threadId] = PendingAssistantDelta(targetId: targetId, text: delta)
        }
        scheduleAssistantDeltaFlush(for: threadId)
    }

    func scheduleAssistantDeltaFlush(for threadId: String) {
        guard assistantDeltaFlushTasksByThread[threadId] == nil else { return }
        assistantDeltaFlushTasksByThread[threadId] = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.assistantDeltaFlushDelayNanos)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                self?.flushPendingAssistantDelta(for: threadId)
            }
        }
    }

    func flushPendingAssistantDelta(for threadId: String) {
        assistantDeltaFlushTasksByThread[threadId]?.cancel()
        assistantDeltaFlushTasksByThread[threadId] = nil
        guard let pending = pendingAssistantDeltasByThread.removeValue(forKey: threadId),
              !pending.text.isEmpty else {
            return
        }
        let targetId = pending.targetId
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == targetId }) else {
                activeAssistantMessageIdsByThread[threadId] = targetId
                messages.append(
                    GaryxMobileMessage(
                        id: targetId,
                        role: .assistant,
                        text: pending.text,
                        timestamp: nil,
                        isStreaming: true,
                        localState: .remotePartial
                    )
                )
                return
            }
            messages[index].text += pending.text
            messages[index].isStreaming = true
        }
    }

    func discardPendingAssistantDelta(for threadId: String) {
        assistantDeltaFlushTasksByThread[threadId]?.cancel()
        assistantDeltaFlushTasksByThread[threadId] = nil
        pendingAssistantDeltasByThread[threadId] = nil
    }

    static func isAssistantDeltaEvent(_ event: GaryxChatStreamEvent) -> Bool {
        if case .assistantDelta = event {
            return true
        }
        return false
    }

    func appendRemoteUserMessage(runId: String, threadId: String, text: String, imageCount: Int) {
        let messageId = runId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "remote-user-\(threadId)-\(UUID().uuidString)"
            : "remote-user-\(runId)"
        let visibleText = Self.remoteUserMessageText(text: text, imageCount: imageCount)
        let existingMessages = cachedMessages(for: threadId)
        let materializedLocalUserIndex = existingMessages.firstIndex { message in
            message.role == .user
                && message.localState != .remoteFinal
                && Self.normalizedMergeText(message.text) == Self.normalizedMergeText(visibleText)
        }
        let materializedLocalUserIsPendingDirectFollowUp = materializedLocalUserIndex.map { index in
            hasPendingDirectFollowUpUser(threadId: threadId, userId: existingMessages[index].id)
        } ?? false
        let alreadyRendered = existingMessages.contains { $0.id == messageId || $0.remoteId == messageId }
        if !alreadyRendered {
            if let materializedLocalUserIndex {
                let activeAssistantId = activeAssistantMessageIdsByThread[threadId]
                let activeAssistantIndex = activeAssistantId.flatMap { id in
                    existingMessages.firstIndex { $0.id == id || $0.remoteId == id }
                }
                if let activeAssistantIndex,
                   activeAssistantIndex < materializedLocalUserIndex,
                   !materializedLocalUserIsPendingDirectFollowUp {
                    finishActiveAssistantSegmentBeforeUserTurn(for: threadId)
                }
            } else {
                finishActiveAssistantSegmentBeforeUserTurn(for: threadId)
            }
        }
        mutateMessages(for: threadId) { messages in
            if messages.contains(where: { $0.id == messageId || $0.remoteId == messageId }) {
                return
            }
            if let localIndex = messages.firstIndex(where: { message in
                message.role == .user
                    && message.localState != .remoteFinal
                    && Self.normalizedMergeText(message.text) == Self.normalizedMergeText(visibleText)
            }) {
                // Materialize in place, keeping the local row id so the list
                // row identity stays stable (no re-created rows on echo).
                messages[localIndex].text = visibleText
                messages[localIndex].statusText = nil
                messages[localIndex].isStreaming = false
                messages[localIndex].localState = .remoteFinal
                messages[localIndex].remoteId = messageId
                return
            }
            messages.append(
                GaryxMobileMessage(
                    id: messageId,
                    role: .user,
                    text: visibleText,
                    timestamp: nil,
                    isStreaming: false,
                    localState: .remoteFinal
                )
            )
        }
    }

    func appendAssistantBoundary(threadId: String, assistantMessageId: String) {
        guard let targetId = activeAssistantMessageIdsByThread[threadId] else {
            return
        }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == targetId }) else {
                return
            }
            let hasText = !messages[index].text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            guard hasText else { return }
            messages[index].text += "\n\n"
            messages[index].isStreaming = true
            activeAssistantMessageIdsByThread[threadId] = messages[index].id
        }
    }

    func markStreamingAssistantComplete(for threadId: String, removeEmpty: Bool = false) {
        mutateMessages(for: threadId) { messages in
            if removeEmpty {
                messages.removeAll { message in
                    message.role == .assistant
                        && message.isStreaming
                        && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                }
            }
            for index in messages.indices where messages[index].isStreaming {
                messages[index].isStreaming = false
                messages[index].toolTraceGroup?.live = false
            }
        }
        activeAssistantMessageIdsByThread[threadId] = nil
    }

    func markActiveAssistantSegmentComplete(for threadId: String) {
        guard let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId] else { return }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId }),
                  messages[index].role == .assistant else {
                return
            }
            messages[index].isStreaming = false
        }
    }

    func finishActiveAssistantSegmentBeforeUserTurn(for threadId: String) {
        flushPendingAssistantDelta(for: threadId)
        markActiveAssistantSegmentComplete(for: threadId)
        removeEmptyActiveAssistantPlaceholder(for: threadId)
        activeAssistantMessageIdsByThread[threadId] = nil
    }

    func suspendStreamingAssistantForBackground(threadId: String) -> String? {
        flushPendingAssistantDelta(for: threadId)
        let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId]
        var preservedAssistantId: String?
        mutateMessages(for: threadId) { messages in
            messages.removeAll { message in
                message.role == .assistant
                    && message.isStreaming
                    && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
            if let activeAssistantMessageId,
               let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId }),
               messages[index].role == .assistant {
                messages[index].isStreaming = true
                preservedAssistantId = activeAssistantMessageId
            }
        }
        return preservedAssistantId
    }

    func markLatestLocalUserFailed(for threadId: String, message: String) {
        mutateMessages(for: threadId) { messages in
            guard let index = messages.indices.last(where: { index in
                messages[index].role == .user
            }) else {
                return
            }
            messages[index].statusText = message
        }
    }

    @discardableResult
    func markLocalInputFailed(
        threadId: String,
        clientIntentId: String?,
        pendingInputId: String?,
        message: String
    ) -> Bool {
        let normalizedClientIntentId = clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let normalizedPendingInputId = pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        var didMark = false
        mutateMessages(for: threadId) { messages in
            let preciseIndex = messages.indices.last(where: { index in
                guard messages[index].role == .user else { return false }
                if !normalizedClientIntentId.isEmpty, messages[index].clientIntentId == normalizedClientIntentId {
                    return true
                }
                if !normalizedPendingInputId.isEmpty, messages[index].pendingInputId == normalizedPendingInputId {
                    return true
                }
                return false
            })
            let fallbackIndex: Int?
            if normalizedClientIntentId.isEmpty && normalizedPendingInputId.isEmpty {
                fallbackIndex = messages.indices.last(where: { index in
                    messages[index].role == .user && messages[index].localState == .optimistic
                })
            } else {
                fallbackIndex = nil
            }
            guard let index = preciseIndex ?? fallbackIndex else {
                return
            }
            messages[index].statusText = message
            didMark = true
        }
        return didMark
    }

    func clearLocalInputStatus(threadId: String, clientIntentId: String) {
        let normalizedClientIntentId = clientIntentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedClientIntentId.isEmpty else { return }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.indices.last(where: { index in
                messages[index].role == .user && messages[index].clientIntentId == normalizedClientIntentId
            }) else {
                return
            }
            messages[index].statusText = nil
        }
    }

    func bindLocalPendingInput(
        threadId: String,
        clientIntentId: String?,
        pendingInputId: String?
    ) {
        let normalizedClientIntentId = clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let normalizedPendingInputId = pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !normalizedClientIntentId.isEmpty || !normalizedPendingInputId.isEmpty else { return }
        if !normalizedClientIntentId.isEmpty {
            pendingQueuedInputsByIntentId.removeValue(forKey: normalizedClientIntentId)
        }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.indices.last(where: { index in
                messages[index].role == .user
                    && (messages[index].clientIntentId == normalizedClientIntentId
                        || messages[index].pendingInputId == normalizedPendingInputId)
            }) else {
                return
            }
            if !normalizedClientIntentId.isEmpty {
                messages[index].clientIntentId = normalizedClientIntentId
            }
            if !normalizedPendingInputId.isEmpty {
                messages[index].pendingInputId = normalizedPendingInputId
            }
            messages[index].statusText = nil
        }
    }

    func mergedMessages(
        _ remoteMessages: [GaryxMobileMessage],
        withLocal localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int? = nil,
        threadRunActive: Bool = true
    ) -> [GaryxMobileMessage] {
        GaryxTranscriptMerge.mergedMessages(
            remoteMessages,
            withLocal: localMessages,
            preserveRemoteBeforeIndex: preserveRemoteBeforeIndex,
            threadRunActive: threadRunActive
        )
    }








    static func normalizedMergeText(_ text: String) -> String {
        GaryxTranscriptMerge.normalizedMergeText(text)
    }

    static func attachmentSummary(from attachments: [GaryxMobileMessageAttachment]) -> String? {
        GaryxStructuredContentRenderer.attachmentSummary(
            from: attachments.map(\.contentDescriptor)
        )
    }

    static func remoteUserMessageText(text: String, imageCount: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            return trimmed
        }
        if imageCount == 1 {
            return "[1 image]"
        }
        if imageCount > 1 {
            return "[\(imageCount) images]"
        }
        return "User message"
    }

    static func visibleUserText(text: String, attachments: [GaryxMobileComposerAttachment]) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.isEmpty else {
            return text
        }
        let imageCount = attachments.filter { $0.kind == "image" || $0.mediaType.hasPrefix("image/") }.count
        let fileCount = max(attachments.count - imageCount, 0)
        var parts: [String] = []
        if imageCount > 0 {
            parts.append("\(imageCount) image\(imageCount == 1 ? "" : "s")")
        }
        if fileCount > 0 {
            parts.append("\(fileCount) file\(fileCount == 1 ? "" : "s")")
        }
        if parts.isEmpty {
            return "User message"
        }
        return "[\(parts.joined(separator: ", "))]"
    }

    static func pendingUserInputText(
        _ input: GaryxPendingUserInput,
        attachments: [GaryxMobileMessageAttachment] = []
    ) -> String {
        let trimmed = input.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            return input.text
        }
        if !attachments.isEmpty {
            return input.content.flatMap { GaryxStructuredContentRenderer.text(from: $0) } ?? ""
        }
        if let contentSummary = input.content.flatMap({ GaryxStructuredContentRenderer.summaryText(from: $0) }),
           !contentSummary.isEmpty {
            return contentSummary
        }
        return "User message"
    }

    static func dataUrl(mediaType: String, base64: String) -> String {
        let normalizedType = mediaType.trimmingCharacters(in: .whitespacesAndNewlines)
        let type = normalizedType.isEmpty ? "application/octet-stream" : normalizedType
        return "data:\(type);base64,\(base64)"
    }

    static func matchedUploadPreview(
        for file: GaryxUploadedChatAttachment,
        from previews: inout [GaryxPendingUploadPreview]
    ) -> GaryxPendingUploadPreview? {
        let fileName = file.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let fileMediaType = file.mediaType.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        let exactMatches = previews.indices.filter { index in
            previews[index].name == fileName
                && (fileMediaType.isEmpty || previews[index].mediaType.lowercased() == fileMediaType)
        }
        if exactMatches.count == 1 {
            return previews.remove(at: exactMatches[0])
        }

        let nameMatches = previews.indices.filter { previews[$0].name == fileName }
        if nameMatches.count == 1 {
            return previews.remove(at: nameMatches[0])
        }

        return nil
    }

    static func messageAttachments(from attachments: [GaryxMobileComposerAttachment]) -> [GaryxMobileMessageAttachment] {
        attachments.map { attachment in
            GaryxMobileMessageAttachment(
                id: attachment.id,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                path: attachment.path,
                dataUrl: attachment.previewDataUrl,
                remoteUrl: nil
            )
        }
    }

    func preserveRemoteBeforeIndex(from transcript: GaryxThreadTranscript) -> Int? {
        transcript.pageInfo?.returnedStartIndex ?? transcript.messages.compactMap(\.index).min()
    }

    func mobileMessages(from transcript: GaryxThreadTranscript, threadId: String, live: Bool = false) -> [GaryxMobileMessage] {
        GaryxMobileTranscriptMapper.appendPendingUserInputs(
            to: mobileMessages(from: transcript.messages, live: live),
            from: transcript
        )
    }

    func mobileMessages(from transcript: [GaryxTranscriptMessage], live: Bool = false) -> [GaryxMobileMessage] {
        GaryxMobileTranscriptMapper.mobileMessages(from: transcript, live: live)
    }

    private func appendToolTraceEvent(_ eventKind: GaryxMobileToolTraceEventKind, threadId: String, message: GaryxJSONValue?) {
        removeEmptyActiveAssistantPlaceholder(for: threadId)
        activeAssistantMessageIdsByThread[threadId] = nil
        guard let entry = GaryxMobileToolTraceEntry(eventKind: eventKind, value: message) else {
            return
        }

        let kind: GaryxMobileTranscriptToolTraceKind = eventKind == .toolResult ? .toolResult : .toolUse
        mutateMessages(for: threadId) { messages in
            GaryxTranscriptMerge.appendLiveToolTraceEntry(entry, kind: kind, into: &messages)
        }
    }

    func removeEmptyActiveAssistantPlaceholder(for threadId: String) {
        guard let activeAssistantMessageId = activeAssistantMessageIdsByThread[threadId] else {
            return
        }
        mutateMessages(for: threadId) { messages in
            guard let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId }),
                  messages[index].role == .assistant,
                  messages[index].text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                return
            }
            messages.remove(at: index)
        }
    }

    func startGlobalEventStream() {
        guard hasGatewaySettings, canConnectGateway else { return }
        globalEventStreamTask?.cancel()
        let generation = UUID()
        globalEventStreamGeneration = generation
        globalEventStreamActive = false
        globalEventStreamTask = Task { [weak self] in
            guard let self else { return }
            await self.runGlobalEventStream(generation: generation)
        }
    }

    func cancelGlobalEventStream() {
        globalEventStreamTask?.cancel()
        globalEventStreamTask = nil
        globalEventStreamGeneration = nil
        globalEventStreamActive = false
    }

    func runGlobalEventStream(generation: UUID) async {
        var retryDelay: UInt64 = 1_000_000_000
        while !Task.isCancelled, hasGatewaySettings {
            guard globalEventStreamGeneration == generation else { break }
            do {
                let request = try client().eventStreamRequest(historyLimit: 50)
                let (bytes, response) = try await URLSession.shared.bytes(for: request)
                guard let http = response as? HTTPURLResponse,
                      (200..<300).contains(http.statusCode) else {
                    throw GaryxGatewayError.invalidHTTPResponse
                }
                guard globalEventStreamGeneration == generation else { break }
                globalEventStreamActive = true
                retryDelay = 1_000_000_000
                var dataLines: [String] = []
                for try await line in bytes.lines {
                    if Task.isCancelled { break }
                    guard globalEventStreamGeneration == generation else { break }
                    if line.isEmpty {
                        if !dataLines.isEmpty {
                            await handleGlobalEventStreamPayload(dataLines.joined(separator: "\n"))
                            dataLines.removeAll()
                        }
                        continue
                    }
                    if line.hasPrefix(":") {
                        continue
                    }
                    guard line.hasPrefix("data:") else {
                        continue
                    }
                    var value = String(line.dropFirst(5))
                    if value.hasPrefix(" ") {
                        value.removeFirst()
                    }
                    dataLines.append(value)
                }
            } catch {
                if !Task.isCancelled, globalEventStreamGeneration == generation {
                    globalEventStreamActive = false
                    if case .ready = connectionState {
                        gatewaySettingsStatus = "Live updates disconnected"
                    }
                    await refreshThreads()
                    if selectedThread != nil {
                        await loadSelectedThreadHistory()
                    }
                }
            }
            if globalEventStreamGeneration == generation {
                globalEventStreamActive = false
            }
            if Task.isCancelled { break }
            try? await Task.sleep(nanoseconds: retryDelay)
            retryDelay = min(retryDelay * 2, 10_000_000_000)
            if globalEventStreamGeneration == generation, case .ready = connectionState {
                gatewaySettingsStatus = nil
            }
        }
        if globalEventStreamGeneration == generation {
            globalEventStreamActive = false
        }
    }

    func handleGlobalEventStreamPayload(_ payload: String, replay: Bool = false) async {
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if let data = trimmed.data(using: .utf8),
           let object = try? JSONSerialization.jsonObject(with: data),
           let dictionary = object as? [String: Any],
            let type = dictionary["type"] as? String {
            if type == "history" {
                let events = dictionary["events"] as? [String] ?? []
                var shouldReloadSelectedHistory = false
                var titleUpdates: [(threadId: String, title: String)] = []
                for eventPayload in events {
                    if let event = try? client().decodeStreamEvent(eventPayload) {
                        if case .threadTitleUpdated(_, let threadId, let title) = event {
                            titleUpdates.append((threadId: threadId, title: title))
                        }
                        if selectedThread?.id == Self.threadId(from: event) {
                            switch event {
                            case .done, .runComplete, .error, .interrupt:
                                shouldReloadSelectedHistory = true
                            default:
                                break
                            }
                        }
                    }
                    await handleGlobalEventStreamPayload(eventPayload, replay: true)
                }
                await refreshThreads()
                for update in titleUpdates {
                    applyThreadTitleUpdate(threadId: update.threadId, title: update.title)
                }
                if shouldReloadSelectedHistory {
                    await loadSelectedThreadHistory()
                }
                return
            }
            if type == "snapshot", dictionary["thread_id"] == nil, dictionary["threadId"] == nil {
                return
            }
        }
        guard let event = try? client().decodeStreamEvent(trimmed) else {
            return
        }
        await handleGlobalStreamEvent(event, replay: replay)
    }

    func handleGlobalStreamEvent(
        _ event: GaryxChatStreamEvent,
        replay: Bool = false,
        bypassStreamOwnership: Bool = false
    ) async {
        let threadId = Self.threadId(from: event)
        guard !threadId.isEmpty else { return }
        // S5: when the resumable per-thread stream owns this thread, it applies the
        // thread's transcript events; the global stream must skip them to avoid
        // double-applying deltas. The per-thread consumer calls this with
        // bypassStreamOwnership: true.
        if !bypassStreamOwnership, threadId == streamOwnedThreadId {
            return
        }

        if replay {
            updateRemoteBusyState(from: event)
            switch event {
            case .threadTitleUpdated(_, let threadId, let title):
                applyThreadTitleUpdate(threadId: threadId, title: title)
            default:
                break
            }
            return
        }

        let assistantMessageId = activeAssistantMessageIdsByThread[threadId]
            ?? "stream-assistant-\(threadId)-\(UUID().uuidString)"
        handle(event, threadId: threadId, assistantMessageId: assistantMessageId, affectsActiveRun: true)

        switch event {
        case .threadTitleUpdated(_, let threadId, let title):
            applyThreadTitleUpdate(threadId: threadId, title: title)
        case .done, .runComplete:
            await refreshThreads()
            if selectedThread?.id == threadId {
                await loadSelectedThreadHistory()
            }
        case .error(_, _, let error):
            if Self.isTransientGatewayErrorMessage(error), selectedThread?.id == threadId {
                await loadSelectedThreadHistory()
            }
        default:
            break
        }
    }

    /// Full reset of all tracked run state (gateway switch, debug snapshot).
    func clearActiveRunState() {
        for threadId in Array(pendingAssistantDeltasByThread.keys) {
            flushPendingAssistantDelta(for: threadId)
        }
        if let activeRunThreadId {
            activeAssistantMessageIdsByThread[activeRunThreadId] = nil
        }
        runTracker = GaryxConversationRunTracker()
    }

    func clearActiveRunState(for threadId: String) {
        flushPendingAssistantDelta(for: threadId)
        activeAssistantMessageIdsByThread[threadId] = nil
        runTracker.clearLocalRun(threadId: threadId)
    }

    /// Stream/transcript-side cleanup after a run terminated. The tracker has
    /// already released the runtime via `apply(streamEvent:)` or
    /// `interruptConfirmed`; this clears the model-side streaming state.
    func clearActiveRun(threadId: String?) {
        guard let resolvedThreadId = threadId else {
            clearActiveRunState()
            return
        }

        flushPendingAssistantDelta(for: resolvedThreadId)
        activeAssistantMessageIdsByThread[resolvedThreadId] = nil
        runTracker.clearLocalRun(threadId: resolvedThreadId)
        cancelSelectedThreadRecoveryIfNeeded(threadId: resolvedThreadId)
    }

    func cancelSelectedThreadRecoveryIfNeeded(threadId: String) {
        guard selectedThreadRecoveryThreadId == threadId else { return }
        selectedThreadRecoveryTask?.cancel()
        selectedThreadRecoveryTask = nil
        selectedThreadRecoveryThreadId = nil
    }

    static func localChatTimestamp() -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: Date())
    }

    static func isTransientGatewayErrorMessage(_ message: String) -> Bool {
        GaryxGatewayStreamStatusClassifier.isTransientGatewayErrorMessage(message)
    }

    static func isSuccessfulStreamInputStatus(_ status: String) -> Bool {
        GaryxGatewayStreamStatusClassifier.isSuccessfulStreamInput(status)
    }

    static func shouldFallbackStreamInputStatus(_ status: String) -> Bool {
        GaryxGatewayStreamStatusClassifier.shouldFallbackStreamInput(status)
    }

    static func threadId(from event: GaryxChatStreamEvent) -> String {
        switch event {
        case .accepted(_, let threadId),
             .assistantDelta(_, let threadId, _, _),
             .assistantBoundary(_, let threadId),
             .toolUse(_, let threadId, _),
             .toolResult(_, let threadId, _),
             .userMessage(_, let threadId, _, _),
             .userAck(_, let threadId, _),
             .threadTitleUpdated(_, let threadId, _),
             .done(_, let threadId),
             .runComplete(_, let threadId),
             .streamInput(_, let threadId, _, _),
             .interrupt(_, let threadId, _),
             .snapshot(let threadId, _),
             .error(_, let threadId, _):
            return threadId
        case .ping, .unknown:
            return ""
        }
    }

    static func runId(from event: GaryxChatStreamEvent) -> String {
        switch event {
        case .accepted(let runId, _),
             .assistantDelta(let runId, _, _, _),
             .assistantBoundary(let runId, _),
             .toolUse(let runId, _, _),
             .toolResult(let runId, _, _),
             .userMessage(let runId, _, _, _),
             .userAck(let runId, _, _),
             .threadTitleUpdated(let runId, _, _),
             .done(let runId, _),
             .runComplete(let runId, _),
             .error(let runId, _, _):
            return runId
        case .streamInput, .interrupt, .snapshot, .ping, .unknown:
            return ""
        }
    }

}
