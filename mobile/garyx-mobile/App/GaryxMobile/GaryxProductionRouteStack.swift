import SwiftUI
import UIKit

/// Main-actor bridge between product navigation intents and the UIKit-owned
/// canonical stack. The container remains the only path writer; this store is
/// its observable projection for application state and tests.
@MainActor
final class GaryxProductionRouteStore: ObservableObject {
    @Published private(set) var path: [GaryxRouteEntry] = []

    private weak var container: GaryxRouteStackContainer?

    var isAttached: Bool { container != nil }

    func attach(_ container: GaryxRouteStackContainer) {
        self.container = container
        if container.path != path {
            _ = container.requestHardSnap(to: path)
        }
    }

    func detach(_ container: GaryxRouteStackContainer) {
        if self.container === container {
            self.container = nil
        }
    }

    @discardableResult
    func open(
        _ destination: GaryxRouteDestination,
        source: GaryxMobilePanelOpenSource,
        animated: Bool = true
    ) -> GaryxRouteEntry {
        if case .conversationDraft(let draftID) = destination,
           let existingIndex = path.firstIndex(where: {
               $0.destination == .conversationDraft(draftID: draftID)
           }) {
            let existing = path[existingIndex]
            let descendants = path.count - existingIndex - 1
            if descendants > 0 {
                if let container {
                    _ = container.pop(count: descendants, animated: animated)
                } else {
                    path.removeLast(descendants)
                }
            }
            return existing
        }

        let entry = GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: UUID().uuidString.lowercased()),
            destination: destination
        )
        guard let container else {
            path = source == .current ? path + [entry] : [entry]
            return entry
        }

        if path.isEmpty || source == .current {
            _ = container.push(entry, animated: animated)
        } else {
            _ = container.requestHardSnap(to: [entry])
        }
        return entry
    }

    /// Opens a suffix in one container transaction so no intermediate push
    /// can be lost to the first entry's settle. A detail route can therefore
    /// install its canonical overview predecessor in the same user action.
    @discardableResult
    func open(
        _ destinations: [GaryxRouteDestination],
        source: GaryxMobilePanelOpenSource,
        animated: Bool = true
    ) -> [GaryxRouteEntry] {
        precondition(!destinations.isEmpty, "route suffix must not be empty")
        let entries = destinations.map { destination in
            GaryxRouteEntry(
                id: GaryxRouteInstanceID(rawValue: UUID().uuidString.lowercased()),
                destination: destination
            )
        }
        guard let container else {
            path = source == .current ? path + entries : entries
            return entries
        }

        if path.isEmpty || source == .current {
            _ = container.push(entries, animated: animated)
        } else {
            _ = container.requestHardSnap(to: entries)
        }
        return entries
    }

    func popToHome(animated: Bool = true) {
        guard !path.isEmpty else { return }
        if let container {
            _ = container.pop(count: path.count, animated: animated)
        } else {
            path.removeAll()
        }
    }

    func popOne(animated: Bool = true) {
        guard !path.isEmpty else { return }
        if let container {
            _ = container.pop(animated: animated)
        } else {
            path.removeLast()
        }
    }

    @discardableResult
    func promoteVisibleDraft(draftID: String, threadID: String) -> Bool {
        guard let index = path.lastIndex(where: {
            $0.destination == .conversationDraft(draftID: draftID)
        }) else { return false }
        let instanceID = path[index].id
        if let container {
            return container.promoteVisibleDraft(
                instanceID: instanceID,
                draftID: draftID,
                threadID: threadID
            )
        }
        var replacement = path
        replacement[index].replacePayload(with: .conversation(threadID: threadID))
        path = replacement
        return true
    }

    @discardableResult
    func replaceVisibleDraftKey(oldDraftID: String, newDraftID: String) -> Bool {
        guard oldDraftID != newDraftID,
              let index = path.lastIndex(where: {
                  $0.destination == .conversationDraft(draftID: oldDraftID)
              }) else { return false }
        let instanceID = path[index].id
        if let container {
            return container.replaceVisibleDraftKey(
                instanceID: instanceID,
                oldDraftID: oldDraftID,
                newDraftID: newDraftID
            )
        }
        var replacement = path
        replacement[index].replacePayload(with: .conversationDraft(draftID: newDraftID))
        path = replacement
        return true
    }

    func applyCanonicalPath(_ canonicalPath: [GaryxRouteEntry]) {
        guard path != canonicalPath else { return }
        path = canonicalPath
    }

    func sceneDidBecomeInactive() {
        container?.sceneDidBecomeInactive()
    }

    func sceneDidBecomeActive() {
        container?.sceneDidBecomeActive()
    }
}

