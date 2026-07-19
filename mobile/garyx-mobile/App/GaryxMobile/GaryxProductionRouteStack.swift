import SwiftUI
import UIKit

/// Main-actor bridge between product navigation intents and the UIKit-owned
/// canonical stack. The container remains the only path writer; this store is
/// its observable projection for application state and tests.
@MainActor
final class GaryxProductionRouteStore: ObservableObject {
    struct NavigationPreparation {
        let ticket: GaryxNavigationPreparationTicket
        let source: GaryxMobilePanelOpenSource
        let animated: Bool
        let dependency: GaryxNavigationIntentDependency
    }

    struct NavigationSubmission {
        let result: GaryxNavigationQueueResult
        let entries: [GaryxRouteEntry]
    }

    private struct PendingRoutePlan {
        let intent: GaryxPreparedNavigationIntent
        let preparedScope: GaryxGatewayScope
        let entries: [GaryxRouteEntry]
        let source: GaryxMobilePanelOpenSource
        let animated: Bool
        let onVisible: (() -> Void)?
    }

    @Published private(set) var path: [GaryxRouteEntry] = []
    @Published private(set) var hasPresentationBarrier = false
    let presentationCoordinator = GaryxPresentationLeaseCoordinator()

    private weak var container: GaryxRouteStackContainer?
    private var canonicalProjection = GaryxCanonicalRouteState()
    private var intentCoordinator = GaryxNavigationIntentCoordinator()
    private var pendingPlans: [GaryxNavigationIntentID: PendingRoutePlan] = [:]
    private var activePlan: PendingRoutePlan?
    var presentationBarrierActivated: @MainActor () -> Void = {}
    private var navigationScopes: @MainActor () -> GaryxGatewayScopeRegistry = {
        GaryxGatewayScopeRegistry(
            initialActiveScope: GaryxGatewayScope(identity: "route-runtime", epoch: 1)
        )
    }
    private var hasNavigationScopeProvider = false
    private var lastKnownNavigationScopes = GaryxGatewayScopeRegistry(
        initialActiveScope: GaryxGatewayScope(identity: "route-runtime", epoch: 1)
    )

    var isAttached: Bool { container != nil }

    func configureNavigationScopes(
        _ provider: @escaping @MainActor () -> GaryxGatewayScopeRegistry
    ) {
        navigationScopes = provider
        hasNavigationScopeProvider = true
    }

    func attach(_ container: GaryxRouteStackContainer) {
        self.container = container
        presentationCoordinator.attach(container: container, routeStore: self)
        if container.path != path {
            _ = container.requestHardSnap(to: path)
        }
    }

    func detach(_ container: GaryxRouteStackContainer) {
        if self.container === container {
            presentationCoordinator.detach(container: container)
            self.container = nil
        }
    }

    func beginNavigationPreparation(
        source: GaryxMobilePanelOpenSource,
        animated: Bool = true,
        scopes explicitScopes: GaryxGatewayScopeRegistry? = nil
    ) -> NavigationPreparation {
        let scopes = effectiveScopes(explicitScopes)
        let scope = scopes.activeScope!
        let id = GaryxNavigationIntentID(rawValue: UUID().uuidString.lowercased())
        let dependency: GaryxNavigationIntentDependency
        if source == .current, let top = canonicalProjection.path.last {
            dependency = .relative(
                base: top.id,
                payloadRevision: top.payloadRevision,
                stackRevision: canonicalProjection.stackRevision,
                mismatch: .discard
            )
        } else {
            dependency = .absolute
        }
        return NavigationPreparation(
            ticket: intentCoordinator.beginPreparation(
                intentID: id,
                key: .ordinaryNavigation,
                scope: scope
            ),
            source: source,
            animated: animated,
            dependency: dependency
        )
    }

