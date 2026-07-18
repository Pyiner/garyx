import Foundation

/// Icon vocabulary for tool-call rows. Views map these to platform glyphs;
/// no view-level switch tables on raw tool names.
enum GaryxToolCallIcon: Equatable {
    case command
    case read
    case edit
    case search
    case web
    case generic
}

/// One row in the tool-call list sheet: a verb ("Ran", "Read", …) plus a
/// concise detail (a description or a file path), mirroring how the turn
/// presents tool activity in natural language.
struct GaryxToolCallListRow: Identifiable, Equatable {
    let id: String
    let icon: GaryxToolCallIcon
    let verb: String
    let detail: String?
    let metadata: String?
    let isRunning: Bool
    let isError: Bool
}

/// An image a tool read or produced, surfaced inline below the tool summary
/// row as a thumbnail.
struct GaryxToolCallImageRef: Identifiable, Equatable {
    let id: String
    let path: String

    var fileName: String {
        (path as NSString).lastPathComponent
    }
}

/// One rendered line of an edit diff.
struct GaryxToolCallDiffLine: Identifiable, Equatable {
    enum Kind: Equatable {
        case added
        case removed
        case context
    }

    let id: Int
    let kind: Kind
    let text: String
}

/// Section content vocabulary for the call detail page: a bare monospace
/// value (file paths), a code card (commands, raw payloads), a colored
/// diff (file edits), or an inline image preview (image files — never
/// their raw base64 payload).
enum GaryxToolCallDetailContent: Equatable {
    case plainMonospace(String)
    case codeCard(String)
    case diff([GaryxToolCallDiffLine])
    case imagePreview(String)
}

struct GaryxToolCallDetailSectionModel: Identifiable, Equatable {
    let label: String
    let content: GaryxToolCallDetailContent

    var id: String { label }
}

/// Detail page content for a single tool call, sectioned per tool category:
/// commands show Command/Output, file edits show File plus a diff, reads
/// show File plus the result.
struct GaryxToolCallDetail: Equatable {
    let title: String
    let isRunning: Bool
    let isError: Bool
    let sections: [GaryxToolCallDetailSectionModel]
}

private extension GaryxJSONValue {
    /// Raw string content for detail rendering. Unlike trimming accessors,
    /// this preserves whitespace because diff lines depend on it.
    var garyxDetailStringValue: String? {
        switch self {
        case .string(let value):
            return value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : value
        case .number(let value):
            return value.rounded() == value ? String(Int(value)) : String(value)
        case .bool(let value):
            return value ? "true" : "false"
        case .null, .array, .object:
            return nil
        }
    }
}

enum GaryxToolCallPresentation {
    static let imagePathExtensions: Set<String> = [
        "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "bmp", "tiff", "svg",
    ]

    static func listRows(from entries: [GaryxMobileToolTraceEntry]) -> [GaryxToolCallListRow] {
        entries.map { entry in
            GaryxToolCallListRow(
                id: entry.id,
                icon: icon(for: entry),
                verb: verb(for: entry),
                detail: detail(for: entry),
                metadata: entry.fieldProjection?.metadataText,
                isRunning: entry.status == .running,
                isError: entry.isError
            )
        }
    }

    /// Images referenced by read-style tools (Claude Code `Read`, Codex
    /// `view_image`) or produced at an image path by write-style tools.
    /// Detection is path-based so every provider's tool naming works the
    /// same way; duplicates keep their first appearance.
    static func imageRefs(from entries: [GaryxMobileToolTraceEntry]) -> [GaryxToolCallImageRef] {
        var seenPaths = Set<String>()
        var refs: [GaryxToolCallImageRef] = []
        for entry in entries {
            guard let path = entry.primaryPath?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !path.isEmpty,
                  isImagePath(path),
                  !seenPaths.contains(path) else {
                continue
            }
            seenPaths.insert(path)
            refs.append(GaryxToolCallImageRef(id: "\(entry.id):\(path)", path: path))
        }
        return refs
    }

    static func detail(for entry: GaryxMobileToolTraceEntry) -> GaryxToolCallDetail {
        if isProviderNeutralFallback(entry) {
            return GaryxToolCallDetail(
                title: entry.title,
                isRunning: entry.status == .running,
                isError: entry.isError,
                sections: []
            )
        }
        if let sections = projectedSections(for: entry) {
            return GaryxToolCallDetail(
                title: entry.title,
                isRunning: entry.status == .running,
                isError: entry.isError,
                sections: sections
            )
        }
        return GaryxToolCallDetail(
            title: entry.title,
            isRunning: entry.status == .running,
            isError: entry.isError,
            sections: []
        )
    }

    private static func projectedSections(
        for entry: GaryxMobileToolTraceEntry
    ) -> [GaryxToolCallDetailSectionModel]? {
        guard let projection = entry.fieldProjection else {
            return nil
        }
        var sections: [GaryxToolCallDetailSectionModel] = []
        if let summary = projection.summary, summary.format == .path {
            sections.append(projectedSection(summary))
        }
        if let call = projection.call {
            sections.append(projectedSection(call))
        }
        if !projection.diff.isEmpty {
            sections.append(.init(label: "Diff", content: .diff(projection.diff)))
        }
        if let result = projection.result {
            sections.append(projectedSection(result))
        } else if entry.status == .running, entry.isCommand {
            sections.append(.init(label: "Output", content: .codeCard("Running…")))
        }
        return sections
    }

