import Foundation

public struct GaryxThreadRenderFrame: Decodable, Equatable, Sendable {
    public var type: String
    public var threadId: String
    public var events: [GaryxThreadRenderFrameEvent]
    public var renderState: GaryxRenderSnapshot

    enum CodingKeys: String, CodingKey {
        case type
        case threadId
        case threadIdSnake = "thread_id"
        case events
        case renderState = "render_state"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        type = try container.garyxDecodeFirstString(.type) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadIdSnake, .threadId) ?? ""
        events = try container.decodeIfPresent([GaryxThreadRenderFrameEvent].self, forKey: .events) ?? []
        renderState = try container.decode(GaryxRenderSnapshot.self, forKey: .renderState)
    }
}

public struct GaryxThreadRenderFrameEvent: Decodable, Equatable, Sendable {
    public var type: String
    public var threadId: String?
    public var runId: String?
    public var seq: Int?
    public var message: GaryxTranscriptMessage?

    enum CodingKeys: String, CodingKey {
        case type
        case threadId
        case threadIdSnake = "thread_id"
        case runId
        case runIdSnake = "run_id"
        case seq
        case message
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        type = try container.garyxDecodeFirstString(.type) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadIdSnake, .threadId)
        runId = try container.garyxDecodeFirstString(.runIdSnake, .runId)
        seq = try container.garyxDecodeFirstInt(.seq)
        message = try container.decodeIfPresent(GaryxTranscriptMessage.self, forKey: .message)
    }
}

private struct GaryxLossyDecodableArray<Element: Decodable>: Decodable {
    var elements: [Element]

    init(from decoder: Decoder) throws {
        var container = try decoder.unkeyedContainer()
        var decoded: [Element] = []
        while !container.isAtEnd {
            if let element = try? container.decode(Element.self) {
                decoded.append(element)
            } else {
                _ = try? container.decode(GaryxDiscardedDecodable.self)
            }
        }
        elements = decoded
    }
}

private struct GaryxDiscardedDecodable: Decodable {}

public struct GaryxRenderSnapshot: Codable, Equatable, Sendable {
    public var basedOnSeq: Int
    public var rows: [GaryxRenderRow]
    public var tailActivity: GaryxRenderTailActivity
    public var activeToolGroupId: String?
    public var progressLocus: GaryxRenderProgressLocus
    public var visibleMessageIds: [String]
    public var filteredPlaceholders: [GaryxRenderFilteredPlaceholder]
    public var rateLimit: GaryxRenderRateLimit?
    public var window: GaryxRenderWindow?

    public init(
        basedOnSeq: Int,
        rows: [GaryxRenderRow],
        tailActivity: GaryxRenderTailActivity = .none,
        activeToolGroupId: String? = nil,
        progressLocus: GaryxRenderProgressLocus = .none,
        visibleMessageIds: [String] = [],
        filteredPlaceholders: [GaryxRenderFilteredPlaceholder] = [],
        rateLimit: GaryxRenderRateLimit? = nil,
        window: GaryxRenderWindow? = nil
    ) {
        self.basedOnSeq = basedOnSeq
        self.rows = rows
        self.tailActivity = tailActivity
        self.activeToolGroupId = activeToolGroupId
        self.progressLocus = progressLocus
        self.visibleMessageIds = visibleMessageIds
        self.filteredPlaceholders = filteredPlaceholders
        self.rateLimit = rateLimit
        self.window = window
    }