struct GaryxProductionRouteStack: UIViewControllerRepresentable {
    @ObservedObject var store: GaryxProductionRouteStore
    let model: GaryxMobileModel
    let homeContent: AnyView
    let routeContent: @MainActor (GaryxRoutePresentationNode) -> AnyView
    let onOpenDrawer: @MainActor () -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.garyxPrefersCrossFadeTransitions) private var prefersCrossFadeTransitions
    @Environment(\.layoutDirection) private var layoutDirection

    final class Coordinator {
        weak var store: GaryxProductionRouteStore?
        var preferences: GaryxRouteVisualPreferences

        init(store: GaryxProductionRouteStore, preferences: GaryxRouteVisualPreferences) {
            self.store = store
            self.preferences = preferences
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(store: store, preferences: currentPreferences)
    }

    func makeUIViewController(context: Context) -> GaryxRouteStackContainer {
        #if DEBUG
        let diagnostics = GaryxProductionRouteDiagnostics.makeIfEnabled()
        #endif
        var callbacks = GaryxRouteStackContainerCallbacks()
        callbacks.phaseChanged = { phase in
            #if DEBUG
            diagnostics?.transitionPhaseChanged(phase)
            #endif
        }
        callbacks.commitReleased = { [weak model] source, destination in
            model?.composerPayloadCoordinator.routeCommitReleased(
                sourceOccurrenceID: source.occurrenceID,
                sourceKey: source.composerKey,
                destinationOccurrenceID: destination.occurrenceID,
                destinationKey: destination.composerKey
            )
        }
        callbacks.canonicalPathChanged = { [weak store, weak model] path in
            store?.applyCanonicalPath(path)
            model?.applyCanonicalRouteProjection(path)
            #if DEBUG
            diagnostics?.canonicalPathChanged(path)
            #endif
        }
        callbacks.terminalReached = { [weak model] terminal in
            model?.composerPayloadCoordinator.routeReachedTerminal(terminal)
            #if DEBUG
            diagnostics?.terminalReached(terminal)
            #endif
        }
        let container = GaryxRouteStackContainer(
            initialPath: store.path,
            callbacks: callbacks,
            preferencesProvider: { [weak coordinator = context.coordinator] in
                coordinator?.preferences
                    ?? .init(reduceMotion: false, prefersCrossFadeTransitions: false)
            },
            hostBuilder: { node in
                let content: AnyView = switch node {
                case .home:
                    homeContent
                case .entry:
                    routeContent(node)
                }
                return AnyView(
                    GaryxProductionRouteHostEnvironment(
                        model: model,
                        content: content,
                        onOpenDrawer: onOpenDrawer
                    )
                )
            }
        )
        container.layoutDirectionOverride = layoutDirection == .rightToLeft
            ? .rightToLeft
            : .leftToRight
        // The home drawer has its own interactive SwiftUI drag. Keeping the
        // UIKit route recognizer inert at depth zero avoids opening the drawer
        // at touch-down and leaves a single gesture owner for that surface.
        container.homeLeadingEdgeAction = nil
        #if DEBUG
        diagnostics?.install(in: container)
        container.transitionFrameObserver = { [weak container, weak diagnostics] phase, progress, timestamp in
            guard let container else { return }
            diagnostics?.recordTransitionFrame(
                in: container,
                phase: phase,
                progress: progress,
                timestamp: timestamp
            )
        }
        #endif
        store.attach(container)
        let initialPath = store.path
        DispatchQueue.main.async { [weak model] in
            guard let model else { return }
            if let composerKey = initialPath.last?.destination.composerKey {
                Task { @MainActor [weak model] in
                    guard let model else { return }
                    await model.composerPayloadCoordinator.activate(
                        scope: model.gatewayRequestToken.scope,
                        key: composerKey
                    )
                }
            }
        }
        return container
    }

    func updateUIViewController(
        _ container: GaryxRouteStackContainer,
        context: Context
    ) {
        context.coordinator.preferences = currentPreferences
        container.layoutDirectionOverride = layoutDirection == .rightToLeft
            ? .rightToLeft
            : .leftToRight
    }

    static func dismantleUIViewController(
        _ container: GaryxRouteStackContainer,
        coordinator: Coordinator
    ) {
        coordinator.store?.detach(container)
    }

    private var currentPreferences: GaryxRouteVisualPreferences {
        GaryxRouteVisualPreferences(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }
}

#if DEBUG
/// Opt-in diagnostics for XCUITest coverage of the real conversation host.
/// Every rendered frame is checked against the frozen iOS 26 pop geometry;
/// settle frames additionally verify monotonic progress and frame cadence.
@MainActor
private final class GaryxProductionRouteDiagnostics {
    private let statusLabel = UILabel()
    private let automaticallyRegrabs: Bool
    private weak var container: GaryxRouteStackContainer?
    private var depth = 0
    private var phase = GaryxPresentationTransactionPhase.active
    private var terminalOutcome = "none"
    private var transactionCount = 0
    private var regrabCount = 0
    private var curvePassed = true
    private var checkedFrameCount = 0
    private var settleFrameCount = 0
    private var settleTarget: CGFloat?
    private var lastSettleProgress: CGFloat?
    private var backwardsFrameCount = 0
    private var lastFrameTimestamp: CFTimeInterval?
    private var maximumFrameGapMilliseconds: Double = 0
    private var liveAdapterCount = 0
    private var focusedAdapterCount = 0
    private var focusAtTransactionStart = -1
    private var previousFocusedAdapterCount: Int?
    private var focusLossPhase = "none"

    static func makeIfEnabled(
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> GaryxProductionRouteDiagnostics? {
        guard environment["GARYX_MOBILE_PRODUCTION_ROUTE_DIAGNOSTICS"] == "1" else {
            return nil
        }
        return GaryxProductionRouteDiagnostics(
            automaticallyRegrabs: environment["GARYX_MOBILE_PRODUCTION_ROUTE_AUTO_REGRAB"] == "1"
        )
    }

    private init(automaticallyRegrabs: Bool) {
        self.automaticallyRegrabs = automaticallyRegrabs
    }

    func install(in container: GaryxRouteStackContainer) {
        self.container = container
        depth = container.path.count
        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        statusLabel.font = .monospacedSystemFont(ofSize: 9, weight: .regular)
        statusLabel.textColor = .secondaryLabel
        statusLabel.backgroundColor = UIColor.systemBackground.withAlphaComponent(0.92)
        statusLabel.numberOfLines = 3
        statusLabel.isUserInteractionEnabled = false
        statusLabel.accessibilityIdentifier = "production.route.status"
        container.view.addSubview(statusLabel)
        NSLayoutConstraint.activate([
            statusLabel.leadingAnchor.constraint(equalTo: container.view.leadingAnchor, constant: 8),
            statusLabel.trailingAnchor.constraint(equalTo: container.view.trailingAnchor, constant: -8),
            statusLabel.bottomAnchor.constraint(
                equalTo: container.view.safeAreaLayoutGuide.bottomAnchor,
                constant: -4
            ),
        ])
        publishStatus(in: container)
    }

    func transitionPhaseChanged(_ newPhase: GaryxPresentationTransactionPhase) {
        if newPhase == .preCommit {
            if phase == .cancelSettle {
                regrabCount += 1
            } else {
                transactionCount += 1
                curvePassed = true
                checkedFrameCount = 0
                backwardsFrameCount = 0
                maximumFrameGapMilliseconds = 0
                focusAtTransactionStart = -1
                previousFocusedAdapterCount = nil
                focusLossPhase = "none"
            }
        }
        if newPhase == .commitSettle || newPhase == .cancelSettle {
            settleTarget = newPhase == .commitSettle ? 1 : 0
            settleFrameCount = 0
            lastSettleProgress = nil
            lastFrameTimestamp = nil
            if newPhase == .cancelSettle, automaticallyRegrabs {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) { [weak self] in
                    self?.performAutomaticRegrab()
                }
            }
        } else if newPhase == .terminal {
            settleTarget = nil
        }
        phase = newPhase
        publishStatus()
    }

    private func performAutomaticRegrab() {
        guard let container,
              container.metrics.transitionPhase == .cancelSettle,
              container.regrabCancelSettle() != nil else { return }
        let width = container.view.bounds.width
        container.updateInteractivePop(logicalTranslation: width * 0.80)
        _ = container.endInteractivePop(logicalVelocity: width * 2)
    }

    func canonicalPathChanged(_ path: [GaryxRouteEntry]) {
        depth = path.count
        publishStatus()
    }

    func terminalReached(_ terminal: GaryxPresentationTerminalState) {
        terminalOutcome = "\(terminal.outcome.rawValue)-\(terminal.visibility.rawValue)"
        publishStatus()
    }

    func recordTransitionFrame(
        in container: GaryxRouteStackContainer,
        phase: GaryxPresentationTransactionPhase,
        progress: CGFloat,
        timestamp: CFTimeInterval
    ) {
        checkedFrameCount += 1
        let adapters = composerAdapters(in: container.view)
        liveAdapterCount = adapters.filter(\.isLive).count
        focusedAdapterCount = adapters.filter(\.isFirstResponder).count
        if focusAtTransactionStart < 0 {
            focusAtTransactionStart = focusedAdapterCount
        }
        if let previousFocusedAdapterCount,
           previousFocusedAdapterCount > 0,
           focusedAdapterCount == 0,
           focusLossPhase == "none" {
            focusLossPhase = phase.rawValue
        }
        previousFocusedAdapterCount = focusedAdapterCount
        let expected = GaryxRouteTransitionGeometry.visualState(
            kind: .pop,
            policy: container.visualPolicyForActiveTransaction ?? .spatial,
            progress: progress,
            viewportWidth: container.view.bounds.width,
            layoutDirection: container.layoutDirectionOverride ?? .leftToRight
        )
        let actualTranslations = container.view.subviews
            .compactMap { $0 as? GaryxRouteTransitionWrapperView }
            .filter { !$0.isHidden }
            .map { $0.transform.tx }
            .sorted()
        let expectedTranslations = [
            expected.sourceTranslationX,
            expected.destinationTranslationX,
        ].sorted()
        curvePassed = curvePassed
            && actualTranslations.count == expectedTranslations.count
            && zip(actualTranslations, expectedTranslations).allSatisfy { pair in
                abs(pair.0 - pair.1) < 0.01
            }

        if phase == .commitSettle || phase == .cancelSettle,
           let settleTarget {
            settleFrameCount += 1
            if let lastSettleProgress {
                if settleTarget == 1, progress + 0.000_1 < lastSettleProgress {
                    backwardsFrameCount += 1
                } else if settleTarget == 0, progress - 0.000_1 > lastSettleProgress {
                    backwardsFrameCount += 1
                }
            }
            if let lastFrameTimestamp {
                maximumFrameGapMilliseconds = max(
                    maximumFrameGapMilliseconds,
                    (timestamp - lastFrameTimestamp) * 1_000
                )
            }
            lastSettleProgress = progress
            lastFrameTimestamp = timestamp
        }
        container.view.bringSubviewToFront(statusLabel)
        publishStatus(in: container)
    }

    private func publishStatus(in container: GaryxRouteStackContainer? = nil) {
        let curveStatus = checkedFrameCount == 0 ? "idle" : (curvePassed ? "pass" : "fail")
        let status = [
            "depth=\(depth)",
            "phase=\(phase.rawValue)",
            "terminal=\(terminalOutcome)",
            "transactions=\(transactionCount)",
            "regrabs=\(regrabCount)",
            "curve=\(curveStatus)",
            "frames=\(checkedFrameCount)",
            "settleFrames=\(settleFrameCount)",
            "maxGapMs=\(String(format: "%.2f", maximumFrameGapMilliseconds))",
            "backwards=\(backwardsFrameCount)",
            "liveAdapters=\(liveAdapterCount)",
            "focusedAdapters=\(focusedAdapterCount)",
            "focusAtStart=\(focusAtTransactionStart)",
            "focusLoss=\(focusLossPhase)",
        ].joined(separator: ";")
        statusLabel.text = status
        statusLabel.accessibilityValue = status
        if let container {
            container.view.bringSubviewToFront(statusLabel)
        }
    }

    private func composerAdapters(in root: UIView) -> [GaryxComposerOrderedTextView] {
        var result: [GaryxComposerOrderedTextView] = []
        var pending = [root]
        while let view = pending.popLast() {
            if let adapter = view as? GaryxComposerOrderedTextView {
                result.append(adapter)
            }
            pending.append(contentsOf: view.subviews)
        }
        return result
    }
}
#endif

private struct GaryxProductionRouteHostEnvironment: View {
    @ObservedObject var model: GaryxMobileModel
    let content: AnyView
    let onOpenDrawer: @MainActor () -> Void

    var body: some View {
        content
            .environmentObject(model)
            .environment(model.homeObservationStore)
            .environment(\.garyxAvatarImageProvider, model.avatarImageProvider)
            .environment(\.garyxAvatarScopeId, model.currentGatewayScopeId)
            .environment(\.garyxOpenSidebar, onOpenDrawer)
            .garyxAccessibilityPreferences()
    }
}

private extension GaryxRoutePresentationNode {
    var occurrenceID: GaryxRouteInstanceID? {
        guard case .entry(let entry) = self else { return nil }
        return entry.id
    }

    var composerKey: GaryxComposerKey? {
        guard case .entry(let entry) = self else { return nil }
        return entry.destination.composerKey
    }
}
