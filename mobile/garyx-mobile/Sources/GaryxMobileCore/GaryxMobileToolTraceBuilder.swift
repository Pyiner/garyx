import Foundation

extension GaryxMobileTranscriptMapper {
    static func mobileMessages(from transcript: [GaryxTranscriptMessage], live: Bool = false) -> [GaryxMobileMessage] {
        transcript.compactMap { item in
            if item.role == .tool || GaryxMobileTranscriptToolTraceClassifier.kind(for: item) != nil {
                return nil
            }
            let attachments = messageAttachments(fromTranscript: item)
            let displayText = transcriptMessageText(item, attachments: attachments)
            let trimmed = displayText.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty, attachments.isEmpty, item.role != .user, item.role != .assistant {
                return nil
            }
            return GaryxMobileMessage(
                id: item.id,
                role: mobileRole(for: item.role),
                text: displayText,
                attachments: attachments,
                timestamp: item.timestamp,
                isStreaming: false,
                clientIntentId: item.originId,
                localState: .remoteFinal,
                historyIndex: item.index
            )
        }
    }

    static func transcriptStructuredContent(_ item: GaryxTranscriptMessage) -> GaryxJSONValue? {
        if let messageContent = item.message?.garyxToolTraceDecodedIfNeeded.garyxToolTraceObjectValue?["content"] {
            return messageContent.garyxToolTraceDecodedIfNeeded
        }
        return item.content?.garyxToolTraceDecodedIfNeeded
    }

    static func transcriptMessageText(
        _ item: GaryxTranscriptMessage,
        attachments: [GaryxMobileMessageAttachment]
    ) -> String {
        if item.role == .user,
           !attachments.isEmpty,
           let content = transcriptStructuredContent(item) {
            return GaryxStructuredContentRenderer.text(from: content) ?? ""
        }
        return item.text
    }

    static func messageAttachments(fromTranscript item: GaryxTranscriptMessage) -> [GaryxMobileMessageAttachment] {
        guard let content = transcriptStructuredContent(item) else { return [] }
        return messageAttachments(fromStructuredContent: content)
    }

    private static func messageAttachments(fromStructuredContent content: GaryxJSONValue?) -> [GaryxMobileMessageAttachment] {
        GaryxStructuredContentRenderer.attachments(from: content).map { attachment in
            GaryxMobileMessageAttachment(
                id: attachment.id,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                path: attachment.path,
                dataUrl: attachment.dataUrl,
                remoteUrl: attachment.remoteUrl
            )
        }
    }

    private static func mobileRole(for role: GaryxTranscriptRole) -> GaryxMobileMessage.Role {
        switch role {
        case .assistant:
            .assistant
        case .user:
            .user
        case .tool, .toolUse, .toolResult:
            .tool
        case .system, .unknown:
            .system
        }
    }
}

struct GaryxPreparedThreadTranscriptUpdate: Equatable, Sendable {
    var activitySignature: String
    var runState: GaryxTranscriptRunState
    var remoteMessages: [GaryxMobileMessage]

    static func make(
        from transcript: GaryxThreadTranscript,
        live: Bool
    ) -> GaryxPreparedThreadTranscriptUpdate {
        let runState = GaryxTranscriptRunStateReducer.reduce(transcript.messages)
        return GaryxPreparedThreadTranscriptUpdate(
            activitySignature: GaryxThreadActivitySignature.make(from: transcript),
            runState: runState,
            remoteMessages: GaryxMobileTranscriptMapper.mobileMessages(from: transcript.messages, live: live)
        )
    }

    static func make(from transcript: GaryxThreadTranscript) -> GaryxPreparedThreadTranscriptUpdate {
        let runState = GaryxTranscriptRunStateReducer.reduce(transcript.messages)
        return GaryxPreparedThreadTranscriptUpdate(
            activitySignature: GaryxThreadActivitySignature.make(from: transcript),
            runState: runState,
            remoteMessages: GaryxMobileTranscriptMapper.mobileMessages(from: transcript.messages, live: runState.busy)
        )
    }
}

