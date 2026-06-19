import Foundation

public enum GaryxTranscriptMessageKind: String, Equatable, Sendable {
    case assistantReply = "assistant_reply"
    case control
    case internalMessage = "internal"
    case system
    case toolTrace = "tool_trace"
    case userInput = "user_input"
}

public enum GaryxTranscriptRunActivity: String, Equatable, Sendable {
    case idle
    case thinking
    case usingTool = "using_tool"
    case reconciling
}

public struct GaryxTranscriptRewriteRange: Equatable, Sendable {
    public var noticeSeq: Int?
    public var startSeq: Int
    public var endSeq: Int

    public init(noticeSeq: Int?, startSeq: Int, endSeq: Int) {
        self.noticeSeq = noticeSeq
        self.startSeq = startSeq
        self.endSeq = endSeq
    }
}

public struct GaryxTranscriptRunState: Equatable, Sendable {
    public var busy: Bool
    public var activeRunId: String?
    public var activity: GaryxTranscriptRunActivity
    public var terminalStatus: String?
    public var lastUserAckSeq: Int?
    public var lastUserAckPendingInputId: String?
    public var title: String?
    public var rewriteRanges: [GaryxTranscriptRewriteRange]
    public var lastTranscriptResetSeq: Int?

    public init(
        busy: Bool = false,
        activeRunId: String? = nil,
        activity: GaryxTranscriptRunActivity = .idle,
        terminalStatus: String? = nil,
        lastUserAckSeq: Int? = nil,
        lastUserAckPendingInputId: String? = nil,
        title: String? = nil,
        rewriteRanges: [GaryxTranscriptRewriteRange] = [],
        lastTranscriptResetSeq: Int? = nil
    ) {
        self.busy = busy
        self.activeRunId = activeRunId
        self.activity = activity
        self.terminalStatus = terminalStatus
        self.lastUserAckSeq = lastUserAckSeq
        self.lastUserAckPendingInputId = lastUserAckPendingInputId
        self.title = title
        self.rewriteRanges = rewriteRanges
        self.lastTranscriptResetSeq = lastTranscriptResetSeq
    }
}

public enum GaryxTranscriptKindResolver {
    public static func kind(for message: GaryxTranscriptMessage) -> GaryxTranscriptMessageKind {
        let role = message.role.rawValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let toolRelated = isToolRelated(message)
        if isControlMessage(message) {
            return .control
        }
        switch role {
        case "user":
            return .userInput
        case "assistant":
            return toolRelated ? .toolTrace : .assistantReply
        case "tool", "tool_use", "tool_result":
            return .toolTrace
        case "system":
            return .system
        default:
            return toolRelated ? .toolTrace : .internalMessage
        }
    }

    public static func isControlMessage(_ message: GaryxTranscriptMessage) -> Bool {
        isControlText(message.kind) || isControlText(message.internalKind)
    }

    public static func isToolRelated(_ message: GaryxTranscriptMessage) -> Bool {
        switch message.role {
        case .tool, .toolUse, .toolResult:
            return true
        case .assistant, .system, .user, .unknown:
            break
        }

        if message.toolUseResult {
            return true
        }

        if trimmedString(message.toolName) != nil {
            return true
        }

        return containsToolHint(message.content)
            || containsToolHint(message.metadata)
            || containsToolHint(message.input)
            || containsToolHint(message.result)
    }

    private static func isControlText(_ value: String?) -> Bool {
        trimmedString(value)?.lowercased() == "control"
    }

    private static func containsToolHint(_ value: GaryxJSONValue?) -> Bool {
        guard let value else { return false }
        func inspect(_ value: GaryxJSONValue, depth: Int) -> Bool {
            if depth > 64 {
                return false
            }
            switch value {
            case .string(let text):
                guard depth > 0 else { return false }
                let lower = text.lowercased()
                return lower.contains("tool_use")
                    || lower.contains("tool_result")
                    || lower.contains("tool_call")
                    || lower.contains("mcp__")
            case .array(let items):
                return items.contains { inspect($0, depth: depth + 1) }
            case .object(let object):
                return object.contains { key, item in
                    let lower = key.lowercased()
                    return lower == "tool_use_id"
                        || lower == "tool_call_id"
                        || lower == "tool_calls"
                        || lower.contains("mcp__")
                        || lower.contains("tool_")
                        || inspect(item, depth: depth + 1)
                }
            case .number, .bool, .null:
                return false
            }
        }
        return inspect(value, depth: 0)
    }
}

