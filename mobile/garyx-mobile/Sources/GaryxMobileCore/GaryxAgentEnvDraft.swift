import Foundation

/// One editable environment-variable row in the agent env editor. The stable
/// `id` keeps SwiftUI `ForEach` row identity across key/value edits.
public struct GaryxAgentEnvRow: Identifiable, Equatable {
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
public enum GaryxAgentEnvIntent: Equatable {
    case unchanged
    case replace([String: String])
    case clear
}

/// Pure, testable model backing the agent environment-variable editor.
///
/// Seed it from the agent's *authoritative* env (never a cache projection that
/// strips `provider_env`); an untouched draft resolves to `.unchanged`, so a
/// save that did not edit env never clears hidden variables.
public struct GaryxAgentEnvDraft: Equatable {
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
}
