import Foundation

enum GaryxMobileTranscriptBlock: Identifiable, Equatable {
    case message(GaryxMobileMessage)
    case toolGroup(GaryxMobileMessage)

    var id: String {
        switch self {
        case .message(let message), .toolGroup(let message):
            message.id
        }
    }

    var message: GaryxMobileMessage {
        switch self {
        case .message(let message), .toolGroup(let message):
            message
        }
    }

    var isPending: Bool {
        switch self {
        case .message(let message):
            message.isStreaming
        case .toolGroup(let message):
            message.toolTraceGroup?.isActive == true
        }
    }

    var isUserMessage: Bool {
        if case .message(let message) = self {
            return message.role == .user
        }
        return false
    }

    var timestamp: String? {
        message.timestamp
    }
}

struct GaryxMobileTurnRow: Identifiable, Equatable {
    enum ActivityRow: Identifiable, Equatable {
        case flat(GaryxMobileTranscriptBlock)
        case turn(GaryxMobileAgentTurn)

        var id: String {
            switch self {
            case .flat(let block):
                "flat:\(block.id)"
            case .turn(let turn):
                turn.id
            }
        }
    }

    let id: String
    let userBlock: GaryxMobileTranscriptBlock?
    let activityRows: [ActivityRow]
}

struct GaryxMobileAgentTurn: Identifiable, Equatable {
    let id: String
    let steps: [GaryxMobileTranscriptBlock]
    let finalBlock: GaryxMobileTranscriptBlock?
    let isRunning: Bool
    let startedAt: String?
    let finishedAt: String?

    var hasBody: Bool {
        !steps.isEmpty
    }
}

enum GaryxMobileTurnRenderer {
    static func transcriptBlocks(from messages: [GaryxMobileMessage]) -> [GaryxMobileTranscriptBlock] {
        messages.map { message in
            if message.role == .tool {
                return .toolGroup(message)
            }
            return .message(message)
        }
    }

    static func buildTurnRows(
        messages: [GaryxMobileMessage],
        isRunningThread: Bool
    ) -> [GaryxMobileTurnRow] {
        buildTurnRows(
            blocks: transcriptBlocks(from: removeStreamingThinkingPlaceholders(messages)),
            deferTrailingFinalAssistant: isRunningThread,
            isRunningThread: isRunningThread
        )
    }

    /// An empty, still-streaming assistant message is a "Thinking" placeholder
    /// with no content of its own. Placeholders never render as transcript
    /// blocks: the tail thinking indicator is the single "Thinking" surface.
    /// Rendering them as blocks turned pure-text replies into multi-step
    /// turns (showing a bogus Working summary) and stacked a second
    /// "Thinking" bubble above the tail indicator.
    private static func removeStreamingThinkingPlaceholders(
        _ messages: [GaryxMobileMessage]
    ) -> [GaryxMobileMessage] {
        messages.filter { !isEmptyStreamingAssistant($0) }
    }

