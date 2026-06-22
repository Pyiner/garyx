import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func cachedMessages(for threadId: String) -> [GaryxMobileMessage] {
        messagesByThread[threadId] ?? []
    }

    func renderSnapshot(for threadId: String) -> GaryxRenderSnapshot? {
        renderSnapshotsByThread[threadId] ?? cachedTranscriptSnapshots[threadId]?.renderSnapshot
    }

    func setRenderSnapshot(_ snapshot: GaryxRenderSnapshot?, for threadId: String) {
        guard renderSnapshotsByThread[threadId] != snapshot else { return }
        renderSnapshotsByThread[threadId] = snapshot
    }

    func selectedThreadTurnRows() -> [GaryxMobileTurnRow] {
        guard let threadId = selectedThread?.id else {
            return GaryxMobileRenderStateMapper.rows(
                snapshot: nil,
                messages: messages,
                transcriptMessages: []
            )
        }
        return GaryxMobileRenderStateMapper.rows(
            snapshot: renderSnapshot(for: threadId),
            messages: messages,
            transcriptMessages: cachedTranscriptSnapshots[threadId]?.messages ?? []
        )
    }

    func setMessages(
        _ nextMessages: [GaryxMobileMessage],
        for threadId: String,
        reconcileActiveAssistant: Bool = false
    ) {
        var adjustedMessages = nextMessages
        if reconcileActiveAssistant {
            reconcileActiveAssistantMessageId(threadId: threadId, messages: &adjustedMessages)
        }
        let nextSignature = Self.messageListSignature(for: adjustedMessages)
        if messageSignaturesByThread[threadId] == nextSignature,
           !nextSignature.sampled,
           (selectedThread?.id != threadId || selectedMessagesSignature == nextSignature) {
            return
        }
        messagesByThread[threadId] = adjustedMessages
        messageSignaturesByThread[threadId] = nextSignature
        if selectedThread?.id == threadId {
            pendingSelectedMessagesSignature = nextSignature
            messages = adjustedMessages
        }
    }

    func setPreparedMessages(_ prepared: GaryxPreparedThreadMessages, for threadId: String) {
        activeAssistantMessageIdsByThread[threadId] = prepared.activeAssistantMessageId
        if messageSignaturesByThread[threadId] == prepared.signature,
           !prepared.signature.sampled,
           (selectedThread?.id != threadId || selectedMessagesSignature == prepared.signature) {
            return
        }
        messagesByThread[threadId] = prepared.messages
        messageSignaturesByThread[threadId] = prepared.signature
        if selectedThread?.id == threadId {
            pendingSelectedMessagesSignature = prepared.signature
            messages = prepared.messages
        }
    }

    func reconcileActiveAssistantMessageId(threadId: String, messages: inout [GaryxMobileMessage]) {
        guard isThreadBusy(threadId) else {
            activeAssistantMessageIdsByThread[threadId] = nil
            return
        }
        if let activeId = activeAssistantMessageIdsByThread[threadId],
           let index = messages.firstIndex(where: { $0.id == activeId && $0.role == .assistant }) {
            messages[index].isStreaming = true
            return
        }
        if let index = messages.indices.last(where: { messages[$0].role == .assistant && messages[$0].isStreaming }) {
            messages[index].isStreaming = true
            activeAssistantMessageIdsByThread[threadId] = messages[index].id
        } else {
            activeAssistantMessageIdsByThread[threadId] = nil
        }
    }

    func clearMessages(for threadId: String) {
        messagesByThread[threadId] = []
        messageSignaturesByThread[threadId] = Self.messageListSignature(for: [])
        activeAssistantMessageIdsByThread[threadId] = nil
        renderSnapshotsByThread[threadId] = nil
        if selectedThread?.id == threadId {
            pendingSelectedMessagesSignature = messageSignaturesByThread[threadId]
            messages = []
        }
    }

    func resetSelectedThreadHistoryPagination() {
        isLoadingOlderThreadHistory = false
        selectedThreadHasMoreHistoryBefore = false
        selectedThreadNextHistoryBeforeIndex = nil
    }

    func resetThreadListPagination() {
        isLoadingMoreThreads = false
        hasMoreThreadSummaries = false
        nextThreadListOffset = 0
    }

    func syncVisibleMessages(for threadId: String) {
        if selectedThread?.id == threadId {
            messages = cachedMessages(for: threadId)
        }
    }

    func mutateMessages(for threadId: String, _ update: (inout [GaryxMobileMessage]) -> Void) {
        var nextMessages = cachedMessages(for: threadId)
        update(&nextMessages)
        setMessages(nextMessages, for: threadId)
    }

    /// The unsent text bound to the current composer context (thread or new-thread).
    var activeComposerDraft: String {
        composerDraftStore.current
    }

    /// Persist the live composer text for the current context. Called on every
    /// edit; cheap because the store is not `@Published`.
    func setComposerDraft(_ text: String) {
        composerDraftStore.setCurrent(text)
    }

    /// Switch the composer to another thread/new-thread context, preserving the
    /// outgoing context's unsent draft and loading the incoming one. Bumps
    /// `composerContextVersion` so the field reloads only when the key changes.
    func switchComposerDraft(to key: String) {
        guard composerDraftStore.switchTo(key) else { return }
        composerAttachments = []
        composerContextVersion &+= 1
    }

    /// Clear only the current context's draft — after a successful send.
    func resetComposerDraft() {
        composerDraftStore.reset()
        composerAttachments = []
        composerContextVersion &+= 1
    }

    /// Drop a thread's draft (it was deleted or unbound); reload the field only
    /// when that thread was the active context.
    func discardComposerDraft(forThread threadId: String) {
        guard composerDraftStore.discard(threadId: threadId) else { return }
        composerAttachments = []
        composerContextVersion &+= 1
    }

    /// Drop every draft — the gateway changed, so the whole thread set is gone.
    func clearAllComposerDrafts() {
        composerDraftStore.clearAll()
        composerAttachments = []
        composerContextVersion &+= 1
    }

    /// Draft key for the current new-thread composer, scoped to the target it will
    /// create a thread for (the pending bot, else the agent), so composing for one
    /// target and then switching to another does not show or send the first
    /// target's draft under the second.
    var newThreadComposerDraftKey: String {
        let botTarget = pendingBotId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let target = botTarget.isEmpty ? newThreadAgentTargetId() : botTarget
        return target.isEmpty
            ? GaryxComposerDraftStore.newThreadKey
            : "\(GaryxComposerDraftStore.newThreadKey):\(target)"
    }

    nonisolated static func messageListSignature(for messages: [GaryxMobileMessage]) -> MessageListSignature {
        var hasher = Hasher()
        var sampled = false
        for message in messages {
            hasher.combine(message.id)
            hasher.combine(Self.roleSignature(message.role))
            sampled = Self.combineTextSignature(message.text, into: &hasher) || sampled
            hasher.combine(message.timestamp)
            hasher.combine(message.isStreaming)
            hasher.combine(message.statusText)
            hasher.combine(message.clientIntentId)
            hasher.combine(message.pendingInputId)
            hasher.combine(message.attachments.count)
            for attachment in message.attachments {
                hasher.combine(attachment.id)
                hasher.combine(attachment.kind)
                hasher.combine(attachment.name)
                hasher.combine(attachment.mediaType)
                hasher.combine(attachment.path)
                sampled = Self.combineTextSignature(attachment.dataUrl, into: &hasher) || sampled
                hasher.combine(attachment.remoteUrl)
            }
            if let group = message.toolTraceGroup {
                hasher.combine(group.live)
                hasher.combine(group.entries.count)
                for entry in group.entries {
                    hasher.combine(entry.id)
                    hasher.combine(entry.toolUseId)
                    hasher.combine(entry.parentToolUseId)
                    hasher.combine(entry.toolName)
                    hasher.combine(entry.title)
                    hasher.combine(entry.inputLabel)
                    hasher.combine(entry.resultLabel)
                    hasher.combine(entry.summaryText)
                    hasher.combine(entry.status.rawValue)
                    hasher.combine(entry.isError)
                    hasher.combine(entry.timestamp)
                    hasher.combine(entry.primaryPathBadge)
                    sampled = Self.combineTextSignature(entry.inputText, into: &hasher) || sampled
                    sampled = Self.combineTextSignature(entry.resultText, into: &hasher) || sampled
                }
            }
        }
        return MessageListSignature(count: messages.count, fingerprint: hasher.finalize(), sampled: sampled)
    }

    @discardableResult
    nonisolated static func combineTextSignature(_ value: String?, into hasher: inout Hasher) -> Bool {
        guard let value else {
            hasher.combine(-1)
            return false
        }
        return combineTextSignature(value, into: &hasher)
    }

    @discardableResult
    nonisolated static func combineTextSignature(_ value: String, into hasher: inout Hasher) -> Bool {
        hasher.combine(value.count)
        if value.count <= 1_024 {
            hasher.combine(value)
            return false
        }
        hasher.combine(value.prefix(256))
        let middleOffset = max(0, (value.count / 2) - 128)
        let middleStart = value.index(value.startIndex, offsetBy: middleOffset)
        let middleEnd = value.index(middleStart, offsetBy: min(256, value.distance(from: middleStart, to: value.endIndex)))
        hasher.combine(value[middleStart..<middleEnd])
        hasher.combine(value.suffix(256))
        return true
    }

    nonisolated static func roleSignature(_ role: GaryxMobileMessage.Role) -> String {
        switch role {
        case .user:
            "user"
        case .assistant:
            "assistant"
        case .system:
            "system"
        case .tool:
            "tool"
        }
    }
}

