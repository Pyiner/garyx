import Foundation

public struct GaryxInterruptRequest: Encodable, Equatable, Sendable {
    public var threadId: String

    public init(threadId: String) {
        self.threadId = threadId
    }
}


public struct GaryxStartChatRequest: Encodable, Equatable, Sendable {
    public var threadId: String
    public var message: String
    public var attachments: [GaryxPromptAttachment]
    public var images: [GaryxInlineImagePayload]
    public var files: [GaryxInlineFilePayload]
    public var accountId: String
    public var fromId: String
    public var waitForResponse: Bool
    public var workspacePath: String?
    public var metadata: [String: String]

    public init(
        threadId: String,
        message: String,
        attachments: [GaryxPromptAttachment] = [],
        images: [GaryxInlineImagePayload] = [],
        files: [GaryxInlineFilePayload] = [],
        accountId: String = "main",
        fromId: String = "garyx-mobile",
        waitForResponse: Bool = false,
        workspacePath: String? = nil,
        metadata: [String: String] = [:]
    ) {
        self.threadId = threadId
        self.message = message
        self.attachments = attachments
        self.images = images
        self.files = files
        self.accountId = accountId
        self.fromId = fromId
        self.waitForResponse = waitForResponse
        self.workspacePath = workspacePath
        self.metadata = metadata
    }
}


public struct GaryxStartChatResult: Decodable, Equatable, Sendable {
    public var status: String
    public var runId: String
    public var threadId: String

    enum CodingKeys: String, CodingKey {
        case status
        case runId
        case runIdSnake = "run_id"
        case threadId
        case threadIdSnake = "thread_id"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.garyxDecodeFirstString(.status) ?? ""
        runId = try container.garyxDecodeFirstString(.runId, .runIdSnake) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? ""
    }
}


public struct GaryxStreamInputRequest: Encodable, Equatable, Sendable {
    public var threadId: String
    public var clientIntentId: String?
    public var message: String
    public var attachments: [GaryxPromptAttachment]
    public var metadata: [String: String]

    public init(
        threadId: String,
        clientIntentId: String? = nil,
        message: String,
        attachments: [GaryxPromptAttachment] = [],
        metadata: [String: String] = [:]
    ) {
        self.threadId = threadId
        self.clientIntentId = clientIntentId
        self.message = message
        self.attachments = attachments
        self.metadata = metadata
    }
}


public struct GaryxStreamInputResult: Decodable, Equatable, Sendable {
    public var status: String
    public var threadStatus: String?
    public var clientIntentId: String?
    public var pendingInputId: String?
    public var threadId: String

    enum CodingKeys: String, CodingKey {
        case status
        case threadStatus
        case threadStatusSnake = "thread_status"
        case clientIntentId
        case clientIntentIdSnake = "client_intent_id"
        case pendingInputId
        case pendingInputIdSnake = "pending_input_id"
        case threadId
        case threadIdSnake = "thread_id"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.garyxDecodeFirstString(.status) ?? ""
        threadStatus = try container.garyxDecodeFirstString(.threadStatus, .threadStatusSnake)
        clientIntentId = try container.garyxDecodeFirstString(.clientIntentId, .clientIntentIdSnake)
        pendingInputId = try container.garyxDecodeFirstString(.pendingInputId, .pendingInputIdSnake)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? ""
    }
}


public struct GaryxInterruptResult: Decodable, Equatable, Sendable {
    public var status: String
    public var threadId: String
    public var abortedRuns: [String]

    enum CodingKeys: String, CodingKey {
        case status
        case threadId
        case threadIdSnake = "thread_id"
        case abortedRuns
        case abortedRunsSnake = "aborted_runs"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.garyxDecodeFirstString(.status) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? ""
        abortedRuns = try container.decodeIfPresent([String].self, forKey: .abortedRuns)
            ?? container.decodeIfPresent([String].self, forKey: .abortedRunsSnake)
            ?? []
    }
}
