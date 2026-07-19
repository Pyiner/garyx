import QuartzCore
import SwiftUI
import UIKit

struct GaryxRouteStackContainerCallbacks {
    var hostMounted: @MainActor (GaryxRoutePresentationIdentity) -> Void = { _ in }
    var hostUnmounted: @MainActor (GaryxRoutePresentationIdentity) -> Void = { _ in }
    var hostLifecycleChanged: @MainActor (
        GaryxRoutePresentationIdentity,
        GaryxRouteHostLifecyclePhase
    ) -> Void = { _, _ in }
    var phaseChanged: @MainActor (GaryxPresentationTransactionPhase) -> Void = { _ in }
    var commitReleased: @MainActor (
        GaryxRoutePresentationNode,
        GaryxRoutePresentationNode
    ) -> Void = { _, _ in }
    var canonicalPathChanged: @MainActor ([GaryxRouteEntry]) -> Void = { _ in }
    var terminalReached: @MainActor (GaryxPresentationTerminalState) -> Void = { _ in }
    var visibleRouteActivated: @MainActor (GaryxRoutePresentationNode) -> Void = { _ in }
    var rendererBecameIdle: @MainActor () -> Void = {}
    var screenChanged: @MainActor (UIView) -> Void = { view in
        UIAccessibility.post(notification: .screenChanged, argument: view)
    }
    var budgetFault: @MainActor (GaryxRouteStateStoreMetrics) -> Void = { _ in }
}

@MainActor
struct GaryxRouteStackContainerMetrics: Equatable {
    let mountedHostCount: Int
    let peakMountedHostCount: Int
    let pooledWrapperCount: Int
    let transitionPhase: GaryxPresentationTransactionPhase
    let transitionProgress: CGFloat?
    let stateStore: GaryxRouteStateStoreMetrics
}

/// The transform boundary for one mounted SwiftUI route host.
///
/// The hosting controller's view always keeps identity geometry. Interactive
/// and settle frames write only this wrapper, with implicit Core Animation
/// actions disabled by the caller.
@MainActor
final class GaryxRouteTransitionWrapperView: UIView {
    private(set) var representedIdentity: GaryxRoutePresentationIdentity?
    let contentView = UIView()
    let scrimView = UIView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = true
        backgroundColor = .systemBackground
        contentView.frame = bounds
        contentView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        contentView.backgroundColor = .systemBackground
        contentView.clipsToBounds = true
        addSubview(contentView)

        scrimView.frame = bounds
        scrimView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        scrimView.backgroundColor = .black
        scrimView.alpha = 0
        scrimView.isUserInteractionEnabled = false
        addSubview(scrimView)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func prepareForReuse(identity: GaryxRoutePresentationIdentity) {
        detachHostedView()
        resetTransitionState()
        representedIdentity = identity
        isHidden = false
        accessibilityElementsHidden = false
        isUserInteractionEnabled = true
        assert(!hasTransitionResidue, "route wrapper identity reuse retained transition state")
    }

    func attachHostedView(_ hostedView: UIView) {
        assert(contentView.subviews.isEmpty, "route wrapper must contain exactly one host view")
        hostedView.frame = contentView.bounds
        hostedView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        contentView.addSubview(hostedView)
        bringSubviewToFront(scrimView)
    }

    func detachHostedView() {
        contentView.subviews.forEach { $0.removeFromSuperview() }
    }

    func resetTransitionState() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        transform = .identity
        alpha = 1
        scrimView.alpha = 0
        layer.shadowOpacity = 0
        layer.shadowOffset = .zero
        layer.shadowRadius = 0
        layer.shadowPath = nil
        layer.shouldRasterize = false
        CATransaction.commit()
    }

    func apply(
        translationX: CGFloat,
        alpha: CGFloat,
        scrimAlpha: CGFloat,
        shadowOpacity: Float,
        shadowOffsetX: CGFloat
    ) {
        transform = CGAffineTransform(translationX: translationX, y: 0)
        self.alpha = alpha
        scrimView.alpha = scrimAlpha
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = shadowOpacity
        layer.shadowOffset = CGSize(width: shadowOffsetX, height: 0)
        layer.shadowRadius = shadowOpacity > 0 ? 8 : 0
        layer.shadowPath = shadowOpacity > 0
            ? UIBezierPath(rect: bounds).cgPath
            : nil
    }

    var hasTransitionResidue: Bool {
        transform != .identity
            || abs(alpha - 1) > 0.000_001
            || scrimView.alpha > 0.000_001
            || layer.shadowOpacity > 0.000_001
            || layer.shadowOffset != .zero
            || layer.shadowRadius > 0.000_001
            || layer.shadowPath != nil
            || layer.shouldRasterize
    }
}

@MainActor
struct GaryxRouteEdgePanInteraction {
    var isEligible: () -> Bool
    var requiresEdgeZone: () -> Bool
    var acceptedDirection: () -> GaryxRouteGestureDirection
    var began: () -> Void
    var changed: (_ logicalTranslation: CGFloat, _ logicalVelocity: CGFloat) -> Void
    var ended: (_ logicalVelocity: CGFloat) -> Void
    var cancelled: () -> Void
}

/// UIKit-owned route renderer for the fluid-navigation stack.
///
/// A4a established this renderer with fake routes; A4b also uses it for the
/// production home/conversation content stack.
@MainActor
final class GaryxRouteStackContainer: UIViewController, UIGestureRecognizerDelegate {
    typealias HostBuilder = @MainActor (GaryxRoutePresentationNode) -> AnyView

    static let maximumMountedHostCount = 4
    static let maximumPooledWrapperCount = 2

    /// Public UIKit edge adapters. Edge ownership is intentionally decided
    /// from the delegate's touch-down snapshot instead of UIKit's private
    /// UIScreenEdgePan activation zone.
    let leadingEdgePanGestureRecognizer = UIPanGestureRecognizer()
    let trailingEdgePanGestureRecognizer = UIPanGestureRecognizer()

    private final class HostRecord {
        let identity: GaryxRoutePresentationIdentity
        var node: GaryxRoutePresentationNode
        let controller: UIHostingController<AnyView>
        let wrapper: GaryxRouteTransitionWrapperView
        let contextStore: GaryxRouteHostContextStore
        var lifecycle = GaryxRouteHostLifecycle()
        var lastAccess: UInt64

        init(
            identity: GaryxRoutePresentationIdentity,
            node: GaryxRoutePresentationNode,
            controller: UIHostingController<AnyView>,
            wrapper: GaryxRouteTransitionWrapperView,
            contextStore: GaryxRouteHostContextStore,
            lastAccess: UInt64
        ) {
            self.identity = identity
            self.node = node
            self.controller = controller
            self.wrapper = wrapper
            self.contextStore = contextStore
            self.lastAccess = lastAccess
        }
    }

    private enum PendingCanonicalMutation {
        case push([GaryxRouteEntry])
        case pop(Int)
    }

    private enum EdgePanOwner {
        case routePop
        case homeLeading
        case trailing
    }

    private struct PendingPayloadReplacement {
        let expected: GaryxRouteDestination
        let replacement: GaryxRouteDestination
    }

