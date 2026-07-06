import Foundation

/// One selectable provider entry for the agent form provider picker.
public struct GaryxAgentProviderPickerOption: Identifiable, Equatable, Sendable {
    public let id: String
    public let label: String

    public init(id: String, label: String) {
        self.id = id
        self.label = label
    }
}

/// Shared presentation for the agent form provider picker.
///
/// Labels always derive from `GaryxProviderPresentation.displayName` so
/// provider naming keeps a single source of truth; this type only owns the
/// canonical picker id order.
public enum GaryxAgentProviderPickerPresentation {
    /// Canonical provider choices offered by the agent form picker, in
    /// display order.
    public static let standardProviderIds: [String] = [
        "claude_code",
        "codex_app_server",
        "traex",
        "gemini_cli",
        "gpt",
        "anthropic",
        "google",
    ]

    public static var standardOptions: [GaryxAgentProviderPickerOption] {
        standardProviderIds.map {
            GaryxAgentProviderPickerOption(id: $0, label: GaryxProviderPresentation.displayName(for: $0))
        }
    }

    /// Standard options, with a non-standard current provider prepended so the
    /// picker can still render and select it. Matching against the standard
    /// table is an exact (case-sensitive) id comparison.
    public static func options(includingCurrent providerType: String?) -> [GaryxAgentProviderPickerOption] {
        let current = providerType?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !current.isEmpty, !standardProviderIds.contains(current) else {
            return standardOptions
        }
        return [GaryxAgentProviderPickerOption(id: current, label: label(for: current))] + standardOptions
    }

    /// Picker and read-only "Provider" row label: empty selects the choose
    /// placeholder, any other value resolves through the shared provider
    /// display name.
    public static func label(for providerType: String) -> String {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return "Choose provider" }
        return GaryxProviderPresentation.displayName(for: normalized)
    }
}
