import Foundation

public struct GaryxInterruptRequest: Encodable, Equatable, Sendable {
    public var threadId: String

    public init(threadId: String) {
        self.threadId = threadId
    }
}


public struct GaryxStreamInputRequest: Encodable, Equatable, Sendable {
    public var threadId: String
    public var clientIntentId: String?
    public var message: String
    public var attachments: [GaryxPromptAttachment]

    public init(
        threadId: String,
        clientIntentId: String? = nil,
        message: String,
        attachments: [GaryxPromptAttachment] = []
    ) {
        self.threadId = threadId
        self.clientIntentId = clientIntentId
        self.message = message
        self.attachments = attachments
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


public struct GaryxChatWebSocketCommand: Encodable, Equatable, Sendable {
    public var op: String
    public var threadId: String?
    public var message: String?
    public var clientIntentId: String?
    public var attachments: [GaryxPromptAttachment]?
    public var images: [GaryxInlineImagePayload]?
    public var files: [GaryxInlineFilePayload]?
    public var accountId: String?
    public var fromId: String?
    public var waitForResponse: Bool?
    public var workspacePath: String?
    public var limit: Int?
    public var metadata: [String: String]

    public static func start(
        threadId: String,
        message: String,
        accountId: String = "main",
        fromId: String = "mobile",
        waitForResponse: Bool = false,
        workspacePath: String? = nil,
        attachments: [GaryxPromptAttachment] = [],
        images: [GaryxInlineImagePayload] = [],
        files: [GaryxInlineFilePayload] = [],
        metadata: [String: String] = [:]
    ) -> Self {
        Self(
            op: "start",
            threadId: threadId,
            message: message,
            attachments: attachments.isEmpty ? nil : attachments,
            images: images.isEmpty ? nil : images,
            files: files.isEmpty ? nil : files,
            accountId: accountId,
            fromId: fromId,
            waitForResponse: waitForResponse,
            workspacePath: workspacePath,
            metadata: metadata
        )
    }

    public static func input(
        threadId: String,
        message: String,
        clientIntentId: String? = nil,
        attachments: [GaryxPromptAttachment] = [],
        images: [GaryxInlineImagePayload] = [],
        files: [GaryxInlineFilePayload] = []
    ) -> Self {
        Self(
            op: "input",
            threadId: threadId,
            message: message,
            clientIntentId: clientIntentId,
            attachments: attachments.isEmpty ? nil : attachments,
            images: images.isEmpty ? nil : images,
            files: files.isEmpty ? nil : files,
            metadata: [:]
        )
    }

    public static func recover(threadId: String, limit: Int = 200) -> Self {
        Self(
            op: "recover",
            threadId: threadId,
            limit: limit,
            metadata: [:]
        )
    }

    public static func interrupt(threadId: String) -> Self {
        Self(
            op: "interrupt",
            threadId: threadId,
            metadata: [:]
        )
    }
}


public enum GaryxChatStreamEvent: Decodable, Equatable, Sendable {
    case ping
    case accepted(runId: String, threadId: String)
    case assistantDelta(runId: String, threadId: String, delta: String, metadata: [String: GaryxJSONValue]?)
    case assistantBoundary(runId: String, threadId: String)
    case toolUse(runId: String, threadId: String, message: GaryxJSONValue?)
    case toolResult(runId: String, threadId: String, message: GaryxJSONValue?)
    case userMessage(runId: String, threadId: String, text: String, imageCount: Int)
    case userAck(runId: String, threadId: String, pendingInputId: String?)
    case threadTitleUpdated(runId: String, threadId: String, title: String)
    case done(runId: String, threadId: String)
    case runComplete(runId: String, threadId: String)
    case streamInput(status: String, threadId: String, clientIntentId: String?, pendingInputId: String?)
    case interrupt(status: String, threadId: String, abortedRuns: [String])
    case snapshot(threadId: String, payload: [String: GaryxJSONValue])
    case error(runId: String, threadId: String, error: String)
    case unknown(type: String, payload: [String: GaryxJSONValue])

    enum CodingKeys: String, CodingKey {
        case type
        case runId
        case run_id
        case threadId
        case thread_id
        case sessionKey
        case delta
        case metadata
        case message
        case text
        case imageCount = "imageCount"
        case imageCountSnake = "image_count"
        case pendingInputId
        case pending_input_id
        case clientIntentId
        case client_intent_id
        case status
        case abortedRuns
        case aborted_runs
        case error
        case title
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.garyxDecodeFirstString(.type) ?? ""
        let payload = (try? GaryxJSONValue(from: decoder).garyxGatewayObjectValue) ?? [:]
        let runId = try container.garyxDecodeFirstString(.runId, .run_id) ?? ""
        let threadId = try container.garyxDecodeFirstString(.threadId, .thread_id, .sessionKey) ?? ""

        switch type {
        case "", "ping":
            self = .ping
        case "accepted":
            self = .accepted(runId: runId, threadId: threadId)
        case "assistant_delta":
            let metadata = try container.decodeIfPresent([String: GaryxJSONValue].self, forKey: .metadata)
            self = .assistantDelta(
                runId: runId,
                threadId: threadId,
                delta: try container.garyxDecodeFirstString(.delta) ?? "",
                metadata: metadata
            )
        case "assistant_boundary":
            self = .assistantBoundary(runId: runId, threadId: threadId)
        case "tool_use":
            self = .toolUse(
                runId: runId,
                threadId: threadId,
                message: try container.decodeIfPresent(GaryxJSONValue.self, forKey: .message)
            )
        case "tool_result":
            self = .toolResult(
                runId: runId,
                threadId: threadId,
                message: try container.decodeIfPresent(GaryxJSONValue.self, forKey: .message)
            )
        case "user_message":
            self = .userMessage(
                runId: runId,
                threadId: threadId,
                text: try container.garyxDecodeFirstString(.text, .message) ?? "",
                imageCount: try container.decodeIfPresent(Int.self, forKey: .imageCount)
                    ?? container.decodeIfPresent(Int.self, forKey: .imageCountSnake)
                    ?? 0
            )
        case "user_ack":
            self = .userAck(
                runId: runId,
                threadId: threadId,
                pendingInputId: try container.garyxDecodeFirstString(.pendingInputId, .pending_input_id)
            )
        case "thread_title_updated":
            self = .threadTitleUpdated(
                runId: runId,
                threadId: threadId,
                title: try container.garyxDecodeFirstString(.title) ?? ""
            )
        case "done":
            self = .done(runId: runId, threadId: threadId)
        case "run_complete":
            self = .runComplete(runId: runId, threadId: threadId)
        case "stream_input":
            self = .streamInput(
                status: try container.garyxDecodeFirstString(.status) ?? "",
                threadId: threadId,
                clientIntentId: try container.garyxDecodeFirstString(.clientIntentId, .client_intent_id),
                pendingInputId: try container.garyxDecodeFirstString(.pendingInputId, .pending_input_id)
            )
        case "interrupt":
            let abortedRuns = try container.decodeIfPresent([String].self, forKey: .abortedRuns)
                ?? container.decodeIfPresent([String].self, forKey: .aborted_runs)
                ?? []
            self = .interrupt(
                status: try container.garyxDecodeFirstString(.status) ?? "",
                threadId: threadId,
                abortedRuns: abortedRuns
            )
        case "snapshot":
            self = .snapshot(threadId: threadId, payload: payload)
        case "error":
            self = .error(
                runId: runId,
                threadId: threadId,
                error: try container.garyxDecodeFirstString(.error) ?? "agent run failed"
            )
        default:
            self = .unknown(type: type, payload: payload)
        }
    }
}