    private let hostBuilder: HostBuilder
    private let preferencesProvider: @MainActor () -> GaryxRouteVisualPreferences
    private let settleDriver: GaryxGestureSettleDriver
    private var callbacks: GaryxRouteStackContainerCallbacks
    private var canonicalState: GaryxCanonicalRouteState
    private var hosts: [GaryxRoutePresentationIdentity: HostRecord] = [:]
    private var wrapperPool: [GaryxRouteTransitionWrapperView] = []
    private var accessClock: UInt64 = 0
    private var peakMountedHostCount = 0
    private var stateStore = GaryxRouteStateStore()
    private var transition: GaryxRouteTransitionSession?
    private var pendingMutation: PendingCanonicalMutation?
    private var committedRemovedIdentities: Set<GaryxRoutePresentationIdentity> = []
    private var gestureBaseProgress: CGFloat = 0
    private var leadingTouchSnapshot: GaryxRouteEdgeTouchSnapshot?
    private var trailingTouchSnapshot: GaryxRouteEdgeTouchSnapshot?
    private var leadingPanOwner: EdgePanOwner?
    private var trailingPanOwner: EdgePanOwner?
    private var sceneIsActive = true
    private var deferredTerminalEffects: GaryxPresentationTerminalState?
    private var interactionFrozenAfterTerminal = false
    private var screenChangedDelivered = false
    private var presentationLeases = GaryxPresentationLeaseTree()
    private var pendingHardSnapPath: [GaryxRouteEntry]?
    private var pendingPayloadReplacements: [
        GaryxRouteInstanceID: PendingPayloadReplacement
    ] = [:]
    private var isTearingDown = false
    private let interactionShield = UIView()

    var layoutDirectionOverride: GaryxRouteLayoutDirection? {
        didSet {
            guard isViewLoaded else { return }
            applyLayoutDirectionOverride()
            rederiveWrapperGeometry()
        }
    }

    var homeLeadingEdgeInteraction: GaryxRouteEdgePanInteraction?
    var trailingEdgeInteraction: GaryxRouteEdgePanInteraction?
    var interactivePopEligible: @MainActor () -> Bool = { true }
    var gestureDiagnostic: (@MainActor (String) -> Void)?
    var transitionFrameObserver: (@MainActor (
        GaryxPresentationTransactionPhase,
        CGFloat,
        CFTimeInterval
    ) -> Void)?

