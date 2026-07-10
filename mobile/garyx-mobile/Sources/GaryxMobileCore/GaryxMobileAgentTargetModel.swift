import Foundation

public struct GaryxMobileAgentTarget: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public let subtitle: String
    public let avatarDataUrl: String
    public let providerType: String
    /// The agent's configured model; empty when the provider default applies.
    public let model: String
    public let builtIn: Bool

    public init(
        id: String,
        title: String,
        subtitle: String,
        avatarDataUrl: String,
        providerType: String,
        model: String = "",
        builtIn: Bool
    ) {
        self.id = id
        self.title = title
        self.subtitle = subtitle
        self.avatarDataUrl = avatarDataUrl
        self.providerType = providerType
        self.model = model
        self.builtIn = builtIn
    }
}

public enum GaryxMobileAgentTargetMapper {
    public static func makeTargets(
        agents: [GaryxAgentSummary]
    ) -> [GaryxMobileAgentTarget] {
        let agentItems = agents
            .filter(\.standalone)
            .map {
                GaryxMobileAgentTarget(
                    id: $0.id,
                    title: $0.displayName.isEmpty ? $0.id : $0.displayName,
                    subtitle: "",
                    avatarDataUrl: $0.avatarDataUrl,
                    providerType: $0.providerType,
                    model: $0.model,
                    builtIn: $0.builtIn
                )
            }
        return agentItems
    }

    public static func selectedTarget(
        id selectedId: String,
        targets: [GaryxMobileAgentTarget]
    ) -> GaryxMobileAgentTarget? {
        let normalizedId = selectedId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedId.isEmpty else { return nil }
        return targets.first { $0.id == normalizedId }
    }

    public static func selectedThreadTarget(
        thread: GaryxThreadSummary?,
        selectedAgentTargetId: String,
        targets: [GaryxMobileAgentTarget]
    ) -> GaryxMobileAgentTarget? {
        guard let thread else {
            return selectedTarget(id: selectedAgentTargetId, targets: targets)
        }
        if let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !agentId.isEmpty,
           let target = targets.first(where: { $0.id == agentId }) {
            return target
        }
        return nil
    }

    public static func selectedAgentLabel(
        selectedAgentTargetId: String,
        target: GaryxMobileAgentTarget?
    ) -> String {
        target?.title ?? selectedAgentTargetId
    }

    public static func selectedThreadAgentLabel(
        thread: GaryxThreadSummary?,
        target: GaryxMobileAgentTarget?,
        fallbackSelectedAgentLabel: String
    ) -> String {
        if let target {
            return target.title
        }
        if let agentId = thread?.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !agentId.isEmpty {
            return agentId
        }
        return fallbackSelectedAgentLabel
    }
}
