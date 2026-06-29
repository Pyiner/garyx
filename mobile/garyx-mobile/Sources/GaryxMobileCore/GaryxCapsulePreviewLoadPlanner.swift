import Foundation

// Capsule preview thumbnails are now cached rendered images (see
// `GaryxCapsuleThumbnailRendering` + the app-target store/renderer), so the old
// visibility-admission planner and chat-card admission that bounded live
// `WKWebView`s are gone — display no longer needs gating, and the one-shot
// render on a cache miss is concurrency-capped at render time. The pure
// presentation helpers below remain in use by the gallery and chat cards.

/// Pure presentation for a chat capsule card's secondary line. Keeps the
/// action→label mapping in Core so the SwiftUI card stays a dumb renderer.
public enum GaryxCapsuleChatCardPresentation {
    public static func subtitle(action: GaryxRenderCapsuleAction) -> String {
        switch action {
        case .created: return "Created"
        case .updated: return "Updated"
        }
    }
}

/// Pure presentation for a gallery capsule card's single-line subinfo, mirroring
/// the Mac gallery card's `.capsule-card-subline` ("time · creator"). Keeping the
/// creator precedence and the join in Core lets the SwiftUI card stay a dumb
/// renderer (no pill chips, no local switch tables).
public enum GaryxCapsuleGalleryCardPresentation {
    /// Creator name precedence — an iOS superset of desktop `describeCreator`
    /// (which lacks the team lookup): agent name → team name → agentId →
    /// prettified provider → "Agent". The team tier preserves the resolution the
    /// current owner badge already performs, so a team-created capsule shows the
    /// team's display name instead of a raw id.
    public static func creatorName(
        agentId: String?,
        providerType: String?,
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary]
    ) -> String {
        let trimmedAgentId = agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !trimmedAgentId.isEmpty {
            if let name = agents.first(where: { $0.id == trimmedAgentId })?.displayName
                .trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty {
                return name
            }
            if let name = teams.first(where: { $0.id == trimmedAgentId })?.displayName
                .trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty {
                return name
            }
            return trimmedAgentId
        }
        let provider = providerType?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !provider.isEmpty {
            return GaryxProviderPresentation.displayName(for: provider)
        }
        return "Agent"
    }

    /// Joins the relative time and creator into the Mac-style "time · creator"
    /// single line. When the time is empty/nil, the creator is shown alone so
    /// there is never a dangling separator.
    public static func subline(timeDisplay: String?, creator: String) -> String {
        let time = timeDisplay?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let trimmedCreator = creator.trimmingCharacters(in: .whitespacesAndNewlines)
        if time.isEmpty {
            return trimmedCreator
        }
        if trimmedCreator.isEmpty {
            return time
        }
        return "\(time) · \(trimmedCreator)"
    }
}
