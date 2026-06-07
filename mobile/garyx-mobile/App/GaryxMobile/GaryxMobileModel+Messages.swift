import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func cachedMessages(for threadId: String) -> [GaryxMobileMessage] {
        messagesByThread[threadId] ?? []
    }

    func selectedThreadTurnRows() -> [GaryxMobileTurnRow] {
        let isRunning = isSelectedThreadSending
        let key = TurnRowsCacheKey(
            isRunning: isRunning,
            messages: selectedMessagesSignature
        )
        if selectedThreadTurnRowsCacheKey != key {
            selectedThreadTurnRowsCache = GaryxMobileTurnRenderer.buildTurnRows(
                messages: messages,
                isRunningThread: isRunning
            )
            selectedThreadTurnRowsCacheKey = key
        }
        return selectedThreadTurnRowsCache
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

    func reconcileActiveAssistantMessageId(threadId: String, messages: inout [GaryxMobileMessage]) {
        let isBusy = activeRunThreadId == threadId || remoteBusyThreadIds.contains(threadId)
        guard isBusy else {
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
        discardPendingAssistantDelta(for: threadId)
        messagesByThread[threadId] = []
        messageSignaturesByThread[threadId] = Self.messageListSignature(for: [])
        activeAssistantMessageIdsByThread[threadId] = nil
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

    func resetComposerDraft() {
        draft = ""
        composerAttachments = []
        composerContextVersion &+= 1
    }

    static func messageListSignature(for messages: [GaryxMobileMessage]) -> MessageListSignature {
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
    static func combineTextSignature(_ value: String?, into hasher: inout Hasher) -> Bool {
        guard let value else {
            hasher.combine(-1)
            return false
        }
        return combineTextSignature(value, into: &hasher)
    }

    @discardableResult
    static func combineTextSignature(_ value: String, into hasher: inout Hasher) -> Bool {
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

    static func roleSignature(_ role: GaryxMobileMessage.Role) -> String {
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
