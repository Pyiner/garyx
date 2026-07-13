import Foundation

/// One editable environment-variable row in the agent env editor. The stable
/// `id` keeps SwiftUI `ForEach` row identity across key/value edits.
public struct GaryxAgentEnvRow: Identifiable, Equatable, Sendable {
    public let id: UUID
    public var key: String
    public var value: String

    public init(id: UUID = UUID(), key: String, value: String) {
        self.id = id
        self.key = key
        self.value = value
    }
}

/// The save intent expressed by an edited env draft, mapped to the wire
/// contract: `.unchanged` omits `provider_env` (gateway preserves the stored
/// value), `.replace` sends the full desired map, `.clear` sends an empty map.
public enum GaryxAgentEnvIntent: Equatable, Sendable {
    case unchanged
    case replace([String: String])
    case clear
}

/// Pure, testable model backing the agent environment-variable editor.
///
/// Seed it from the agent's *authoritative* env (never a cache projection that
/// strips `provider_env`); an untouched draft resolves to `.unchanged`, so a
/// save that did not edit env never clears hidden variables.
public struct GaryxAgentEnvDraft: Equatable, Sendable {
    public private(set) var rows: [GaryxAgentEnvRow]
    /// Whether the user has edited env in this session. Drives `.unchanged`.
    public private(set) var isDirty: Bool

    public init(rows: [GaryxAgentEnvRow] = [], isDirty: Bool = false) {
        self.rows = rows
        self.isDirty = isDirty
    }

    public static let empty = GaryxAgentEnvDraft()

    /// Seed sorted rows from an env map; the seeded draft is not dirty.
    public static func seeded(from env: [String: String]) -> GaryxAgentEnvDraft {
        let rows = env.keys.sorted().map { GaryxAgentEnvRow(key: $0, value: env[$0] ?? "") }
        return GaryxAgentEnvDraft(rows: rows, isDirty: false)
    }

    public var isEmpty: Bool { rows.isEmpty }

    public mutating func addRow() {
        rows.append(GaryxAgentEnvRow(key: "", value: ""))
        isDirty = true
    }

    public mutating func removeRow(id: UUID) {
        rows.removeAll { $0.id == id }
        isDirty = true
    }

    public mutating func updateKey(id: UUID, _ key: String) {
        guard let index = rows.firstIndex(where: { $0.id == id }) else { return }
        rows[index].key = key
        isDirty = true
    }

    public mutating func updateValue(id: UUID, _ value: String) {
        guard let index = rows.firstIndex(where: { $0.id == id }) else { return }
        rows[index].value = value
        isDirty = true
    }

    /// Re-seed from authoritative env only if the user hasn't started editing,
    /// so a late authoritative fetch never clobbers in-progress edits.
    public mutating func reseedIfPristine(from env: [String: String]) {
        guard !isDirty else { return }
        rows = GaryxAgentEnvDraft.seeded(from: env).rows
    }

    /// Build the desired env map from the rows: empty (trimmed) keys are dropped
    /// and the last row wins on duplicate keys. Values are kept verbatim,
    /// including the empty string.
    public func currentEnvMap() -> [String: String] {
        var map: [String: String] = [:]
        for row in rows {
            let key = row.key.trimmingCharacters(in: .whitespacesAndNewlines)
            if !key.isEmpty {
                map[key] = row.value
            }
        }
        return map
    }

    /// Resolve the save intent. An untouched draft is `.unchanged`; an edited
    /// draft is `.clear` when the resulting map is empty, else `.replace`.
    public func resolvedIntent() -> GaryxAgentEnvIntent {
        guard isDirty else { return .unchanged }
        let map = currentEnvMap()
        return map.isEmpty ? .clear : .replace(map)
    }

    /// Whether a string is a valid POSIX-style env var name
    /// (`^[A-Za-z_][A-Za-z0-9_]*$`).
    public static func isValidKey(_ key: String) -> Bool {
        guard !key.isEmpty else { return false }
        for (index, scalar) in key.unicodeScalars.enumerated() {
            let isLetter = (scalar >= "A" && scalar <= "Z") || (scalar >= "a" && scalar <= "z")
            let isDigit = scalar >= "0" && scalar <= "9"
            if index == 0 {
                if !(isLetter || scalar == "_") { return false }
            } else if !(isLetter || isDigit || scalar == "_") {
                return false
            }
        }
        return true
    }

