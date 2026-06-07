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
            blocks: transcriptBlocks(from: messages),
            deferTrailingFinalAssistant: isRunningThread
        )
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