    private static func projectedSection(
        _ field: GaryxResolvedToolField
    ) -> GaryxToolCallDetailSectionModel {
        let content: GaryxToolCallDetailContent
        switch field.format {
        case .path:
            content = .plainMonospace(field.text)
        case .image:
            content = isImagePath(field.text) ? .imagePreview(field.text) : .codeCard(field.text)
        case .text, .code, .json:
            content = .codeCard(field.text)
        }
        return .init(label: field.label, content: content)
    }

    private static func decodedInput(_ inputText: String?) -> [String: GaryxJSONValue]? {
        guard let inputText else { return nil }
        let trimmed = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{"),
              let data = trimmed.data(using: .utf8),
              let decoded = try? JSONDecoder().decode(GaryxJSONValue.self, from: data),
              case .object(let object) = decoded else {
            return nil
        }
        for key in ["input", "arguments", "params"] {
            if case .object(var nested)? = object[key] {
                for passthroughKey in ["label", "name", "title", "description"] where nested[passthroughKey] == nil {
                    nested[passthroughKey] = object[passthroughKey]
                }
                return nested
            }
        }
        return object
    }

    static func isImagePath(_ path: String) -> Bool {
        let ext = (path as NSString).pathExtension.lowercased()
        return imagePathExtensions.contains(ext)
    }

    private static func icon(for entry: GaryxMobileToolTraceEntry) -> GaryxToolCallIcon {
        if let kind = entry.fieldProjection?.kind {
            switch kind {
            case .command: return .command
            case .fileRead: return .read
            case .fileWrite, .fileEdit: return .edit
            case .search: return .search
            case .web: return .web
            case .agent, .task, .image, .system, .generic: return .generic
            }
        }
        if entry.isCommand { return .command }
        if entry.isFileRead { return .read }
        if entry.isFileEdit || entry.isFileWrite { return .edit }
        let name = entry.toolName.lowercased()
        if name.contains("grep") || name.contains("search") || name.contains("glob") || name.contains("find") {
            return name.contains("web") ? .web : .search
        }
        if name.contains("web") || name.contains("fetch") || name.contains("url") {
            return .web
        }
        return .generic
    }

    private static func verb(for entry: GaryxMobileToolTraceEntry) -> String {
        let running = entry.status == .running
        if entry.isCommand { return running ? "Running" : "Ran" }
        if entry.isFileRead { return running ? "Reading" : "Read" }
        if entry.isFileWrite { return running ? "Writing" : "Wrote" }
        if entry.isFileEdit { return running ? "Editing" : "Edited" }
        // Non-file tools (Agent, TaskCreate, ToolSearch, Skill, mcp__*, …) name the
        // tool directly, with no "Used"/"Using" prefix — the running state is shown
        // by the row's shimmer, not the verb.
        return entry.title
    }

    /// The concise per-call line: prefer the full file path for file-style
    /// tools (matching how reads are listed), then the model-provided
    /// description/summary. With no useful detail, render only the verb.
    private static func detail(for entry: GaryxMobileToolTraceEntry) -> String? {
        if isProviderNeutralFallback(entry) {
            return nil
        }
        if entry.fieldProjection?.summary?.format == .path,
           let badge = entry.primaryPathBadge?.trimmingCharacters(in: .whitespacesAndNewlines),
           !badge.isEmpty {
            return badge
        }
        if entry.isFileRead || entry.isFileWrite || entry.isFileEdit,
           let path = entry.primaryPath?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return path
        }
        if let inputPreview = inputPreviewDetail(for: entry) {
            return inputPreview
        }
        if let projectedSummary = entry.fieldProjection?.summary?.previewText
            .trimmingCharacters(in: .whitespacesAndNewlines),
           !projectedSummary.isEmpty {
            return GaryxMobileToolSummaryFormatter.singleLineTruncated(
                projectedSummary,
                limit: 112
            )
        }
        if entry.isCommand {
            return commandDetail(for: entry)
        }
        if let summary = entry.summaryText?.trimmingCharacters(in: .whitespacesAndNewlines),
           !summary.isEmpty {
            return summary
        }
        if let badge = entry.primaryPathBadge?.trimmingCharacters(in: .whitespacesAndNewlines),
           !badge.isEmpty {
            return badge
        }
        return nil
    }

    private static func isProviderNeutralFallback(_ entry: GaryxMobileToolTraceEntry) -> Bool {
        entry.fieldProjection == nil
            && entry.inputText == nil
            && entry.resultText == nil
            && entry.summaryText == nil
            && entry.primaryPath == nil
            && entry.primaryPathBadge == nil
    }

    private static func inputPreviewDetail(for entry: GaryxMobileToolTraceEntry) -> String? {
        guard let input = decodedInput(entry.inputText) else {
            return nil
        }
        for key in ["label", "name", "title", "description"] {
            if let value = input[key]?.garyxDetailStringValue {
                return GaryxMobileToolSummaryFormatter.singleLineTruncated(value, limit: 112)
            }
        }
        return nil
    }

    private static func commandDetail(for entry: GaryxMobileToolTraceEntry) -> String? {
        let input = decodedInput(entry.inputText)
        if let command = input?["command"]?.garyxDetailStringValue
            ?? input?["cmd"]?.garyxDetailStringValue {
            return GaryxMobileToolSummaryFormatter.shellSummary(command)
        }
        if input == nil,
           let inputText = entry.inputText?.trimmingCharacters(in: .whitespacesAndNewlines),
           !inputText.isEmpty {
            return GaryxMobileToolSummaryFormatter.shellSummary(inputText)
        }
        let summary = entry.summaryText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !summary.isEmpty {
            return summary
        }
        return nil
    }
}
