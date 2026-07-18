import SwiftUI

struct GaryxRouteContext: Equatable {
    let node: GaryxRoutePresentationNode
    let isCanonicalTop: Bool
    let lifecycle: GaryxRouteHostLifecyclePhase

    var occurrenceID: GaryxRouteInstanceID? {
        guard case .entry(let entry) = node else { return nil }
        return entry.id
    }

    var composerKey: GaryxComposerKey? {
        guard case .entry(let entry) = node else { return nil }
        return entry.destination.composerKey
    }

    static let unavailable = GaryxRouteContext(
        node: .home,
        isCanonicalTop: false,
        lifecycle: .mounted
    )
}

private struct GaryxRouteContextKey: EnvironmentKey {
    static let defaultValue = GaryxRouteContext.unavailable
}

extension EnvironmentValues {
    var garyxRouteContext: GaryxRouteContext {
        get { self[GaryxRouteContextKey.self] }
        set { self[GaryxRouteContextKey.self] = newValue }
    }
}

@MainActor
final class GaryxRouteHostContextStore: ObservableObject {
    @Published private(set) var context: GaryxRouteContext

    init(_ context: GaryxRouteContext) {
        self.context = context
    }

    func apply(_ next: GaryxRouteContext) {
        guard context != next else { return }
        context = next
    }
}

/// Keeps immutable route identity in the environment while allowing the
/// container to project canonical-top and lifecycle ownership without
/// rebuilding the hosted feature subtree on gesture frames.
struct GaryxRouteContextHost: View {
    @ObservedObject var store: GaryxRouteHostContextStore
    let content: AnyView

    var body: some View {
        content.environment(\.garyxRouteContext, store.context)
    }
}