    init(
        initialPath: [GaryxRouteEntry] = [],
        settleDriver: GaryxGestureSettleDriver? = nil,
        callbacks: GaryxRouteStackContainerCallbacks = .init(),
        preferencesProvider: @escaping @MainActor () -> GaryxRouteVisualPreferences,
        hostBuilder: @escaping HostBuilder
    ) {
        canonicalState = GaryxCanonicalRouteState(path: initialPath)
        self.settleDriver = settleDriver ?? .displayLinked()
        self.callbacks = callbacks
        self.preferencesProvider = preferencesProvider
        self.hostBuilder = hostBuilder
        super.init(nibName: nil, bundle: nil)

        leadingEdgePanGestureRecognizer.addTarget(
            self,
            action: #selector(handleLeadingEdgePan(_:))
        )
        trailingEdgePanGestureRecognizer.addTarget(
            self,
            action: #selector(handleTrailingEdgePan(_:))
        )
        configure(recognizer: leadingEdgePanGestureRecognizer)
        configure(recognizer: trailingEdgePanGestureRecognizer)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    deinit {
        MainActor.assumeIsolated {
            tearDownRenderer()
        }
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        view.clipsToBounds = true
        installEdgeRecognizers(on: view)
        applyLayoutDirectionOverride()
        mountInitialHosts()
        interactionShield.frame = view.bounds
        interactionShield.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        interactionShield.backgroundColor = .clear
        interactionShield.isAccessibilityElement = false
        interactionShield.accessibilityElementsHidden = true
        interactionShield.isHidden = true
        view.addSubview(interactionShield)
        refreshGestureAvailability()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        // The home drawer is a SwiftUI sibling of the route controller. Hosting
        // both public edge recognizers on the shared window gives drawer,
        // route-pop, task-tree, and descendant pans one failure graph.
        installEdgeRecognizers(on: view.window ?? view)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        // A root controller can enter a visible UIWindow without receiving a
        // synchronous appearance callback (notably during scene restoration
        // and app-host tests). Keep the public recognizers in the shared
        // window failure graph as soon as layout proves that host exists.
        if let window = view.window {
            installEdgeRecognizers(on: window)
        }
        updateRecognizerEdges()
        rederiveWrapperGeometry()
    }

    var path: [GaryxRouteEntry] { canonicalState.path }

    var metrics: GaryxRouteStackContainerMetrics {
        GaryxRouteStackContainerMetrics(
            mountedHostCount: hosts.count,
            peakMountedHostCount: peakMountedHostCount,
            pooledWrapperCount: wrapperPool.count,
            transitionPhase: transition?.coordinator.phase ?? .active,
            transitionProgress: transition?.progress,
            stateStore: stateStore.metrics
        )
    }

    var visualPolicyForActiveTransaction: GaryxRouteVisualPolicy? {
        transition?.visualPolicy
    }

    var mountedHostIdentities: Set<GaryxRoutePresentationIdentity> {
        Set(hosts.keys)
    }

    var hasTerminalResidue: Bool {
        transition != nil
            || settleDriver.isSettling
            || pendingMutation != nil
            || hosts.values.contains(where: { $0.wrapper.hasTransitionResidue })
    }

    // MARK: Public fake-route operations

    @discardableResult
    func push(_ entry: GaryxRouteEntry, animated: Bool = true) -> Bool {
        push([entry], animated: animated)
    }

    /// Appends a route suffix as one presentation transaction. Only the last
    /// entry is rendered during the push; intermediate entries become real
    /// canonical predecessors and are mounted if a later pop reveals them.
    @discardableResult
    func push(_ entries: [GaryxRouteEntry], animated: Bool = true) -> Bool {
        loadViewIfNeeded()
        guard transition == nil, let destinationEntry = entries.last else { return false }
        precondition(
            Set(entries.map(\.id)).count == entries.count,
            "batch push occurrence IDs must be unique"
        )
        precondition(
            canonicalState.path.allSatisfy { existing in
                !entries.contains(where: { $0.id == existing.id })
            },
            "batch push occurrence ID reused"
        )
        let source = canonicalState.topNode
        let destination = GaryxRoutePresentationNode.entry(destinationEntry)
        guard beginTransition(
            kind: .push,
            source: source,
            destination: destination,
            mutation: .push(entries)
        ) else { return false }
        releaseProgrammaticCommit(animated: animated)
        return true
    }

    @discardableResult
    func pop(count: Int = 1, animated: Bool = true) -> Bool {
        loadViewIfNeeded()
        guard transition == nil, count > 0, !canonicalState.path.isEmpty else { return false }
        let removalCount = min(count, canonicalState.path.count)
        let destinationIndex = canonicalState.path.count - removalCount - 1
        let destination: GaryxRoutePresentationNode = destinationIndex >= 0
            ? .entry(canonicalState.path[destinationIndex])
            : .home
        guard beginTransition(
            kind: .pop,
            source: canonicalState.topNode,
            destination: destination,
            mutation: .pop(removalCount)
        ) else { return false }
        releaseProgrammaticCommit(animated: animated)
        return true
    }

    @discardableResult
    func beginInteractivePop() -> Bool {
        loadViewIfNeeded()
        guard transition == nil, !canonicalState.path.isEmpty else { return false }
        guard beginTransition(
            kind: .pop,
            source: canonicalState.topNode,
            destination: canonicalState.predecessorNode,
            mutation: .pop(1)
        ) else { return false }
        gestureBaseProgress = 0
        return true
    }

    func updateInteractivePop(logicalTranslation: CGFloat) {
        guard var transition, transition.coordinator.phase == .preCommit else { return }
        let progress = GaryxRouteEdgeGestureArbitrator.progress(
            logicalTranslation: logicalTranslation,
            viewportWidth: viewportWidth
        )
        guard transition.update(progress: progress) else { return }
        self.transition = transition
        applyTransitionVisualState()
    }

    @discardableResult
    func endInteractivePop(logicalVelocity: CGFloat) -> GaryxPresentationTerminalOutcome? {
        guard var transition, transition.coordinator.phase == .preCommit else { return nil }
        let logicalTranslation = transition.progress * viewportWidth
        guard let outcome = transition.release(
            logicalTranslation: logicalTranslation,
            logicalVelocity: logicalVelocity,
            viewportWidth: viewportWidth
        ) else { return nil }
        self.transition = transition
        callbacks.phaseChanged(transition.coordinator.phase)
        if outcome == .committed {
            callbacks.commitReleased(transition.source, transition.destination)
            commitPendingCanonicalMutation()
            deactivateSourceAtCommitBoundary()
        }
        startSettle(
            logicalVelocity: logicalVelocity,
            curve: GaryxRouteTransitionCalibration.settleCurve
        )
        return outcome
    }

    func cancelInteractivePop() {
        guard var transition, transition.coordinator.phase == .preCommit else { return }
        guard transition.handle(.recognizerCancelled) == .transitioned(.cancelSettle) else {
            return
        }
        self.transition = transition
        callbacks.phaseChanged(.cancelSettle)
        startSettle(
            logicalVelocity: 0,
            curve: GaryxRouteTransitionCalibration.programmaticSettleCurve
        )
    }

    @discardableResult
    func regrabCancelSettle() -> GaryxMotionPhysics.MotionSample? {
        guard let interruption = settleDriver.interrupt(), var transition,
              transition.regrabCancelSettle(progress: interruption.value)
        else { return nil }
        self.transition = transition
        gestureBaseProgress = interruption.value
        callbacks.phaseChanged(.preCommit)
        applyTransitionVisualState()
        refreshGestureAvailability()
        return .init(value: interruption.value, velocity: interruption.velocity)
    }

    /// Used by deterministic fake-host tests and hard-snap paths. Production
    /// gesture settling still completes from the display link.
    func completeSettleImmediately() {
        guard var transition,
              transition.coordinator.phase == .cancelSettle
                || transition.coordinator.phase == .commitSettle
        else { return }
        settleDriver.invalidate()
        let target: CGFloat = transition.coordinator.phase == .commitSettle ? 1 : 0
        _ = transition.updateSettle(progress: target)
        self.transition = transition
        applyTransitionVisualState()
        finishTransition(visibility: currentSceneVisibility)
    }

    func sceneDidBecomeInactive() {
        sceneIsActive = false
        refreshGestureAvailability()
        guard var transition else {
            if let active = activeHostRecord() {
                deactivateIfActive(active)
            }
            return
        }
        let effect = transition.handle(.sceneInactive)
        self.transition = transition
        guard case .reachedTerminal(let terminal) = effect else { return }
        settleDriver.invalidate()
        if terminal.outcome == .cancelled, let source = host(for: transition.source) {
            deactivateIfActive(source)
        }
        finalizeForcedTerminal(terminal)
    }

    func sceneDidBecomeActive() {
        sceneIsActive = true
        if let deferredTerminalEffects {
            let owner = deferredTerminalEffects.outcome == .committed
                ? canonicalState.topNode
                : transition?.source ?? canonicalState.topNode
            var visibleCommittedDestination: HostRecord?
            if let record = host(for: owner) {
                activate(record)
                if deferredTerminalEffects.outcome == .committed {
                    visibleCommittedDestination = record
                }
            }
            self.deferredTerminalEffects = nil
            interactionFrozenAfterTerminal = false
            reconcileHostVisibility()
            refreshGestureAvailability()
            if let visibleCommittedDestination {
                callbacks.visibleRouteActivated(owner)
                emitScreenChangedOnce(for: visibleCommittedDestination)
            }
        } else if transition == nil, let record = activeHostRecord() {
            activate(record)
            refreshGestureAvailability()
        }
    }

    func supersedeActiveTransition() {
        guard var transition else { return }
        let effect = transition.handle(.routeInvalidated)
        self.transition = transition
        guard case .reachedTerminal(let terminal) = effect else { return }
        settleDriver.invalidate()
        finalizeForcedTerminal(terminal)
    }

    // MARK: Presentation leases and hard snap

    @discardableResult
    func acquirePresentationLease(
        _ token: GaryxPresentationLeaseToken,
        parent: GaryxPresentationLeaseToken? = nil,
        resultBearing: Bool = false
    ) -> Bool {
        let acquired = presentationLeases.acquire(
            token,
            parent: parent,
            resultBearing: resultBearing
        )
        refreshGestureAvailability()
        return acquired
    }

    @discardableResult
    func reclaimReleasedPresentationLeases() -> Int {
        presentationLeases.garbageCollectReleased()
    }

    func markPresentationLeasePresented(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.markPresented(token)
    }

    func markPresentationLeaseDismissing(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.markDismissing(token)
    }

    func recordPresentationResult(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.recordResult(token)
        presentationLeaseMayHaveReleased()
    }

    func recordPresentationNoResult(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.recordNoResult(token)
        presentationLeaseMayHaveReleased()
    }

    func presentationDismissalCompleted(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.dismissalCompleted(token)
        presentationLeaseMayHaveReleased()
    }

    func presentationFailed(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.presentationFailed(token)
        presentationLeaseMayHaveReleased()
    }

    func forceDismissPresentationSubtree(_ token: GaryxPresentationLeaseToken) {
        presentationLeases.forceDismissSubtree(token)
        presentationLeaseMayHaveReleased()
    }

    var hasPresentationBarrier: Bool { presentationLeases.hasBarrier }

    func presentationLeaseRecord(
        _ token: GaryxPresentationLeaseToken
    ) -> GaryxPresentationLeaseRecord? {
        presentationLeases.records[token]
    }

    @discardableResult
    func requestHardSnap(to replacement: [GaryxRouteEntry]) -> Bool {
        loadViewIfNeeded()
        guard !presentationLeases.hasBarrier else {
            pendingHardSnapPath = replacement
            return false
        }
        performHardSnap(to: replacement)
        return true
    }

    /// Replaces a draft's domain payload in place. The occurrence identity,
    /// host lifecycle, and wrapper geometry are deliberately unchanged.
    @discardableResult
    func promoteVisibleDraft(
        instanceID: GaryxRouteInstanceID,
        draftID: String,
        threadID: String
    ) -> Bool {
        let expected = GaryxRouteDestination.conversationDraft(draftID: draftID)
        let replacement = GaryxRouteDestination.conversation(threadID: threadID)
        if transition != nil {
            guard canonicalState.path.contains(where: {
                $0.id == instanceID && $0.destination == expected
            }) else { return false }
            pendingPayloadReplacements[instanceID] = PendingPayloadReplacement(
                expected: expected,
                replacement: replacement
            )
            return true
        }
        return replaceRoutePayload(
            instanceID: instanceID,
            expected: expected,
            replacement: replacement
        )
    }

    private func replaceRoutePayload(
        instanceID: GaryxRouteInstanceID,
        expected: GaryxRouteDestination,
        replacement: GaryxRouteDestination
    ) -> Bool {
        guard canonicalState.replaceRoutePayload(
            instanceID: instanceID,
            expected: expected,
            with: replacement
        ), let entry = canonicalState.path.first(where: { $0.id == instanceID }) else {
            return false
        }
        let identity = GaryxRoutePresentationIdentity.entry(instanceID)
        if let record = hosts[identity] {
            let node = GaryxRoutePresentationNode.entry(entry)
            record.node = node
            record.controller.rootView = wrappedHost(
                node: node,
                contextStore: record.contextStore
            )
            refreshContext(for: record)
        }
        callbacks.canonicalPathChanged(canonicalState.path)
        return true
    }

    private func applyPendingPayloadReplacements() {
        guard !pendingPayloadReplacements.isEmpty else { return }
        let pending = pendingPayloadReplacements
        pendingPayloadReplacements.removeAll()
        for (instanceID, replacement) in pending.sorted(by: {
            $0.key.rawValue < $1.key.rawValue
        }) {
            _ = replaceRoutePayload(
                instanceID: instanceID,
                expected: replacement.expected,
                replacement: replacement.replacement
            )
        }
    }

    /// A draft target/key switch is a payload replacement on the same route
    /// occurrence, but unlike promotion it is an input-session handoff. The
    /// old key is finalized before the replacement becomes canonical.
    @discardableResult
    func replaceVisibleDraftKey(
        instanceID: GaryxRouteInstanceID,
        oldDraftID: String,
        newDraftID: String
    ) -> Bool {
        guard transition == nil,
              let oldEntry = canonicalState.path.first(where: { $0.id == instanceID }),
              oldEntry.destination == .conversationDraft(draftID: oldDraftID) else {
            return false
        }
        var newEntry = oldEntry
        newEntry.replacePayload(with: .conversationDraft(draftID: newDraftID))
        let source = GaryxRoutePresentationNode.entry(oldEntry)
        let destination = GaryxRoutePresentationNode.entry(newEntry)
        callbacks.commitReleased(source, destination)
        guard canonicalState.replaceRoutePayload(
            instanceID: instanceID,
            expected: oldEntry.destination,
            with: newEntry.destination
        ) else { return false }
        if let record = hosts[.entry(instanceID)] {
            record.node = destination
            record.controller.rootView = wrappedHost(
                node: destination,
                contextStore: record.contextStore
            )
            refreshContext(for: record)
        }
        callbacks.canonicalPathChanged(canonicalState.path)
        callbacks.terminalReached(
            .init(outcome: .committed, visibility: currentSceneVisibility)
        )
        return true
    }

    // MARK: State-store seam

    func storeRouteState(
        _ value: GaryxRouteStateFieldValue?,
        field: GaryxRouteStateField,
        identity: GaryxRoutePresentationIdentity
    ) {
        stateStore.set(value, field: field, identity: identity)
        reportPinnedBudgetFaultIfNeeded()
    }

    func routeState(
        field: GaryxRouteStateField,
        identity: GaryxRoutePresentationIdentity
    ) -> GaryxRouteStateFieldValue? {
        stateStore.value(field: field, identity: identity)
    }

    // MARK: Gesture delegate

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldReceive touch: UITouch
    ) -> Bool {
        guard routeOwnsGestureTouch(in: touch.view) else { return false }
        let edge: GaryxRouteLogicalEdge
        if gestureRecognizer === leadingEdgePanGestureRecognizer {
            edge = .leading
        } else if gestureRecognizer === trailingEdgePanGestureRecognizer {
            edge = .trailing
        } else {
            return true
        }
        guard let coordinateView = gestureRecognizer.view ?? view else { return false }
        recordEdgeTouchDown(
            physicalX: touch.location(in: coordinateView).x,
            viewportWidth: max(coordinateView.bounds.width, 1),
            edge: edge
        )
        return true
    }