    enum CodingKeys: String, CodingKey {
        case basedOnSeq = "based_on_seq"
        case rows
        case tailActivity
        case activeToolGroupId
        case progressLocus = "progress_locus"
        case visibleMessageIds
        case filteredPlaceholders = "filtered_placeholders"
        case rateLimit
        case window
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        basedOnSeq = try container.decodeIfPresent(Int.self, forKey: .basedOnSeq) ?? 0
        rows = try container.decodeIfPresent(GaryxLossyDecodableArray<GaryxRenderRow>.self, forKey: .rows)?.elements ?? []
        tailActivity = try container.decodeIfPresent(GaryxRenderTailActivity.self, forKey: .tailActivity) ?? .none
        activeToolGroupId = try container.decodeIfPresent(String.self, forKey: .activeToolGroupId)
        progressLocus = try container.decodeIfPresent(GaryxRenderProgressLocus.self, forKey: .progressLocus) ?? .none
        visibleMessageIds = try container.decodeIfPresent([String].self, forKey: .visibleMessageIds) ?? []
        filteredPlaceholders = try container.decodeIfPresent([GaryxRenderFilteredPlaceholder].self, forKey: .filteredPlaceholders) ?? []
        rateLimit = try container.decodeIfPresent(GaryxRenderRateLimit.self, forKey: .rateLimit)
        window = try container.decodeIfPresent(GaryxRenderWindow.self, forKey: .window)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(basedOnSeq, forKey: .basedOnSeq)
        try container.encode(rows, forKey: .rows)
        try container.encode(tailActivity, forKey: .tailActivity)
        try container.encodeIfPresent(activeToolGroupId, forKey: .activeToolGroupId)
        try container.encode(progressLocus, forKey: .progressLocus)
        try container.encode(visibleMessageIds, forKey: .visibleMessageIds)
        try container.encode(filteredPlaceholders, forKey: .filteredPlaceholders)
        try container.encodeIfPresent(rateLimit, forKey: .rateLimit)
        try container.encodeIfPresent(window, forKey: .window)
    }
}

public struct GaryxRenderWindow: Codable, Equatable, Sendable {
    public var floorSeq: Int
    public var hasMoreAbove: Bool

    public init(floorSeq: Int, hasMoreAbove: Bool) {
        self.floorSeq = floorSeq
        self.hasMoreAbove = hasMoreAbove
    }

    enum CodingKeys: String, CodingKey {
        case floorSeq = "floor_seq"
        case hasMoreAbove = "has_more_above"
    }
}

/// Provider quota / rate-limit context surfaced on the render snapshot when the
/// thread's most recent run terminated because the provider's rolling usage
/// quota was exhausted. The chat view renders a banner with a live countdown to
/// `resetAt`; `willAutoResend` indicates the gateway scheduled an automatic
/// resend of the original message when the window recovers.
public struct GaryxRenderRateLimit: Codable, Equatable, Sendable {
    public var provider: String?
    public var resetAt: String?
    public var window: String?
    public var message: String?
    public var willAutoResend: Bool

    public init(
        provider: String? = nil,
        resetAt: String? = nil,
        window: String? = nil,
        message: String? = nil,
        willAutoResend: Bool = false
    ) {
        self.provider = provider
        self.resetAt = resetAt
        self.window = window
        self.message = message
        self.willAutoResend = willAutoResend
    }

    enum CodingKeys: String, CodingKey {
        case provider
        case resetAt
        case window
        case message
        case willAutoResend
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        provider = try container.decodeIfPresent(String.self, forKey: .provider)
        resetAt = try container.decodeIfPresent(String.self, forKey: .resetAt)
        window = try container.decodeIfPresent(String.self, forKey: .window)
        message = try container.decodeIfPresent(String.self, forKey: .message)
        willAutoResend = try container.decodeIfPresent(Bool.self, forKey: .willAutoResend) ?? false
    }
}

public enum GaryxRenderTailActivity: String, Codable, Equatable, Sendable {
    case none
    case thinking
    case assistantStreaming = "assistant_streaming"
    case toolActive = "tool_active"
}

public enum GaryxRenderProgressLocus: String, Codable, Equatable, Sendable {
    case none
    case tail
    case toolGroup = "tool_group"
}

public enum GaryxRenderRow: Codable, Equatable, Sendable {
    case userTurn(GaryxRenderUserTurnRow)
    case unknown

    enum CodingKeys: String, CodingKey {
        case kind
    }

    enum Kind: String, Codable {
        case userTurn = "user_turn"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        guard let kind = try? container.decode(Kind.self, forKey: .kind) else {
            self = .unknown
            return
        }
        switch kind {
        case .userTurn:
            self = try .userTurn(GaryxRenderUserTurnRow(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case let .userTurn(row):
            try row.encode(to: encoder)
        case .unknown:
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode("unknown", forKey: .kind)
        }
    }
}

public enum GaryxRenderCapsuleAction: String, Codable, Equatable, Sendable {
    case created
    case updated
}

public struct GaryxRenderCapsuleCard: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var capsuleId: String
    public var title: String
    public var revision: Int
    public var action: GaryxRenderCapsuleAction

    public init(
        id: String,
        capsuleId: String,
        title: String,
        revision: Int,
        action: GaryxRenderCapsuleAction
    ) {
        self.id = id
        self.capsuleId = capsuleId
        self.title = title
        self.revision = revision
        self.action = action
    }

