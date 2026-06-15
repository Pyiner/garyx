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
        let input = decodedInput(entry.inputText)
        var sections: [GaryxToolCallDetailSectionModel] = []

        if entry.isCommand {
            // Surface the bare command, not the raw call payload.
            let command = input?["command"]?.garyxDetailStringValue
                ?? input?["cmd"]?.garyxDetailStringValue
                ?? entry.inputText
            if let command {
                sections.append(.init(label: "Command", content: .codeCard(command)))
            }
            sections.append(outputSection(for: entry, label: "Output"))
        } else if entry.isFileEdit {
            let path = filePath(for: entry, input: input)
            if let path {
                sections.append(.init(label: "File", content: .plainMonospace(path)))
            }
            if let path, isImagePath(path) {
                sections.append(.init(label: "Content", content: .imagePreview(path)))
            } else if let diff = diffLines(input: input, inputText: entry.inputText) {
                sections.append(.init(label: "Output", content: .diff(diff)))
            } else {
                sections.append(outputSection(for: entry, label: "Output"))
            }
        } else if entry.isFileRead {
            let path = filePath(for: entry, input: input)
            if let path {
                sections.append(.init(label: "File", content: .plainMonospace(path)))
            }
            if let path, isImagePath(path) {
                // The raw result of reading an image is base64 noise; show
                // the image itself, like the reference design's Content
                // section.
                sections.append(.init(label: "Content", content: .imagePreview(path)))
            } else {
                sections.append(outputSection(for: entry, label: entry.resultLabel))
            }
        } else {
            if let inputText = entry.inputText {
                sections.append(.init(label: entry.inputLabel, content: .codeCard(inputText)))
            }
            sections.append(outputSection(for: entry, label: entry.resultLabel))
        }

        return GaryxToolCallDetail(
            title: entry.title,
            isRunning: entry.status == .running,
            isError: entry.isError,
            sections: sections
        )
    }

    private static func outputSection(
        for entry: GaryxMobileToolTraceEntry,
        label: String
    ) -> GaryxToolCallDetailSectionModel {
        let text = entry.resultText
            ?? (entry.status == .running ? "Running…" : "No output")
        return .init(label: label, content: .codeCard(text))
    }

    private static func filePath(
        for entry: GaryxMobileToolTraceEntry,
        input: [String: GaryxJSONValue]?
    ) -> String? {
        if let path = entry.primaryPath?.trimmingCharacters(in: .whitespacesAndNewlines), !path.isEmpty {
            return path
        }
        for key in ["file_path", "filePath", "path", "file"] {
            if let path = input?[key]?.garyxDetailStringValue {
                return path
            }
        }
        return entry.primaryPathBadge
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

    /// Build diff lines for edit-style calls: `old_string`/`new_string`
    /// pairs (Claude Code Edit/MultiEdit) become removed-then-added blocks,
    /// `content` (Write) renders as all-added, and patch-style input text
    /// (Codex apply_patch) is split by its +/- line prefixes.
    static func diffLines(
        input: [String: GaryxJSONValue]?,
        inputText: String?
    ) -> [GaryxToolCallDiffLine]? {
        var pairs: [(old: String?, new: String?)] = []
        if let input {
            if input["old_string"] != nil || input["new_string"] != nil {
                pairs.append((
                    input["old_string"]?.garyxDetailStringValue,
                    input["new_string"]?.garyxDetailStringValue
                ))
            } else if case .array(let edits)? = input["edits"] {
                for edit in edits {
                    guard case .object(let object) = edit else { continue }
                    pairs.append((
                        object["old_string"]?.garyxDetailStringValue,
                        object["new_string"]?.garyxDetailStringValue
                    ))
                }
            } else if let content = input["content"]?.garyxDetailStringValue {
                pairs.append((nil, content))
            }
        }

        var lines: [GaryxToolCallDiffLine] = []
        func append(_ kind: GaryxToolCallDiffLine.Kind, _ text: String) {
            lines.append(GaryxToolCallDiffLine(id: lines.count, kind: kind, text: text))
        }

        if !pairs.isEmpty {
            for pair in pairs {
                for line in (pair.old ?? "").split(separator: "\n", omittingEmptySubsequences: false) where pair.old != nil {
                    append(.removed, String(line))
                }
                for line in (pair.new ?? "").split(separator: "\n", omittingEmptySubsequences: false) where pair.new != nil {
                    append(.added, String(line))
                }
            }
            return lines.isEmpty ? nil : lines
        }

        // Patch-style fallback: only treat the text as a diff when it has
        // real +/- lines, so ordinary payloads keep the code-card rendering.
        guard let inputText else { return nil }
        let rawLines = inputText.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        var markerCount = 0
        for line in rawLines {
            if line.hasPrefix("+"), !line.hasPrefix("+++") {
                markerCount += 1
                continue
            }
            if line.hasPrefix("-"), !line.hasPrefix("---") {
                markerCount += 1
            }
        }
        guard markerCount >= 2 else { return nil }
        for line in rawLines {
            if line.hasPrefix("+"), !line.hasPrefix("+++") {
                append(.added, String(line.dropFirst()))
            } else if line.hasPrefix("-"), !line.hasPrefix("---") {
                append(.removed, String(line.dropFirst()))
            } else {
                append(.context, line)
            }
        }
        return lines.isEmpty ? nil : lines
    }

    static func isImagePath(_ path: String) -> Bool {
        let ext = (path as NSString).pathExtension.lowercased()
        return imagePathExtensions.contains(ext)
    }

    private static func icon(for entry: GaryxMobileToolTraceEntry) -> GaryxToolCallIcon {
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
        return running ? "Using \(entry.title)" : "Used \(entry.title)"
    }

    /// The concise per-call line: prefer the full file path for file-style
    /// tools (matching how reads are listed), then the model-provided
    /// description/summary, then the short title.
    private static func detail(for entry: GaryxMobileToolTraceEntry) -> String? {
        if entry.isFileRead || entry.isFileWrite || entry.isFileEdit,
           let path = entry.primaryPath?.trimmingCharacters(in: .whitespacesAndNewlines),
           !path.isEmpty {
            return path
        }
        if let inputPreview = inputPreviewDetail(for: entry) {
            return inputPreview
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
        if entry.isCommand { return nil }
        return entry.title
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