public enum GaryxTranscriptRunStateReducer {
    public static func reduce(_ messages: [GaryxTranscriptMessage]) -> GaryxTranscriptRunState {
        var state = GaryxTranscriptRunState()
        for message in messages {
            apply(message: message, to: &state)
        }
        return state
    }

    public static func apply(message: GaryxTranscriptMessage, to state: inout GaryxTranscriptRunState) {
        apply(message: message, seq: message.index.map { $0 + 1 }, to: &state)
    }

    public static func apply(message: GaryxTranscriptMessage, seq: Int?, to state: inout GaryxTranscriptRunState) {
        switch GaryxTranscriptKindResolver.kind(for: message) {
        case .control:
            applyControl(message.control ?? nestedControl(from: message.content), seq: seq, to: &state)
        case .toolTrace:
            if state.busy {
                state.activity = .usingTool
            }
        case .assistantReply, .userInput:
            if state.busy && state.activity != .reconciling {
                state.activity = .thinking
            }
        case .internalMessage, .system:
            break
        }
    }

    private static func applyControl(_ value: GaryxJSONValue?, seq: Int?, to state: inout GaryxTranscriptRunState) {
        guard case let .object(control)? = value,
              let kind = trimmedString(control.runStateStringValue(forKeys: ["kind"])) else {
            return
        }

        switch kind {
        case "run_start":
            state.busy = true
            state.activeRunId = control.runStateStringValue(forKeys: ["run_id", "runId"])
            state.terminalStatus = nil
            state.activity = .thinking
        case "user_ack":
            state.lastUserAckSeq = seq
            state.lastUserAckPendingInputId = control.runStateStringValue(forKeys: ["pending_input_id", "pendingInputId"])
        case "assistant_boundary":
            if state.busy && state.activity != .reconciling {
                state.activity = .thinking
            }
        case "done":
            if state.busy {
                state.activity = .reconciling
            }
        case "run_complete":
            state.busy = false
            state.activeRunId = nil
            state.activity = .idle
            state.terminalStatus = control.runStateStringValue(forKeys: ["status"]) ?? "completed"
        case "run_interrupted", "interrupt_confirmed":
            state.busy = false
            state.activeRunId = nil
            state.activity = .idle
            state.terminalStatus = "interrupted"
        case "thread_title_updated":
            state.title = control.runStateStringValue(forKeys: ["title"])
        case "transcript_reset":
            state.lastTranscriptResetSeq = seq
        case "range_rewrite":
            let startSeq = control.runStateIntValue(forKeys: ["start_seq"]) ?? seq ?? 0
            let endSeq = control.runStateIntValue(forKeys: ["end_seq"]) ?? startSeq
            state.rewriteRanges.append(GaryxTranscriptRewriteRange(
                noticeSeq: seq,
                startSeq: startSeq,
                endSeq: endSeq
            ))
        default:
            break
        }
    }

    private static func nestedControl(from value: GaryxJSONValue?) -> GaryxJSONValue? {
        guard case let .object(object)? = value else { return nil }
        return object["control"]
    }
}

private func trimmedString(_ value: String?) -> String? {
    let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return trimmed.isEmpty ? nil : trimmed
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    func runStateStringValue(forKeys keys: [String]) -> String? {
        for key in keys {
            guard let value = self[key]?.runStateStringValue else { continue }
            return value
        }
        return nil
    }

    func runStateIntValue(forKeys keys: [String]) -> Int? {
        for key in keys {
            guard let value = self[key]?.runStateIntValue else { continue }
            return value
        }
        return nil
    }
}

private extension GaryxJSONValue {
    var runStateStringValue: String? {
        switch self {
        case .string(let value):
            return trimmedString(value)
        case .number(let value):
            if value.rounded() == value {
                return String(Int(value))
            }
            return trimmedString(String(value))
        case .bool(let value):
            return value ? "true" : "false"
        case .array, .object, .null:
            return nil
        }
    }

    var runStateIntValue: Int? {
        switch self {
        case .number(let value):
            guard value.isFinite else { return nil }
            return max(0, Int(value))
        case .string(let value):
            return Int(value.trimmingCharacters(in: .whitespacesAndNewlines)).map { max(0, $0) }
        case .bool, .array, .object, .null:
            return nil
        }
    }
}