    enum CodingKeys: String, CodingKey {
        case id
        case capsuleId = "capsule_id"
        case title
        case revision
        case action
    }
}

public struct GaryxRenderUserTurnRow: Codable, Equatable, Sendable {
    public var id: String
    public var user: GaryxRenderMessageRef?
    public var activity: [GaryxRenderActivityRow]
    public var startedAt: String?
    public var finishedAt: String?
    public var capsuleCards: [GaryxRenderCapsuleCard]

    public init(
        id: String,
        user: GaryxRenderMessageRef?,
        activity: [GaryxRenderActivityRow],
        startedAt: String? = nil,
        finishedAt: String? = nil,
        capsuleCards: [GaryxRenderCapsuleCard] = []
    ) {
        self.id = id
        self.user = user
        self.activity = activity
        self.startedAt = startedAt
        self.finishedAt = finishedAt
        self.capsuleCards = capsuleCards
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case id
        case user
        case activity
        case startedAt = "started_at"
        case finishedAt = "finished_at"
        case capsuleCards = "capsule_cards"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        user = try container.decodeIfPresent(GaryxRenderMessageRef.self, forKey: .user)
        activity = try container.decodeIfPresent(GaryxLossyDecodableArray<GaryxRenderActivityRow>.self, forKey: .activity)?.elements ?? []
        startedAt = try container.decodeIfPresent(String.self, forKey: .startedAt)
        finishedAt = try container.decodeIfPresent(String.self, forKey: .finishedAt)
        capsuleCards = try container.decodeIfPresent([GaryxRenderCapsuleCard].self, forKey: .capsuleCards) ?? []
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("user_turn", forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encodeIfPresent(user, forKey: .user)
        try container.encode(activity, forKey: .activity)
        try container.encodeIfPresent(startedAt, forKey: .startedAt)
        try container.encodeIfPresent(finishedAt, forKey: .finishedAt)
        if !capsuleCards.isEmpty {
            try container.encode(capsuleCards, forKey: .capsuleCards)
        }
    }
}

public enum GaryxRenderActivityRow: Codable, Equatable, Sendable {
    case assistantReply(GaryxRenderAssistantReplyRow)
    case step(GaryxRenderStepRow)
    case unknown

    enum CodingKeys: String, CodingKey {
        case kind
    }

    enum Kind: String, Codable {
        case assistantReply = "assistant_reply"
        case step
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        guard let kind = try? container.decode(Kind.self, forKey: .kind) else {
            self = .unknown
            return
        }
        switch kind {
        case .assistantReply:
            self = try .assistantReply(GaryxRenderAssistantReplyRow(from: decoder))
        case .step:
            self = try .step(GaryxRenderStepRow(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case let .assistantReply(row):
            try row.encode(to: encoder)
        case let .step(row):
            try row.encode(to: encoder)
        case .unknown:
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode("unknown", forKey: .kind)
        }
    }
}

public struct GaryxRenderAssistantReplyRow: Codable, Equatable, Sendable {
    public var id: String
    public var message: GaryxRenderMessageRef
    public var streaming: Bool

    public init(id: String, message: GaryxRenderMessageRef, streaming: Bool = false) {
        self.id = id
        self.message = message
        self.streaming = streaming
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case id
        case message
        case streaming
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        message = try container.decode(GaryxRenderMessageRef.self, forKey: .message)
        streaming = try container.decodeIfPresent(Bool.self, forKey: .streaming) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("assistant_reply", forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encode(message, forKey: .message)
        try container.encode(streaming, forKey: .streaming)
    }
}

public struct GaryxRenderStepRow: Codable, Equatable, Sendable {
    public var id: String
    public var steps: [GaryxRenderStepItem]
    public var finalMessage: GaryxRenderMessageRef?
    public var running: Bool
    public var startedAt: String?
    public var finishedAt: String?

