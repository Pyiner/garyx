import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func receiveEvents(from task: URLSessionWebSocketTask, threadId: String, assistantMessageId: String) async {
        while !Task.isCancelled {
            do {
                let message = try await task.receive()
                let text: String
                switch message {
                case .string(let value):
                    text = value
                case .data(let data):
                    text = String(data: data, encoding: .utf8) ?? ""
                @unknown default:
                    text = ""
                }
                guard !text.isEmpty else { continue }
                let event = try client().decodeStreamEvent(text)
                let eventThreadId = Self.threadId(from: event)
                let affectsActiveRun = eventThreadId == threadId
                    || (eventThreadId.isEmpty && activeTasksByThread[threadId] === task)
                if !affectsActiveRun {
                    updateRemoteBusyState(from: event)
                    continue
                }
                let isVisibleThread = selectedThread?.id == threadId
                if !isVisibleThread {
                    handle(event, threadId: threadId, assistantMessageId: assistantMessageId, affectsActiveRun: affectsActiveRun)
                    if case .done = event {
                        task.cancel(with: .normalClosure, reason: nil)
                        clearActiveRun(task: task, threadId: threadId)
                        await refreshThreads()
                        return
                    }
                    if case .runComplete = event {
                        task.cancel(with: .normalClosure, reason: nil)
                        clearActiveRun(task: task, threadId: threadId)
                        await refreshThreads()
                        return
                    }
                    if case let .error(_, _, error) = event {
                        if Self.isTransientGatewayErrorMessage(error) {
                            remoteBusyThreadIds.insert(threadId)
                            await refreshThreads()
                        }
                        task.cancel(with: .goingAway, reason: nil)
                        clearActiveRun(task: task, threadId: threadId)
                        return
                    }
                    if case .interrupt = event {
                        clearActiveRun(task: task, threadId: threadId)
                        return
                    }
                    continue
                }
                handle(event, threadId: threadId, assistantMessageId: assistantMessageId, affectsActiveRun: affectsActiveRun)
                if case .done = event, affectsActiveRun {
                    task.cancel(with: .normalClosure, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    await refreshThreads()
                    if selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
                if case .runComplete = event, affectsActiveRun {
                    task.cancel(with: .normalClosure, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    await refreshThreads()
                    if selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
                if case let .error(_, _, error) = event, affectsActiveRun {
                    task.cancel(with: .goingAway, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    if Self.isTransientGatewayErrorMessage(error), selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
                if case .interrupt = event, affectsActiveRun {
                    task.cancel(with: .normalClosure, reason: nil)
                    clearActiveRun(task: task, threadId: threadId)
                    await refreshThreads()
                    if selectedThread?.id == threadId {
                        await loadSelectedThreadHistory()
                    }
                    return
                }
            } catch {
                guard activeTasksByThread[threadId] === task else {
                    return
                }
                let message = displayMessage(for: error)
                let isTransient = Self.isTransientGatewayErrorMessage(message)
                if isTransient {
                    remoteBusyThreadIds.insert(threadId)
                    gatewaySettingsStatus = "Waiting to sync with gateway"
                } else if isSending {
                    lastError = message
                }
                clearActiveRun(task: task, threadId: threadId)
                if selectedThread?.id == threadId {
                    if isTransient {
                        await loadSelectedThreadHistory()
                    } else {
                        markStreamingAssistantComplete(for: threadId, removeEmpty: true)
                    }
                }
                return
            }
        }
    }

    func updateRemoteBusyState(from event: GaryxChatStreamEvent) {
        let threadId = Self.threadId(from: event)
        guard !threadId.isEmpty else { return }
        switch event {
        case .accepted,
             .userMessage,
             .assistantDelta,
             .assistantBoundary,
             .userAck,
             .toolUse,
             .toolResult:
            if activeTasksByThread[threadId] == nil {
                remoteBusyThreadIds.insert(threadId)
            } else {
                remoteBusyThreadIds.remove(threadId)
            }
        case .streamInput(let status, _, _, _):
            if Self.isSuccessfulStreamInputStatus(status) {
                if activeTasksByThread[threadId] == nil {
                    remoteBusyThreadIds.insert(threadId)
                } else {
                    remoteBusyThreadIds.remove(threadId)
                }
            } else if activeTasksByThread[threadId] == nil {
                remoteBusyThreadIds.remove(threadId)
            }
        case .done, .runComplete, .error, .interrupt:
            remoteBusyThreadIds.remove(threadId)
        default:
            break
        }
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
            markActiveAssistantSegmentComplete(for: threadId)
            activeAssistantMessageIdsByThread[threadId] = nil
            if selectedThread?.id == eventThreadId,
               activeTasksByThread[eventThreadId] == nil {
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
            if !eventThreadId.isEmpty {
                remoteBusyThreadIds.remove(eventThreadId)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .runComplete where affectsActiveRun:
            if !eventThreadId.isEmpty {
                remoteBusyThreadIds.remove(eventThreadId)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .error(_, _, let error) where affectsActiveRun:
            if Self.isTransientGatewayErrorMessage(error) {
                if !eventThreadId.isEmpty {
                    remoteBusyThreadIds.insert(eventThreadId)
                }
                gatewaySettingsStatus = "Waiting to sync with gateway"
                markStreamingAssistantComplete(for: threadId, removeEmpty: true)
            } else {
                if !eventThreadId.isEmpty {
                    remoteBusyThreadIds.remove(eventThreadId)
                }
                lastError = error
                markLatestLocalUserFailed(for: threadId, message: error)
                markStreamingAssistantComplete(for: threadId, removeEmpty: true)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
        case .interrupt where affectsActiveRun:
            if !eventThreadId.isEmpty {
                remoteBusyThreadIds.remove(eventThreadId)
            }
            clearActiveRun(task: nil, threadId: eventThreadId.isEmpty ? threadId : eventThreadId)
            markStreamingAssistantComplete(for: threadId, removeEmpty: true)
        case .snapshot(let threadId, let payload):
            guard selectedThread?.id == threadId,
                  let transcript = try? transcript(fromSnapshotPayload: payload) else {
                return
            }
            selectedThreadActivitySignatures[threadId] = GaryxThreadActivitySignature.make(from: transcript)
            updateThreadRuntimeState(threadId: threadId, transcript: transcript)
            scheduleSelectedThreadRecoveryIfNeeded(threadId: threadId)
            if activeTasksByThread[threadId] != nil {
                return
            }
            let remoteMessages = mobileMessages(from: transcript, threadId: threadId, live: remoteBusyThreadIds.contains(threadId))
            setMessages(
                mergedMessages(
                    remoteMessages,
                    withLocal: cachedMessages(for: threadId),
                    preserveRemoteBeforeIndex: preserveRemoteBeforeIndex(from: transcript)
                ),
                for: threadId,
                reconcileActiveAssistant: true
            )
        default:
            break
        }
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
                        isStreaming: true
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
        let materializesExistingLocalUser = existingMessages.contains { message in
            message.role == .user
                && (message.id.hasPrefix("local-user-") || message.id.hasPrefix("pending-user:"))
                && Self.normalizedMergeText(message.text) == Self.normalizedMergeText(visibleText)
        }
        if !existingMessages.contains(where: { $0.id == messageId }) && !materializesExistingLocalUser {
            finishActiveAssistantSegmentBeforeUserTurn(for: threadId)
        }
        mutateMessages(for: threadId) { messages in
            if messages.contains(where: { $0.id == messageId }) {
                return
            }
            if let localIndex = messages.firstIndex(where: { message in
                message.role == .user
                    && (message.id.hasPrefix("local-user-") || message.id.hasPrefix("pending-user:"))
                    && Self.normalizedMergeText(message.text) == Self.normalizedMergeText(visibleText)
            }) {
                let local = messages[localIndex]
                let remoteMessage = GaryxMobileMessage(
                    id: messageId,
                    role: .user,
                    text: visibleText,
                    attachments: local.attachments,
                    timestamp: local.timestamp,
                    isStreaming: false,
                    statusText: local.statusText,
                    clientIntentId: local.clientIntentId,
                    pendingInputId: local.pendingInputId
                )
                messages[localIndex] = remoteMessage
                return
            }
            messages.append(
                GaryxMobileMessage(
                    id: messageId,
                    role: .user,
                    text: visibleText,
                    timestamp: nil,
                    isStreaming: false
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
                    messages[index].role == .user && messages[index].id.hasPrefix("local-user-")
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
        preserveRemoteBeforeIndex: Int? = nil
    ) -> [GaryxMobileMessage] {
        guard !remoteMessages.isEmpty else {
            return localMessages
        }

        var merged = remoteMessages
        var preservedOlderRemoteMessages: [GaryxMobileMessage] = []
        var preservedOlderRemoteIds = Set<String>()
        var remoteUserTextCounts = Dictionary(
            grouping: remoteMessages.filter { $0.role == .user }.map(Self.userMergeKey),
            by: { $0 }
        )
        .mapValues(\.count)
        for localRemoteUserText in localMessages
            .filter({ $0.role == .user && !$0.id.hasPrefix("local-user-") })
            .map(Self.userMergeKey) {
            if let count = remoteUserTextCounts[localRemoteUserText], count > 0 {
                remoteUserTextCounts[localRemoteUserText] = count - 1
            }
        }
        let currentTurnRemoteAssistantTexts = Self.currentTurnAssistantTexts(in: remoteMessages)
        let remoteClientIntentIds = Set(remoteMessages.compactMap { $0.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty })
        let remotePendingInputIds = Set(remoteMessages.compactMap { $0.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty })

        var isAfterUnmaterializedLocalUser = false
        for local in localMessages {
            if let remoteIndex = merged.firstIndex(where: { $0.id == local.id }) {
                if local.role == .assistant,
                   local.isStreaming,
                   merged[remoteIndex].role == .assistant,
                   Self.normalizedMergeText(local.text).count > Self.normalizedMergeText(merged[remoteIndex].text).count {
                    merged[remoteIndex] = local
                }
                continue
            }
            if let preserveRemoteBeforeIndex,
               let historyIndex = Self.historyIndex(fromMessageId: local.id),
               historyIndex < preserveRemoteBeforeIndex,
               preservedOlderRemoteIds.insert(local.id).inserted {
                preservedOlderRemoteMessages.append(local)
                continue
            }
            let localClientIntentId = local.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let localPendingInputId = local.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !localClientIntentId.isEmpty,
               remoteClientIntentIds.contains(localClientIntentId) {
                isAfterUnmaterializedLocalUser = false
                continue
            }
            if !localPendingInputId.isEmpty,
               remotePendingInputIds.contains(localPendingInputId) {
                isAfterUnmaterializedLocalUser = false
                continue
            }
            let normalizedText = Self.normalizedMergeText(local.text)
            switch local.role {
            case .user:
                if local.id.hasPrefix("local-user-") {
                    let mergeKey = Self.userMergeKey(local)
                    if let count = remoteUserTextCounts[mergeKey],
                       count > 0 {
                        remoteUserTextCounts[mergeKey] = count - 1
                        isAfterUnmaterializedLocalUser = false
                        continue
                    }
                    merged.append(local)
                    isAfterUnmaterializedLocalUser = true
                } else {
                    isAfterUnmaterializedLocalUser = false
                }
            case .assistant:
                if local.isStreaming || local.id.hasPrefix("local-assistant-") {
                    if isAfterUnmaterializedLocalUser {
                        merged.append(local)
                        continue
                    }
                    let alreadyMaterialized = currentTurnRemoteAssistantTexts.contains { remoteText in
                        !normalizedText.isEmpty
                            && !remoteText.isEmpty
                            && remoteText.count >= normalizedText.count
                            && remoteText.hasPrefix(normalizedText)
                    }
                    if !alreadyMaterialized {
                        merged.append(local)
                    }
                }
            case .tool:
                if local.isStreaming || local.toolTraceGroup?.isActive == true {
                    if let localGroup = local.toolTraceGroup,
                       let remoteIndex = merged.indices.first(where: { remoteIndex in
                           let remote = merged[remoteIndex]
                           guard let remoteGroup = remote.toolTraceGroup else { return false }
                           return Self.toolTraceGroupsOverlap(
                               remoteGroup,
                               localGroup,
                               allowFingerprint: Self.isInCurrentTurn(index: remoteIndex, messages: merged)
                           )
                       }) {
                        if var remoteGroup = merged[remoteIndex].toolTraceGroup {
                            remoteGroup = Self.mergedToolTraceGroup(remoteGroup, with: localGroup)
                            merged[remoteIndex].toolTraceGroup = remoteGroup
                            merged[remoteIndex].text = remoteGroup.summary
                            merged[remoteIndex].isStreaming = remoteGroup.isActive
                        }
                        continue
                    }
                    merged.append(local)
                }
            case .system:
                if local.statusText != nil || local.id.hasPrefix("local-") {
                    merged.append(local)
                }
            }
        }

        if !preservedOlderRemoteMessages.isEmpty {
            merged = preservedOlderRemoteMessages + merged
        }
        return merged
    }

    static func historyIndex(fromMessageId id: String) -> Int? {
        guard let range = id.range(of: "history:") else { return nil }
        let suffix = id[range.upperBound...]
        let digits = suffix.prefix { $0.isNumber }
        guard !digits.isEmpty else { return nil }
        return Int(digits)
    }

    static func currentTurnAssistantTexts(in messages: [GaryxMobileMessage]) -> [String] {
        let startIndex: Array<GaryxMobileMessage>.Index
        if let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) {
            startIndex = messages.index(after: lastUserIndex)
        } else {
            startIndex = messages.startIndex
        }
        return messages[startIndex...]
            .filter { $0.role == .assistant }
            .map { Self.normalizedMergeText($0.text) }
    }

    static func normalizedMergeText(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\r\n", with: "\n")
    }

    static func toolTraceGroupsOverlap(
        _ left: GaryxMobileToolTraceGroup,
        _ right: GaryxMobileToolTraceGroup,
        allowFingerprint: Bool
    ) -> Bool {
        let leftKeys = Set(left.entries.compactMap { toolTraceMergeKey($0, includeFingerprint: allowFingerprint) })
        let rightKeys = Set(right.entries.compactMap { toolTraceMergeKey($0, includeFingerprint: allowFingerprint) })
        return !leftKeys.isDisjoint(with: rightKeys)
    }

    static func isInCurrentTurn(index: Int, messages: [GaryxMobileMessage]) -> Bool {
        guard let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) else {
            return true
        }
        return index > lastUserIndex
    }

    static func mergedToolTraceGroup(
        _ remote: GaryxMobileToolTraceGroup,
        with local: GaryxMobileToolTraceGroup
    ) -> GaryxMobileToolTraceGroup {
        var merged = remote
        merged.live = remote.live || local.live
        for localEntry in local.entries {
            if let localKey = toolTraceMergeKey(localEntry),
               let index = merged.entries.firstIndex(where: { toolTraceMergeKey($0) == localKey }) {
                if localEntry.status != .running {
                    merged.entries[index].absorb(result: localEntry)
                }
                continue
            }
            merged.entries.append(localEntry)
        }
        return merged
    }

    static func toolTraceMergeKey(
        _ entry: GaryxMobileToolTraceEntry,
        includeFingerprint: Bool = true
    ) -> String? {
        if let toolUseId = entry.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !toolUseId.isEmpty {
            return "id:\(toolUseId)"
        }
        guard includeFingerprint else {
            return nil
        }
        let normalizedTool = entry.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let input = entry.inputText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let summary = entry.summaryText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !normalizedTool.isEmpty, !input.isEmpty || !summary.isEmpty else {
            return nil
        }
        return "fp:\(normalizedTool):\(input):\(summary):\(entry.isError)"
    }

    static func userMergeKey(_ message: GaryxMobileMessage) -> String {
        GaryxStructuredContentRenderer.userMergeKey(
            text: message.text,
            attachments: message.attachments.map(\.contentDescriptor)
        )
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

    static func transcriptStructuredContent(_ item: GaryxTranscriptMessage) -> GaryxJSONValue? {
        if let messageContent = item.message?.jsonStringDecodedIfNeeded.objectValue?["content"] {
            return messageContent.jsonStringDecodedIfNeeded
        }
        return item.content?.jsonStringDecodedIfNeeded
    }

    static func transcriptMessageText(
        _ item: GaryxTranscriptMessage,
        attachments: [GaryxMobileMessageAttachment]
    ) -> String {
        if item.role == .user,
           !attachments.isEmpty,
           let content = transcriptStructuredContent(item) {
            return GaryxStructuredContentRenderer.text(from: content) ?? ""
        }
        return item.text
    }

    static func messageAttachments(fromTranscript item: GaryxTranscriptMessage) -> [GaryxMobileMessageAttachment] {
        guard let content = transcriptStructuredContent(item) else { return [] }
        return messageAttachments(fromStructuredContent: content)
    }

    static func messageAttachments(fromStructuredContent content: GaryxJSONValue?) -> [GaryxMobileMessageAttachment] {
        GaryxStructuredContentRenderer.attachments(from: content).map { attachment in
            GaryxMobileMessageAttachment(
                id: attachment.id,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                path: attachment.path,
                dataUrl: attachment.dataUrl,
                remoteUrl: attachment.remoteUrl
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
        var rendered: [GaryxMobileMessage] = []
        var pendingToolGroup: GaryxMobileToolTraceGroup?

        func flushToolGroup() {
            guard let group = pendingToolGroup, !group.entries.isEmpty else {
                pendingToolGroup = nil
                return
            }
            let firstEntry = group.entries[0]
            rendered.append(
                GaryxMobileMessage(
                    id: "tool-group:\(firstEntry.id)",
                    role: .tool,
                    text: group.summary,
                    timestamp: firstEntry.timestamp,
                    isStreaming: live && group.entries.contains { $0.status == .running },
                    toolTraceGroup: GaryxMobileToolTraceGroup(
                        entries: group.entries,
                        live: live && group.entries.contains { $0.status == .running }
                    )
                )
            )
            pendingToolGroup = nil
        }

        for item in transcript {
            let toolTraceKind = GaryxMobileTranscriptToolTraceClassifier.kind(for: item)
            if toolTraceKind != nil {
                guard let entry = GaryxMobileToolTraceEntry(transcript: item) else {
                    continue
                }
                var group = pendingToolGroup ?? GaryxMobileToolTraceGroup(entries: [], live: false)
                if toolTraceKind == .toolResult, mergeToolResult(entry, into: &group) {
                    pendingToolGroup = group
                    continue
                }
                group.entries.append(entry)
                pendingToolGroup = group
                continue
            }

            flushToolGroup()

            let attachments = Self.messageAttachments(fromTranscript: item)
            let displayText = Self.transcriptMessageText(item, attachments: attachments)
            let trimmed = displayText.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty, attachments.isEmpty, item.role != .user, item.role != .assistant {
                continue
            }
            rendered.append(
                GaryxMobileMessage(
                    id: item.id,
                    role: mobileRole(for: item.role),
                    text: displayText,
                    attachments: attachments,
                    timestamp: item.timestamp,
                    isStreaming: false
                )
            )
        }

        flushToolGroup()
        return rendered
    }

    private func appendToolTraceEvent(_ eventKind: GaryxMobileToolTraceEventKind, threadId: String, message: GaryxJSONValue?) {
        removeEmptyActiveAssistantPlaceholder(for: threadId)
        activeAssistantMessageIdsByThread[threadId] = nil
        guard let entry = GaryxMobileToolTraceEntry(eventKind: eventKind, value: message) else {
            return
        }

        mutateMessages(for: threadId) { messages in
            if eventKind == .toolResult {
                for index in messages.indices.reversed() {
                    if messages[index].role == .user {
                        break
                    }
                    guard var group = messages[index].toolTraceGroup else { continue }
                    if mergeToolResult(entry, into: &group) {
                        messages[index].toolTraceGroup = group
                        messages[index].text = group.summary
                        messages[index].isStreaming = group.isActive
                        return
                    }
                }
            }

            if let index = messages.indices.last, messages[index].role == .tool, var group = messages[index].toolTraceGroup {
                group.live = true
                group.entries.append(entry)
                messages[index].toolTraceGroup = group
                messages[index].text = group.summary
                messages[index].isStreaming = group.isActive
                return
            }

            let group = GaryxMobileToolTraceGroup(entries: [entry], live: true)
            messages.append(
                GaryxMobileMessage(
                    id: "tool-group:\(entry.id)",
                    role: .tool,
                    text: group.summary,
                    timestamp: entry.timestamp,
                    isStreaming: group.isActive,
                    toolTraceGroup: group
                )
            )
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

    func mergeToolResult(
        _ result: GaryxMobileToolTraceEntry,
        into group: inout GaryxMobileToolTraceGroup
    ) -> Bool {
        if let toolUseId = result.toolUseId,
           let match = group.entries.lastIndex(where: { $0.toolUseId == toolUseId && $0.resultText == nil }) {
            group.entries[match].absorb(result: result)
            return true
        }
        if result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false {
            return false
        }

        let fallbackMatches = group.entries.indices.filter {
            canMergeToolResultFallback(result, into: group.entries[$0])
        }
        if let match = fallbackMatches.last {
            group.entries[match].absorb(result: result)
            return true
        }

        return false
    }

    func canMergeToolResultFallback(
        _ result: GaryxMobileToolTraceEntry,
        into candidate: GaryxMobileToolTraceEntry
    ) -> Bool {
        guard candidate.status == .running, candidate.resultText == nil else {
            return false
        }
        if let resultToolUseId = result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !resultToolUseId.isEmpty,
           let candidateToolUseId = candidate.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !candidateToolUseId.isEmpty,
           resultToolUseId != candidateToolUseId {
            return false
        }
        let resultTool = result.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let candidateTool = candidate.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !resultTool.isEmpty, resultTool == candidateTool {
            return true
        }
        if candidateTool == "tool" || resultTool == "tool" {
            return true
        }
        if result.title.caseInsensitiveCompare(candidate.title) == .orderedSame {
            return true
        }
        if let resultSummary = result.summaryText,
           let candidateSummary = candidate.summaryText,
           resultSummary == candidateSummary {
            return true
        }
        return false
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

    func handleGlobalStreamEvent(_ event: GaryxChatStreamEvent, replay: Bool = false) async {
        let threadId = Self.threadId(from: event)
        guard !threadId.isEmpty else { return }

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

        if activeTasksByThread[threadId] != nil {
            updateRemoteBusyState(from: event)
            if case .threadTitleUpdated(_, let eventThreadId, let title) = event {
                applyThreadTitleUpdate(threadId: eventThreadId, title: title)
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

    func cancelActiveSocket() {
        for threadId in Array(pendingAssistantDeltasByThread.keys) {
            flushPendingAssistantDelta(for: threadId)
        }
        for task in activeReaderTasksByThread.values {
            task.cancel()
        }
        for task in activeTasksByThread.values {
            task.cancel(with: .goingAway, reason: nil)
        }
        activeReaderTasksByThread = [:]
        activeTasksByThread = [:]
        activeTask = nil
        activeReaderTask = nil
        if let activeRunThreadId {
            activeAssistantMessageIdsByThread[activeRunThreadId] = nil
        }
        activeRunThreadId = nil
        isSending = false
    }

    func cancelActiveSocket(for threadId: String) {
        flushPendingAssistantDelta(for: threadId)
        activeReaderTasksByThread[threadId]?.cancel()
        activeReaderTasksByThread[threadId] = nil
        activeTasksByThread[threadId]?.cancel(with: .goingAway, reason: nil)
        activeTasksByThread[threadId] = nil
        activeAssistantMessageIdsByThread[threadId] = nil
        if activeRunThreadId == threadId {
            activeRunThreadId = activeTasksByThread.keys.first
        }
        activeTask = activeRunThreadId.flatMap { activeTasksByThread[$0] }
        activeReaderTask = activeRunThreadId.flatMap { activeReaderTasksByThread[$0] }
        isSending = !activeTasksByThread.isEmpty
    }

    func clearActiveRun(task: URLSessionWebSocketTask?, threadId: String?) {
        let resolvedThreadId: String?
        if let threadId {
            resolvedThreadId = threadId
        } else if let task {
            resolvedThreadId = activeTasksByThread.first(where: { $0.value === task })?.key
        } else {
            resolvedThreadId = nil
        }

        guard let resolvedThreadId else {
            cancelActiveSocket()
            return
        }

        if let task,
           let current = activeTasksByThread[resolvedThreadId],
           current !== task {
            return
        }

        flushPendingAssistantDelta(for: resolvedThreadId)
        activeReaderTasksByThread[resolvedThreadId]?.cancel()
        activeReaderTasksByThread[resolvedThreadId] = nil
        activeTasksByThread[resolvedThreadId] = nil
        activeAssistantMessageIdsByThread[resolvedThreadId] = nil
        if activeRunThreadId == resolvedThreadId {
            activeRunThreadId = activeTasksByThread.keys.first
        }
        activeTask = activeRunThreadId.flatMap { activeTasksByThread[$0] }
        activeReaderTask = activeRunThreadId.flatMap { activeReaderTasksByThread[$0] }
        isSending = !activeTasksByThread.isEmpty
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
        let normalized = message.lowercased()
        return normalized.contains("timed out")
            || normalized.contains("timeout")
            || normalized.contains("network connection was lost")
            || normalized.contains("not connected to the internet")
            || normalized.contains("connection reset")
            || normalized.contains("connection closed")
            || normalized.contains("websocket")
            || normalized.contains("socket")
            || normalized.contains("gateway unavailable")
            || normalized.contains("bad gateway")
            || normalized.contains("http 502")
            || normalized.contains("http 503")
            || normalized.contains("http 504")
            || normalized.contains("service unavailable")
    }

    static func isSuccessfulStreamInputStatus(_ status: String) -> Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "queued"
            || normalized == "accepted"
            || normalized == "ok"
            || normalized == "success"
    }

    static func shouldFallbackStreamInputStatus(_ status: String) -> Bool {
        let normalized = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "no_active_session"
            || normalized == "no active session"
            || normalized == "inactive"
            || normalized == "closed"
            || normalized == "not_found"
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
}

private enum GaryxMobileToolTraceEventKind {
    case toolUse
    case toolResult
}

private struct GaryxMobileToolTracePayload {
    var toolUseId: String?
    var parentToolUseId: String?
    var toolName: String?
    var contentText: String?
    var summaryText: String?
    var timestamp: String?
    var primaryPathBadge: String?
    var source: String?
    var itemType: String?
    var isError: Bool

    static func fromEvent(_ value: GaryxJSONValue?, eventKind: GaryxMobileToolTraceEventKind) -> GaryxMobileToolTracePayload {
        from(value: value, eventKind: eventKind, fallbackText: nil, fallbackToolName: nil, fallbackTimestamp: nil)
    }

    static func fromTranscript(_ message: GaryxTranscriptMessage) -> GaryxMobileToolTracePayload {
        let eventKind = eventKind(fromTranscript: message)
        return from(
            value: message.message ?? message.content ?? GaryxJSONValue.decoded(from: message.text),
            eventKind: eventKind,
            fallbackText: message.text,
            fallbackToolName: message.kind,
            fallbackTimestamp: message.timestamp
        )
    }

    private static func from(
        value: GaryxJSONValue?,
        eventKind: GaryxMobileToolTraceEventKind,
        fallbackText: String?,
        fallbackToolName: String?,
        fallbackTimestamp: String?
    ) -> GaryxMobileToolTracePayload {
        let decodedValue = value?.jsonStringDecodedIfNeeded
        guard let object = decodedValue?.objectValue else {
            return GaryxMobileToolTracePayload(
                toolUseId: nil,
                parentToolUseId: nil,
                toolName: fallbackToolName?.garyxTrimmedNilIfEmpty,
                contentText: fallbackText?.garyxTrimmedNilIfEmpty,
                summaryText: fallbackText.flatMap(GaryxMobileToolSummaryFormatter.safeSummary),
                timestamp: fallbackTimestamp,
                primaryPathBadge: nil,
                source: nil,
                itemType: fallbackToolName?.garyxTrimmedNilIfEmpty,
                isError: false
            )
        }

        let payloadValue = object.unwrappedToolPayloadValue ?? decodedValue ?? .object(object)
        let payloadObject = payloadValue.objectValue
        let nestedContent = payloadObject ?? object.objectValue(forKeys: ["content", "message", "payload"])
        let metadata = object.objectValue(forKeys: ["metadata"])
            ?? payloadObject?.objectValue(forKeys: ["metadata"])
            ?? nestedContent?.objectValue(forKeys: ["metadata"])
        let source = metadata?.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? object.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? payloadObject?.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? nestedContent?.stringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
        let toolUseId = object.stringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? payloadObject?.stringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? nestedContent?.stringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
        let parentToolUseId = object.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? payloadObject?.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? nestedContent?.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? metadata?.stringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
        let toolName = object.stringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title"])
            ?? payloadObject?.stringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title", "type"])
            ?? nestedContent?.stringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title"])
            ?? fallbackToolName?.garyxTrimmedNilIfEmpty
        let itemType = object.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? payloadObject?.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? nestedContent?.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? metadata?.stringValue(forKeys: ["type", "item_type", "itemType"])
            ?? toolName
        let detailKeys = eventKind == .toolUse
            ? ["input", "arguments", "params", "content", "command", "path", "file_path", "text"]
            : ["result", "output", "content", "stdout", "stderr", "text", "message"]
        let content = payloadObject?.detailText(forKeys: detailKeys)
            ?? object.detailText(forKeys: detailKeys)
            ?? fallbackText?.garyxTrimmedNilIfEmpty
        let summary = Self.summaryText(
            toolName: toolName,
            payload: payloadObject,
            payloadValue: payloadValue,
            eventKind: eventKind
        ) ?? fallbackText.flatMap(GaryxMobileToolSummaryFormatter.safeSummary)
        let timestamp = object.stringValue(forKeys: ["timestamp", "createdAt", "created_at"]) ?? fallbackTimestamp
        let primaryPathBadge = Self.primaryPathBadge(
            payload: payloadObject,
            nestedContent: nestedContent
        )
        let isError = object.boolValue(forKeys: ["isError", "is_error", "error"])
            ?? payloadObject?.boolValue(forKeys: ["isError", "is_error", "error"])
            ?? nestedContent?.boolValue(forKeys: ["isError", "is_error", "error"])
            ?? false

        return GaryxMobileToolTracePayload(
            toolUseId: toolUseId,
            parentToolUseId: parentToolUseId,
            toolName: toolName,
            contentText: content,
            summaryText: summary,
            timestamp: timestamp,
            primaryPathBadge: primaryPathBadge,
            source: source,
            itemType: itemType,
            isError: isError
        )
    }

    private static func primaryPathBadge(
        payload: [String: GaryxJSONValue]?,
        nestedContent: [String: GaryxJSONValue]?
    ) -> String? {
        let input = payload?.objectValue(forKeys: ["input", "arguments", "params"])
            ?? nestedContent?.objectValue(forKeys: ["input", "arguments", "params"])
            ?? payload
            ?? nestedContent
        return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
            .map { GaryxMobileToolSummaryFormatter.pathTail($0) }
    }

    private static func summaryText(
        toolName: String?,
        payload: [String: GaryxJSONValue]?,
        payloadValue: GaryxJSONValue,
        eventKind: GaryxMobileToolTraceEventKind
    ) -> String? {
        let normalizedTool = toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        let input = payload?.objectValue(forKeys: ["input", "arguments", "params"]) ?? payload
        let result = payload?.objectValue(forKeys: ["result", "output"]) ?? payload

        if eventKind == .toolResult {
            let text = result?.stringValue(forKeys: ["summary", "message", "text", "stdout", "stderr"])
                ?? payload?.stringValue(forKeys: ["summary", "message", "text", "stdout", "stderr"])
            return text.flatMap(GaryxMobileToolSummaryFormatter.safeSummary)
        }

        switch normalizedTool {
        case "bash", "shell", "exec_command", "command", "commandexecution":
            return input?.stringValue(forKeys: ["description"])
                ?? input?.stringValue(forKeys: ["command", "cmd"])
                    .map { GaryxMobileToolSummaryFormatter.shellSummary($0) }
        case "read", "view", "open", "cat":
            return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "read \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "write", "create":
            return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "write \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "edit", "multiedit", "apply_patch":
            return input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "edit \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "grep", "search", "rg":
            let pattern = input?.stringValue(forKeys: ["pattern", "query"])
            let path = input?.stringValue(forKeys: ["path", "include", "glob"])
            if let pattern, let path {
                return "search \(pattern) in \(GaryxMobileToolSummaryFormatter.pathTail(path))"
            }
            return pattern.map { "search \($0)" }
        case "glob", "find":
            return input?.stringValue(forKeys: ["pattern", "path"])
                .map { "find \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "ls", "list":
            return input?.stringValue(forKeys: ["path", "directory"])
                .map { "list \(GaryxMobileToolSummaryFormatter.pathTail($0))" } ?? "list files"
        case "todowrite", "todo_write":
            if let todos = input?["todos"]?.arrayValue, !todos.isEmpty {
                return "\(todos.count) todo items"
            }
            return nil
        case "webfetch", "web_fetch":
            return input?.stringValue(forKeys: ["url"])
                .flatMap { URL(string: $0)?.host }
                .map { "fetch \($0)" }
        case "websearch", "web_search":
            return input?.stringValue(forKeys: ["query"]).map { "search web for \($0)" }
        default:
            if let path = input?.stringValue(forKeys: ["file_path", "filePath", "path", "file"]) {
                return GaryxMobileToolSummaryFormatter.pathTail(path)
            }
            if let command = input?.stringValue(forKeys: ["command", "cmd"]) {
                return GaryxMobileToolSummaryFormatter.shellSummary(command)
            }
            if case .string(let text) = payloadValue {
                return GaryxMobileToolSummaryFormatter.safeSummary(text)
            }
            return nil
        }
    }
}

private extension GaryxMobileToolTraceEntry {
    init?(transcript message: GaryxTranscriptMessage) {
        let eventKind = GaryxMobileToolTracePayload.eventKind(fromTranscript: message)
        let payload = GaryxMobileToolTracePayload.fromTranscript(message)
        guard payload.shouldRender else {
            return nil
        }
        self.init(
            id: "\(message.id):\(eventKind.idSuffix)",
            toolUseId: payload.toolUseId,
            parentToolUseId: payload.parentToolUseId,
            toolName: payload.normalizedToolName,
            title: GaryxMobileToolTraceEntry.title(for: payload.normalizedToolName),
            inputText: eventKind == .toolUse ? payload.contentText : nil,
            resultText: eventKind == .toolResult ? payload.contentText : nil,
            summaryText: payload.summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: eventKind == .toolResult ? (payload.isError ? .failed : .completed) : .running,
            isError: payload.isError,
            timestamp: payload.timestamp,
            primaryPathBadge: payload.primaryPathBadge
        )
    }

    init?(eventKind: GaryxMobileToolTraceEventKind, value: GaryxJSONValue?) {
        let payload = GaryxMobileToolTracePayload.fromEvent(value, eventKind: eventKind)
        guard payload.shouldRender else {
            return nil
        }
        let generatedId = payload.toolUseId ?? UUID().uuidString
        self.init(
            id: "\(eventKind.idSuffix):\(generatedId):\(UUID().uuidString)",
            toolUseId: payload.toolUseId,
            parentToolUseId: payload.parentToolUseId,
            toolName: payload.normalizedToolName,
            title: GaryxMobileToolTraceEntry.title(for: payload.normalizedToolName),
            inputText: eventKind == .toolUse ? payload.contentText : nil,
            resultText: eventKind == .toolResult ? payload.contentText : nil,
            summaryText: payload.summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: eventKind == .toolUse ? .running : (payload.isError ? .failed : .completed),
            isError: payload.isError,
            timestamp: payload.timestamp,
            primaryPathBadge: payload.primaryPathBadge
        )
    }

    static func title(for toolName: String) -> String {
        switch toolName.lowercased() {
        case "exec_command", "command":
            return "Command"
        case "write_stdin":
            return "Input"
        case "apply_patch":
            return "Edit"
        case "view_image":
            return "Image"
        case "read_mcp_resource":
            return "MCP resource"
        case "list_mcp_resources":
            return "MCP resources"
        default:
            let words = toolName
                .replacingOccurrences(of: "-", with: "_")
                .split(separator: "_")
                .map { $0.capitalized }
            return words.isEmpty ? "Tool" : words.joined(separator: " ")
        }
    }
}

private extension GaryxMobileToolTracePayload {
    var shouldRender: Bool {
        let normalizedItemType = itemType?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let normalizedToolName = toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if normalizedItemType == "reasoning" || normalizedToolName == "reasoning" {
            return false
        }
        return true
    }

    static func eventKind(fromTranscript message: GaryxTranscriptMessage) -> GaryxMobileToolTraceEventKind {
        switch GaryxMobileTranscriptToolTraceClassifier.kind(for: message) {
        case .toolResult:
            return .toolResult
        case .toolUse, .none:
            return .toolUse
        }
    }

    var normalizedToolName: String {
        toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased().garyxTrimmedNilIfEmpty ?? "tool"
    }
}

private extension GaryxMobileToolTraceEventKind {
    var idSuffix: String {
        switch self {
        case .toolUse:
            "tool-use"
        case .toolResult:
            "tool-result"
        }
    }
}
