import Foundation

/// Immutable route scope plus destination-indexed reads into the model's live
/// caches. A mounted predecessor and a staged destination can therefore render
/// different conversations at the same time without consulting selection.
@MainActor
struct GaryxConversationLiveStore {
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
        renderInput(in: model).messages
    }

    func turnRows(in model: GaryxMobileModel, isCanonicalTop: Bool) -> [GaryxMobileTurnRow] {
        if let threadID, isCanonicalTop, model.selectedThread?.id == threadID {
            return model.selectedThreadTurnRows()
        }
        let input = renderInput(in: model)
        return GaryxMobileRenderStateMapper.rows(
            snapshot: input.snapshot,
            messages: input.messages,
            transcriptMessages: input.transcriptMessages
        )
    }

    func isThinking(in model: GaryxMobileModel) -> Bool {
        renderInput(in: model).showsTailThinking
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

    func isAwaitingInitialHistory(in model: GaryxMobileModel, isCanonicalTop: Bool) -> Bool {
        isCanonicalTop && model.selectedThread?.id == threadID
            && model.isSelectedThreadAwaitingInitialHistory
    }

    func hasRenderedSnapshot(in model: GaryxMobileModel) -> Bool {
        guard let threadID else { return false }
        return model.renderSnapshot(for: threadID) != nil
    }

    private func renderInput(in model: GaryxMobileModel) -> GaryxConversationRouteRenderInput {
        let threadMessages = threadID.map { model.cachedMessages(for: $0) } ?? []
        let snapshot = threadID.flatMap { model.renderSnapshot(for: $0) }
        let transcriptMessages = threadID
            .flatMap { model.transcriptMirror.snapshot(for: $0)?.messages }
            ?? []
        return GaryxConversationRouteRenderInputResolver.resolve(
            destination: destination,
            draftMessages: model.messages,
            threadMessages: threadMessages,
            threadSnapshot: snapshot,
            threadTranscriptMessages: transcriptMessages
        )
    }
}