    public init(
        id: String,
        steps: [GaryxRenderStepItem],
        finalMessage: GaryxRenderMessageRef? = nil,
        running: Bool = false,
        startedAt: String? = nil,
        finishedAt: String? = nil
    ) {
        self.id = id
        self.steps = steps
        self.finalMessage = finalMessage
        self.running = running
        self.startedAt = startedAt
        self.finishedAt = finishedAt
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case id
        case steps
        case finalMessage = "final_message"
        case running
        case startedAt = "started_at"
        case finishedAt = "finished_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        steps = try container.decodeIfPresent(GaryxLossyDecodableArray<GaryxRenderStepItem>.self, forKey: .steps)?.elements ?? []
        finalMessage = try container.decodeIfPresent(GaryxRenderMessageRef.self, forKey: .finalMessage)
        running = try container.decodeIfPresent(Bool.self, forKey: .running) ?? false
        startedAt = try container.decodeIfPresent(String.self, forKey: .startedAt)
        finishedAt = try container.decodeIfPresent(String.self, forKey: .finishedAt)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("step", forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encode(steps, forKey: .steps)
        try container.encodeIfPresent(finalMessage, forKey: .finalMessage)
        try container.encode(running, forKey: .running)
        try container.encodeIfPresent(startedAt, forKey: .startedAt)
        try container.encodeIfPresent(finishedAt, forKey: .finishedAt)
    }
}

public enum GaryxRenderStepItem: Codable, Equatable, Sendable {
    case assistantMessage(GaryxRenderAssistantStep)
    case toolGroup(GaryxRenderToolGroup)
    case unknown

    enum CodingKeys: String, CodingKey {
        case kind
    }

    enum Kind: String, Codable {
        case assistantMessage = "assistant_message"
        case toolGroup = "tool_group"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        guard let kind = try? container.decode(Kind.self, forKey: .kind) else {
            self = .unknown
            return
        }
        switch kind {
        case .assistantMessage:
            self = try .assistantMessage(GaryxRenderAssistantStep(from: decoder))
        case .toolGroup:
            self = try .toolGroup(GaryxRenderToolGroup(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case let .assistantMessage(step):
            try step.encode(to: encoder)
        case let .toolGroup(group):
            try group.encode(to: encoder)
        case .unknown:
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode("unknown", forKey: .kind)
        }
    }
}

public struct GaryxRenderAssistantStep: Codable, Equatable, Sendable {
    public var id: String
    public var message: GaryxRenderMessageRef
    public var streaming: Bool

    public init(id: String, message: GaryxRenderMessageRef, streaming: Bool = false) {
        self.id = id
        self.message = message
        self.streaming = streaming
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case id
        case message
        case streaming
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        message = try container.decode(GaryxRenderMessageRef.self, forKey: .message)
        streaming = try container.decodeIfPresent(Bool.self, forKey: .streaming) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("assistant_message", forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encode(message, forKey: .message)
        try container.encode(streaming, forKey: .streaming)
    }
}

public struct GaryxRenderToolGroup: Codable, Equatable, Sendable {
    public var id: String
    public var status: GaryxRenderToolGroupStatus
    public var entries: [GaryxRenderToolEntry]
    public var startedAt: String?
    public var finishedAt: String?

    public init(
        id: String,
        status: GaryxRenderToolGroupStatus,
        entries: [GaryxRenderToolEntry],
        startedAt: String? = nil,
        finishedAt: String? = nil
    ) {
        self.id = id
        self.status = status
        self.entries = entries
        self.startedAt = startedAt
        self.finishedAt = finishedAt
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case id
        case status
        case entries
        case startedAt = "started_at"
        case finishedAt = "finished_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        status = try container.decode(GaryxRenderToolGroupStatus.self, forKey: .status)
        entries = try container.decodeIfPresent([GaryxRenderToolEntry].self, forKey: .entries) ?? []
        startedAt = try container.decodeIfPresent(String.self, forKey: .startedAt)
        finishedAt = try container.decodeIfPresent(String.self, forKey: .finishedAt)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("tool_group", forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encode(status, forKey: .status)
        try container.encode(entries, forKey: .entries)
        try container.encodeIfPresent(startedAt, forKey: .startedAt)
        try container.encodeIfPresent(finishedAt, forKey: .finishedAt)
    }
}

public enum GaryxRenderToolGroupStatus: String, Codable, Equatable, Sendable {
    case active
    case completed
}

public struct GaryxRenderToolEntry: Codable, Equatable, Sendable {
    public var id: String
    public var toolUseId: String?
    public var status: GaryxRenderToolEntryStatus
    public var toolUse: GaryxRenderMessageRef?
    public var toolResult: GaryxRenderMessageRef?

    public init(
        id: String,
        toolUseId: String? = nil,
        status: GaryxRenderToolEntryStatus,
        toolUse: GaryxRenderMessageRef? = nil,
        toolResult: GaryxRenderMessageRef? = nil
    ) {
        self.id = id
        self.toolUseId = toolUseId
        self.status = status
        self.toolUse = toolUse
        self.toolResult = toolResult
    }

