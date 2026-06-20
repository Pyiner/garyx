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

public struct GaryxRenderSnapshot: Codable, Equatable, Sendable {
    public var basedOnSeq: Int
    public var rows: [GaryxRenderRow]
    public var tailActivity: GaryxRenderTailActivity
    public var activeToolGroupId: String?
    public var progressLocus: GaryxRenderProgressLocus
    public var visibleMessageIds: [String]
    public var filteredPlaceholders: [GaryxRenderFilteredPlaceholder]

    public init(
        basedOnSeq: Int,
        rows: [GaryxRenderRow],
        tailActivity: GaryxRenderTailActivity = .none,
        activeToolGroupId: String? = nil,
        progressLocus: GaryxRenderProgressLocus = .none,
        visibleMessageIds: [String] = [],
        filteredPlaceholders: [GaryxRenderFilteredPlaceholder] = []
    ) {
        self.basedOnSeq = basedOnSeq
        self.rows = rows
        self.tailActivity = tailActivity
        self.activeToolGroupId = activeToolGroupId
        self.progressLocus = progressLocus
        self.visibleMessageIds = visibleMessageIds
        self.filteredPlaceholders = filteredPlaceholders
    }

    enum CodingKeys: String, CodingKey {
        case basedOnSeq = "based_on_seq"
        case rows
        case tailActivity
        case activeToolGroupId
        case progressLocus = "progress_locus"
        case visibleMessageIds
        case filteredPlaceholders = "filtered_placeholders"
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

    enum CodingKeys: String, CodingKey {
        case kind
    }

    enum Kind: String, Codable {
        case userTurn = "user_turn"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(Kind.self, forKey: .kind)
        switch kind {
        case .userTurn:
            self = .userTurn(try GaryxRenderUserTurnRow(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .userTurn(let row):
            try row.encode(to: encoder)
        }
    }
}

public struct GaryxRenderUserTurnRow: Codable, Equatable, Sendable {
    public var id: String
    public var user: GaryxRenderMessageRef?
    public var activity: [GaryxRenderActivityRow]
    public var startedAt: String?
    public var finishedAt: String?

    public init(
        id: String,
        user: GaryxRenderMessageRef?,
        activity: [GaryxRenderActivityRow],
        startedAt: String? = nil,
        finishedAt: String? = nil
    ) {
        self.id = id
        self.user = user
        self.activity = activity
        self.startedAt = startedAt
        self.finishedAt = finishedAt
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case id
        case user
        case activity
        case startedAt = "started_at"
        case finishedAt = "finished_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        user = try container.decodeIfPresent(GaryxRenderMessageRef.self, forKey: .user)
        activity = try container.decodeIfPresent([GaryxRenderActivityRow].self, forKey: .activity) ?? []
        startedAt = try container.decodeIfPresent(String.self, forKey: .startedAt)
        finishedAt = try container.decodeIfPresent(String.self, forKey: .finishedAt)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("user_turn", forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encodeIfPresent(user, forKey: .user)
        try container.encode(activity, forKey: .activity)
        try container.encodeIfPresent(startedAt, forKey: .startedAt)
        try container.encodeIfPresent(finishedAt, forKey: .finishedAt)
    }
}

public enum GaryxRenderActivityRow: Codable, Equatable, Sendable {
    case assistantReply(GaryxRenderAssistantReplyRow)
    case step(GaryxRenderStepRow)

    enum CodingKeys: String, CodingKey {
        case kind
    }

    enum Kind: String, Codable {
        case assistantReply = "assistant_reply"
        case step
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(Kind.self, forKey: .kind)
        switch kind {
        case .assistantReply:
            self = .assistantReply(try GaryxRenderAssistantReplyRow(from: decoder))
        case .step:
            self = .step(try GaryxRenderStepRow(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .assistantReply(let row):
            try row.encode(to: encoder)
        case .step(let row):
            try row.encode(to: encoder)
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
        steps = try container.decodeIfPresent([GaryxRenderStepItem].self, forKey: .steps) ?? []
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

    enum CodingKeys: String, CodingKey {
        case kind
    }

    enum Kind: String, Codable {
        case assistantMessage = "assistant_message"
        case toolGroup = "tool_group"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(Kind.self, forKey: .kind)
        switch kind {
        case .assistantMessage:
            self = .assistantMessage(try GaryxRenderAssistantStep(from: decoder))
        case .toolGroup:
            self = .toolGroup(try GaryxRenderToolGroup(from: decoder))
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .assistantMessage(let step):
            try step.encode(to: encoder)
        case .toolGroup(let group):
            try group.encode(to: encoder)
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
        hasRemoteFinalMessages: Bool = false
    ) -> Bool {
        guard let threadId = threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return false
        }
        if liveRenderSnapshot != nil || cachedTranscript?.renderSnapshot != nil {
            return false
        }
        guard historyLoaded else {
            return true
        }
        // Use the committed ledger boundary, not `likelyUserVisible`: tool-only
        // rows can be renderable even when their individual messages are not
        // likely user-visible, while internal control rows never form turns.
        return hasRemoteFinalMessages || (cachedTranscript?.messages.contains { !$0.internalMessage } == true)
    }
}

enum GaryxMobileRenderStateMapper {
    static func rows(
        snapshot: GaryxRenderSnapshot?,
        messages: [GaryxMobileMessage],
        transcriptMessages: [GaryxTranscriptMessage]
    ) -> [GaryxMobileTurnRow] {
        let lookup = MessageLookup(messages: messages, transcriptMessages: transcriptMessages)
        return snapshot?.rows.compactMap { row in
            row.mobileRow(lookup: lookup)
        } ?? []
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
        case .userTurn(let row):
            return row.mobileRow(lookup: lookup)
        }
    }
}

private extension GaryxRenderUserTurnRow {
    func mobileRow(lookup: MessageLookup) -> GaryxMobileTurnRow? {
        let userBlock = user.flatMap { lookup.mobileMessage(for: $0) }.map(GaryxMobileTranscriptBlock.message)
        let activityRows = activity.compactMap { $0.mobileActivityRow(lookup: lookup) }
        guard userBlock != nil || !activityRows.isEmpty else { return nil }
        return GaryxMobileTurnRow(id: id, userBlock: userBlock, activityRows: activityRows)
    }
}

private extension GaryxRenderActivityRow {
    func mobileActivityRow(lookup: MessageLookup) -> GaryxMobileTurnRow.ActivityRow? {
        switch self {
        case .assistantReply(let row):
            guard let message = lookup.mobileMessage(for: row.message) else { return nil }
            return .flat(.message(message))
        case .step(let row):
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
        }
    }
}

private extension GaryxRenderStepItem {
    func mobileBlock(lookup: MessageLookup) -> GaryxMobileTranscriptBlock? {
        switch self {
        case .assistantMessage(let step):
            return lookup.mobileMessage(for: step.message).map(GaryxMobileTranscriptBlock.message)
        case .toolGroup(let group):
            return group.mobileBlock(lookup: lookup)
        }
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
        case .userTurn(let row):
            return row.messageRefs
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
        case .assistantReply(let row):
            return [row.message]
        case .step(let row):
            var refs = row.steps.flatMap(\.messageRefs)
            if let finalMessage = row.finalMessage {
                refs.append(finalMessage)
            }
            return refs
        }
    }
}

private extension GaryxRenderStepItem {
    var messageRefs: [GaryxRenderMessageRef] {
        switch self {
        case .assistantMessage(let step):
            return [step.message]
        case .toolGroup(let group):
            return group.entries.flatMap(\.messageRefs)
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
        case .some(let value):
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