struct GaryxPreparedThreadMessages: Equatable, Sendable {
    var messages: [GaryxMobileMessage]
    var signature: GaryxMobileModel.MessageListSignature
    var activeAssistantMessageId: String?

    static func make(
        remoteMessages: [GaryxMobileMessage],
        localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int?,
        isThreadBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedThreadMessages {
        let merged = GaryxTranscriptMerge.mergedMessages(
            remoteMessages,
            withLocal: localMessages,
            preserveRemoteBeforeIndex: preserveRemoteBeforeIndex
        )
        return make(
            messages: merged,
            isThreadBusy: isThreadBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
    }

    static func make(
        messages: [GaryxMobileMessage],
        isThreadBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedThreadMessages {
        let reconciled = reconciledActiveAssistantMessages(
            messages,
            isThreadBusy: isThreadBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
        return GaryxPreparedThreadMessages(
            messages: reconciled.messages,
            signature: GaryxMobileModel.messageListSignature(for: reconciled.messages),
            activeAssistantMessageId: reconciled.activeAssistantMessageId
        )
    }

    private static func reconciledActiveAssistantMessages(
        _ messages: [GaryxMobileMessage],
        isThreadBusy: Bool,
        activeAssistantMessageId: String?
    ) -> (messages: [GaryxMobileMessage], activeAssistantMessageId: String?) {
        guard isThreadBusy else {
            return (messages, nil)
        }
        var adjustedMessages = messages
        if let activeAssistantMessageId,
           let index = adjustedMessages.firstIndex(where: { $0.id == activeAssistantMessageId && $0.role == .assistant }) {
            adjustedMessages[index].isStreaming = true
            return (adjustedMessages, activeAssistantMessageId)
        }
        if let index = adjustedMessages.indices.last(where: {
            adjustedMessages[$0].role == .assistant && adjustedMessages[$0].isStreaming
        }) {
            adjustedMessages[index].isStreaming = true
            return (adjustedMessages, adjustedMessages[index].id)
        }
        return (adjustedMessages, nil)
    }
}

struct GaryxPreparedSelectedThreadTranscriptUpdate: Equatable, Sendable {
    var activitySignature: String
    var runState: GaryxTranscriptRunState
    var messages: GaryxPreparedThreadMessages
    var threadRunActive: Bool

    static func make(
        from transcript: GaryxThreadTranscript,
        localMessages: [GaryxMobileMessage],
        localRunTrackerBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedSelectedThreadTranscriptUpdate {
        make(
            transcriptMessages: transcript.messages,
            activitySignature: GaryxThreadActivitySignature.make(from: transcript),
            localMessages: localMessages,
            preserveRemoteBeforeIndex: transcript.pageInfo?.returnedStartIndex
                ?? transcript.messages.compactMap(\.index).min(),
            localRunTrackerBusy: localRunTrackerBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
    }

    static func make(
        from window: GaryxCachedTranscript,
        localMessages: [GaryxMobileMessage],
        localRunTrackerBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedSelectedThreadTranscriptUpdate {
        make(
            transcriptMessages: window.messages,
            activitySignature: GaryxThreadActivitySignature.make(messages: window.messages, pendingUserInputs: []),
            localMessages: localMessages,
            preserveRemoteBeforeIndex: window.firstIndex,
            localRunTrackerBusy: localRunTrackerBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
    }

    private static func make(
        transcriptMessages: [GaryxTranscriptMessage],
        activitySignature: String,
        localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int?,
        localRunTrackerBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedSelectedThreadTranscriptUpdate {
        let runState = GaryxTranscriptRunStateReducer.reduce(transcriptMessages)
        let threadRunActive = localRunTrackerBusy || runState.busy
        let remoteMessages = GaryxMobileTranscriptMapper.mobileMessages(
            from: transcriptMessages,
            live: threadRunActive
        )
        return GaryxPreparedSelectedThreadTranscriptUpdate(
            activitySignature: activitySignature,
            runState: runState,
            messages: GaryxPreparedThreadMessages.make(
                remoteMessages: remoteMessages,
                localMessages: localMessages,
                preserveRemoteBeforeIndex: preserveRemoteBeforeIndex,
                isThreadBusy: threadRunActive,
                activeAssistantMessageId: activeAssistantMessageId
            ),
            threadRunActive: threadRunActive
        )
    }
}