    /// Freezes the physical touch-down coordinate before UIKit's recognition
    /// hysteresis moves the finger. Kept internal so the hosted integration
    /// target can drive the exact LTR/RTL arbitration seam without private
    /// `UITouch` construction.
    func recordEdgeTouchDown(
        physicalX: CGFloat,
        viewportWidth: CGFloat,
        edge: GaryxRouteLogicalEdge
    ) {
        setPanOwner(nil, for: edge)
        let snapshot = GaryxRouteEdgeTouchSnapshot(
            physicalX: physicalX,
            viewportWidth: max(viewportWidth, 1),
            logicalEdge: edge,
            layoutDirection: routeLayoutDirection
        )
        if edge == .leading {
            leadingTouchSnapshot = snapshot
        } else {
            trailingTouchSnapshot = snapshot
        }
        gestureDiagnostic?("touch-\(edge.rawValue)-\(Int(snapshot.physicalX.rounded()))")
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer else { return false }
        let edge: GaryxRouteLogicalEdge
        if gestureRecognizer === leadingEdgePanGestureRecognizer {
            edge = .leading
        } else if gestureRecognizer === trailingEdgePanGestureRecognizer {
            edge = .trailing
        } else {
            return false
        }
        let coordinateView = gestureRecognizer.view ?? view
        return shouldBeginEdgePan(
            edge: edge,
            translation: CGSize(
                width: pan.translation(in: coordinateView).x,
                height: pan.translation(in: coordinateView).y
            ),
            velocity: CGSize(
                width: pan.velocity(in: coordinateView).x,
                height: pan.velocity(in: coordinateView).y
            )
        )
    }

    /// Evaluates one frozen touch-down snapshot. The hosted integration suite
    /// supplies vectors directly because UIKit does not expose synthetic pan
    /// velocity injection; production always enters through the delegate.
    func shouldBeginEdgePan(
        edge: GaryxRouteLogicalEdge,
        translation: CGSize,
        velocity: CGSize
    ) -> Bool {
        let snapshot: GaryxRouteEdgeTouchSnapshot?
        let owner: EdgePanOwner
        let requiresEdgeZone: Bool
        let direction: GaryxRouteGestureDirection
        if edge == .leading {
            snapshot = leadingTouchSnapshot
            let phase = transition?.coordinator.phase
            if phase == .cancelSettle {
                owner = .routePop
                requiresEdgeZone = true
                direction = .positive
            } else if transition == nil, canonicalState.path.isEmpty,
                      let interaction = homeLeadingEdgeInteraction,
                      interaction.isEligible() {
                owner = .homeLeading
                requiresEdgeZone = interaction.requiresEdgeZone()
                direction = interaction.acceptedDirection()
            } else if transition == nil, !canonicalState.path.isEmpty,
                      interactivePopEligible() {
                owner = .routePop
                requiresEdgeZone = true
                direction = .positive
            } else {
                return false
            }
        } else {
            snapshot = trailingTouchSnapshot
            guard transition == nil,
                  let interaction = trailingEdgeInteraction,
                  interaction.isEligible() else { return false }
            owner = .trailing
            requiresEdgeZone = interaction.requiresEdgeZone()
            direction = interaction.acceptedDirection()
        }
        guard let snapshot, snapshot.logicalEdge == edge else { return false }
        let shouldBegin = GaryxRouteEdgeGestureArbitrator.shouldBegin(
            touch: snapshot,
            translation: translation,
            velocity: velocity,
            modalBarrierActive: presentationLeases.hasBarrier,
            actionEligible: true,
            requiresEdgeZone: requiresEdgeZone,
            direction: direction
        )
        if shouldBegin {
            setPanOwner(owner, for: edge)
        }
        gestureDiagnostic?(
            "shouldBegin-\(edge.rawValue)-\(shouldBegin ? "yes" : "no")"
                + "-t\(Int(translation.width.rounded()))"
                + "-v\(Int(velocity.width.rounded()))"
        )
        return shouldBegin
    }

