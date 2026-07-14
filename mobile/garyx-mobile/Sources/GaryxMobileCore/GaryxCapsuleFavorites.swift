import Foundation

public enum GaryxCapsuleGalleryTab: String, CaseIterable, Identifiable, Sendable {
    case all = "All"
    case favorites = "Favorites"

    public var id: Self { self }

    public func filter(
        _ capsules: [GaryxCapsuleSummary],
        favoriteState: GaryxCapsuleFavoriteReducerState
    ) -> [GaryxCapsuleSummary] {
        switch self {
        case .all:
            return capsules
        case .favorites:
            return capsules.filter {
                GaryxCapsuleFavoriteReducer.isFavorited($0, state: favoriteState)
            }
        }
    }
}

public struct GaryxCapsuleFavoriteMutationState: Equatable, Sendable {
    public var serverFavoritedAt: String?
    public var desiredFavorited: Bool
    public var inFlight: Bool

    public init(serverFavoritedAt: String?, desiredFavorited: Bool, inFlight: Bool) {
        self.serverFavoritedAt = serverFavoritedAt
        self.desiredFavorited = desiredFavorited
        self.inFlight = inFlight
    }
}

public struct GaryxCapsuleFavoriteReducerState: Equatable, Sendable {
    public var favoritesGeneration: Int
    public var mutations: [String: GaryxCapsuleFavoriteMutationState]

    public init(
        favoritesGeneration: Int = 0,
        mutations: [String: GaryxCapsuleFavoriteMutationState] = [:]
    ) {
        self.favoritesGeneration = favoritesGeneration
        self.mutations = mutations
    }
}

public struct GaryxCapsuleFavoriteEffect: Equatable, Sendable {
    public var capsuleId: String
    public var favorited: Bool

    public init(capsuleId: String, favorited: Bool) {
        self.capsuleId = capsuleId
        self.favorited = favorited
    }
}

public struct GaryxCapsuleFavoriteTransition: Equatable, Sendable {
    public var capsules: [GaryxCapsuleSummary]
    public var state: GaryxCapsuleFavoriteReducerState
    public var effect: GaryxCapsuleFavoriteEffect?

    public init(
        capsules: [GaryxCapsuleSummary],
        state: GaryxCapsuleFavoriteReducerState,
        effect: GaryxCapsuleFavoriteEffect? = nil
    ) {
        self.capsules = capsules
        self.state = state
        self.effect = effect
    }
}

public enum GaryxCapsuleFavoriteReducer {
    public static func isFavorited(
        _ capsule: GaryxCapsuleSummary,
        state: GaryxCapsuleFavoriteReducerState
    ) -> Bool {
        state.mutations[capsule.id]?.desiredFavorited ?? capsule.isFavorited
    }

    public static func toggle(
        capsules: [GaryxCapsuleSummary],
        state: GaryxCapsuleFavoriteReducerState,
        capsuleId: String,
        favorited: Bool
    ) -> GaryxCapsuleFavoriteTransition {
        guard let capsule = capsules.first(where: { $0.id == capsuleId }) else {
            return GaryxCapsuleFavoriteTransition(capsules: capsules, state: state)
        }
        let current = state.mutations[capsuleId] ?? initialMutation(capsule)
        guard current.desiredFavorited != favorited else {
            return GaryxCapsuleFavoriteTransition(capsules: capsules, state: state)
        }

        var mutation = current
        mutation.desiredFavorited = favorited
        var nextState = state
        var effect: GaryxCapsuleFavoriteEffect?
        if !current.inFlight {
            mutation.inFlight = true
            nextState.favoritesGeneration += 1
            effect = GaryxCapsuleFavoriteEffect(capsuleId: capsuleId, favorited: favorited)
        }
        nextState.mutations[capsuleId] = mutation
        return GaryxCapsuleFavoriteTransition(
            capsules: capsules,
            state: nextState,
            effect: effect
        )
    }