    enum CodingKeys: String, CodingKey {
        case id
        case toolUseId = "tool_use_id"
        case status
        case toolUse = "tool_use"
        case toolResult = "tool_result"
    }
}

public enum GaryxRenderToolEntryStatus: String, Codable, Equatable, Sendable {
    case running
    case completed
    case failed
}

public struct GaryxRenderMessageRef: Codable, Equatable, Sendable {
    public var id: String
    public var seq: Int
    public var role: String

    public init(id: String, seq: Int, role: String) {
        self.id = id
        self.seq = seq
        self.role = role
    }
}

public struct GaryxRenderFilteredPlaceholder: Codable, Equatable, Sendable {
    public var message: GaryxRenderMessageRef
    public var reason: GaryxRenderPlaceholderFilterReason

    public init(message: GaryxRenderMessageRef, reason: GaryxRenderPlaceholderFilterReason) {
        self.message = message
        self.reason = reason
    }
}

public enum GaryxRenderPlaceholderFilterReason: String, Codable, Equatable, Sendable {
    case emptyStreamingAssistant = "empty_streaming_assistant"
}

public enum GaryxSelectedThreadHistoryPresentation {
    public static func isAwaitingInitialHistory(
        threadId: String?,
        historyLoaded: Bool,
        liveRenderSnapshot: GaryxRenderSnapshot?,
        cachedTranscript: GaryxCachedTranscript?,
        resolvedMessageIds: Set<String> = [],
        resolvedHistoryIndexes: Set<Int> = [],
        hasRemoteFinalMessages: Bool = false
    ) -> Bool {
        guard let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty
        else {
            return false
        }
        if let snapshot = liveRenderSnapshot ?? cachedTranscript?.renderSnapshot {
            return hasUnresolvedVisibleRefs(
                snapshot: snapshot,
                resolvedMessageIds: resolvedMessageIds,
                resolvedHistoryIndexes: resolvedHistoryIndexes,
                transcriptMessages: cachedTranscript?.messages ?? []
            )
        }
        guard historyLoaded else {
            return true
        }
        // Use the committed ledger boundary, not `likelyUserVisible`: tool-only
        // rows can be renderable even when their individual messages are not
        // likely user-visible, while internal control rows never form turns.
        return hasRemoteFinalMessages || (cachedTranscript?.messages.contains { !$0.internalMessage } == true)
    }

    private static func hasUnresolvedVisibleRefs(
        snapshot: GaryxRenderSnapshot,
        resolvedMessageIds: Set<String>,
        resolvedHistoryIndexes: Set<Int>,
        transcriptMessages: [GaryxTranscriptMessage]
    ) -> Bool {
        let lookup = MessageLookup(messages: [], transcriptMessages: transcriptMessages)
        return snapshot.rows.flatMap(\.messageRefs).contains { ref in
            let historyIndex = max(ref.seq - 1, 0)
            let resolvedByMobileCache =
                resolvedMessageIds.contains(ref.id) || resolvedHistoryIndexes.contains(historyIndex)
            return !resolvedByMobileCache && lookup.transcriptMessage(for: ref) == nil
        }
    }
}

enum GaryxMobileRenderStateMapper {
    static func rows(
        snapshot: GaryxRenderSnapshot?,
        messages: [GaryxMobileMessage],
        transcriptMessages: [GaryxTranscriptMessage]
    ) -> [GaryxMobileTurnRow] {
        let lookup = MessageLookup(messages: messages, transcriptMessages: transcriptMessages)
        var rows = snapshot?.rows.compactMap { row in
            row.mobileRow(lookup: lookup)
        } ?? []
        let representedMessageIds = Set((snapshot?.rows ?? []).flatMap(\.messageRefs).map(\.id))
        rows += localUserRows(messages: messages, representedBy: representedMessageIds)
        return rows
    }