    /// Whether any row has a non-empty but invalid key (drives a UX warning).
    public var hasInvalidKey: Bool {
        rows.contains { row in
            let key = row.key.trimmingCharacters(in: .whitespacesAndNewlines)
            return !key.isEmpty && !GaryxAgentEnvDraft.isValidKey(key)
        }
    }

    // MARK: - Text view (dotenv-style) round trip

    /// Render rows as dotenv-style text, one `KEY=VALUE` per line.
    ///
    /// Values are emitted verbatim — never quoted — so what the user sees is
    /// byte-for-byte what the provider subprocess receives. Two exceptions
    /// ride a double-quoted escape carrier so `parseEnvText` round-trips them
    /// exactly: values containing a newline (cannot survive a line-oriented
    /// format) and values that themselves start and end with a double quote
    /// (would otherwise be mistaken for the carrier and stripped on parse).
    /// Mirrors the desktop `agent-env-editor.ts` semantics.
    public static func formatEnvText(_ rows: [GaryxAgentEnvRow]) -> String {
        rows.filter { !$0.key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map { row in
                let key = row.key.trimmingCharacters(in: .whitespacesAndNewlines)
                let value = row.value
                let looksQuoted = value.count >= 2 && value.hasPrefix("\"") && value.hasSuffix("\"")
                let hasEdgeWhitespace =
                    !value.isEmpty
                    && value != value.trimmingCharacters(in: .whitespaces)
                if value.contains("\n") || value.contains("\r") || looksQuoted || hasEdgeWhitespace {
                    var escaped = ""
                    for character in value {
                        switch character {
                        case "\\": escaped += "\\\\"
                        case "\"": escaped += "\\\""
                        case "\r": escaped += "\\r"
                        case "\n": escaped += "\\n"
                        default: escaped.append(character)
                        }
                    }
                    return "\(key)=\"\(escaped)\""
                }
                return "\(key)=\(value)"
            }
            .joined(separator: "\n")
    }

    /// Parse dotenv-style text into rows. Inverse of `formatEnvText`.
    ///
    /// - Blank lines and `#` comment lines are skipped.
    /// - Lines are whitespace-trimmed; values that genuinely need edge
    ///   whitespace survive via the quoted carrier `formatEnvText` emits.
    /// - Each line splits on the first `=`; the value keeps everything after
    ///   it verbatim (numbers stay unquoted, inner/partial quotes preserved).
    /// - Only a value fully wrapped in double quotes is treated as the escape
    ///   carrier: quotes stripped, `\n`/`\r`/`\"`/`\\` unescaped. Matches
    ///   dotenv intuition for users who habitually quote values.
    /// - A line with no `=` becomes a row with an empty value so the
    ///   invalid-key save gate surfaces it instead of silently dropping input.
    public static func parseEnvText(_ text: String) -> [GaryxAgentEnvRow] {
        var rows: [GaryxAgentEnvRow] = []
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        for rawLine in lines {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            if line.isEmpty || line.hasPrefix("#") { continue }
            guard let eq = line.firstIndex(of: "=") else {
                rows.append(GaryxAgentEnvRow(key: line, value: ""))
                continue
            }
            let key = String(line[line.startIndex..<eq]).trimmingCharacters(in: .whitespaces)
            var value = String(line[line.index(after: eq)...])
            if value.count >= 2, value.hasPrefix("\""), value.hasSuffix("\"") {
                let inner = String(value.dropFirst().dropLast())
                var unescaped = ""
                var valid = true
                var pendingEscape = false
                for character in inner {
                    if pendingEscape {
                        switch character {
                        case "n": unescaped += "\n"
                        case "r": unescaped += "\r"
                        case "\"": unescaped += "\""
                        case "\\": unescaped += "\\"
                        default:
                            unescaped.append("\\")
                            unescaped.append(character)
                        }
                        pendingEscape = false
                        continue
                    }
                    if character == "\\" {
                        pendingEscape = true
                        continue
                    }
                    if character == "\"" {
                        valid = false
                        break
                    }
                    unescaped.append(character)
                }
                if pendingEscape { unescaped.append("\\") }
                if valid { value = unescaped }
            }
            rows.append(GaryxAgentEnvRow(key: key, value: value))
        }
        return rows
    }

    /// Replace the draft's rows from edited dotenv-style text and mark the
    /// draft dirty. Backs the text view of the env editor.
    public mutating func applyEnvText(_ text: String) {
        rows = GaryxAgentEnvDraft.parseEnvText(text)
        isDirty = true
    }

    /// The draft rendered as dotenv-style text (text-view seed).
    public func envText() -> String {
        GaryxAgentEnvDraft.formatEnvText(rows)
    }
}
