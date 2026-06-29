import Foundation

enum GaryxMobileTranscriptBlock: Identifiable, Equatable {
    case message(GaryxMobileMessage)
    case toolGroup(GaryxMobileMessage)

    var id: String {
        switch self {
        case let .message(message), let .toolGroup(message):
            message.id
        }
    }

    var message: GaryxMobileMessage {
        switch self {
        case let .message(message), let .toolGroup(message):
            message
        }
    }

    var isUserMessage: Bool {
        if case let .message(message) = self {
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
            case let .flat(block):
                "flat:\(block.id)"
            case let .turn(turn):
                turn.id
            }
        }
    }

    let id: String
    let userBlock: GaryxMobileTranscriptBlock?
    let activityRows: [ActivityRow]
    let capsuleCards: [GaryxRenderCapsuleCard]

    init(
        id: String,
        userBlock: GaryxMobileTranscriptBlock?,
        activityRows: [ActivityRow],
        capsuleCards: [GaryxRenderCapsuleCard] = []
    ) {
        self.id = id
        self.userBlock = userBlock
        self.activityRows = activityRows
        self.capsuleCards = capsuleCards
    }
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