    @discardableResult
    func completeNavigationPreparation(
        _ preparation: NavigationPreparation,
        outcome: GaryxPrepareOutcome<[GaryxRouteDestination]>,
        scopes explicitScopes: GaryxGatewayScopeRegistry? = nil,
        onVisible: (() -> Void)? = nil
    ) -> NavigationSubmission {
        let scopes = effectiveScopes(explicitScopes)
        let preparedOutcome: GaryxPrepareOutcome<GaryxPreparedNavigationIntent>
        var entries: [GaryxRouteEntry] = []

        switch outcome {
        case .ready(let destinations):
            guard let terminalDestination = destinations.last else {
                preparedOutcome = .internalFault(code: "empty_route_plan")
                break
            }
            entries = destinations.map { destination in
                GaryxRouteEntry(
                    id: GaryxRouteInstanceID(rawValue: UUID().uuidString.lowercased()),
                    destination: destination
                )
            }
            let intent = GaryxPreparedNavigationIntent(
                id: preparation.ticket.intentID,
                effect: .ordinaryNavigation(terminalDestination),
                dependency: preparation.dependency
            )
            pendingPlans[intent.id] = PendingRoutePlan(
                intent: intent,
                preparedScope: preparation.ticket.epoch.scope,
                entries: entries,
                source: preparation.source,
                animated: preparation.animated,
                onVisible: onVisible
            )
            preparedOutcome = .ready(intent)
        case .userVisibleNotFound:
            preparedOutcome = .userVisibleNotFound
        case .retryableFailure(let message):
            preparedOutcome = .retryableFailure(message: message)
        case .authenticationRequired:
            preparedOutcome = .authenticationRequired
        case .cancelledOrStale:
            preparedOutcome = .cancelledOrStale
        case .internalFault(let code):
            preparedOutcome = .internalFault(code: code)
        }

        let result = intentCoordinator.completePreparation(
            preparation.ticket,
            outcome: preparedOutcome,
            scopes: scopes,
            routeState: canonicalProjection,
            presentationBarrier: container?.hasPresentationBarrier ?? false
        )
        discardPlansNoLongerQueued()
        if result == .admittedImmediately {
            drainAdmissiblePlans()
        }
        return NavigationSubmission(result: result, entries: entries)
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
                    applyCanonicalPath(Array(path.dropLast(descendants)))
                }
            }
            return existing
        }

        let preparation = beginNavigationPreparation(source: source, animated: animated)
        let submission = completeNavigationPreparation(
            preparation,
            outcome: .ready([destination])
        )
        return submission.entries[0]
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
        let preparation = beginNavigationPreparation(source: source, animated: animated)
        return completeNavigationPreparation(
            preparation,
            outcome: .ready(destinations)
        ).entries
    }

    func resetToHome(animated: Bool = true) {
        guard !path.isEmpty else { return }
        if let container {
            _ = container.pop(count: path.count, animated: animated)
        } else {
            applyCanonicalPath([])
        }
    }

    func popOne(animated: Bool = true) {
        guard !path.isEmpty else { return }
        if let container {
            _ = container.pop(animated: animated)
        } else {
            applyCanonicalPath(Array(path.dropLast()))
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
        applyCanonicalPath(replacement)
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
        applyCanonicalPath(replacement)
        return true
    }

    func applyCanonicalPath(_ canonicalPath: [GaryxRouteEntry]) {
        guard path != canonicalPath else { return }
        let topologyChanged = path.map(\.id) != canonicalPath.map(\.id)
        canonicalProjection = GaryxCanonicalRouteState(
            path: canonicalPath,
            stackRevision: topologyChanged
                ? canonicalProjection.stackRevision &+ 1
                : canonicalProjection.stackRevision
        )
        path = canonicalPath
    }

    func routePhaseChanged(_ phase: GaryxPresentationTransactionPhase) {
        switch phase {
        case .active:
            intentCoordinator.setTransactionStatus(.terminal)
        case .preCommit, .cancelSettle, .commitSettle, .terminal:
            intentCoordinator.setTransactionStatus(.nonTerminal)
        }
    }

    func visibleRouteActivated(_ node: GaryxRoutePresentationNode) {
        guard let activePlan,
              case .entry(let entry) = node,
              activePlan.entries.last?.id == entry.id else { return }
        self.activePlan = nil
        activePlan.onVisible?()
    }

    func rendererBecameIdle() {
        intentCoordinator.setTransactionStatus(.terminal)
        drainAdmissiblePlans()
    }

    func presentationBarrierDidChange() {
        hasPresentationBarrier = container?.hasPresentationBarrier ?? false
        guard container?.hasPresentationBarrier != true else { return }
        drainAdmissiblePlans()
    }

    func presentationBarrierStateChanged(_ active: Bool) {
        guard hasPresentationBarrier != active else { return }
        hasPresentationBarrier = active
        if active {
            presentationBarrierActivated()
        }
    }

    func sceneDidBecomeInactive() {
        container?.sceneDidBecomeInactive()
    }

    func sceneDidBecomeActive() {
        container?.sceneDidBecomeActive()
    }

    private func drainAdmissiblePlans() {
        let hasBarrier = container?.hasPresentationBarrier ?? false
        let intents = intentCoordinator.drainAdmissible(presentationBarrier: hasBarrier)
        guard !intents.isEmpty else { return }

        for intent in intents {
            guard let plan = pendingPlans.removeValue(forKey: intent.id) else { continue }
            let scopes = hasNavigationScopeProvider
                ? navigationScopes()
                : lastKnownNavigationScopes
            guard scopes.activeScope == plan.preparedScope,
                  scopes.lifecycle(of: plan.preparedScope) == .active else {
                continue
            }
            guard intentCoordinator.dependencyDisposition(
                for: intent,
                routeState: canonicalProjection
            ) == .admit else {
                continue
            }
            execute(plan)
        }
        discardPlansNoLongerQueued()
    }

    private func execute(_ plan: PendingRoutePlan) {
        activePlan = plan
        guard let container else {
            let nextPath = plan.source == .current
                ? path + plan.entries
                : plan.entries
            applyCanonicalPath(nextPath)
            activePlan = nil
            plan.onVisible?()
            return
        }

        let accepted: Bool
        if plan.source == .current
            || (plan.source == .sidebar && path.isEmpty)
            || (plan.source == .replace && path.isEmpty) {
            accepted = container.push(plan.entries, animated: plan.animated)
        } else {
            accepted = container.requestHardSnap(to: plan.entries)
        }
        if !accepted {
            activePlan = nil
            assertionFailure("terminal navigation plan was rejected by the route renderer")
        }
    }

    private func discardPlansNoLongerQueued() {
        var retained = Set(intentCoordinator.queued.map(\.id))
        if let activePlan { retained.insert(activePlan.intent.id) }
        pendingPlans = pendingPlans.filter { retained.contains($0.key) }
    }

    private func effectiveScopes(
        _ explicit: GaryxGatewayScopeRegistry?
    ) -> GaryxGatewayScopeRegistry {
        var scopes = explicit ?? navigationScopes()
        if scopes.activeScope == nil {
            let identity = "route-runtime"
            let epoch = (scopes.revokedThroughEpoch[identity] ?? 0) &+ 1
            _ = scopes.switchActive(
                to: GaryxGatewayScope(identity: identity, epoch: max(1, epoch))
            )
        }
        lastKnownNavigationScopes = scopes
        return scopes
    }
}

