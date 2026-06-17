import Foundation

extension GaryxMobileTranscriptMapper {
    static func mobileMessages(from transcript: [GaryxTranscriptMessage], live: Bool = false) -> [GaryxMobileMessage] {
        var rendered: [GaryxMobileMessage] = []
        var pendingToolGroup: GaryxMobileToolTraceGroup?
        var pendingToolGroupHistoryIndex: Int?

        func flushToolGroup() {
            guard let group = pendingToolGroup, !group.entries.isEmpty else {
                pendingToolGroup = nil
                pendingToolGroupHistoryIndex = nil
                return
            }
            let firstEntry = group.entries[0]
            let groupIsLive = live && group.entries.contains { $0.status == .running }
            rendered.append(
                GaryxMobileMessage(
                    id: "tool-group:\(firstEntry.id)",
                    role: .tool,
                    text: group.summary,
                    timestamp: firstEntry.timestamp,
                    isStreaming: groupIsLive,
                    toolTraceGroup: GaryxMobileToolTraceGroup(
                        entries: group.entries,
                        live: groupIsLive
                    ),
                    localState: groupIsLive ? .remotePartial : .remoteFinal,
                    // Carry the first grouped row's transcript index so the
                    // merge's older-page preservation keeps tool groups like
                    // it keeps text rows instead of silently dropping them.
                    historyIndex: pendingToolGroupHistoryIndex
                )
            )
            pendingToolGroup = nil
            pendingToolGroupHistoryIndex = nil
        }

        for item in transcript {
            let toolTraceKind = GaryxMobileTranscriptToolTraceClassifier.kind(for: item)
            if toolTraceKind != nil {
                guard let entry = GaryxMobileToolTraceEntry(transcript: item) else {
                    continue
                }
                if toolTraceKind == .toolResult {
                    // Absorb into the open group, else into the earlier group the
                    // matching tool_use was flushed to when an intervening text row
                    // split them (a sub-agent runs while the parent narrates) - so a
                    // straddling result is never rendered as a stray "Used 1 tool".
                    if var group = pendingToolGroup, absorbToolResult(entry, into: &group) {
                        pendingToolGroup = group
                        continue
                    }
                    if absorbResultIntoFlushedToolGroup(entry, in: &rendered) {
                        continue
                    }
                    continue
                }
                var group = pendingToolGroup ?? GaryxMobileToolTraceGroup(entries: [], live: false)
                if pendingToolGroupHistoryIndex == nil {
                    pendingToolGroupHistoryIndex = item.index
                }
                group.entries.append(entry)
                pendingToolGroup = group
                continue
            }

            flushToolGroup()

            let attachments = messageAttachments(fromTranscript: item)
            let displayText = transcriptMessageText(item, attachments: attachments)
            let trimmed = displayText.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty, attachments.isEmpty, item.role != .user, item.role != .assistant {
                continue
            }
            rendered.append(
                GaryxMobileMessage(
                    id: item.id,
                    role: mobileRole(for: item.role),
                    text: displayText,
                    attachments: attachments,
                    timestamp: item.timestamp,
                    isStreaming: false,
                    localState: .remoteFinal,
                    historyIndex: item.index
                )
            )
        }

        flushToolGroup()
        return rendered
    }

    /// Absorb a committed `tool_result` entry into the tool-use entry it belongs
    /// to. An identified result matches by `toolUseId`; id-less provider results
    /// can only complete an open running entry in the current pending group.
    private static func absorbToolResult(
        _ result: GaryxMobileToolTraceEntry,
        into group: inout GaryxMobileToolTraceGroup,
        allowIdlessFallback: Bool = true
    ) -> Bool {
        if let resultId = result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !resultId.isEmpty {
            if let index = group.entries.lastIndex(where: { entry in
                guard let entryId = entry.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
                      !entryId.isEmpty else { return false }
                return entryId == resultId
            }) {
                group.entries[index].absorb(result: result)
                return true
            }
            return false
        }

        guard allowIdlessFallback else { return false }
        if let index = group.entries.lastIndex(where: { canAbsorbToolResultFallback(result, into: $0) }) {
            group.entries[index].absorb(result: result)
            return true
        }
        return false
    }