    private static func localUserRows(
        messages: [GaryxMobileMessage],
        representedBy representedMessageIds: Set<String>
    ) -> [GaryxMobileTurnRow] {
        messages.compactMap { message in
            guard message.role == .user,
                  message.localState != nil,
                  message.localState != .remoteFinal,
                  !representedMessageIds.contains(message.id)
            else {
                return nil
            }
            return GaryxMobileTurnRow(
                id: "user_turn:\(message.id)",
                userBlock: .message(message),
                activityRows: [],
                capsuleCards: []
            )
        }
    }
}

private struct MessageLookup {
    private var mobileByHistoryIndex: [Int: GaryxMobileMessage] = [:]
    private var mobileById: [String: GaryxMobileMessage] = [:]
    private var transcriptByHistoryIndex: [Int: GaryxTranscriptMessage] = [:]
    private var transcriptById: [String: GaryxTranscriptMessage] = [:]

    init(messages: [GaryxMobileMessage], transcriptMessages: [GaryxTranscriptMessage]) {
        for message in messages {
            mobileById[message.id] = message
            if let historyIndex = message.historyIndex {
                mobileByHistoryIndex[historyIndex] = message
            }
        }
        for message in transcriptMessages {
            transcriptById[message.id] = message
            if let index = message.index {
                transcriptByHistoryIndex[index] = message
            }
        }
    }

    func mobileMessage(for ref: GaryxRenderMessageRef) -> GaryxMobileMessage? {
        mobileByHistoryIndex[ref.seq - 1] ?? mobileById[ref.id]
    }

    func transcriptMessage(for ref: GaryxRenderMessageRef?) -> GaryxTranscriptMessage? {
        guard let ref else { return nil }
        return transcriptByHistoryIndex[ref.seq - 1] ?? transcriptById[ref.id]
    }
}

private extension GaryxRenderRow {
    func mobileRow(lookup: MessageLookup) -> GaryxMobileTurnRow? {
        switch self {
        case let .userTurn(row):
            return row.mobileRow(lookup: lookup)
        case .unknown:
            return nil
        }
    }
}

private extension GaryxRenderUserTurnRow {
    func mobileRow(lookup: MessageLookup) -> GaryxMobileTurnRow? {
        let userBlock = user
            .map { lookup.mobileMessage(for: $0) ?? .userStepPlaceholder(for: $0) }
            .map(GaryxMobileTranscriptBlock.message)
        let activityRows = activity.compactMap { $0.mobileActivityRow(lookup: lookup) }
        guard userBlock != nil || !activityRows.isEmpty else { return nil }
        return GaryxMobileTurnRow(
            id: id,
            userBlock: userBlock,
            activityRows: activityRows,
            capsuleCards: capsuleCards
        )
    }
}

private extension GaryxRenderActivityRow {
    func mobileActivityRow(lookup: MessageLookup) -> GaryxMobileTurnRow.ActivityRow? {
        switch self {
        case let .assistantReply(row):
            guard let message = lookup.mobileMessage(for: row.message) else { return nil }
            return .flat(.message(message))
        case let .step(row):
            let steps = row.steps.compactMap { $0.mobileBlock(lookup: lookup) }
            let finalBlock = row.finalMessage
                .flatMap { lookup.mobileMessage(for: $0) }
                .map(GaryxMobileTranscriptBlock.message)
            guard !steps.isEmpty || finalBlock != nil else { return nil }
            return .turn(GaryxMobileAgentTurn(
                id: row.id,
                steps: steps,
                finalBlock: finalBlock,
                isRunning: row.running,
                startedAt: row.startedAt,
                finishedAt: row.finishedAt
            ))
        case .unknown:
            return nil
        }
    }
}

private extension GaryxRenderStepItem {
    func mobileBlock(lookup: MessageLookup) -> GaryxMobileTranscriptBlock? {
        switch self {
        case let .assistantMessage(step):
            // The server `render_state` owns the step structure: this assistant
            // sits between two tool groups. If its body has not yet reached the
            // local message store, fall back to a placeholder instead of dropping
            // the step — dropping it would collapse the surrounding tool groups
            // into adjacent rows (TASK-1021). Mirrors the tool-group fallback,
            // which never vanishes when its refs are unresolved.
            let message = lookup.mobileMessage(for: step.message)
                ?? .assistantStepPlaceholder(for: step.message)
            return .message(message)
        case let .toolGroup(group):
            return group.mobileBlock(lookup: lookup)
        case .unknown:
            return nil
        }
    }
}

private extension GaryxMobileMessage {
    /// Body-less placeholder for a user turn whose server render row arrived
    /// before the committed body reached the local message cache. The id and
    /// history index mirror the committed body so the real message replaces it
    /// in place.
    static func userStepPlaceholder(for ref: GaryxRenderMessageRef) -> GaryxMobileMessage {
        let historyIndex = max(ref.seq - 1, 0)
        return GaryxMobileMessage(
            id: "history:\(historyIndex)",
            role: .user,
            text: "",
            timestamp: nil,
            isStreaming: true,
            localState: .remotePartial,
            historyIndex: historyIndex
        )
    }

