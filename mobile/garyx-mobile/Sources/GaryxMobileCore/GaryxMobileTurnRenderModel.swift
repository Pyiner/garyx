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