    /// A sheet, full-screen cover, alert, or menu owns its own touches above
    /// the route stack. Resolve the controller for this specific touch rather
    /// than globally looking for any presentation: keyboard/focus internals
    /// may install system presentations while the visible route still owns an
    /// interactive-pop touch.
    func routeOwnsGestureTouch(in touchedView: UIView?) -> Bool {
        var responder: UIResponder? = touchedView
        while let current = responder {
            if let controller = current as? UIViewController {
                return ownsController(controller)
            }
            responder = current.next
        }
        return true
    }

    private func ownsController(_ candidate: UIViewController) -> Bool {
        func contains(_ root: UIViewController) -> Bool {
            if root === candidate { return true }
            return root.children.contains(where: contains)
        }
        if contains(self) { return true }
        var ancestor = parent
        while let current = ancestor {
            if current === candidate { return true }
            ancestor = current.parent
        }
        return false
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldBeRequiredToFailBy otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        guard gestureRecognizer === leadingEdgePanGestureRecognizer
                || gestureRecognizer === trailingEdgePanGestureRecognizer,
              otherGestureRecognizer !== leadingEdgePanGestureRecognizer,
              otherGestureRecognizer !== trailingEdgePanGestureRecognizer,
              otherGestureRecognizer is UIPanGestureRecognizer,
              let descendantView = otherGestureRecognizer.view,
              let recognizerHost = gestureRecognizer.view,
              descendantView === recognizerHost
                || descendantView.isDescendant(of: recognizerHost)
        else { return false }
        return true
    }

    // MARK: Transition orchestration

    private var viewportWidth: CGFloat { max(view.bounds.width, 1) }

    private var currentSceneVisibility: GaryxPresentationVisibility {
        sceneIsActive ? .visible : .inactive
    }

    private var routeLayoutDirection: GaryxRouteLayoutDirection {
        if let layoutDirectionOverride { return layoutDirectionOverride }
        return view.effectiveUserInterfaceLayoutDirection == .rightToLeft
            ? .rightToLeft
            : .leftToRight
    }

    private func beginTransition(
        kind: GaryxRouteTransitionKind,
        source: GaryxRoutePresentationNode,
        destination: GaryxRoutePresentationNode,
        mutation: PendingCanonicalMutation
    ) -> Bool {
        guard transition == nil,
              let newTransition = GaryxRouteTransitionSession(
                  kind: kind,
                  source: source,
                  destination: destination,
                  preferences: preferencesProvider()
              )
        else { return false }
        // A new accepted transaction permanently supersedes any inactive
        // terminal effects that have not yet become visible.
        deferredTerminalEffects = nil
        let sourceHost = ensureMounted(source)
        let destinationHost = ensureMounted(destination)
        pendingMutation = mutation
        committedRemovedIdentities = []
        transition = newTransition
        interactionFrozenAfterTerminal = false
        screenChangedDelivered = false
        callbacks.phaseChanged(.preCommit)
        setTransitionZOrder(
            kind: kind,
            source: sourceHost,
            destination: destinationHost
        )
        sourceHost.wrapper.isHidden = false
        destinationHost.wrapper.isHidden = false
        // Disabling the source wrapper would make UIKit revoke its current
        // first responder. A sibling shield freezes content interaction while
        // preserving the composer/keyboard across pre-commit and cancel.
        sourceHost.wrapper.isUserInteractionEnabled = true
        destinationHost.wrapper.isUserInteractionEnabled = false
        sourceHost.wrapper.accessibilityElementsHidden = true
        destinationHost.wrapper.accessibilityElementsHidden = true
        interactionShield.isHidden = false
        view.bringSubviewToFront(interactionShield)
        applyTransitionVisualState()
        trimMountedHosts()
        refreshGestureAvailability()
        return true
    }

    private func releaseProgrammaticCommit(animated: Bool) {
        guard var transition else { return }
        let outcome = transition.release(
            logicalTranslation: viewportWidth,
            logicalVelocity: 0,
            viewportWidth: viewportWidth
        )
        guard outcome == .committed else { return }
        self.transition = transition
        callbacks.phaseChanged(.commitSettle)
        callbacks.commitReleased(transition.source, transition.destination)
        commitPendingCanonicalMutation()
        deactivateSourceAtCommitBoundary()
        if !animated || transition.visualPolicy == .immediate {
            completeSettleImmediately()
        } else {
            startSettle(
                logicalVelocity: 0,
                curve: GaryxRouteTransitionCalibration.programmaticSettleCurve
            )
        }
    }

    private func startSettle(
        logicalVelocity: CGFloat,
        curve: GaryxMotionPhysics.SpringCurve
    ) {
        guard let transition, let target = transition.settleTarget else { return }
        if transition.visualPolicy == .immediate {
            completeSettleImmediately()
            return
        }
        let normalizedVelocity = logicalVelocity / viewportWidth
        let settlesForward = target >= transition.progress
        settleDriver.settle(
            from: transition.progress,
            to: target,
            initialVelocity: normalizedVelocity,
            curve: curve,
            onUpdate: { [weak self] sample in
                guard let self, var transition = self.transition else { return }
                // A momentum-release token may be slightly underdamped and
                // overshoot under a high-velocity regrab. Full-screen
                // navigation progress is endpoint-bounded and monotonic for
                // both release and critically damped programmatic tokens.
                let progress = settlesForward
                    ? min(max(sample.value, transition.progress), target)
                    : max(min(sample.value, transition.progress), target)
                guard transition.updateSettle(progress: progress) else { return }
                self.transition = transition
                self.applyTransitionVisualState()
            },
            onCompletion: { [weak self] in
                guard let self else { return }
                self.finishTransition(visibility: self.currentSceneVisibility)
            }
        )
        refreshGestureAvailability()
    }

    private func finishTransition(visibility: GaryxPresentationVisibility) {
        guard var transition,
              let terminal = transition.finish(visibility: visibility)
        else { return }
        self.transition = transition
        callbacks.phaseChanged(.terminal)
        finalizeTerminal(terminal, transition: transition)
    }

    private func finalizeForcedTerminal(_ terminal: GaryxPresentationTerminalState) {
        guard let transition else { return }
        callbacks.phaseChanged(.terminal)
        finalizeTerminal(terminal, transition: transition)
    }

