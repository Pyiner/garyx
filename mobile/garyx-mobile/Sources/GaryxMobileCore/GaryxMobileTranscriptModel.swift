import Foundation

struct GaryxMobileMessage: Identifiable, Equatable {
    enum Role: Equatable {
        case user
        case assistant
        case system
        case tool
    }

    let id: String
    var role: Role
    var text: String
    var attachments: [GaryxMobileMessageAttachment] = []
    var timestamp: String?
    var isStreaming: Bool
    var statusText: String? = nil
    var clientIntentId: String? = nil
    var pendingInputId: String? = nil
    var toolTraceGroup: GaryxMobileToolTraceGroup? = nil
}

struct GaryxMobileMessageAttachment: Identifiable, Equatable {
    var id: String
    var kind: String
    var name: String
    var mediaType: String
    var path: String?
    var dataUrl: String?
    var remoteUrl: String?

    var isImage: Bool {
        kind.caseInsensitiveCompare("image") == .orderedSame || mediaType.hasPrefix("image/")
    }

    var contentDescriptor: GaryxContentAttachmentDescriptor {
        GaryxContentAttachmentDescriptor(
            id: id,
            kind: kind,
            name: name,
            mediaType: mediaType,
            path: path,
            dataUrl: dataUrl,
            remoteUrl: remoteUrl
        )
    }
}

enum GaryxMobileTranscriptMapper {
    static func appendPendingUserInputs(
        to messages: [GaryxMobileMessage],
        from transcript: GaryxThreadTranscript
    ) -> [GaryxMobileMessage] {
        var rendered = messages
        var existingPendingIds = Set(
            rendered
                .compactMap { $0.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )
        for input in transcript.pendingUserInputs {
            let pendingId = input.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard input.active,
                  !pendingId.isEmpty,
                  (input.status ?? "awaiting_ack").lowercased() != "abandoned",
                  !existingPendingIds.contains(pendingId) else {
                continue
            }
            existingPendingIds.insert(pendingId)
            let attachments = messageAttachments(fromStructuredContent: input.content)
            rendered.append(
                GaryxMobileMessage(
                    id: "pending-user:\(pendingId)",
                    role: .user,
                    text: pendingUserInputText(input, attachments: attachments),
                    attachments: attachments,
                    timestamp: input.timestamp,
                    isStreaming: false,
                    pendingInputId: pendingId
                )
            )
        }
        return rendered
    }

    private static func pendingUserInputText(
        _ input: GaryxPendingUserInput,
        attachments: [GaryxMobileMessageAttachment]
    ) -> String {
        let trimmed = input.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            return input.text
        }
        if !attachments.isEmpty {
            return input.content.flatMap { GaryxStructuredContentRenderer.text(from: $0) } ?? ""
        }
        if let contentSummary = input.content.flatMap({ GaryxStructuredContentRenderer.summaryText(from: $0) }),
           !contentSummary.isEmpty {
            return contentSummary
        }
        return "User message"
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
}

enum GaryxMobileToolTraceStatus: String, Equatable {
    case running
    case completed
    case failed

    var label: String {
        switch self {
        case .running:
            "running"
        case .completed:
            "done"
        case .failed:
            "error"
        }
    }
}

struct GaryxMobileToolTraceGroup: Equatable {
    var entries: [GaryxMobileToolTraceEntry]
    var live: Bool = false

    var isActive: Bool {
        live && entries.contains { $0.status == .running }
    }

    var defaultExpanded: Bool {
        false
    }

    var summary: String {
        guard !entries.isEmpty else { return "Tool activity" }
        let commandCount = entries.filter(\.isCommand).count
        let editEntries = entries.filter(\.isFileEdit)
        let fileCount = Set(editEntries.compactMap(\.primaryPathBadge)).count
        var parts: [String] = []
        if fileCount > 0 {
            parts.append("Edited \(fileCount) file\(fileCount == 1 ? "" : "s")")
        }
        if commandCount > 0 {
            parts.append("Ran \(commandCount) command\(commandCount == 1 ? "" : "s")")
        }
        let otherCount = entries.count - commandCount - editEntries.count
        if otherCount > 0 || parts.isEmpty {
            parts.append("Used \(max(otherCount, entries.count)) tool\(max(otherCount, entries.count) == 1 ? "" : "s")")
        }
        return parts.joined(separator: ", ")
    }
}

struct GaryxMobileToolTraceEntry: Identifiable, Equatable {
    var id: String
    var toolUseId: String?
    var parentToolUseId: String?
    var toolName: String
    var title: String
    var inputText: String?
    var resultText: String?
    var summaryText: String?
    var inputLabel: String
    var resultLabel: String
    var status: GaryxMobileToolTraceStatus
    var isError: Bool
    var timestamp: String?
    var primaryPathBadge: String?

    var isCommand: Bool {
        let normalized = toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "exec_command"
            || normalized == "command"
            || normalized == "bashtool"
            || normalized == "commandexecution"
            || normalized.contains("command")
    }

    var isFileEdit: Bool {
        let normalized = toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "write"
            || normalized == "edit"
            || normalized == "multiedit"
            || normalized == "apply_patch"
            || normalized.contains("edit")
            || normalized.contains("patch")
    }

    var groupSummary: String {
        if status == .running {
            return isCommand ? "Running command" : "Using \(title)"
        }
        if status == .failed {
            return "\(title) failed"
        }
        return isCommand ? "Ran command" : "Used \(title)"
    }

    var previewText: String? {
        summaryText.map { Self.singleLineTruncated($0, limit: 120) }
    }

    mutating func absorb(result: GaryxMobileToolTraceEntry) {
        if toolUseId == nil {
            toolUseId = result.toolUseId
        }
        if parentToolUseId == nil {
            parentToolUseId = result.parentToolUseId
        }
        if toolName == "tool", result.toolName != "tool" {
            toolName = result.toolName
            title = result.title
        }
        resultText = result.resultText ?? result.inputText ?? resultText
        summaryText = result.summaryText ?? summaryText
        resultLabel = result.resultLabel
        isError = result.isError
        status = result.isError ? .failed : .completed
        timestamp = result.timestamp ?? timestamp
        primaryPathBadge = primaryPathBadge ?? result.primaryPathBadge
    }

    private static func singleLineTruncated(_ value: String, limit: Int) -> String {
        let normalized = value
            .replacingOccurrences(of: "\r", with: "\n")
            .split(whereSeparator: \.isNewline)
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty } ?? value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard normalized.count > limit else { return normalized }
        let end = normalized.index(normalized.startIndex, offsetBy: max(0, limit - 1))
        return "\(normalized[..<end])..."
    }
}

struct GaryxMobileComposerAttachment: Identifiable, Equatable {
    var id: String
    var kind: String
    var name: String
    var mediaType: String
    var path: String
    var previewDataUrl: String?

    var promptAttachment: GaryxPromptAttachment {
        GaryxPromptAttachment(kind: kind, path: path, name: name, mediaType: mediaType)
    }
}

struct GaryxMobileSelectedImage: Equatable, Sendable {
    var name: String
    var mediaType: String
    var data: Data
}
