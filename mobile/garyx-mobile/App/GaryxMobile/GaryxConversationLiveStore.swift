import Foundation

/// Immutable route scope plus destination-indexed reads into the model's live
/// caches. A mounted predecessor and a staged destination can therefore render
/// different conversations at the same time without consulting selection.
@MainActor
final class GaryxConversationLiveStore: ObservableObject {
    let destination: GaryxRouteDestination

    init(destination: GaryxRouteDestination) {
        precondition(destination.composerKey != nil, "conversation store requires a composer route")
        self.destination = destination
    }

    var threadID: String? {
        guard case .conversation(let threadID) = destination else { return nil }
        return threadID
    }

    var routeIdentity: String {
        switch destination {
        case .conversation(let threadID):
            "thread:\(threadID)"
        case .conversationDraft(let draftID):
            "draft:\(draftID)"
        case .panel, .settingsDetail, .workspaceDrilldown:
            preconditionFailure("non-conversation destination")
        }
    }

    func summary(in model: GaryxMobileModel) -> GaryxThreadSummary? {
        threadID.flatMap { model.cachedThreadSummary(for: $0) }
    }

    func messages(in model: GaryxMobileModel) -> [GaryxMobileMessage] {
        threadID.map { model.cachedMessages(for: $0) } ?? []
    }

    func turnRows(in model: GaryxMobileModel, isCanonicalTop: Bool) -> [GaryxMobileTurnRow] {
        guard let threadID else { return [] }
        if isCanonicalTop, model.selectedThread?.id == threadID {
            return model.selectedThreadTurnRows()
        }
        let snapshot = model.renderSnapshot(for: threadID)
        let localMessages = model.cachedMessages(for: threadID)
        let transcriptMessages = model.transcriptMirror.snapshot(for: threadID)?.messages ?? []
        return GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: localMessages,
            transcriptMessages: transcriptMessages
        )
    }

    func isThinking(in model: GaryxMobileModel) -> Bool {
        guard let threadID else { return false }
        return model.renderSnapshot(for: threadID)?.tailActivity == .thinking
    }

    func rateLimit(in model: GaryxMobileModel) -> GaryxRenderRateLimit? {
        guard let threadID else { return nil }
        return model.renderSnapshot(for: threadID)?.rateLimit
    }

    func hasCapsuleCards(in model: GaryxMobileModel) -> Bool {
        guard let threadID, let snapshot = model.renderSnapshot(for: threadID) else { return false }
        return snapshot.rows.contains { row in
            guard case .userTurn(let turn) = row else { return false }
            return !turn.capsuleCards.isEmpty
        }
    }

    func hasMoreRenderableHistory(in model: GaryxMobileModel, isCanonicalTop: Bool) -> Bool {
        isCanonicalTop && model.selectedThread?.id == threadID
            ? model.selectedThreadHasMoreRenderableHistory
            : false
    }

    func isLoadingInitialHistory(in model: GaryxMobileModel, isCanonicalTop: Bool) -> Bool {
        isCanonicalTop && model.selectedThread?.id == threadID
            && model.isSelectedThreadLoadingInitialHistory
    }
}