    private func finalizeTerminal(
        _ terminal: GaryxPresentationTerminalState,
        transition completedTransition: GaryxRouteTransitionSession
    ) {
        settleDriver.invalidate()
        callbacks.terminalReached(terminal)
        interactionFrozenAfterTerminal = terminal.visibility != .visible

        let source = host(for: completedTransition.source)
        let destination = host(for: completedTransition.destination)
        var visibleCommittedDestination: HostRecord?
        if terminal.outcome == .committed {
            let removedHosts = committedRemovedIdentities.compactMap { hosts[$0] }
            for removedHost in removedHosts {
                disappearAndUnmount(removedHost)
            }
            if terminal.visibility == .visible, let destination {
                activate(destination)
                visibleCommittedDestination = destination
            } else if terminal.visibility == .inactive {
                deferredTerminalEffects = terminal
            }
        } else {
            if terminal.visibility == .visible, let source {
                activate(source)
            } else if terminal.visibility == .inactive {
                deferredTerminalEffects = terminal
            }
        }

        source?.wrapper.resetTransitionState()
        destination?.wrapper.resetTransitionState()
        pendingMutation = nil
        committedRemovedIdentities = []
        transition = nil
        applyPendingPayloadReplacements()
        interactionShield.isHidden = true
        reconcileHostVisibility()
        trimMountedHosts()
        refreshGestureAvailability()
        if let visibleCommittedDestination {
            callbacks.visibleRouteActivated(completedTransition.destination)
            emitScreenChangedOnce(for: visibleCommittedDestination)
        }
        assertTerminalHasZeroResidue()
        callbacks.rendererBecameIdle()
    }

    private func commitPendingCanonicalMutation() {
        guard let pendingMutation else { return }
        switch pendingMutation {
        case .push(let entries):
            for entry in entries {
                _ = canonicalState.open(entry)
            }
        case .pop(let count):
            let removed = canonicalState.pop(count: count)
            committedRemovedIdentities = Set(
                removed.map { GaryxRoutePresentationIdentity.entry($0.id) }
            )
            for identity in committedRemovedIdentities {
                stateStore.removePermanently(identity: identity)
            }
        }
        refreshAllRouteContexts()
        callbacks.canonicalPathChanged(canonicalState.path)
    }

    private func deactivateSourceAtCommitBoundary() {
        guard let transition, let source = host(for: transition.source) else { return }
        deactivateIfActive(source)
    }

    private func deactivateIfActive(_ record: HostRecord) {
        switch record.lifecycle.phase {
        case .active:
            transitionLifecycle(record, to: .inactive)
        case .mounted, .inactive:
            break
        case .appeared, .disappeared:
            assertionFailure(
                "route host reached deactivation in illegal lifecycle phase \(record.lifecycle.phase)"
            )
        }
    }

    private func applyTransitionVisualState() {
        guard let transition,
              let source = host(for: transition.source),
              let destination = host(for: transition.destination)
        else { return }
        let state = transition.visualState(
            viewportWidth: viewportWidth,
            layoutDirection: routeLayoutDirection
        )

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        source.wrapper.apply(
            translationX: state.sourceTranslationX,
            alpha: state.sourceAlpha,
            scrimAlpha: transition.kind == .push ? state.scrimAlpha : 0,
            shadowOpacity: transition.kind == .pop ? state.movingShadowOpacity : 0,
            shadowOffsetX: state.movingShadowOffsetX
        )
        destination.wrapper.apply(
            translationX: state.destinationTranslationX,
            alpha: state.destinationAlpha,
            scrimAlpha: transition.kind == .pop ? state.scrimAlpha : 0,
            shadowOpacity: transition.kind == .push ? state.movingShadowOpacity : 0,
            shadowOffsetX: state.movingShadowOffsetX
        )
        CATransaction.commit()
        transitionFrameObserver?(
            transition.coordinator.phase,
            min(max(transition.progress, 0), 1),
            CACurrentMediaTime()
        )
    }

    private func setTransitionZOrder(
        kind: GaryxRouteTransitionKind,
        source: HostRecord,
        destination: HostRecord
    ) {
        switch kind {
        case .pop:
            view.insertSubview(destination.wrapper, belowSubview: source.wrapper)
            view.bringSubviewToFront(source.wrapper)
        case .push, .replace:
            view.insertSubview(source.wrapper, belowSubview: destination.wrapper)
            view.bringSubviewToFront(destination.wrapper)
        }
    }

    // MARK: Host lifecycle and containment

    private func wrappedHost(
        node: GaryxRoutePresentationNode,
        contextStore: GaryxRouteHostContextStore
    ) -> AnyView {
        AnyView(
            GaryxRouteContextHost(
                store: contextStore,
                content: hostBuilder(node)
            )
        )
    }

    private func routeContext(
        node: GaryxRoutePresentationNode,
        lifecycle: GaryxRouteHostLifecyclePhase
    ) -> GaryxRouteContext {
        GaryxRouteContext(
            node: node,
            isCanonicalTop: GaryxRoutePresentationIdentity(node)
                == GaryxRoutePresentationIdentity(canonicalState.topNode),
            lifecycle: lifecycle
        )
    }

    private func refreshContext(for record: HostRecord) {
        record.contextStore.apply(
            routeContext(node: record.node, lifecycle: record.lifecycle.phase)
        )
    }

    private func refreshAllRouteContexts() {
        for record in hosts.values {
            refreshContext(for: record)
        }
    }

    private func mountInitialHosts() {
        let activeNode = canonicalState.topNode
        if canonicalState.path.count > 0 {
            _ = ensureMounted(canonicalState.predecessorNode)
        }
        let active = ensureMounted(activeNode, initialContextLifecycle: .active)
        activate(active, coalescingInitialContext: true)
        reconcileHostVisibility()
        trimMountedHosts()
    }

    @discardableResult
    private func ensureMounted(
        _ node: GaryxRoutePresentationNode,
        initialContextLifecycle: GaryxRouteHostLifecyclePhase = .mounted
    ) -> HostRecord {
        let identity = GaryxRoutePresentationIdentity(node)
        accessClock &+= 1
        if let existing = hosts[identity] {
            existing.lastAccess = accessClock
            if existing.node != node {
                existing.node = node
                existing.controller.rootView = wrappedHost(
                    node: node,
                    contextStore: existing.contextStore
                )
                refreshContext(for: existing)
            }
            return existing
        }

        makeRoomForHost(mounting: identity)

        let wrapper = dequeueWrapper(identity: identity)
        let contextStore = GaryxRouteHostContextStore(
            routeContext(node: node, lifecycle: initialContextLifecycle)
        )
        let controller = UIHostingController(
            rootView: wrappedHost(node: node, contextStore: contextStore)
        )
        controller.view.backgroundColor = .systemBackground
        addChild(controller)
        wrapper.attachHostedView(controller.view)
        view.addSubview(wrapper)
        controller.didMove(toParent: self)
        let record = HostRecord(
            identity: identity,
            node: node,
            controller: controller,
            wrapper: wrapper,
            contextStore: contextStore,
            lastAccess: accessClock
        )
        hosts[identity] = record
        peakMountedHostCount = max(peakMountedHostCount, hosts.count)
        callbacks.hostMounted(identity)
        assert(hosts.count <= Self.maximumMountedHostCount, "mounted route host budget exceeded")
        return record
    }

    private func activate(
        _ record: HostRecord,
        coalescingInitialContext: Bool = false
    ) {
        switch record.lifecycle.phase {
        case .mounted:
            transitionLifecycle(
                record,
                to: .appeared,
                refreshesContext: !coalescingInitialContext
            )
            transitionLifecycle(
                record,
                to: .active,
                refreshesContext: !coalescingInitialContext
            )
            if coalescingInitialContext {
                refreshContext(for: record)
            }
        case .appeared, .inactive:
            transitionLifecycle(record, to: .active)
        case .active:
            break
        case .disappeared:
            assertionFailure("a disappeared host must be remounted with a new record")
        }
    }

