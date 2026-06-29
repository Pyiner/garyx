import Foundation

/// Visibility-based admission for capsule preview thumbnails in the **gallery**.
///
/// The gallery is a `LazyVGrid`, so `onAppear`/`onDisappear` are true visibility
/// signals. Cards report visible ids (in appearance order); the planner admits
/// the first `maxActive` still-visible ids (FIFO) to mount a `WKWebView`. The
/// rest render a skeleton until an earlier card scrolls off and frees a slot.
///
/// Pure value type so the admission policy is unit-testable without SwiftUI. The
/// chat transcript uses a different policy (`GaryxCapsuleChatCardAdmission`)
/// because it is an eager `VStack` where `onAppear` is not a visibility signal.
public struct GaryxCapsulePreviewLoadPlanner: Equatable, Sendable {
    public private(set) var maxActive: Int
    private var visibleOrder: [String]

    public init(maxActive: Int, visibleOrder: [String] = []) {
        self.maxActive = max(0, maxActive)
        self.visibleOrder = visibleOrder
    }

    public var visibleIds: [String] { visibleOrder }

    /// The first `maxActive` still-visible ids, in appearance order.
    public var activeIds: [String] { Array(visibleOrder.prefix(maxActive)) }

    public func isActive(_ id: String) -> Bool {
        guard let index = visibleOrder.firstIndex(of: id) else { return false }
        return index < maxActive
    }

    /// Append on first appearance; idempotent (never reorders an already-visible
    /// id). Returns whether the visible set changed.
    @discardableResult
    public mutating func markVisible(_ id: String) -> Bool {
        guard !visibleOrder.contains(id) else { return false }
        visibleOrder.append(id)
        return true
    }

    @discardableResult
    public mutating func markHidden(_ id: String) -> Bool {
        guard let index = visibleOrder.firstIndex(of: id) else { return false }
        visibleOrder.remove(at: index)
        return true
    }

    public mutating func setMaxActive(_ n: Int) {
        maxActive = max(0, n)
    }

    /// Drop visible ids that are no longer valid (e.g. capsule deleted), keeping
    /// appearance order for the survivors.
    public mutating func prune(keeping valid: Set<String>) {
        visibleOrder = visibleOrder.filter { valid.contains($0) }
    }
}

/// Conversation-level admission for capsule preview thumbnails in the **chat
/// transcript**.
///
/// The transcript is a deliberately eager `VStack` (see
/// `GaryxMobileConversationViews`), so every turn — and therefore every capsule
/// card — is mounted at once and `onAppear` is not a visibility signal. A
/// per-turn cap would let N historical turns each mount their own thumbnails and
/// blow past the global WKWebView budget. Instead the conversation flattens all
/// capsule-card instance keys in transcript order and admits the most-recent
/// `maxActive` (the tail), since the transcript opens scrolled to the bottom so
/// the newest cards are the ones most likely on screen. Non-admitted cards show
/// a static shell and still open the focused preview on tap.
public enum GaryxCapsuleChatCardAdmission {
    /// `orderedKeys` are per-instance keys (`"<turnId>:<capsuleId>"`) in
    /// transcript order, newest last. Returns the most-recent `maxActive`.
    public static func activeKeys(orderedKeys: [String], maxActive: Int) -> [String] {
        Array(orderedKeys.suffix(max(0, maxActive)))
    }
}

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