/// Full-screen SwiftUI boundary for the UIKit route renderer.
///
/// SwiftUI otherwise proposes only the safe-area-sized region to an embedded
/// view-controller representable. That clips every route host, including page
/// content and glass materials which intentionally render behind system chrome.
struct GaryxProductionRouteCanvas: View {
    @ObservedObject var store: GaryxProductionRouteStore
    let model: GaryxMobileModel
    let homeContent: AnyView
    let routeContent: @MainActor (GaryxRoutePresentationNode) -> AnyView
    let onOpenDrawer: @MainActor () -> Void

    var body: some View {
        GaryxProductionRouteStack(
            store: store,
            model: model,
            homeContent: homeContent,
            routeContent: routeContent,
            onOpenDrawer: onOpenDrawer
        )
        .ignoresSafeArea(.container)
    }
}

private struct GaryxProductionRouteStack: UIViewControllerRepresentable {
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
        callbacks.phaseChanged = { [weak store] phase in
            store?.routePhaseChanged(phase)
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
        callbacks.visibleRouteActivated = { [weak store] node in
            store?.visibleRouteActivated(node)
        }
        callbacks.rendererBecameIdle = { [weak store] in
            store?.rendererBecameIdle()
        }
        store.configureNavigationScopes { [weak model] in
            model?.gatewayScopeRegistry ?? GaryxGatewayScopeRegistry(
                initialActiveScope: GaryxGatewayScope(identity: "route-runtime", epoch: 1)
            )
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
                        store: store,
                        node: node,
                        content: content,
                        onOpenDrawer: onOpenDrawer
                    )
                )
            }
        )
        container.layoutDirectionOverride = layoutDirection == .rightToLeft
            ? .rightToLeft
            : .leftToRight
        let drawerInteraction = model.drawerRevealInteraction
        container.homeLeadingEdgeInteraction = GaryxRouteEdgePanInteraction(
            isEligible: {
                drawerInteraction.isGestureEligible
            },
            requiresEdgeZone: {
                drawerInteraction.requiresEdgeZone
            },
            acceptedDirection: {
                drawerInteraction.acceptedDirection
            },
            began: {
                GaryxMobileHaptics.shared.prepare(.drawerVisibilityCommitted)
                drawerInteraction.beginGesture()
            },
            changed: { translation, _ in
                drawerInteraction.updateGesture(logicalTranslation: translation)
            },
            ended: { [weak model] velocity in
                guard let target = drawerInteraction.endGesture(logicalVelocity: velocity)
                else { return }
                model?.setSidebarVisible(target == .open, animated: true)
            },
            cancelled: { [weak model] in
                guard let target = drawerInteraction.cancelGesture() else { return }
                model?.setSidebarVisible(target == .open, animated: true)
            }
        )

        let taskTreeInteraction = model.taskTreeRevealInteraction
        container.trailingEdgeInteraction = GaryxRouteEdgePanInteraction(
            isEligible: { [weak model] in
                guard let model,
                      taskTreeInteraction.isGestureEligible,
                      !drawerInteraction.isDragging,
                      store.path.last?.destination.composerKey != nil else { return false }
                if taskTreeInteraction.presentation.phase != .idle
                    || abs(taskTreeInteraction.reveal) > 0.5 {
                    return true
                }
                return model.selectedThread != nil
                    && (model.taskTreeForestPage == nil || model.isTaskTreeSidebarAvailable)
            },
            requiresEdgeZone: {
                taskTreeInteraction.requiresEdgeZone
            },
            acceptedDirection: {
                taskTreeInteraction.acceptedDirection
            },
            began: {
                GaryxMobileHaptics.shared.prepare(.taskTreeVisibilityCommitted)
                taskTreeInteraction.beginGesture()
            },
            changed: { translation, _ in
                taskTreeInteraction.updateGesture(logicalTranslation: translation)
            },
            ended: { [weak model] velocity in
                guard let model,
                      let target = taskTreeInteraction.endGesture(logicalVelocity: velocity)
                else { return }
                if target == .open {
                    model.openTaskTreeSidebar()
                } else {
                    model.closeTaskTreeSidebar()
                }
            },
            cancelled: { [weak model] in
                guard let model,
                      let target = taskTreeInteraction.cancelGesture() else { return }
                if target == .open {
                    model.openTaskTreeSidebar()
                } else {
                    model.closeTaskTreeSidebar()
                }
            }
        )
        container.interactivePopEligible = { [weak model] in
            guard let model else { return false }
            let taskTree = model.taskTreeRevealInteraction
            return !model.isTaskTreeSidebarOpen
                && taskTree.presentation.phase == .idle
                && taskTree.presentation.target == .closed
                && abs(taskTree.reveal) <= 0.5
        }
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
    @ObservedObject var store: GaryxProductionRouteStore
    @Environment(\.garyxRouteContext) private var routeContext
    let node: GaryxRoutePresentationNode
    let content: AnyView
    let onOpenDrawer: @MainActor () -> Void

    var body: some View {
        content
            .environmentObject(model)
            .environment(model.homeObservationStore)
            .environment(\.garyxAvatarImageProvider, model.avatarImageProvider)
            .environment(\.garyxAvatarScopeId, model.currentGatewayScopeId)
            .environment(\.garyxOpenSidebar, onOpenDrawer)
            .environment(\.garyxRouteNavigationActions, navigationActions)
            .environment(
                \.garyxPresentationLeaseCoordinator,
                store.presentationCoordinator
            )
            .modifier(
                GaryxRouteEscapeActionModifier(
                    isEnabled: GaryxRouteAccessibilityGate.allowsEscape(
                        isCanonicalTop: routeContext.isCanonicalTop,
                        lifecycle: routeContext.lifecycle,
                        hasPresentationBarrier: store.hasPresentationBarrier
                    ) && navigationActions.dismiss != nil,
                    action: { navigationActions.dismiss?() }
                )
            )
            .garyxAccessibilityPreferences()
    }

    private var navigationActions: GaryxRouteNavigationActions {
        let dismiss: (() -> Void)?
        switch node {
        case .home:
            dismiss = nil
        case .entry:
            dismiss = {
                guard GaryxRouteAccessibilityGate.allowsEscape(
                    isCanonicalTop: routeContext.isCanonicalTop,
                    lifecycle: routeContext.lifecycle,
                    hasPresentationBarrier: store.hasPresentationBarrier
                ) else { return }
                store.popOne()
            }
        }
        return GaryxRouteNavigationActions(
            dismiss: dismiss,
            push: { destinations in
                guard routeContext.allowsPageInteraction, !destinations.isEmpty else { return }
                _ = store.open(destinations, source: .current)
            },
            backLabel: backLabel
        )
    }

    private var backLabel: String {
        guard case .entry(let entry) = node,
              let index = store.path.firstIndex(where: { $0.id == entry.id }),
              index > 0 else {
            return "Back"
        }
        return store.path[index - 1].destination.backNavigationLabel
    }
}

private struct GaryxRouteEscapeActionModifier: ViewModifier {
    let isEnabled: Bool
    let action: () -> Void

    func body(content: Content) -> some View {
        // Keep the presentation anchor's SwiftUI identity stable when a
        // lease publishes its modal barrier. Switching between differently
        // shaped modifier branches here can tear down a sheet/full-screen
        // cover in the same update that synchronously acquired its lease.
        content.accessibilityAction(.escape) {
            guard isEnabled else { return }
            action()
        }
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