    private func transitionLifecycle(
        _ record: HostRecord,
        to phase: GaryxRouteHostLifecyclePhase,
        refreshesContext: Bool = true
    ) {
        guard record.lifecycle.phase != phase else { return }
        guard record.lifecycle.transition(to: phase) else {
            assertionFailure("illegal route host lifecycle \(record.lifecycle.phase) -> \(phase)")
            return
        }
        if refreshesContext {
            refreshContext(for: record)
        }
        callbacks.hostLifecycleChanged(record.identity, phase)
    }

    private func disappearAndUnmount(_ record: HostRecord) {
        if record.lifecycle.phase == .active {
            transitionLifecycle(record, to: .inactive)
        }
        if record.lifecycle.phase == .inactive {
            transitionLifecycle(record, to: .disappeared)
        }
        unmount(record)
    }

    private func unmount(_ record: HostRecord, notify: Bool = true) {
        guard hosts.removeValue(forKey: record.identity) != nil else { return }
        record.controller.willMove(toParent: nil)
        record.controller.view.removeFromSuperview()
        record.controller.removeFromParent()
        record.wrapper.detachHostedView()
        record.wrapper.resetTransitionState()
        record.wrapper.removeFromSuperview()
        if wrapperPool.count < Self.maximumPooledWrapperCount, !isTearingDown {
            wrapperPool.append(record.wrapper)
        }
        if notify { callbacks.hostUnmounted(record.identity) }
    }

    private func dequeueWrapper(
        identity: GaryxRoutePresentationIdentity
    ) -> GaryxRouteTransitionWrapperView {
        let wrapper = wrapperPool.popLast() ?? GaryxRouteTransitionWrapperView()
        wrapper.prepareForReuse(identity: identity)
        wrapper.frame = view.bounds
        wrapper.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        return wrapper
    }

    private func trimMountedHosts() {
        let protected = protectedHostIdentities()

        while hosts.count > Self.maximumMountedHostCount {
            guard let victim = hosts.values
                .filter({ !protected.contains($0.identity) })
                .min(by: { $0.lastAccess < $1.lastAccess })
            else {
                assertionFailure("all route hosts are protected above the mounted-host budget")
                break
            }
            disappearAndUnmount(victim)
        }
    }

    private func makeRoomForHost(mounting identity: GaryxRoutePresentationIdentity) {
        guard hosts[identity] == nil else { return }
        while hosts.count >= Self.maximumMountedHostCount {
            let protected = protectedHostIdentities()
            guard let victim = hosts.values
                .filter({ !protected.contains($0.identity) })
                .min(by: { $0.lastAccess < $1.lastAccess })
            else {
                assertionFailure("no evictable route host is available before mounting")
                return
            }
            disappearAndUnmount(victim)
        }
    }

    private func protectedHostIdentities() -> Set<GaryxRoutePresentationIdentity> {
        var protected: Set<GaryxRoutePresentationIdentity> = [
            GaryxRoutePresentationIdentity(canonicalState.topNode),
        ]
        if !canonicalState.path.isEmpty {
            protected.insert(GaryxRoutePresentationIdentity(canonicalState.predecessorNode))
        }
        if let transition {
            protected.insert(GaryxRoutePresentationIdentity(transition.source))
            protected.insert(GaryxRoutePresentationIdentity(transition.destination))
        }
        return protected
    }

    private func reconcileHostVisibility() {
        let activeIdentity = GaryxRoutePresentationIdentity(canonicalState.topNode)
        for record in hosts.values {
            let isActive = record.identity == activeIdentity
            record.wrapper.isHidden = !isActive
            record.wrapper.isUserInteractionEnabled = isActive && !interactionFrozenAfterTerminal
            record.wrapper.accessibilityElementsHidden = !isActive || interactionFrozenAfterTerminal
            record.wrapper.resetTransitionState()
        }
        if let active = hosts[activeIdentity] {
            view.bringSubviewToFront(active.wrapper)
        }
    }

    private func activeHostRecord() -> HostRecord? {
        hosts[GaryxRoutePresentationIdentity(canonicalState.topNode)]
    }

    private func host(for node: GaryxRoutePresentationNode) -> HostRecord? {
        hosts[GaryxRoutePresentationIdentity(node)]
    }

    private func emitScreenChangedOnce(for record: HostRecord) {
        guard !screenChangedDelivered else { return }
        screenChangedDelivered = true
        callbacks.screenChanged(record.controller.view)
    }

    // MARK: Geometry and hard snap

    private func rederiveWrapperGeometry() {
        guard isViewLoaded else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for record in hosts.values {
            record.wrapper.transform = .identity
            record.wrapper.frame = view.bounds
        }
        CATransaction.commit()
        if transition != nil {
            applyTransitionVisualState()
        }
    }

    private func performHardSnap(to replacement: [GaryxRouteEntry]) {
        if transition != nil {
            supersedeActiveTransition()
        }
        settleDriver.invalidate()
        let sourceNode = canonicalState.topNode
        let destinationNode = replacement.last.map(GaryxRoutePresentationNode.entry) ?? .home
        let changesVisibleRoute = sourceNode != destinationNode
        if changesVisibleRoute {
            screenChangedDelivered = false
            // A root replacement has no animated release callback, so this is
            // its commit boundary. Composer finalization still precedes the
            // canonical path write exactly as it does for push/pop.
            callbacks.commitReleased(sourceNode, destinationNode)
        }
        let oldIdentities = Set(canonicalState.path.map {
            GaryxRoutePresentationIdentity.entry($0.id)
        })
        if let current = activeHostRecord() {
            disappearAndUnmount(current)
        }
        for record in Array(hosts.values) {
            unmount(record)
        }
        canonicalState.replacePath(replacement)
        interactionFrozenAfterTerminal = false
        deferredTerminalEffects = nil
        let newIdentities = Set(replacement.map {
            GaryxRoutePresentationIdentity.entry($0.id)
        })
        for identity in oldIdentities.subtracting(newIdentities) {
            stateStore.removePermanently(identity: identity)
        }
        callbacks.canonicalPathChanged(replacement)
        mountInitialHosts()
        refreshAllRouteContexts()
        if changesVisibleRoute {
            let terminal = GaryxPresentationTerminalState(
                outcome: .committed,
                visibility: currentSceneVisibility
            )
            callbacks.terminalReached(terminal)
            if terminal.visibility == .visible, let active = activeHostRecord() {
                callbacks.visibleRouteActivated(destinationNode)
                emitScreenChangedOnce(for: active)
            } else if terminal.visibility == .inactive,
                      let active = activeHostRecord() {
                deactivateIfActive(active)
                deferredTerminalEffects = terminal
                interactionFrozenAfterTerminal = true
                reconcileHostVisibility()
                refreshGestureAvailability()
            }
        }
        assertTerminalHasZeroResidue()
        callbacks.rendererBecameIdle()
    }

    private func presentationLeaseMayHaveReleased() {
        refreshGestureAvailability()
        guard !presentationLeases.hasBarrier, let pendingHardSnapPath else { return }
        self.pendingHardSnapPath = nil
        performHardSnap(to: pendingHardSnapPath)
    }

    private func reportPinnedBudgetFaultIfNeeded() {
        let metrics = stateStore.metrics
        if metrics.pinnedBudgetFaultCount > 0 {
            callbacks.budgetFault(metrics)
        }
    }

    // MARK: UIKit gesture adapter