    /// Body-less placeholder for an assistant step whose committed body has not
    /// yet reached the local `messages` store (the synchronously-updated render
    /// snapshot can reference a seq before the throttled message flush ingests
    /// its body). `id`/`historyIndex` mirror the committed body's
    /// (`history:<seq-1>`) so the row upgrades in place — not re-inserts — once
    /// the body arrives. Rendered as a loading state, never an empty bubble.
    static func assistantStepPlaceholder(for ref: GaryxRenderMessageRef) -> GaryxMobileMessage {
        let historyIndex = max(ref.seq - 1, 0)
        return GaryxMobileMessage(
            id: "history:\(historyIndex)",
            role: .assistant,
            text: "",
            timestamp: nil,
            isStreaming: true,
            localState: .remotePartial,
            historyIndex: historyIndex
        )
    }
}

private extension GaryxRenderToolGroup {
    func mobileBlock(lookup: MessageLookup) -> GaryxMobileTranscriptBlock? {
        let historyIndex = entries.flatMap(\.messageRefs).map { $0.seq - 1 }.min()
        let mobileEntries = entries.map { $0.mobileEntry(lookup: lookup) }
        guard !mobileEntries.isEmpty else { return nil }
        let live = status == .active
        let group = GaryxMobileToolTraceGroup(entries: mobileEntries, live: live)
        let message = GaryxMobileMessage(
            id: id,
            role: .tool,
            text: group.summary,
            timestamp: startedAt,
            isStreaming: live,
            toolTraceGroup: group,
            localState: live ? .remotePartial : .remoteFinal,
            historyIndex: historyIndex
        )
        return .toolGroup(message)
    }
}

private extension GaryxRenderToolEntry {
    func mobileEntry(lookup: MessageLookup) -> GaryxMobileToolTraceEntry {
        let useMessage = lookup.transcriptMessage(for: toolUse)
        let resultMessage = lookup.transcriptMessage(for: toolResult)
        let usePayload = useMessage.map(GaryxMobileToolTracePayload.fromTranscript)
        let resultPayload = resultMessage.map(GaryxMobileToolTracePayload.fromTranscript)
        let resolvedToolUseId = toolUseId.garyxRenderTrimmedNilIfEmpty
            ?? usePayload?.toolUseId
            ?? resultPayload?.toolUseId
        let toolName = usePayload?.normalizedToolName.garyxRenderTrimmedNilIfEmpty
            ?? resultPayload?.normalizedToolName.garyxRenderTrimmedNilIfEmpty
            ?? "tool"
        let title = GaryxMobileToolTraceEntry.title(for: toolName)
        return GaryxMobileToolTraceEntry(
            id: id,
            toolUseId: resolvedToolUseId,
            parentToolUseId: usePayload?.parentToolUseId ?? resultPayload?.parentToolUseId,
            toolName: toolName,
            title: title,
            inputText: usePayload?.contentText,
            resultText: resultPayload?.contentText,
            summaryText: usePayload?.summaryText ?? resultPayload?.summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: mobileStatus,
            isError: status == .failed || resultPayload?.isError == true || usePayload?.isError == true,
            timestamp: usePayload?.timestamp ?? resultPayload?.timestamp,
            primaryPathBadge: usePayload?.primaryPathBadge ?? resultPayload?.primaryPathBadge,
            primaryPath: usePayload?.primaryPath ?? resultPayload?.primaryPath
        )
    }

