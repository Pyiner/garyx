import Foundation

struct GaryxResolvedToolField: Equatable {
    var text: String
    var label: String
    var format: GaryxRenderToolFieldFormat

    /// A bounded value for collapsed rows. The full selected field remains in
    /// `text` and is only handed to the detail presentation.
    var previewText: String {
        String(text.prefix(4_096))
    }
}

struct GaryxResolvedToolFieldProjection: Equatable {
    var kind: GaryxRenderToolKind
    var toolName: String?
    var call: GaryxResolvedToolField?
    var result: GaryxResolvedToolField?
    var status: String?
    var exitCode: Int?
    var durationMs: Int?

    var metadataText: String? {
        var parts: [String] = []
        if let exitCode {
            parts.append("exit \(exitCode)")
        }
        if let durationMs {
            if durationMs >= 1_000 {
                parts.append(String(format: "%.1f s", Double(durationMs) / 1_000))
            } else {
                parts.append("\(durationMs) ms")
            }
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    var isError: Bool {
        if let exitCode, exitCode != 0 {
            return true
        }
        let normalized = status?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return ["failed", "declined", "errored", "error", "canceled", "cancelled"].contains(normalized)
    }

    var title: String {
        switch kind {
        case .command: "Command"
        case .fileRead: "Read"
        case .fileWrite: "Write"
        case .fileEdit: "Edit"
        case .search: "Search"
        case .web: "Web"
        case .agent: "Agent"
        case .task: "Task"
        case .image: "Image"
        case .system: "Activity"
        case .generic: GaryxMobileToolTraceEntry.title(for: toolName ?? "tool")
        }
    }
}

enum GaryxToolFieldProjectionResolver {
    static func resolve(
        _ projection: GaryxRenderToolFieldProjection?,
        toolUse: GaryxTranscriptMessage?,
        toolResult: GaryxTranscriptMessage?
    ) -> GaryxResolvedToolFieldProjection? {
        guard let projection else { return nil }
        return GaryxResolvedToolFieldProjection(
            kind: projection.kind,
            toolName: projection.toolName,
            call: resolve(projection.call, from: toolUse),
            result: resolve(projection.result, from: toolResult),
            status: projection.status,
            exitCode: projection.exitCode,
            durationMs: projection.durationMs
        )
    }

    private static func resolve(
        _ selector: GaryxRenderToolFieldSelector?,
        from message: GaryxTranscriptMessage?
    ) -> GaryxResolvedToolField? {
        guard let selector,
              let value = selectedValue(selector, from: message),
              let text = displayText(value, format: selector.format),
              hasVisibleContent(text) else {
            return nil
        }
        return GaryxResolvedToolField(
            text: text,
            label: label(selector.label),
            format: selector.format
        )
    }

    private static func selectedValue(
        _ selector: GaryxRenderToolFieldSelector,
        from message: GaryxTranscriptMessage?
    ) -> GaryxJSONValue? {
        guard var value = rootValue(selector.root, from: message) else { return nil }
        for component in selector.path {
            value = decodedIfNeeded(value)
            switch value {
            case .object(let object):
                guard let next = object[component] else { return nil }
                value = next
            case .array(let values):
                guard let index = Int(component), values.indices.contains(index) else { return nil }
                value = values[index]
            default:
                return nil
            }
        }
        return value
    }

    private static func rootValue(
        _ root: GaryxRenderToolFieldRoot,
        from message: GaryxTranscriptMessage?
    ) -> GaryxJSONValue? {
        guard let message else { return nil }
        let key: String
        let fallback: GaryxJSONValue?
        switch root {
        case .content:
            key = "content"
            fallback = message.content
        case .input:
            key = "input"
            fallback = message.input
        case .result:
            key = "result"
            fallback = message.result
        case .text:
            key = "text"
            fallback = .string(message.text)
        }
        if case .object(let wrapper)? = message.message.map(decodedIfNeeded),
           let nested = wrapper[key] {
            return nested
        }
        return fallback
    }

    private static func decodedIfNeeded(_ value: GaryxJSONValue) -> GaryxJSONValue {
        guard case .string(let raw) = value else { return value }
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return value }
        return (try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))) ?? value
    }

    private static func displayText(
        _ value: GaryxJSONValue,
        format: GaryxRenderToolFieldFormat
    ) -> String? {
        if format == .diff {
            return diffText(value)
        }
        if format == .image {
            // A string path can be rendered as an image preview. Structured
            // image blocks carry base64 and are deliberately omitted from text.
            guard case .string = value else { return nil }
        }
        switch value {
        case .string(let raw):
            if let encoded = encodedStringCandidate(raw),
               let decoded = try? JSONDecoder().decode(String.self, from: Data(encoded.utf8)) {
                return decoded
            }
            return raw
        case .number(let value):
            return value.rounded() == value ? String(Int(value)) : String(value)
        case .bool(let value):
            return value ? "true" : "false"
        case .object, .array:
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
            guard let data = try? encoder.encode(value) else { return nil }
            return String(data: data, encoding: .utf8)
        case .null:
            return nil
        }
    }

    private static func hasVisibleContent(_ value: String) -> Bool {
        value.unicodeScalars.contains { scalar in
            !CharacterSet.whitespacesAndNewlines.contains(scalar)
        }
    }

    private static func encodedStringCandidate(_ value: String) -> String? {
        if value.first == "\"", value.last == "\"" {
            return value
        }
        // Some providers wrap a short JSON scalar in whitespace. Bound the
        // normalization so large command output is never copied just to test
        // for this compatibility shape.
        guard value.utf16.prefix(16_385).count <= 16_384 else {
            return nil
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.first == "\"" && trimmed.last == "\"" ? trimmed : nil
    }

    private static func diffText(_ value: GaryxJSONValue) -> String? {
        switch value {
        case .string(let value):
            return value
        case .object(let object):
            if case .string(let diff)? = object["diff"] {
                return diff
            }
            return displayText(value, format: .json)
        case .array(let values):
            let parts = values.compactMap { entry -> String? in
                if case .object(let object) = entry,
                   case .string(let diff)? = object["diff"] {
                    return diff
                }
                return displayText(entry, format: .json)
            }
            return parts.isEmpty ? nil : parts.joined(separator: "\n")
        case .number, .bool, .null:
            return displayText(value, format: .json)
        }
    }

    private static func label(_ label: GaryxRenderToolFieldLabel) -> String {
        switch label {
        case .url: "URL"
        case .call: "Call"
        case .command: "Command"
        case .file: "File"
        case .query: "Query"
        case .prompt: "Prompt"
        case .parameters: "Parameters"
        case .content: "Content"
        case .output: "Output"
        case .result: "Result"
        case .response: "Response"
        case .diff: "Diff"
        case .image: "Image"
        case .error: "Error"
        }
    }
}