struct GaryxMessageListSignature: Equatable, Sendable {
    let count: Int
    let fingerprint: Int
    let sampled: Bool

    static func make(for messages: [GaryxMobileMessage]) -> GaryxMessageListSignature {
        var hasher = Hasher()
        var sampled = false
        for message in messages {
            hasher.combine(message.id)
            hasher.combine(roleSignature(message.role))
            sampled = combineTextSignature(message.text, into: &hasher) || sampled
            hasher.combine(message.timestamp)
            hasher.combine(message.isStreaming)
            hasher.combine(message.statusText)
            hasher.combine(message.clientIntentId)
            hasher.combine(message.pendingInputId)
            hasher.combine(message.attachments.count)
            for attachment in message.attachments {
                hasher.combine(attachment.id)
                hasher.combine(attachment.kind)
                hasher.combine(attachment.name)
                hasher.combine(attachment.mediaType)
                hasher.combine(attachment.path)
                sampled = combineTextSignature(attachment.dataUrl, into: &hasher) || sampled
                hasher.combine(attachment.remoteUrl)
            }
            if let group = message.toolTraceGroup {
                hasher.combine(group.live)
                hasher.combine(group.entries.count)
                for entry in group.entries {
                    hasher.combine(entry.id)
                    hasher.combine(entry.toolUseId)
                    hasher.combine(entry.parentToolUseId)
                    hasher.combine(entry.toolName)
                    hasher.combine(entry.title)
                    hasher.combine(entry.inputLabel)
                    hasher.combine(entry.resultLabel)
                    hasher.combine(entry.summaryText)
                    hasher.combine(entry.status.rawValue)
                    hasher.combine(entry.isError)
                    hasher.combine(entry.timestamp)
                    hasher.combine(entry.primaryPathBadge)
                    sampled = combineTextSignature(entry.inputText, into: &hasher) || sampled
                    sampled = combineTextSignature(entry.resultText, into: &hasher) || sampled
                }
            }
        }
        return GaryxMessageListSignature(count: messages.count, fingerprint: hasher.finalize(), sampled: sampled)
    }

    @discardableResult
    private static func combineTextSignature(_ value: String?, into hasher: inout Hasher) -> Bool {
        guard let value else {
            hasher.combine(-1)
            return false
        }
        return combineTextSignature(value, into: &hasher)
    }

    @discardableResult
    private static func combineTextSignature(_ value: String, into hasher: inout Hasher) -> Bool {
        hasher.combine(value.count)
        if value.count <= 1_024 {
            hasher.combine(value)
            return false
        }
        hasher.combine(value.prefix(256))
        let middleOffset = max(0, (value.count / 2) - 128)
        let middleStart = value.index(value.startIndex, offsetBy: middleOffset)
        let middleEnd = value.index(middleStart, offsetBy: min(256, value.distance(from: middleStart, to: value.endIndex)))
        hasher.combine(value[middleStart..<middleEnd])
        hasher.combine(value.suffix(256))
        return true
    }

    private static func roleSignature(_ role: GaryxMobileMessage.Role) -> String {
        switch role {
        case .user:
            "user"
        case .assistant:
            "assistant"
        case .system:
            "system"
        case .tool:
            "tool"
        }
    }
}

struct GaryxPreparedThreadMessages: Equatable, Sendable {
    var messages: [GaryxMobileMessage]
    var signature: GaryxMessageListSignature
    var activeAssistantMessageId: String?