    var mobileStatus: GaryxMobileToolTraceStatus {
        switch status {
        case .running:
            return .running
        case .completed:
            return .completed
        case .failed:
            return .failed
        }
    }
}

private extension GaryxRenderRow {
    var messageRefs: [GaryxRenderMessageRef] {
        switch self {
        case let .userTurn(row):
            return row.messageRefs
        case .unknown:
            return []
        }
    }
}

private extension GaryxRenderUserTurnRow {
    var messageRefs: [GaryxRenderMessageRef] {
        var refs = [GaryxRenderMessageRef]()
        if let user {
            refs.append(user)
        }
        refs += activity.flatMap(\.messageRefs)
        return refs
    }
}

private extension GaryxRenderActivityRow {
    var messageRefs: [GaryxRenderMessageRef] {
        switch self {
        case let .assistantReply(row):
            return [row.message]
        case let .step(row):
            var refs = row.steps.flatMap(\.messageRefs)
            if let finalMessage = row.finalMessage {
                refs.append(finalMessage)
            }
            return refs
        case .unknown:
            return []
        }
    }
}

private extension GaryxRenderStepItem {
    var messageRefs: [GaryxRenderMessageRef] {
        switch self {
        case let .assistantMessage(step):
            return [step.message]
        case let .toolGroup(group):
            return group.entries.flatMap(\.messageRefs)
        case .unknown:
            return []
        }
    }
}

private extension GaryxRenderToolEntry {
    var messageRefs: [GaryxRenderMessageRef] {
        [toolUse, toolResult].compactMap { $0 }
    }
}

private extension Optional where Wrapped == String {
    var garyxRenderTrimmedNilIfEmpty: String? {
        switch self {
        case let .some(value):
            return value.garyxRenderTrimmedNilIfEmpty
        case .none:
            return nil
        }
    }
}

private extension String {
    var garyxRenderTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

// MARK: - Rate-limit banner presentation

/// Presentation model for the thread-tail quota / rate-limit banner. Pure
/// formatting derived from `GaryxRenderSnapshot.rateLimit` plus a reference
/// `now`, so the SwiftUI layer stays a dumb renderer and the countdown logic is
/// unit-testable. Mirrors the desktop `RateLimitBanner` wording.
///
/// Defined here (rather than its own file) so it is picked up by both the
/// SwiftPM target and the Xcode app target, which references Core sources by
/// explicit file membership.
public struct GaryxRateLimitBannerModel: Equatable, Sendable {
    public let title: String
    public let detail: String
    /// True when the quota window has recovered and a resend is imminent — the
    /// view can show an active/in-progress treatment.
    public let isResending: Bool

    public init(title: String, detail: String, isResending: Bool) {
        self.title = title
        self.detail = detail
        self.isResending = isResending
    }

    /// Build banner content for a rate-limit context relative to `now`. Returns
    /// `nil` when there is no rate-limit to display.
    public static func make(
        from rateLimit: GaryxRenderRateLimit?,
        now: Date = Date()
    ) -> GaryxRateLimitBannerModel? {
        guard let rateLimit else { return nil }

        let provider = providerLabel(rateLimit.provider)
        let windowText = windowLabel(rateLimit.window)
        let title = windowText.map { "\(provider) \($0) reached" }
            ?? "\(provider) usage limit reached"

        let resetDate = rateLimit.resetAt.flatMap(parseTimestamp)
        let remaining = resetDate.map { $0.timeIntervalSince(now) }
        let recovered = resetDate != nil && (remaining ?? -1) <= 0

        var detail: String
        var isResending = false
        if rateLimit.willAutoResend {
            if let remaining, !recovered {
                detail = "Auto-resend in \(formatRemaining(remaining))"
            } else if resetDate != nil {
                detail = "Quota recovered — resending…"
                isResending = true
            } else {
                detail = "Will auto-resend when the quota recovers."
            }
        } else if let remaining, !recovered {
            detail = "Resets in \(formatRemaining(remaining))"
        } else {
            detail = "Try again shortly."
        }

        return GaryxRateLimitBannerModel(
            title: title,
            detail: detail,
            isResending: isResending
        )
    }

    static func providerLabel(_ provider: String?) -> String {
        let slug = (provider ?? "").trimmingCharacters(in: .whitespaces).lowercased()
        if slug.hasPrefix("codex") { return "Codex" }
        if slug.hasPrefix("trae") { return "TRAE" }
        let trimmed = provider?.trimmingCharacters(in: .whitespaces) ?? ""
        return trimmed.isEmpty ? "Provider" : trimmed
    }

    static func windowLabel(_ window: String?) -> String? {
        switch window {
        case "primary": return "5-hour limit"
        case "secondary": return "weekly limit"
        default: return nil
        }
    }

    static func formatRemaining(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds.rounded(.down)))
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let secs = total % 60
        if hours > 0 {
            return String(format: "%d:%02d:%02d", hours, minutes, secs)
        }
        return String(format: "%02d:%02d", minutes, secs)
    }

    static func parseTimestamp(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) {
            return date
        }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: value)
    }
}