    static func isEmptyStreamingAssistant(_ message: GaryxMobileMessage) -> Bool {
        message.role == .assistant
            && message.isStreaming
            && message.attachments.isEmpty
            && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private static func buildTurnRows(
        blocks: [GaryxMobileTranscriptBlock],
        deferTrailingFinalAssistant: Bool,
        isRunningThread: Bool
    ) -> [GaryxMobileTurnRow] {
        var rows: [GaryxMobileTurnRow] = []
        var currentUserBlock: GaryxMobileTranscriptBlock?
        var currentSteps: [GaryxMobileTranscriptBlock] = []
        var currentKey: String?
        var precedingUserTimestamp: String?

        func flush(isTrailingTurn: Bool) {
            let activityRows = buildActivityRows(
                steps: currentSteps,
                key: currentKey,
                precedingUserTimestamp: precedingUserTimestamp,
                deferTrailingFinalAssistant: deferTrailingFinalAssistant,
                isTrailingTurn: isTrailingTurn,
                isRunningThread: isRunningThread
            )
            if let currentUserBlock {
                rows.append(
                    GaryxMobileTurnRow(
                        id: "user-turn:\(currentUserBlock.id)",
                        userBlock: currentUserBlock,
                        activityRows: activityRows
                    )
                )
            } else if !activityRows.isEmpty {
                rows.append(
                    GaryxMobileTurnRow(
                        id: "orphan-turn:\(currentKey ?? UUID().uuidString)",
                        userBlock: nil,
                        activityRows: activityRows
                    )
                )
            }
            currentUserBlock = nil
            currentSteps = []
            currentKey = nil
            precedingUserTimestamp = nil
        }

        for block in blocks {
            if block.isUserMessage {
                flush(isTrailingTurn: false)
                currentUserBlock = block
                precedingUserTimestamp = block.timestamp
                continue
            }
            if currentKey == nil {
                currentKey = block.id
            }
            currentSteps.append(block)
        }
        flush(isTrailingTurn: true)
        return rows
    }

    private static func buildActivityRows(
        steps: [GaryxMobileTranscriptBlock],
        key: String?,
        precedingUserTimestamp: String?,
        deferTrailingFinalAssistant: Bool,
        isTrailingTurn: Bool,
        isRunningThread: Bool
    ) -> [GaryxMobileTurnRow.ActivityRow] {
        guard let key else { return [] }

        if steps.isEmpty {
            return []
        }

        let isTrailingDeferredTurn = deferTrailingFinalAssistant && isTrailingTurn
        if steps.count == 1,
           case .message(let message) = steps[0],
           message.role == .assistant {
            return [.flat(steps[0])]
        }

        let surfaceFinalAssistant = !isTrailingDeferredTurn
        let picked = surfaceFinalAssistant
            ? pickFinalAssistant(from: steps)
            : (steps: steps, finalBlock: nil as GaryxMobileTranscriptBlock?)
        let turn = summarizeTurn(
            steps: picked.steps,
            finalBlock: picked.finalBlock,
            key: key,
            precedingUserTimestamp: precedingUserTimestamp,
            // The trailing turn of an actively running thread is running by
            // definition: the run-level flag must drive the label and the
            // expansion so a lull between steps (no pending block for a
            // moment) cannot flap the turn into "Worked" and auto-collapse
            // it while output is still coming.
            forceRunning: isRunningThread && isTrailingTurn
        )
        return [.turn(turn)]
    }

    private static func pickFinalAssistant(
        from steps: [GaryxMobileTranscriptBlock]
    ) -> (steps: [GaryxMobileTranscriptBlock], finalBlock: GaryxMobileTranscriptBlock?) {
        guard let last = steps.last,
              case .message(let message) = last,
              message.role == .assistant,
              !message.isStreaming else {
            return (steps, nil)
        }
        return (Array(steps.dropLast()), last)
    }

    private static func summarizeTurn(
        steps: [GaryxMobileTranscriptBlock],
        finalBlock: GaryxMobileTranscriptBlock?,
        key: String,
        precedingUserTimestamp: String?,
        forceRunning: Bool
    ) -> GaryxMobileAgentTurn {
        let allBlocks = finalBlock.map { steps + [$0] } ?? steps
        let isRunning = forceRunning || allBlocks.contains { $0.isPending }
        let timestamps = allBlocks.compactMap(\.timestamp)
        let startedAt = precedingUserTimestamp ?? timestamps.first
        let finishedAt = isRunning ? nil : timestamps.last
        return GaryxMobileAgentTurn(
            id: "turn:\(key)",
            steps: steps,
            finalBlock: finalBlock,
            isRunning: isRunning,
            startedAt: startedAt,
            finishedAt: finishedAt
        )
    }
}

enum GaryxMobileThreadActivityModel {
    static func latestUserMessageAwaitsAssistant(_ messages: [GaryxMobileMessage]) -> Bool {
        // Desktop ignores internal loop-continuation user messages here; mobile does not
        // decode that marker yet, so every user role is treated as user-visible input.
        var latestUserIndex: Int?
        var latestAssistantOrToolIndex: Int?
        for (index, message) in messages.enumerated() {
            if message.role == .user {
                latestUserIndex = index
            }
            if message.role == .assistant || message.role == .tool {
                latestAssistantOrToolIndex = index
            }
        }
        guard let latestUserIndex else { return false }
        return latestAssistantOrToolIndex.map { $0 < latestUserIndex } ?? true
    }

    /// Decide whether a reloaded transcript should mark its thread as busy.
    ///
    /// A run completes on the gateway's thread-store write path before its `.done`
    /// event reaches the client, but a transcript reload triggered by that same event
    /// can still race an older runtime projection and report the finished run as active.
    /// Without this guard the client re-marks the thread busy and the "Thinking"
    /// indicator never clears. The client already observed the run terminate,
    /// so its own terminal signal wins: an `active_run` whose id matches the run we just
    /// saw finish is ignored.
    static func shouldTreatThreadRuntimeAsActive(
        activeRunPresent: Bool,
        activeRunId: String?,
        hasActivePendingInput: Bool,
        lastTerminatedRunId: String?
    ) -> Bool {
        if hasActivePendingInput {
            return true
        }
        guard activeRunPresent else {
            return false
        }
        let normalizedActive = activeRunId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedTerminated = lastTerminatedRunId?.trimmingCharacters(in: .whitespacesAndNewlines)
        if let normalizedActive,
           !normalizedActive.isEmpty,
           normalizedActive == normalizedTerminated {
            return false
        }
        return true
    }

    /// The tail "Thinking" indicator is the single thinking surface, driven
    /// purely by thread state:
    ///
    /// | State                                                | Thinking |
    /// |------------------------------------------------------|----------|
    /// | No run active                                        | hidden   |
    /// | Run active, no visible output yet                    | shown    |
    /// | Run active, assistant text visibly streaming         | hidden   |
    /// | Run active, tool call actively running               | hidden   |
    /// | Run active, last visible output finished (a lull, or |          |
    /// | the final reply not yet started)                     | shown    |
    ///
    /// Empty streaming assistant placeholders are not visible output (the
    /// renderer drops them), so they count as "no visible output yet".
    static func showsTailThinkingIndicator(
        messages: [GaryxMobileMessage],
        runActive: Bool
    ) -> Bool {
        guard runActive else { return false }
        guard let last = messages.last(where: {
            !GaryxMobileTurnRenderer.isEmptyStreamingAssistant($0)
        }) else {
            return true
        }
        if last.role == .assistant,
           last.isStreaming,
           !last.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !last.attachments.isEmpty {
            return false
        }
        if last.role == .tool,
           last.toolTraceGroup?.isActive == true {
            return false
        }
        return true
    }

}
