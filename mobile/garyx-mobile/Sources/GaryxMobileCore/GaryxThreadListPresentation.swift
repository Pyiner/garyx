import Foundation

public enum GaryxThreadListAvailability: Equatable, Sendable {
    case ready
    case unsupportedGateway
    case failed(message: String)
}

public enum GaryxThreadRowActionKind: Equatable, Sendable {
    case pin
    case unpin
    case favorite
    case unfavorite
    case archive(GaryxThreadArchiveStrategy)
}

/// One context-menu action plan shared by Home and every drilldown.
public enum GaryxThreadRowActionPlanner {
    public static func actions(
        capabilities: GaryxThreadRowCapabilities,
        isPinned: Bool,
        isFavorite: Bool
    ) -> [GaryxThreadRowActionKind] {
        var actions: [GaryxThreadRowActionKind] = []
        if capabilities.canPin {
            actions.append(isPinned ? .unpin : .pin)
        }
        switch capabilities.favorite {
        case .addAndRemove:
            actions.append(isFavorite ? .unfavorite : .favorite)
        case .none:
            break
        }
        if capabilities.canArchive, capabilities.archiveStrategy != .none {
            actions.append(.archive(capabilities.archiveStrategy))
        }
        return actions
    }
}

/// The old-gateway picker fallback is intentionally bounded to summaries
/// already resident in the Recent feed. This is not the canonical search
/// implementation; new gateways always own normalization and matching.
public enum GaryxLegacyThreadPickerFallback {
    public static func rows(
        recentRows: [GaryxThreadSummary],
        rawQuery: String?
    ) -> [GaryxThreadSummary] {
        var seen = Set<String>()
        let rows = recentRows.filter { row in
            let id = row.id.trimmingCharacters(in: .whitespacesAndNewlines)
            return !id.isEmpty && seen.insert(id).inserted
        }
        let query = rawQuery?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !query.isEmpty else { return rows }
        return rows.filter { row in
            [
                row.title,
                row.workspacePath ?? "",
                row.agentId ?? "",
                row.lastMessagePreview,
            ].contains { $0.localizedCaseInsensitiveContains(query) }
        }
    }
}
