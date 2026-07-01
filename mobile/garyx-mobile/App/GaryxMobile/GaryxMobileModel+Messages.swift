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
        if reconcileActiveAssistant {
            setPreparedMessages(
                GaryxPreparedThreadMessages.make(
                    messages: nextMessages,
                    isThreadBusy: isThreadBusy(threadId),
                    activeAssistantMessageId: activeAssistantMessageIdsByThread[threadId]
                ),
                for: threadId
            )
            return
        }
        let nextSignature = GaryxMessageListSignature.make(for: nextMessages)
        if messageSignaturesByThread[threadId] == nextSignature,
           !nextSignature.sampled,
           (selectedThread?.id != threadId || selectedMessagesSignature == nextSignature) {
            return
        }
        messagesByThread[threadId] = nextMessages
        messageSignaturesByThread[threadId] = nextSignature
        if selectedThread?.id == threadId {
            pendingSelectedMessagesSignature = nextSignature
            messages = nextMessages
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

    func clearMessages(for threadId: String) {
        messagesByThread[threadId] = []
        messageSignaturesByThread[threadId] = GaryxMessageListSignature.make(for: [])
        activeAssistantMessageIdsByThread[threadId] = nil
        renderSnapshotsByThread[threadId] = nil
        selectedThreadRenderFloorByThread[threadId] = nil
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

}
