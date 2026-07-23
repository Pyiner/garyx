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

    var allowsPageInteraction: Bool {
        isCanonicalTop && lifecycle == .active
    }

    static let unavailable = GaryxRouteContext(
        node: .home,
        isCanonicalTop: false,
        lifecycle: .mounted
    )
}

struct GaryxRouteNavigationActions {
    var dismiss: (() -> Void)?
    var push: ([GaryxRouteDestination]) -> Void
    var backLabel: String

    static let unavailable = GaryxRouteNavigationActions(
        dismiss: nil,
        push: { _ in },
        backLabel: "Back"
    )
}

private struct GaryxRouteContextKey: EnvironmentKey {
    static let defaultValue = GaryxRouteContext.unavailable
}

private struct GaryxRouteNavigationActionsKey: EnvironmentKey {
    static let defaultValue = GaryxRouteNavigationActions.unavailable
}

extension EnvironmentValues {
    var garyxRouteContext: GaryxRouteContext {
        get { self[GaryxRouteContextKey.self] }
        set { self[GaryxRouteContextKey.self] = newValue }
    }


    var garyxRouteNavigationActions: GaryxRouteNavigationActions {
        get { self[GaryxRouteNavigationActionsKey.self] }
        set { self[GaryxRouteNavigationActionsKey.self] = newValue }
    }
}

@MainActor
final class GaryxRouteHostContextStore: ObservableObject {
    @Published private(set) var context: GaryxRouteContext
    private var semanticContext: GaryxRouteContext
    private var hasDeferredObservation = false

    init(_ context: GaryxRouteContext) {
        self.context = context
        semanticContext = context
    }

    func apply(
        _ next: GaryxRouteContext,
        deferringObservationUntilRendererIdle: Bool = false
    ) {
        guard semanticContext != next else {
            if !deferringObservationUntilRendererIdle {
                publishDeferredObservationIfNeeded()
            }
            return
        }
        semanticContext = next
        if deferringObservationUntilRendererIdle {
            hasDeferredObservation = context != next
        } else {
            hasDeferredObservation = false
            if context != next {
                context = next
            }
        }
    }

    func publishDeferredObservationIfNeeded() {
        guard hasDeferredObservation else { return }
        hasDeferredObservation = false
        guard context != semanticContext else { return }
        context = semanticContext
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