    private func configure(recognizer: UIPanGestureRecognizer) {
        recognizer.delegate = self
        recognizer.maximumNumberOfTouches = 1
        recognizer.cancelsTouchesInView = true
        recognizer.delaysTouchesBegan = false
        recognizer.delaysTouchesEnded = false
    }

    private func applyLayoutDirectionOverride() {
        switch layoutDirectionOverride {
        case .leftToRight:
            view.semanticContentAttribute = .forceLeftToRight
        case .rightToLeft:
            view.semanticContentAttribute = .forceRightToLeft
        case nil:
            view.semanticContentAttribute = .unspecified
        }
        updateRecognizerEdges()
    }

    private func updateRecognizerEdges() {
        // Logical-to-physical mapping is resolved from the touch-down snapshot
        // and translation sign. There is no UIKit-private edge zone to update.
    }

    private func refreshGestureAvailability() {
        let barrierFree = !presentationLeases.hasBarrier
        let phase = transition?.coordinator.phase
        leadingEdgePanGestureRecognizer.isEnabled = barrierFree && sceneIsActive
            && !interactionFrozenAfterTerminal
            && phase != .commitSettle
            && phase != .terminal
        trailingEdgePanGestureRecognizer.isEnabled = barrierFree && sceneIsActive
            && !interactionFrozenAfterTerminal && transition == nil
    }

    @objc private func handleLeadingEdgePan(_ recognizer: UIPanGestureRecognizer) {
        handleEdgePan(recognizer, edge: .leading)
    }

    @objc private func handleTrailingEdgePan(_ recognizer: UIPanGestureRecognizer) {
        handleEdgePan(recognizer, edge: .trailing)
    }

    private func handleEdgePan(
        _ recognizer: UIPanGestureRecognizer,
        edge: GaryxRouteLogicalEdge
    ) {
        gestureDiagnostic?("state-\(edge.rawValue)-\(recognizer.state.rawValue)")
        guard let owner = panOwner(for: edge) else { return }
        let coordinateView = recognizer.view ?? view
        let logicalTranslation = GaryxRouteEdgeGestureArbitrator.logicalTranslation(
            physicalTranslationX: recognizer.translation(in: coordinateView).x,
            edge: edge,
            layoutDirection: routeLayoutDirection
        )
        let logicalVelocity = GaryxRouteEdgeGestureArbitrator.logicalTranslation(
            physicalTranslationX: recognizer.velocity(in: coordinateView).x,
            edge: edge,
            layoutDirection: routeLayoutDirection
        )

        switch recognizer.state {
        case .began:
            switch owner {
            case .routePop:
                if transition?.coordinator.phase == .cancelSettle {
                    _ = regrabCancelSettle()
                } else {
                    _ = beginInteractivePop()
                }
            case .homeLeading:
                homeLeadingEdgeInteraction?.began()
                homeLeadingEdgeInteraction?.changed(logicalTranslation, logicalVelocity)
            case .trailing:
                trailingEdgeInteraction?.began()
                trailingEdgeInteraction?.changed(logicalTranslation, logicalVelocity)
            }
        case .changed:
            switch owner {
            case .routePop:
                updateInteractivePop(
                    logicalTranslation: gestureBaseProgress * viewportWidth + logicalTranslation
                )
            case .homeLeading:
                homeLeadingEdgeInteraction?.changed(logicalTranslation, logicalVelocity)
            case .trailing:
                trailingEdgeInteraction?.changed(logicalTranslation, logicalVelocity)
            }
        case .ended:
            gestureDiagnostic?(
                "release-t\(Int(recognizer.translation(in: coordinateView).x.rounded()))"
                    + "-v\(Int(recognizer.velocity(in: coordinateView).x.rounded()))"
            )
            switch owner {
            case .routePop:
                _ = endInteractivePop(logicalVelocity: logicalVelocity)
            case .homeLeading:
                homeLeadingEdgeInteraction?.changed(logicalTranslation, logicalVelocity)
                homeLeadingEdgeInteraction?.ended(logicalVelocity)
            case .trailing:
                trailingEdgeInteraction?.changed(logicalTranslation, logicalVelocity)
                trailingEdgeInteraction?.ended(logicalVelocity)
            }
            setPanOwner(nil, for: edge)
        case .cancelled, .failed:
            switch owner {
            case .routePop:
                cancelInteractivePop()
            case .homeLeading:
                homeLeadingEdgeInteraction?.cancelled()
            case .trailing:
                trailingEdgeInteraction?.cancelled()
            }
            setPanOwner(nil, for: edge)
        default:
            break
        }
    }

    private func panOwner(for edge: GaryxRouteLogicalEdge) -> EdgePanOwner? {
        edge == .leading ? leadingPanOwner : trailingPanOwner
    }

    private func setPanOwner(_ owner: EdgePanOwner?, for edge: GaryxRouteLogicalEdge) {
        if edge == .leading {
            leadingPanOwner = owner
        } else {
            trailingPanOwner = owner
        }
    }

    private func installEdgeRecognizers(on host: UIView) {
        guard leadingEdgePanGestureRecognizer.view !== host
                || trailingEdgePanGestureRecognizer.view !== host else { return }
        leadingEdgePanGestureRecognizer.view?.removeGestureRecognizer(
            leadingEdgePanGestureRecognizer
        )
        trailingEdgePanGestureRecognizer.view?.removeGestureRecognizer(
            trailingEdgePanGestureRecognizer
        )
        host.addGestureRecognizer(leadingEdgePanGestureRecognizer)
        host.addGestureRecognizer(trailingEdgePanGestureRecognizer)
    }

    // MARK: Cleanup assertions

    func assertTerminalHasZeroResidue(
        file: StaticString = #file,
        line: UInt = #line
    ) {
        assert(transition == nil, "terminal transition retained its session", file: file, line: line)
        assert(!settleDriver.isSettling, "terminal transition retained display-link settle", file: file, line: line)
        assert(pendingMutation == nil, "terminal transition retained canonical mutation", file: file, line: line)
        assert(hosts.count <= Self.maximumMountedHostCount, "mounted host budget exceeded", file: file, line: line)
        assert(
            hosts.values.allSatisfy { !$0.wrapper.hasTransitionResidue },
            "terminal wrapper retained transform/scrim/shadow residue",
            file: file,
            line: line
        )
    }

    private func tearDownRenderer() {
        guard !isTearingDown else { return }
        isTearingDown = true
        settleDriver.invalidate()
        leadingEdgePanGestureRecognizer.view?.removeGestureRecognizer(
            leadingEdgePanGestureRecognizer
        )
        trailingEdgePanGestureRecognizer.view?.removeGestureRecognizer(
            trailingEdgePanGestureRecognizer
        )
        leadingEdgePanGestureRecognizer.removeTarget(self, action: nil)
        trailingEdgePanGestureRecognizer.removeTarget(self, action: nil)
        for record in Array(hosts.values) {
            unmount(record, notify: false)
        }
        wrapperPool.removeAll(keepingCapacity: false)
        transition = nil
        pendingMutation = nil
        committedRemovedIdentities.removeAll(keepingCapacity: false)
        pendingHardSnapPath = nil
        pendingPayloadReplacements.removeAll(keepingCapacity: false)
        deferredTerminalEffects = nil
        interactionFrozenAfterTerminal = false
        assert(hosts.isEmpty, "route container deinit retained child hosts")
        assert(wrapperPool.isEmpty, "route container deinit retained wrapper pool")
    }
}
