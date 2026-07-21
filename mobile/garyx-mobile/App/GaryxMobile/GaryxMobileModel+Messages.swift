import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func cachedMessages(for threadId: String) -> [GaryxMobileMessage] {
        messagesByThread[threadId] ?? []
    }

    func renderSnapshot(for threadId: String) -> GaryxRenderSnapshot? {
        renderSnapshotsByThread[threadId] ?? transcriptMirror.snapshot(for: threadId)?.renderSnapshot
    }

    func setRenderSnapshot(_ snapshot: GaryxRenderSnapshot?, for threadId: String) {
        guard renderSnapshotsByThread[threadId] != snapshot else { return }
        renderSnapshotsByThread[threadId] = snapshot
        if selectedThread?.id == threadId {
            lockSelectedTurnRowsWindowFloorIfNeeded()
        }
    }

    /// The **full** prepared turn rows for the selected thread, memoized by
    /// mapper input identity (TASK-1751 P2). The mapper stays a dumb pure
    /// mapping; this only skips the redundant rebuild the view's body +
    /// `.onChange` would otherwise each trigger.
    func selectedThreadFullTurnRows() -> [GaryxMobileTurnRow] {
        let threadId = selectedThread?.id
        let snapshot = threadId.flatMap { renderSnapshot(for: $0) }
        let transcriptMessages = threadId.flatMap { transcriptMirror.snapshot(for: $0)?.messages } ?? []
        let localMessages = messages
        return selectedTurnRowsCache.rows(
            threadId: threadId,
            snapshot: snapshot,
            messages: localMessages,
            transcriptMessages: transcriptMessages
        ) {
            GaryxMobileRenderStateMapper.rows(
                snapshot: snapshot,
                messages: localMessages,
                transcriptMessages: transcriptMessages
            )
        }
    }

    /// The **windowed** turn rows actually rendered (TASK-1751 P3): the
    /// floor-anchored tail slice of the full rows. Pure read — never writes the
    /// window state (the floor is locked from event handlers only).
    func selectedThreadTurnRows() -> [GaryxMobileTurnRow] {
        GaryxTurnRowsWindowPlanner.resolve(
            rows: selectedThreadFullTurnRows(),
            state: selectedTurnRowsWindowState
        ).visible
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
            lockSelectedTurnRowsWindowFloorIfNeeded()
        }
        touchThreadResidency(threadId)
    }

    func setPreparedMessages(_ prepared: GaryxPreparedThreadMessages, for threadId: String) {
        activeAssistantMessageIdsByThread[threadId] = prepared.activeAssistantMessageId
        // Core's bounded signature includes selector-resolved diff lines, so a
        // late-arriving tool body cannot be mistaken for a no-op projection.
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
            lockSelectedTurnRowsWindowFloorIfNeeded()
        }
        touchThreadResidency(threadId)
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
        recentThreadFeeds.resetFeedData()
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

    /// Route-owned payload projection for the current scope/key. Empty text is
    /// data, not an instruction to delete the key or its attachments.
    var activeComposerDraft: String {
        composerPayloadCoordinator.currentText
    }

    var activeComposerPayloadItems: [GaryxMobileComposerAttachment] {
        composerPayloadCoordinator.currentAttachments.map { attachment in
            GaryxMobileComposerAttachment(
                id: attachment.id.rawValue,
                kind: attachment.kind ?? "file",
                name: attachment.name ?? "attachment",
                mediaType: attachment.mediaType ?? "application/octet-stream",
                path: attachment.uploadedPath ?? "",
                previewDataUrl: attachment.previewDataURL
            )
        }
    }

    var activeComposerPayloadKey: GaryxComposerKey {
        selectedThread.map { .thread($0.id) } ?? newThreadComposerPayloadKey
    }

    func activateComposerPayload(for key: GaryxComposerKey) {
        let token = gatewayRequestToken
        Task { [weak self] in
            guard let self, token == gatewayRequestToken else { return }
            await composerPayloadCoordinator.activate(scope: token.scope, key: key)
        }
    }

    func promoteActiveComposerPayload(
        to threadID: String,
        beforePublishingRoute: () -> Void = {}
    ) async throws {
        let draftID: String? = if case .draft(let rawDraftID) = composerPayloadCoordinator.activeKey {
            rawDraftID
        } else {
            nil
        }
        try await composerPayloadCoordinator.promoteActive(to: .thread(threadID))
        // Route replacement synchronously rebuilds the mounted host. Transfer
        // any local transcript overlay before publishing that replacement so
        // the promoted occurrence cannot render an empty intermediate frame.
        beforePublishingRoute()
        if let draftID {
            _ = productionRouteStore.promoteVisibleDraft(
                draftID: draftID,
                threadID: threadID
            )
            if !productionRouteStore.isAttached {
                applyCanonicalRouteProjection(productionRouteStore.path)
            }
        }
    }

    func discardComposerPayload(forThread threadID: String) {
        Task { [weak self] in
            guard let self else { return }
            try? await composerPayloadCoordinator.discard(key: .thread(threadID))
        }
    }

    /// Draft identity is scoped to the target it will create a thread for, so
    /// changing target restores each target's own text and attachments.
    var newThreadComposerPayloadKey: GaryxComposerKey {
        let botTarget = pendingBotId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let target = botTarget.isEmpty ? newThreadAgentTargetId() : botTarget
        return .draft(target.isEmpty ? "new-thread" : "new-thread:\(target)")
    }

}