    private static func canAbsorbToolResultFallback(
        _ result: GaryxMobileToolTraceEntry,
        into candidate: GaryxMobileToolTraceEntry
    ) -> Bool {
        guard candidate.status == .running, candidate.resultText == nil else {
            return false
        }
        let resultId = result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let candidateId = candidate.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !resultId.isEmpty, !candidateId.isEmpty, resultId != candidateId {
            return false
        }
        let resultTool = result.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let candidateTool = candidate.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !resultTool.isEmpty, resultTool == candidateTool {
            return true
        }
        if candidateTool == "tool" || resultTool == "tool" {
            return true
        }
        if result.title.caseInsensitiveCompare(candidate.title) == .orderedSame {
            return true
        }
        if let resultSummary = result.summaryText,
           let candidateSummary = candidate.summaryText,
           resultSummary == candidateSummary {
            return true
        }
        return false
    }

    /// Absorb a committed `tool_result` into the most recent already-flushed tool
    /// group in the current turn. The stable-id-only match avoids attaching a
    /// stray id-less result across text or group boundaries.
    private static func absorbResultIntoFlushedToolGroup(
        _ entry: GaryxMobileToolTraceEntry,
        in messages: inout [GaryxMobileMessage]
    ) -> Bool {
        for index in messages.indices.reversed() {
            if messages[index].role == .user { break }
            guard messages[index].role == .tool,
                  var group = messages[index].toolTraceGroup else { continue }
            if absorbToolResult(entry, into: &group, allowIdlessFallback: false) {
                messages[index].toolTraceGroup = group
                messages[index].text = group.summary
                return true
            }
        }
        return false
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
        case .toolUse, .toolResult:
            .tool
        case .system, .unknown:
            .system
        }
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
    init?(transcript message: GaryxTranscriptMessage) {
        if GaryxMobileTranscriptToolTraceClassifier.isReasoningTrace(message) {
            return nil
        }
        let eventKind = GaryxMobileToolTracePayload.eventKind(fromTranscript: message)
        let payload = GaryxMobileToolTracePayload.fromTranscript(message)
        guard payload.shouldRender else {
            return nil
        }
        self.init(
            id: "\(message.id):\(eventKind.idSuffix)",
            toolUseId: payload.toolUseId,
            parentToolUseId: payload.parentToolUseId,
            toolName: payload.normalizedToolName,
            title: GaryxMobileToolTraceEntry.title(for: payload.normalizedToolName),
            inputText: eventKind == .toolUse ? payload.contentText : nil,
            resultText: eventKind == .toolResult ? payload.contentText : nil,
            summaryText: payload.summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: eventKind == .toolResult ? (payload.isError ? .failed : .completed) : .running,
            isError: payload.isError,
            timestamp: payload.timestamp,
            primaryPathBadge: payload.primaryPathBadge,
            primaryPath: payload.primaryPath
        )
    }

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
    var shouldRender: Bool {
        let normalizedItemType = itemType?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let normalizedToolName = toolName?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if Self.shouldHideDisplayIdentifier(normalizedItemType)
            || Self.shouldHideDisplayIdentifier(normalizedToolName) {
            return false
        }
        // A sub-agent's nested tool call carries a parent tool-use id pointing to
        // ANOTHER call (the Agent that spawned it). It is not rendered on its own -
        // the parent `Agent` row represents that work - which also removes the
        // sub-agent calls that straddle the parent's narration and otherwise
        // duplicated as a stray "Used 1 tool". A normal top-level tool result may
        // carry a parent id equal to its OWN tool-use id (the call it answers);
        // that is kept so it can still absorb into its tool row.
        let normalizedParent = parentToolUseId?.trimmingCharacters(in: .whitespacesAndNewlines)
        if let parent = normalizedParent, !parent.isEmpty,
           parent != toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines) {
            return false
        }
        return true
    }

    private static func shouldHideDisplayIdentifier(_ value: String?) -> Bool {
        guard let value, !value.isEmpty else { return false }
        return [
            "contextcompaction",
            "enteredreviewmode",
            "exitedreviewmode",
            "filechange",
            "hookprompt",
            "plan",
            "reasoning",
        ].contains(value)
    }

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
