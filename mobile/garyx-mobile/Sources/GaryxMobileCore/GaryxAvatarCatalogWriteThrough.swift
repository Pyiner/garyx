import Foundation

public extension GaryxAvatarWriteThroughPlan {
    static func candidates(
        scope: String,
        agents: [GaryxAgentSummary]
    ) -> [GaryxAvatarUpsert] {
        let normalizedScope = scope.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedScope.isEmpty else { return [] }
        let agentUpserts = agents.compactMap { agent -> GaryxAvatarUpsert? in
            let dataUrl = agent.avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !agent.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  !dataUrl.isEmpty else {
                return nil
            }
            return GaryxAvatarUpsert(
                identity: GaryxAvatarIdentity(scope: normalizedScope, id: agent.id),
                dataUrl: dataUrl,
                sourceUpdatedAt: agent.updatedAt
            )
        }
        return agentUpserts
    }
}
