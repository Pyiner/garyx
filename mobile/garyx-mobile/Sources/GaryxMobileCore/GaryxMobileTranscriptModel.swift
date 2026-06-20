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
    /// Birth provenance per the conversation state contract
    /// (docs/agents/conversation-state.md): `optimistic` for local sends,
    /// `remote_partial` for streamed/pending content, `remote_final` for
    /// canonical history rows. Merge and presentation logic branches on this
    /// instead of id-prefix conventions. Failure is carried by `statusText`
    /// as an overlay on the provenance. `nil` only for synthetic fixtures.
    var localState: GaryxTranscriptEntryState? = nil
    /// Canonical transcript index for committed rows; replaces parsing
    /// `history:N` ids and remains stable when user ids are `origin:*`.
    var historyIndex: Int? = nil
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

enum GaryxMobileTranscriptMapper {}

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

    /// Natural-language activity summary in the "Ran 3 commands, read 2
    /// files" style: commands, then reads, then edits, then anything else.
    /// Only the first part is capitalized.
    var summary: String {
        guard !entries.isEmpty else { return "Tool activity" }
        let commandEntries = entries.filter(\.isCommand)
        let readEntries = entries.filter { $0.isFileRead && !$0.isCommand }
        let editEntries = entries.filter { $0.isFileEdit && !$0.isCommand && !$0.isFileRead }
        let editedFileCount = Set(editEntries.compactMap(\.primaryPathBadge)).count
        var parts: [String] = []
        if !commandEntries.isEmpty {
            parts.append("ran \(commandEntries.count) command\(commandEntries.count == 1 ? "" : "s")")
        }
        if !readEntries.isEmpty {
            let readFileCount = max(Set(readEntries.compactMap { $0.primaryPath ?? $0.primaryPathBadge }).count, 1)
            parts.append("read \(readFileCount) file\(readFileCount == 1 ? "" : "s")")
        }
        if editedFileCount > 0 {
            parts.append("edited \(editedFileCount) file\(editedFileCount == 1 ? "" : "s")")
        }
        // Non-file tools (Agent, TaskCreate, ToolSearch, Skill, mcp__*, ...) don't
        // fall into command/read/edit. Name a small distinct set by title, but
        // fall back to a count before the collapsed row becomes too long.
        let otherEntries = entries.filter { !$0.isCommand && !$0.isFileRead && !$0.isFileEdit }
        if !otherEntries.isEmpty {
            var seen = Set<String>()
            let names = otherEntries.compactMap { entry -> String? in
                let title = entry.title.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !title.isEmpty, seen.insert(title).inserted else { return nil }
                return title
            }
            if names.isEmpty || names.count > 3 {
                parts.append("used \(otherEntries.count) tool\(otherEntries.count == 1 ? "" : "s")")
            } else {
                parts.append("used \(names.joined(separator: ", "))")
            }
        } else if parts.isEmpty {
            parts.append("used \(entries.count) tool\(entries.count == 1 ? "" : "s")")
        }
        let joined = parts.joined(separator: ", ")
        return joined.prefix(1).uppercased() + joined.dropFirst()
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
    /// Full file path from the call input, when one exists. The badge above
    /// keeps only the tail for compact rows; thumbnails and per-call list
    /// rows need the whole path.
    var primaryPath: String? = nil

    var isCommand: Bool {
        let normalized = toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "exec_command"
            || normalized == "command"
            || normalized == "bashtool"
            || normalized == "commandexecution"
            || normalized == "bash"
            || normalized == "shell"
            || normalized.contains("command")
    }

    var isFileRead: Bool {
        let normalized = toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "read"
            || normalized == "view"
            || normalized == "open"
            || normalized == "cat"
            || normalized == "view_image"
            || normalized == "imageview"
            || normalized == "notebookread"
    }

    var isFileWrite: Bool {
        let normalized = toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return normalized == "write" || normalized == "create"
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

    var previewText: String? {
        summaryText.map { Self.singleLineTruncated($0, limit: 120) }
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

enum GaryxMobileTranscriptToolTraceKind: Equatable {
    case toolUse
    case toolResult
}

enum GaryxMobileTranscriptToolTraceClassifier {
    static func kind(for message: GaryxTranscriptMessage) -> GaryxMobileTranscriptToolTraceKind? {
        switch message.role {
        case .toolUse:
            return .toolUse
        case .toolResult:
            return .toolResult
        default:
            break
        }

        guard message.toolRelated else {
            return nil
        }

        let kind = message.kind?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        if kind.contains("result") {
            return .toolResult
        }
        if kind.contains("use") {
            return .toolUse
        }

        let object = message.garyxToolTraceObject
        if object?.garyxBoolValue(forKeys: ["tool_use_result", "toolUseResult"]) == true {
            return .toolResult
        }
        if object?.garyxContainsMeaningfulValue(forKeys: ["result", "output", "stdout", "stderr"]) == true {
            return .toolResult
        }
        return .toolUse
    }

}

private extension GaryxTranscriptMessage {
    var garyxToolTraceObject: [String: GaryxJSONValue]? {
        let value = message ?? content ?? GaryxJSONValue.garyxDecoded(from: text)
        return value?.garyxDecodedIfNeeded.garyxObjectValue
    }
}

private extension GaryxJSONValue {
    static func garyxDecoded(from text: String) -> GaryxJSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return nil }
        return try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))
    }

    var garyxDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.garyxDecoded(from: value) {
            return decoded
        }
        return self
    }

    var garyxObjectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var garyxBoolValue: Bool? {
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

    var garyxIsMeaningful: Bool {
        switch self {
        case .null:
            return false
        case .string(let value):
            return !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        case .array(let values):
            return !values.isEmpty
        case .object(let values):
            return !values.isEmpty
        case .number, .bool:
            return true
        }
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    func garyxBoolValue(forKeys keys: [String]) -> Bool? {
        for key in keys {
            if let value = self[key]?.garyxBoolValue {
                return value
            }
        }
        return nil
    }

    func garyxContainsMeaningfulValue(forKeys keys: [String]) -> Bool {
        keys.contains { key in
            self[key]?.garyxIsMeaningful == true
        }
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
