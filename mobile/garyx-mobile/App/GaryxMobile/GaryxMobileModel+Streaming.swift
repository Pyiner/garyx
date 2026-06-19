import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
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
        markActiveAssistantSegmentComplete(for: threadId)
        removeEmptyActiveAssistantPlaceholder(for: threadId)
        activeAssistantMessageIdsByThread[threadId] = nil
    }

    func suspendStreamingAssistantForBackground(threadId: String) -> String? {
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
        preserveRemoteBeforeIndex: Int? = nil
    ) -> [GaryxMobileMessage] {
        GaryxTranscriptMerge.mergedMessages(
            remoteMessages,
            withLocal: localMessages,
            preserveRemoteBeforeIndex: preserveRemoteBeforeIndex
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

    /// Full reset of all tracked run state (gateway switch, debug snapshot).
    func clearActiveRunState() {
        if let activeRunThreadId {
            activeAssistantMessageIdsByThread[activeRunThreadId] = nil
        }
        runTracker = GaryxConversationRunTracker()
        runStateByThread = [:]
    }

    func clearActiveRunState(for threadId: String) {
        activeAssistantMessageIdsByThread[threadId] = nil
        runTracker.clearLocalRun(threadId: threadId)
    }

    /// Local cleanup after a run terminated or the user interrupts it.
    func clearActiveRun(threadId: String?) {
        guard let resolvedThreadId = threadId else {
            clearActiveRunState()
            return
        }

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

}
