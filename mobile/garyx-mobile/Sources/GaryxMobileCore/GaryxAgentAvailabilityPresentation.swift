import Foundation

public enum GaryxAgentDefaultBadgeState: Equatable, Sendable {
    case `default`
    case defaultInactive
    case actingDefault
    case defaultAuto

    public var label: String {
        switch self {
        case .default:
            "Default"
        case .defaultInactive:
            "Default (inactive)"
        case .actingDefault:
            "Acting default"
        case .defaultAuto:
            "Default (auto)"
        }
    }

    public var isMuted: Bool {
        self != .default
    }
}

public enum GaryxAgentAvailabilityPresentation {
    public static func defaultBadge(
        agentId: String,
        enabled: Bool,
        defaultAgentId: String?,
        effectiveDefaultAgentId: String?
    ) -> GaryxAgentDefaultBadgeState? {
        let id = normalized(agentId)
        guard !id.isEmpty else { return nil }
        let rawDefault = normalized(defaultAgentId)
        let effectiveDefault = normalized(effectiveDefaultAgentId)

        if rawDefault == id {
            return enabled ? .default : .defaultInactive
        }
        guard effectiveDefault == id else { return nil }
        return rawDefault.isEmpty ? .defaultAuto : .actingDefault
    }

    public static func statusLabel(enabled: Bool) -> String {
        enabled ? "Enabled" : "Disabled"
    }

    public static func allowsNewBindingActions(enabled: Bool, standalone: Bool) -> Bool {
        enabled && standalone
    }

    private static func normalized(_ value: String?) -> String {
        value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }
}

/// Resolves only the two client-owned new-thread states: a one-draft explicit
/// override, or the gateway's effective default. A stale explicit override is
/// deliberately preserved so the server can reject it; it must never fall back.
public enum GaryxNewThreadAgentSelection {
    public static func agentId(
        draftOverrideAgentId: String?,
        effectiveDefaultAgentId: String?
    ) -> String? {
        if let explicit = normalized(draftOverrideAgentId) {
            return explicit
        }
        return normalized(effectiveDefaultAgentId)
    }

    public static func isAvailable(
        draftOverrideAgentId: String?,
        effectiveDefaultAgentId: String?,
        enabledAgentIds: Set<String>
    ) -> Bool {
        guard let selected = agentId(
            draftOverrideAgentId: draftOverrideAgentId,
            effectiveDefaultAgentId: effectiveDefaultAgentId
        ) else {
            return false
        }
        return enabledAgentIds.contains(selected)
    }

    private static func normalized(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

public enum GaryxBotAgentSelection: Hashable, Sendable {
    case followGlobal
    case agent(String)
}

public struct GaryxBotAgentPickerOption: Identifiable, Equatable, Sendable {
    public var selection: GaryxBotAgentSelection
    public var title: String
    public var subtitle: String
    public var target: GaryxMobileAgentTarget?
    public var isAvailable: Bool
    public var isRecommended: Bool

    public var id: String {
        switch selection {
        case .followGlobal:
            "follow-global"
        case .agent(let id):
            "agent:\(id)"
        }
    }

    public init(
        selection: GaryxBotAgentSelection,
        title: String,
        subtitle: String = "",
        target: GaryxMobileAgentTarget? = nil,
        isAvailable: Bool = true,
        isRecommended: Bool = false
    ) {
        self.selection = selection
        self.title = title
        self.subtitle = subtitle
        self.target = target
        self.isAvailable = isAvailable
        self.isRecommended = isRecommended
    }
}

public enum GaryxBotAgentPickerPresentation {
    public static func preferredConfiguredAgentId(
        targets: [GaryxMobileAgentTarget],
        effectiveDefaultAgentId: String?
    ) -> String? {
        let effectiveId = normalized(effectiveDefaultAgentId)
        guard targets.contains(where: { $0.id == effectiveId }) else { return nil }
        return effectiveId
    }

    public static func makeOptions(
        targets: [GaryxMobileAgentTarget],
        effectiveDefaultAgentId: String?,
        configuredAgentId: String?
    ) -> [GaryxBotAgentPickerOption] {
        let effectiveId = normalized(effectiveDefaultAgentId)
        let configuredId = normalized(configuredAgentId)
        let effectiveTarget = targets.first { $0.id == effectiveId }
        let effectiveLabel = effectiveTarget?.title ?? effectiveId
        let followTitle = effectiveLabel.isEmpty
            ? "Follow global default (currently no enabled agent)"
            : "Follow global default (currently \(effectiveLabel))"

        var options = [
            GaryxBotAgentPickerOption(
                selection: .followGlobal,
                title: followTitle,
                subtitle: "Uses the current global default when a new thread is created"
            ),
        ]

        if !configuredId.isEmpty,
           !targets.contains(where: { $0.id == configuredId }) {
            options.append(
                GaryxBotAgentPickerOption(
                    selection: .agent(configuredId),
                    title: configuredId,
                    subtitle: "Unavailable agent",
                    isAvailable: false
                )
            )
        }

        let orderedTargets = targets.filter { $0.id == effectiveId }
            + targets.filter { $0.id != effectiveId }
        options.append(contentsOf: orderedTargets.map { target in
            GaryxBotAgentPickerOption(
                selection: .agent(target.id),
                title: target.title,
                subtitle: target.id == effectiveId ? "Current global default" : target.subtitle,
                target: target,
                isRecommended: target.id == effectiveId
            )
        })
        return options
    }

    public static func selection(configuredAgentId: String?) -> GaryxBotAgentSelection {
        let id = normalized(configuredAgentId)
        return id.isEmpty ? .followGlobal : .agent(id)
    }

    private static func normalized(_ value: String?) -> String {
        value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }
}

public enum GaryxAutomationAgentPresentation {
    public static func followsThreadLabel(
        resolution: GaryxAutomationAgentResolution,
        effectiveAgentId: String?,
        agents: [GaryxAgentSummary]
    ) -> String {
        if resolution == .targetMissing {
            return "Follows target thread · target unavailable"
        }
        let id = effectiveAgentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !id.isEmpty else {
            return "Follows target thread · unavailable until the thread has an agent"
        }
        let label = agents.first(where: { $0.id == id })?.displayName
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return "Follows target thread · \((label?.isEmpty == false ? label : nil) ?? id)"
    }
}
