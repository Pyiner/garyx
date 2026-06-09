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
            blocks: transcriptBlocks(from: collapseStreamingThinkingPlaceholders(messages)),
            deferTrailingFinalAssistant: isRunningThread
        )
    }

    /// An empty, still-streaming assistant message is a "Thinking" placeholder with no
    /// content of its own. Only a trailing one represents live activity; any earlier empty
    /// placeholder is stale — a newer assistant segment or tool step already superseded it —
    /// and would otherwise render a second, duplicate "Thinking" row. Keep at most the final
    /// placeholder so the transcript never shows stacked "Thinking" labels.
    private static func collapseStreamingThinkingPlaceholders(
        _ messages: [GaryxMobileMessage]
    ) -> [GaryxMobileMessage] {
        guard messages.count > 1 else { return messages }
        let lastIndex = messages.index(before: messages.endIndex)
        return messages.enumerated().compactMap { index, message in
            if index != lastIndex, isEmptyStreamingAssistant(message) {
                return nil
            }
            return message
        }
    }

    private static func isEmptyStreamingAssistant(_ message: GaryxMobileMessage) -> Bool {
        message.role == .assistant
            && message.isStreaming
            && message.attachments.isEmpty
            && message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private static func buildTurnRows(
        blocks: [GaryxMobileTranscriptBlock],
        deferTrailingFinalAssistant: Bool
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
                isTrailingTurn: isTrailingTurn
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
        isTrailingTurn: Bool
    ) -> [GaryxMobileTurnRow.ActivityRow] {
        guard let key else { return [] }

        if steps.isEmpty {
            return []
        }

        let isTrailingDeferredTurn = deferTrailingFinalAssistant && isTrailingTurn
        if steps.count == 1,
           case .message(let message) = steps[0],
           message.role == .assistant {
            if message.isStreaming,
               message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
               message.attachments.isEmpty {
                return []
            }
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
            precedingUserTimestamp: precedingUserTimestamp
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
        precedingUserTimestamp: String?
    ) -> GaryxMobileAgentTurn {
        let allBlocks = finalBlock.map { steps + [$0] } ?? steps
        let isRunning = allBlocks.contains { $0.isPending }
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
    /// can still race a not-yet-repaired `active_run_snapshot` and report the finished
    /// run as active. Without this guard the client re-marks the thread busy and the
    /// "Thinking" indicator never clears. The client already observed the run terminate,
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

    static func showsTailThinkingIndicator(
        messages: [GaryxMobileMessage],
        runActive: Bool
    ) -> Bool {
        guard runActive else { return false }
        guard let last = messages.last else { return true }
        if last.role == .assistant,
           last.isStreaming,
           last.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
           last.attachments.isEmpty {
            return false
        }
        if last.role == .tool,
           last.toolTraceGroup?.isActive == true {
            return false
        }
        return latestUserMessageAwaitsAssistant(messages)
    }

    static func hasVisibleRunningActivity(
        messages: [GaryxMobileMessage],
        runActive: Bool
    ) -> Bool {
        guard runActive else { return false }
        guard !messages.isEmpty else { return true }
        if latestUserMessageAwaitsAssistant(messages) {
            return true
        }
        let activityMessages: ArraySlice<GaryxMobileMessage>
        if let latestUserIndex = messages.lastIndex(where: { $0.role == .user }) {
            activityMessages = messages[messages.index(after: latestUserIndex)...]
        } else {
            activityMessages = messages[...]
        }
        return activityMessages.contains { message in
            if message.isStreaming {
                return true
            }
            return message.toolTraceGroup?.isActive == true
        }
    }
}
