import Foundation

/// Value-type draft over the team form's two string bindings: the leader agent
/// id and the raw comma-separated member id list. Owns member-id parsing,
/// serialization, and the leader/member relationship rules so SwiftUI only
/// forwards selection events.
public struct GaryxTeamMembershipDraft: Equatable, Sendable {
    public var leaderAgentId: String
    public var memberAgentIds: String

    public init(leaderAgentId: String, memberAgentIds: String) {
        self.leaderAgentId = leaderAgentId
        self.memberAgentIds = memberAgentIds
    }

    public var normalizedLeaderId: String {
        leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public var memberIds: [String] {
        Self.memberIds(from: memberAgentIds)
    }

    /// Parses a raw member id string. Separators are exactly `,`, `\n`, and
    /// the plain space; tokens are trimmed and deduplicated (case-sensitive)
    /// while preserving order.
    public static func memberIds(from value: String) -> [String] {
        var seen = Set<String>()
        return value
            .split { $0 == "," || $0 == "\n" || $0 == " " }
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && seen.insert($0).inserted }
    }

    /// Serializes member ids back into the form's raw string: trimmed,
    /// empties dropped, ", "-joined. Does not deduplicate.
    public static func memberIdsString(_ ids: [String]) -> String {
        ids
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: ", ")
    }

    /// Submission-path member ids: the trimmed leader first, then the parsed
    /// member tokens, deduplicated while preserving order.
    public static func normalizedMemberIds(from rawValue: String, leaderAgentId: String) -> [String] {
        let leader = leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        var ids: [String] = leader.isEmpty ? [] : [leader]
        for token in rawValue.split(whereSeparator: { $0 == "," || $0 == "\n" || $0 == " " }) {
            let id = String(token).trimmingCharacters(in: .whitespacesAndNewlines)
            if !id.isEmpty, !ids.contains(id) {
                ids.append(id)
            }
        }
        return ids
    }

    /// Makes `agentId` the leader and ensures it is a member (inserted first
    /// when missing). The member string is rewritten in normalized form.
    public mutating func selectLeader(_ agentId: String) {
        leaderAgentId = agentId
        var members = Self.memberIds(from: memberAgentIds)
        if !members.contains(agentId) {
            members.insert(agentId, at: 0)
        }
        memberAgentIds = Self.memberIdsString(members)
    }

    /// Adds or removes `agentId` from the members. Removing the current leader
    /// hands leadership to the first remaining member (or clears it); adding
    /// the first member while no leader is set makes it the leader.
    public mutating func toggleMember(_ agentId: String) {
        var nextIds = Self.memberIds(from: memberAgentIds)
        if nextIds.contains(agentId) {
            nextIds.removeAll { $0 == agentId }
            if normalizedLeaderId == agentId {
                leaderAgentId = nextIds.first ?? ""
            }
        } else {
            nextIds.append(agentId)
            if normalizedLeaderId.isEmpty {
                leaderAgentId = agentId
            }
        }
        memberAgentIds = Self.memberIdsString(nextIds)
    }
}

/// Pure presentation helpers for the team form rows and sheets.
public enum GaryxTeamFormPresentation {
    /// Selectable agents for the leader/member pickers: standalone agents,
    /// built-in first then by display name (case-insensitive), deduplicated by
    /// id, with referenced-but-unknown ids prepended as placeholder entries
    /// (one by one, so multiple unknown ids end up in reverse order — existing
    /// semantics).
    public static func agentOptions(
        _ agents: [GaryxAgentSummary],
        preserving ids: [String]
    ) -> [GaryxAgentSummary] {
        var seen = Set<String>()
        var result = agents
            .filter(\.standalone)
            .sorted { left, right in
                if left.builtIn != right.builtIn {
                    return left.builtIn && !right.builtIn
                }
                return left.displayName.localizedCaseInsensitiveCompare(right.displayName) == .orderedAscending
            }
            .filter { seen.insert($0.id).inserted }
        for id in ids {
            let trimmed = id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, !seen.contains(trimmed) else { continue }
            result.insert(
                GaryxAgentSummary(
                    id: trimmed,
                    displayName: trimmed,
                    providerType: "",
                    model: "",
                    builtIn: false,
                    standalone: true
                ),
                at: 0
            )
            seen.insert(trimmed)
        }
        return result
    }

    /// Editable "Leader" row label. A resolved option's display name is used
    /// as-is (even when empty); an unresolved id falls back to the id itself.
    public static func leaderLabel(leaderAgentId: String, options: [GaryxAgentSummary]) -> String {
        let normalized = leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return "Choose leader" }
        return options.first(where: { $0.id == normalized })?.displayName ?? normalized
    }

    /// Editable "Members" row label: up to two display names, then "+N". A
    /// resolved agent's display name is used as-is (even when empty); only an
    /// unknown id falls back to the id itself.
    public static func membersLabel(memberIds: [String], agents: [GaryxAgentSummary]) -> String {
        guard !memberIds.isEmpty else { return "" }
        let namesById = Dictionary(uniqueKeysWithValues: agents.map { ($0.id, $0.displayName) })
        let names = memberIds.map { namesById[$0] ?? $0 }
        if names.count <= 2 {
            return names.joined(separator: ", ")
        }
        return "\(names[0]), \(names[1]) +\(names.count - 2)"
    }

    /// Read-only member label: "Display Name (id)", falling back to the bare
    /// id when the agent is unknown or has an empty display name.
    public static func memberDetailLabel(agentId: String, agents: [GaryxAgentSummary]) -> String {
        let trimmed = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        guard let agent = agents.first(where: { $0.id == trimmed }) else { return trimmed }
        return agent.displayName.isEmpty ? agent.id : "\(agent.displayName) (\(agent.id))"
    }

    /// Read-only multiline "Members" value: one detail label per line.
    public static func memberDetailLabels(memberAgentIds: String, agents: [GaryxAgentSummary]) -> String {
        let ids = GaryxTeamMembershipDraft.memberIds(from: memberAgentIds)
        guard !ids.isEmpty else { return "" }
        return ids.map { memberDetailLabel(agentId: $0, agents: agents) }.joined(separator: "\n")
    }
}
