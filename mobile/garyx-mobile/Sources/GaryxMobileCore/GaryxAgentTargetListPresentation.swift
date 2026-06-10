import Foundation

/// Ordering and collapsing rules for the new-thread agent picker sheet.
public enum GaryxAgentTargetListPresentation {
    public static let defaultPrimaryLimit = 5

    /// Built-in agents first, then custom agents, then teams; stable within
    /// each group.
    public static func ordered(_ targets: [GaryxMobileAgentTarget]) -> [GaryxMobileAgentTarget] {
        let builtInAgents = targets.filter { $0.kind == .agent && $0.builtIn }
        let customAgents = targets.filter { $0.kind == .agent && !$0.builtIn }
        let teams = targets.filter { $0.kind == .team }
        return builtInAgents + customAgents + teams
    }

    /// The rows shown before collapsing into the all-agents level. At most
    /// `limit` entries; the current selection always stays visible.
    public static func primary(
        _ targets: [GaryxMobileAgentTarget],
        selectedId: String,
        limit: Int = defaultPrimaryLimit
    ) -> [GaryxMobileAgentTarget] {
        let ordered = ordered(targets)
        guard ordered.count > limit else {
            return ordered
        }
        var visible = Array(ordered.prefix(limit))
        let trimmedId = selectedId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedId.isEmpty,
           !visible.contains(where: { $0.id == trimmedId }),
           let selected = ordered.first(where: { $0.id == trimmedId }),
           !visible.isEmpty {
            visible[visible.count - 1] = selected
        }
        return visible
    }

    /// How many targets are hidden behind the all-agents level.
    public static func overflowCount(
        _ targets: [GaryxMobileAgentTarget],
        limit: Int = defaultPrimaryLimit
    ) -> Int {
        max(0, targets.count - limit)
    }
}