    public static func succeeded(
        capsules: [GaryxCapsuleSummary],
        state: GaryxCapsuleFavoriteReducerState,
        capsuleId: String,
        response: GaryxCapsuleFavoriteResponse
    ) -> GaryxCapsuleFavoriteTransition {
        guard let current = state.mutations[capsuleId], current.inFlight else {
            return GaryxCapsuleFavoriteTransition(capsules: capsules, state: state)
        }

        var nextState = state
        nextState.favoritesGeneration += 1
        var mutation = GaryxCapsuleFavoriteMutationState(
            serverFavoritedAt: response.capsule.favoritedAt,
            desiredFavorited: current.desiredFavorited,
            inFlight: false
        )
        var effect: GaryxCapsuleFavoriteEffect?
        if mutation.desiredFavorited != response.capsule.isFavorited {
            mutation.inFlight = true
            nextState.favoritesGeneration += 1
            effect = GaryxCapsuleFavoriteEffect(
                capsuleId: capsuleId,
                favorited: mutation.desiredFavorited
            )
        } else {
            mutation.desiredFavorited = response.capsule.isFavorited
        }
        nextState.mutations[capsuleId] = mutation

        return GaryxCapsuleFavoriteTransition(
            capsules: replacing(capsules, with: response.capsule),
            state: nextState,
            effect: effect
        )
    }

    public static func failed(
        capsules: [GaryxCapsuleSummary],
        state: GaryxCapsuleFavoriteReducerState,
        capsuleId: String
    ) -> GaryxCapsuleFavoriteTransition {
        guard var mutation = state.mutations[capsuleId], mutation.inFlight else {
            return GaryxCapsuleFavoriteTransition(capsules: capsules, state: state)
        }
        mutation.desiredFavorited = mutation.serverFavoritedAt != nil
        mutation.inFlight = false
        var nextState = state
        nextState.favoritesGeneration += 1
        nextState.mutations[capsuleId] = mutation
        return GaryxCapsuleFavoriteTransition(
            capsules: capsules.map { capsule in
                guard capsule.id == capsuleId else { return capsule }
                var reverted = capsule
                reverted.favoritedAt = mutation.serverFavoritedAt
                return reverted
            },
            state: nextState
        )
    }

    public static func mergingRefresh(
        currentCapsules: [GaryxCapsuleSummary],
        refreshedCapsules: [GaryxCapsuleSummary],
        state: GaryxCapsuleFavoriteReducerState,
        capturedGeneration: Int
    ) -> GaryxCapsuleFavoriteTransition {
        let currentById = Dictionary(uniqueKeysWithValues: currentCapsules.map { ($0.id, $0) })
        let generationIsCurrent = capturedGeneration == state.favoritesGeneration
        let refreshedIds = Set(refreshedCapsules.map(\.id))
        var mutations = state.mutations

        let capsules = refreshedCapsules.map { refreshed -> GaryxCapsuleSummary in
            let mutation = mutations[refreshed.id]
                ?? currentById[refreshed.id].map(initialMutation)
            let pending = mutation.map {
                $0.inFlight || $0.desiredFavorited != ($0.serverFavoritedAt != nil)
            } ?? false

            if generationIsCurrent, !pending {
                mutations[refreshed.id] = initialMutation(refreshed)
                return refreshed
            }
            if let mutation {
                mutations[refreshed.id] = mutation
                var merged = refreshed
                merged.favoritedAt = mutation.serverFavoritedAt
                return merged
            }
            return refreshed
        }

        for (capsuleId, mutation) in mutations
            where !refreshedIds.contains(capsuleId) && !mutation.inFlight {
            mutations.removeValue(forKey: capsuleId)
        }
        var nextState = state
        nextState.mutations = mutations
        return GaryxCapsuleFavoriteTransition(capsules: capsules, state: nextState)
    }

    private static func initialMutation(
        _ capsule: GaryxCapsuleSummary
    ) -> GaryxCapsuleFavoriteMutationState {
        GaryxCapsuleFavoriteMutationState(
            serverFavoritedAt: capsule.favoritedAt,
            desiredFavorited: capsule.isFavorited,
            inFlight: false
        )
    }

    private static func replacing(
        _ capsules: [GaryxCapsuleSummary],
        with replacement: GaryxCapsuleSummary
    ) -> [GaryxCapsuleSummary] {
        capsules.map { $0.id == replacement.id ? replacement : $0 }
    }
}