    static func make(
        remoteMessages: [GaryxMobileMessage],
        localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int?,
        isThreadBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedThreadMessages {
        let merged = GaryxTranscriptMerge.mergedMessages(
            remoteMessages,
            withLocal: localMessages,
            preserveRemoteBeforeIndex: preserveRemoteBeforeIndex
        )
        return make(
            messages: merged,
            isThreadBusy: isThreadBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
    }

    static func make(
        messages: [GaryxMobileMessage],
        isThreadBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedThreadMessages {
        let reconciled = reconciledActiveAssistantMessages(
            messages,
            isThreadBusy: isThreadBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
        return GaryxPreparedThreadMessages(
            messages: reconciled.messages,
            signature: GaryxMessageListSignature.make(for: reconciled.messages),
            activeAssistantMessageId: reconciled.activeAssistantMessageId
        )
    }

    private static func reconciledActiveAssistantMessages(
        _ messages: [GaryxMobileMessage],
        isThreadBusy: Bool,
        activeAssistantMessageId: String?
    ) -> (messages: [GaryxMobileMessage], activeAssistantMessageId: String?) {
        guard isThreadBusy else {
            return (messages, nil)
        }
        var adjustedMessages = messages
        if let activeAssistantMessageId,
           let index = adjustedMessages.firstIndex(where: { $0.id == activeAssistantMessageId && $0.role == .assistant }) {
            adjustedMessages[index].isStreaming = true
            return (adjustedMessages, activeAssistantMessageId)
        }
        if let index = adjustedMessages.indices.last(where: {
            adjustedMessages[$0].role == .assistant && adjustedMessages[$0].isStreaming
        }) {
            adjustedMessages[index].isStreaming = true
            return (adjustedMessages, adjustedMessages[index].id)
        }
        return (adjustedMessages, nil)
    }
}

struct GaryxPreparedSelectedThreadTranscriptUpdate: Equatable, Sendable {
    var activitySignature: String
    var runState: GaryxTranscriptRunState
    var messages: GaryxPreparedThreadMessages
    var threadRunActive: Bool

    static func make(
        from transcript: GaryxThreadTranscript,
        localMessages: [GaryxMobileMessage],
        localRunTrackerBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedSelectedThreadTranscriptUpdate {
        make(
            transcriptMessages: transcript.messages,
            activitySignature: GaryxThreadActivitySignature.make(from: transcript),
            localMessages: localMessages,
            preserveRemoteBeforeIndex: transcript.pageInfo?.returnedStartIndex
                ?? transcript.messages.compactMap(\.index).min(),
            localRunTrackerBusy: localRunTrackerBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
    }

    static func make(
        from window: GaryxCachedTranscript,
        localMessages: [GaryxMobileMessage],
        localRunTrackerBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedSelectedThreadTranscriptUpdate {
        make(
            transcriptMessages: window.messages,
            activitySignature: GaryxThreadActivitySignature.make(messages: window.messages, pendingUserInputs: []),
            localMessages: localMessages,
            preserveRemoteBeforeIndex: window.firstIndex,
            localRunTrackerBusy: localRunTrackerBusy,
            activeAssistantMessageId: activeAssistantMessageId
        )
    }

    private static func make(
        transcriptMessages: [GaryxTranscriptMessage],
        activitySignature: String,
        localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int?,
        localRunTrackerBusy: Bool,
        activeAssistantMessageId: String?
    ) -> GaryxPreparedSelectedThreadTranscriptUpdate {
        let runState = GaryxTranscriptRunStateReducer.reduce(transcriptMessages)
        let threadRunActive = localRunTrackerBusy || runState.busy
        let remoteMessages = GaryxMobileTranscriptMapper.mobileMessages(
            from: transcriptMessages,
            live: threadRunActive
        )
        return GaryxPreparedSelectedThreadTranscriptUpdate(
            activitySignature: activitySignature,
            runState: runState,
            messages: GaryxPreparedThreadMessages.make(
                remoteMessages: remoteMessages,
                localMessages: localMessages,
                preserveRemoteBeforeIndex: preserveRemoteBeforeIndex,
                isThreadBusy: threadRunActive,
                activeAssistantMessageId: activeAssistantMessageId
            ),
            threadRunActive: threadRunActive
        )
    }
}

enum GaryxMobileToolTraceEventKind {
    case toolUse
    case toolResult
}

struct GaryxMobileToolTracePayload {
    var toolUseId: String?
    var parentToolUseId: String?
    var toolName: String?
    var contentText: String?
    var summaryText: String?
    var timestamp: String?
    var primaryPathBadge: String?
    var primaryPath: String?
    var source: String?
    var itemType: String?
    var isError: Bool

    static func fromTranscript(_ message: GaryxTranscriptMessage) -> GaryxMobileToolTracePayload {
        let eventKind = eventKind(fromTranscript: message)
        return from(
            value: message.message ?? message.content ?? GaryxJSONValue.garyxToolTraceDecoded(from: message.text),
            eventKind: eventKind,
            fallbackText: message.text,
            fallbackToolName: message.kind,
            fallbackTimestamp: message.timestamp,
            // The committed envelope carries the tool identity the nested content
            // omits, so committed rows match live rows and sub-agent children can
            // be filtered.
            fallbackToolUseId: message.toolUseId,
            fallbackParentToolUseId: message.garyxParentToolUseId
        )
    }

    private static func from(
        value: GaryxJSONValue?,
        eventKind: GaryxMobileToolTraceEventKind,
        fallbackText: String?,
        fallbackToolName: String?,
        fallbackTimestamp: String?,
        fallbackToolUseId: String? = nil,
        fallbackParentToolUseId: String? = nil
    ) -> GaryxMobileToolTracePayload {
        let decodedValue = value?.garyxToolTraceDecodedIfNeeded
        guard let object = decodedValue?.garyxToolTraceObjectValue else {
            return GaryxMobileToolTracePayload(
                toolUseId: fallbackToolUseId?.garyxToolTraceTrimmedNilIfEmpty,
                parentToolUseId: fallbackParentToolUseId?.garyxToolTraceTrimmedNilIfEmpty,
                toolName: fallbackToolName?.garyxToolTraceTrimmedNilIfEmpty,
                contentText: fallbackText?.garyxToolTraceTrimmedNilIfEmpty,
                summaryText: fallbackText.flatMap(GaryxMobileToolSummaryFormatter.safeSummary),
                timestamp: fallbackTimestamp,
                primaryPathBadge: nil,
                primaryPath: nil,
                source: nil,
                itemType: fallbackToolName?.garyxToolTraceTrimmedNilIfEmpty,
                isError: false
            )
        }

        let payloadValue = object.garyxToolTraceUnwrappedToolPayloadValue ?? decodedValue ?? .object(object)
        let payloadObject = payloadValue.garyxToolTraceObjectValue
        let nestedContent = payloadObject ?? object.garyxToolTraceObjectValue(forKeys: ["content", "message", "payload"])
        let metadata = object.garyxToolTraceObjectValue(forKeys: ["metadata"])
            ?? payloadObject?.garyxToolTraceObjectValue(forKeys: ["metadata"])
            ?? nestedContent?.garyxToolTraceObjectValue(forKeys: ["metadata"])
        let source = metadata?.garyxToolTraceStringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? object.garyxToolTraceStringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? payloadObject?.garyxToolTraceStringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
            ?? nestedContent?.garyxToolTraceStringValue(forKeys: ["source", "provider", "providerType", "provider_type"])
        let toolUseId = object.garyxToolTraceStringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? payloadObject?.garyxToolTraceStringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? nestedContent?.garyxToolTraceStringValue(forKeys: ["toolUseId", "tool_use_id", "id"])
            ?? fallbackToolUseId?.garyxToolTraceTrimmedNilIfEmpty
        let parentToolUseId = object.garyxToolTraceStringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? payloadObject?.garyxToolTraceStringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? nestedContent?.garyxToolTraceStringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? metadata?.garyxToolTraceStringValue(forKeys: ["parentToolUseId", "parent_tool_use_id"])
            ?? fallbackParentToolUseId?.garyxToolTraceTrimmedNilIfEmpty
        let toolName = object.garyxToolTraceStringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title"])
            ?? payloadObject?.garyxToolTraceStringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title", "type"])
            ?? nestedContent?.garyxToolTraceStringValue(forKeys: ["toolName", "tool_name", "name", "tool", "title"])
            ?? fallbackToolName?.garyxToolTraceTrimmedNilIfEmpty
        let itemType = object.garyxToolTraceStringValue(forKeys: ["type", "item_type", "itemType"])
            ?? payloadObject?.garyxToolTraceStringValue(forKeys: ["type", "item_type", "itemType"])
            ?? nestedContent?.garyxToolTraceStringValue(forKeys: ["type", "item_type", "itemType"])
            ?? metadata?.garyxToolTraceStringValue(forKeys: ["type", "item_type", "itemType"])
            ?? toolName
        let detailKeys = eventKind == .toolUse
            ? ["input", "arguments", "params", "content", "command", "path", "file_path", "text"]
            : ["result", "output", "content", "stdout", "stderr", "text", "message"]
        let content = payloadObject?.garyxToolTraceDetailText(forKeys: detailKeys)
            ?? object.garyxToolTraceDetailText(forKeys: detailKeys)
            ?? fallbackText?.garyxToolTraceTrimmedNilIfEmpty
        let summary = Self.summaryText(
            toolName: toolName,
            payload: payloadObject,
            payloadValue: payloadValue,
            eventKind: eventKind
        ) ?? fallbackText.flatMap(GaryxMobileToolSummaryFormatter.safeSummary)
        let timestamp = object.garyxToolTraceStringValue(forKeys: ["timestamp", "createdAt", "created_at"]) ?? fallbackTimestamp
        let primaryPath = Self.primaryPath(
            payload: payloadObject,
            nestedContent: nestedContent
        )
        let primaryPathBadge = primaryPath.map { GaryxMobileToolSummaryFormatter.pathTail($0) }
        let isError = object.garyxToolTraceBoolValue(forKeys: ["isError", "is_error", "error"])
            ?? payloadObject?.garyxToolTraceBoolValue(forKeys: ["isError", "is_error", "error"])
            ?? nestedContent?.garyxToolTraceBoolValue(forKeys: ["isError", "is_error", "error"])
            ?? false

        return GaryxMobileToolTracePayload(
            toolUseId: toolUseId,
            parentToolUseId: parentToolUseId,
            toolName: toolName,
            contentText: content,
            summaryText: summary,
            timestamp: timestamp,
            primaryPathBadge: primaryPathBadge,
            primaryPath: primaryPath,
            source: source,
            itemType: itemType,
            isError: isError
        )
    }

    private static func primaryPath(
        payload: [String: GaryxJSONValue]?,
        nestedContent: [String: GaryxJSONValue]?
    ) -> String? {
        let input = payload?.garyxToolTraceObjectValue(forKeys: ["input", "arguments", "params"])
            ?? nestedContent?.garyxToolTraceObjectValue(forKeys: ["input", "arguments", "params"])
            ?? payload
            ?? nestedContent
        return input?.garyxToolTraceStringValue(forKeys: ["file_path", "filePath", "path", "file"])
    }

    private static func summaryText(
        toolName: String?,
        payload: [String: GaryxJSONValue]?,
        payloadValue: GaryxJSONValue,
        eventKind: GaryxMobileToolTraceEventKind
    ) -> String? {
        let normalizedTool = toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        let input = payload?.garyxToolTraceObjectValue(forKeys: ["input", "arguments", "params"]) ?? payload
        let result = payload?.garyxToolTraceObjectValue(forKeys: ["result", "output"]) ?? payload

        if eventKind == .toolResult {
            let text = result?.garyxToolTraceStringValue(forKeys: ["summary", "message", "text", "stdout", "stderr"])
                ?? payload?.garyxToolTraceStringValue(forKeys: ["summary", "message", "text", "stdout", "stderr"])
            return text.flatMap(GaryxMobileToolSummaryFormatter.safeSummary)
        }

        switch normalizedTool {
        case "bash", "shell", "exec_command", "command", "commandexecution":
            return input?.garyxToolTraceStringValue(forKeys: ["description"])
                ?? input?.garyxToolTraceStringValue(forKeys: ["command", "cmd"])
                    .map { GaryxMobileToolSummaryFormatter.shellSummary($0) }
        case "read", "view", "open", "cat":
            return input?.garyxToolTraceStringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "read \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "write", "create":
            return input?.garyxToolTraceStringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "write \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "edit", "multiedit", "apply_patch":
            return input?.garyxToolTraceStringValue(forKeys: ["file_path", "filePath", "path", "file"])
                .map { "edit \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "grep", "search", "rg":
            let pattern = input?.garyxToolTraceStringValue(forKeys: ["pattern", "query"])
            let path = input?.garyxToolTraceStringValue(forKeys: ["path", "include", "glob"])
            if let pattern, let path {
                return "search \(pattern) in \(GaryxMobileToolSummaryFormatter.pathTail(path))"
            }
            return pattern.map { "search \($0)" }
        case "glob", "find":
            return input?.garyxToolTraceStringValue(forKeys: ["pattern", "path"])
                .map { "find \(GaryxMobileToolSummaryFormatter.pathTail($0))" }
        case "ls", "list":
            return input?.garyxToolTraceStringValue(forKeys: ["path", "directory"])
                .map { "list \(GaryxMobileToolSummaryFormatter.pathTail($0))" } ?? "list files"
        case "todowrite", "todo_write":
            if let todos = input?["todos"]?.garyxToolTraceArrayValue, !todos.isEmpty {
                return "\(todos.count) todo items"
            }
            return nil
        case "webfetch", "web_fetch":
            return input?.garyxToolTraceStringValue(forKeys: ["url"])
                .flatMap { URL(string: $0)?.host }
                .map { "fetch \($0)" }
        case "websearch", "web_search":
            return input?.garyxToolTraceStringValue(forKeys: ["query"]).map { "search web for \($0)" }
        default:
            if let path = input?.garyxToolTraceStringValue(forKeys: ["file_path", "filePath", "path", "file"]) {
                return GaryxMobileToolSummaryFormatter.pathTail(path)
            }
            if let command = input?.garyxToolTraceStringValue(forKeys: ["command", "cmd"]) {
                return GaryxMobileToolSummaryFormatter.shellSummary(command)
            }
            if case .string(let text) = payloadValue {
                return GaryxMobileToolSummaryFormatter.safeSummary(text)
            }
            return nil
        }
    }
}

extension GaryxMobileToolTraceEntry {
    static func title(for toolName: String) -> String {
        let trimmed = toolName.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalized = trimmed.lowercased()
        switch normalized {
        case "exec_command", "command":
            return "Command"
        case "write_stdin":
            return "Input"
        case "apply_patch":
            return "Edit"
        case "view_image":
            return "Image"
        case "read_mcp_resource":
            return "MCP resource"
        case "list_mcp_resources":
            return "MCP resources"
        default:
            guard !trimmed.isEmpty else { return "Tool" }
            if trimmed.contains("_") || trimmed.contains("-") {
                let words = trimmed
                    .replacingOccurrences(of: "-", with: "_")
                    .split(separator: "_")
                    .map { word -> String in
                        let value = String(word)
                        if value.lowercased() == "mcp" {
                            return "MCP"
                        }
                        if value == value.lowercased() {
                            return value.capitalized
                        }
                        return value
                    }
                return words.isEmpty ? "Tool" : words.joined(separator: " ")
            }
            if trimmed == trimmed.lowercased() {
                return trimmed.capitalized
            }
            return trimmed
        }
    }
}

extension GaryxMobileToolTracePayload {
    static func eventKind(fromTranscript message: GaryxTranscriptMessage) -> GaryxMobileToolTraceEventKind {
        switch GaryxMobileTranscriptToolTraceClassifier.kind(for: message) {
        case .toolResult:
            return .toolResult
        case .toolUse, .none:
            return .toolUse
        }
    }

    var normalizedToolName: String {
        toolName?.trimmingCharacters(in: .whitespacesAndNewlines).garyxToolTraceTrimmedNilIfEmpty ?? "tool"
    }
}

extension GaryxMobileToolTraceEventKind {
    var idSuffix: String {
        switch self {
        case .toolUse:
            "tool-use"
        case .toolResult:
            "tool-result"
        }
    }
}

private extension GaryxJSONValue {
    static func garyxToolTraceDecoded(from text: String) -> GaryxJSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return nil }
        return try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))
    }

    var garyxToolTraceObjectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var garyxToolTraceArrayValue: [GaryxJSONValue]? {
        if case .array(let value) = self {
            return value
        }
        return nil
    }

    var garyxToolTraceDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.garyxToolTraceDecoded(from: value) {
            return decoded
        }
        return self
    }

    var garyxToolTraceStringValue: String? {
        switch self {
        case .string(let value):
            return value.garyxToolTraceTrimmedNilIfEmpty
        case .number(let value):
            if value.rounded() == value {
                return String(Int(value))
            }
            return String(value).garyxToolTraceTrimmedNilIfEmpty
        case .bool(let value):
            return value ? "true" : "false"
        case .null:
            return nil
        case .array, .object:
            return garyxToolTracePrettyPrinted
        }
    }

    var garyxToolTraceBoolValue: Bool? {
        switch self {
        case .bool(let value):
            return value
        case .string(let value):
            let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["true", "yes", "1"].contains(normalized) {
                return true
            }
            if ["false", "no", "0"].contains(normalized) {
                return false
            }
            return nil
        default:
            return nil
        }
    }

    var garyxToolTracePrettyPrinted: String {
        if case .string(let value) = self {
            return value
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(self),
              let text = String(data: data, encoding: .utf8) else {
            return ""
        }
        return text
    }

    var garyxToolTraceIsMeaningful: Bool {
        switch self {
        case .null:
            false
        case .string(let value):
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        case .array(let values):
            !values.isEmpty
        case .object(let values):
            !values.isEmpty
        case .number, .bool:
            true
        }
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    var garyxToolTraceUnwrappedToolPayloadValue: GaryxJSONValue? {
        guard let content = self["content"]?.garyxToolTraceDecodedIfNeeded else { return nil }
        let hasEnvelopeMarkers = self["toolName"] != nil
            || self["tool_name"] != nil
            || self["toolUseId"] != nil
            || self["tool_use_id"] != nil
            || self["metadata"] != nil
            || self["role"] != nil
        return hasEnvelopeMarkers ? content : nil
    }

    func garyxToolTraceStringValue(forKeys keys: [String]) -> String? {
        for key in keys {
            if let value = self[key]?.garyxToolTraceStringValue?.garyxToolTraceTrimmedNilIfEmpty {
                return value
            }
        }
        return nil
    }

    func garyxToolTraceBoolValue(forKeys keys: [String]) -> Bool? {
        for key in keys {
            if let value = self[key]?.garyxToolTraceBoolValue {
                return value
            }
        }
        return nil
    }

    func garyxToolTraceObjectValue(forKeys keys: [String]) -> [String: GaryxJSONValue]? {
        for key in keys {
            if let value = self[key]?.garyxToolTraceObjectValue {
                return value
            }
        }
        return nil
    }

    func garyxToolTraceDetailText(forKeys keys: [String]) -> String? {
        for key in keys {
            guard let value = self[key], value.garyxToolTraceIsMeaningful else { continue }
            if key == "message", value.garyxToolTraceObjectValue != nil {
                continue
            }
            if let text = value.garyxToolTraceStringValue?.garyxToolTraceTrimmedNilIfEmpty {
                return text
            }
        }
        return nil
    }
}

private extension String {
    var garyxToolTraceTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
