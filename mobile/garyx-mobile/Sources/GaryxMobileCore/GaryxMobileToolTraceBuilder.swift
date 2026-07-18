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
                    hasher.combine(entry.fieldProjection?.kind.rawValue)
                    hasher.combine(entry.fieldProjection?.call?.format.rawValue)
                    hasher.combine(entry.fieldProjection?.result?.format.rawValue)
                    hasher.combine(entry.fieldProjection?.status)
                    hasher.combine(entry.fieldProjection?.exitCode)
                    hasher.combine(entry.fieldProjection?.durationMs)
                    let diff = entry.fieldProjection?.diff ?? []
                    hasher.combine(diff.count)
                    for line in diff {
                        switch line.kind {
                        case .added:
                            hasher.combine("added")
                        case .removed:
                            hasher.combine("removed")
                        case .context:
                            hasher.combine("context")
                        }
                        sampled = combineTextSignature(line.text, into: &hasher) || sampled
                    }
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

    var garyxToolTraceDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.garyxToolTraceDecoded(from: value) {
            return decoded
        }
        return self
    }
}
