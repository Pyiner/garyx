import SwiftUI
import UIKit

private struct GaryxRootSurfaceOccurrenceEnvironmentKey: EnvironmentKey {
    static let defaultValue: GaryxRootSurfaceOccurrenceID? = nil
}

extension EnvironmentValues {
    var garyxRootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID? {
        get { self[GaryxRootSurfaceOccurrenceEnvironmentKey.self] }
        set { self[GaryxRootSurfaceOccurrenceEnvironmentKey.self] = newValue }
    }
}

struct GaryxHorizontalRevealPresentation: Equatable {
    var reveal: CGFloat
    var phase: GaryxHorizontalRevealPhase
    var target: GaryxHorizontalRevealPosition
}

struct GaryxHorizontalRevealInteractionDiagnostics: Equatable {
    var presentation: GaryxHorizontalRevealPresentation
    var extent: CGFloat
    var settleDriverIsActive: Bool

    var hasTerminalResidue: Bool {
        presentation.phase != .idle || settleDriverIsActive
    }
}

/// Main-actor adapter around the pure Core reveal state. It owns only display
/// link orchestration and publishes the one presentation value SwiftUI draws.
@MainActor
final class GaryxHorizontalRevealInteractionStore: ObservableObject {
    @Published private(set) var presentation: GaryxHorizontalRevealPresentation

    private let observableSettlementScheduler: any GaryxObservableSettlementScheduling
    private let projection: GaryxMotionPhysics.ProjectionPolicy
    private let releaseCurve: GaryxMotionPhysics.SpringCurve
    private let nonMomentumCurve: GaryxMotionPhysics.SpringCurve
    private let settleDriver: GaryxGestureSettleDriver
    private var activeCurve: GaryxMotionPhysics.SpringCurve?
    private var state: GaryxHorizontalRevealState
    private var extent: CGFloat = 0
    private var requestedPosition: GaryxHorizontalRevealPosition
    private var isConfigured = false
    private var hostOwnership: GaryxHorizontalRevealHostOwnership?
    private lazy var presentationSettlement = GaryxObservableStateSettler(
        initialValue: presentation,
        scheduler: observableSettlementScheduler,
        publish: { [weak self] next in
            self?.presentation = next
        }
    )

    init(
        initialPosition: GaryxHorizontalRevealPosition = .closed,
        projection: GaryxMotionPhysics.ProjectionPolicy,
        bindsToRootSurfaceHost: Bool = false,
        releaseCurve: GaryxMotionPhysics.SpringCurve = GaryxMotion.springCurve(for: .settle),
        nonMomentumCurve: GaryxMotionPhysics.SpringCurve = GaryxMotion.springCurve(for: .snapBack),
        settleDriver: GaryxGestureSettleDriver? = nil,
        observableSettlementScheduler: (any GaryxObservableSettlementScheduling)? = nil
    ) {
        requestedPosition = initialPosition
        state = GaryxHorizontalRevealState(position: initialPosition)
        hostOwnership = bindsToRootSurfaceHost
            ? GaryxHorizontalRevealHostOwnership()
            : nil
        self.projection = projection
        self.releaseCurve = releaseCurve
        self.nonMomentumCurve = nonMomentumCurve
        self.settleDriver = settleDriver ?? .displayLinked()
        self.observableSettlementScheduler = observableSettlementScheduler
            ?? GaryxNextMainQueueObservableSettlementScheduler.shared
        presentation = GaryxHorizontalRevealPresentation(
            reveal: 0,
            phase: .idle,
            target: initialPosition
        )
    }

    var reveal: CGFloat { state.reveal }
    var progress: CGFloat {
        guard extent > 0 else { return requestedPosition == .open ? 1 : 0 }
        return min(max(reveal / extent, 0), 1)
    }
    var isDragging: Bool { state.phase == .dragging }
    var isSettling: Bool {
        if case .settling = state.phase { return true }
        return false
    }
    var isGestureEligible: Bool {
        isConfigured && extent > 0 && (hostOwnership?.hasActiveHost ?? true)
    }
    var diagnostics: GaryxHorizontalRevealInteractionDiagnostics {
        GaryxHorizontalRevealInteractionDiagnostics(
            presentation: semanticPresentation,
            extent: extent,
            settleDriverIsActive: settleDriver.isSettling
        )
    }
    var requiresEdgeZone: Bool {
        state.phase == .idle && state.targetPosition == .closed
    }
    var acceptedDirection: GaryxRouteGestureDirection {
        switch state.phase {
        case .settling, .dragging:
            .either
        case .idle:
            state.targetPosition == .open ? .negative : .positive
        }
    }

    private var semanticPresentation: GaryxHorizontalRevealPresentation {
        GaryxHorizontalRevealPresentation(
            reveal: state.reveal,
            phase: state.phase,
            target: state.targetPosition
        )
    }

    func configure(extent newExtent: CGFloat, restingPosition: GaryxHorizontalRevealPosition) {
        let newExtent = max(0, newExtent)
        requestedPosition = restingPosition
        guard !isConfigured else {
            guard newExtent != extent else {
                if state.reconcileRestingPositionIfIdle(
                    restingPosition,
                    extent: extent
                ) {
                    publish()
                }
                return
            }
            extent = newExtent
            // A size-class/safe-area change invalidates the physical gesture
            // track. Snap to the canonical endpoint now; a driver paused by
            // scene or geometry changes must never remain the only owner that
            // can return hit testing to the surface.
            forceTerminal(.geometryChanged, position: restingPosition)
            return
        }

        extent = newExtent
        isConfigured = true
        state.synchronize(to: restingPosition, extent: newExtent)
        publish()
    }

    func configure(
        extent newExtent: CGFloat,
        restingPosition: GaryxHorizontalRevealPosition,
        rootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID
    ) {
        guard hostOwnership?.belongsToRootSurface(rootSurfaceOccurrenceID) == true else { return }
        configure(extent: newExtent, restingPosition: restingPosition)
    }

    func setTarget(
        _ position: GaryxHorizontalRevealPosition,
        animated: Bool,
        initialVelocity: CGFloat = 0
    ) {
        requestedPosition = position
        guard isConfigured,
              extent > 0,
              hostOwnership?.hasActiveHost != false else {
            settleDriver.invalidate()
            activeCurve = nil
            state.synchronize(to: position, extent: extent)
            publish()
            return
        }
        if !animated {
            settleDriver.invalidate()
            activeCurve = nil
            state.synchronize(to: position, extent: extent)
            publish()
            return
        }
        if case .settling(let target) = state.phase, target == position {
            return
        }
        _ = settleDriver.interrupt()
        activeCurve = nil
        guard let settle = state.beginProgrammaticSettle(
            to: position,
            initialVelocity: initialVelocity,
            extent: extent
        ) else {
            publish()
            return
        }
        publish()
        startSettle(settle, animated: true, curve: nonMomentumCurve)
    }

    func setTarget(
        _ position: GaryxHorizontalRevealPosition,
        animated: Bool,
        initialVelocity: CGFloat = 0,
        rootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID
    ) {
        guard hostOwnership?.belongsToRootSurface(rootSurfaceOccurrenceID) == true else { return }
        setTarget(position, animated: animated, initialVelocity: initialVelocity)
    }

    func beginGesture() {
        guard hostOwnership == nil else {
            assertionFailure("host-bound reveal gesture is missing its occurrence identity")
            return
        }
        beginAdmittedGesture()
    }

    func beginGesture(in occurrenceID: GaryxHorizontalRevealHostOccurrenceID) {
        guard hostOwnership?.accepts(occurrenceID) == true else { return }
        beginAdmittedGesture()
    }

    private func beginAdmittedGesture() {
        guard isGestureEligible else { return }
        let interrupted = settleDriver.interrupt()
        // `reveal` is the exact value drawn on the last frame. Adopt it at the
        // seam even if the underlying spring sample was outside an endpoint
        // and therefore rubber-banded for presentation.
        state.beginDrag(
            interruptedReveal: interrupted == nil ? nil : presentation.reveal,
            extent: extent
        )
        activeCurve = nil
        publish()
    }

    func updateGesture(logicalTranslation: CGFloat) {
        guard hostOwnership == nil else {
            assertionFailure("host-bound reveal gesture is missing its occurrence identity")
            return
        }
        updateAdmittedGesture(logicalTranslation: logicalTranslation)
    }

    func updateGesture(
        logicalTranslation: CGFloat,
        in occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) {
        guard hostOwnership?.accepts(occurrenceID) == true else { return }
        updateAdmittedGesture(logicalTranslation: logicalTranslation)
    }

    private func updateAdmittedGesture(logicalTranslation: CGFloat) {
        state.updateDrag(logicalTranslation: logicalTranslation, extent: extent)
        publish()
    }

    @discardableResult
    func endGesture(logicalVelocity: CGFloat) -> GaryxHorizontalRevealPosition? {
        guard hostOwnership == nil else {
            assertionFailure("host-bound reveal gesture is missing its occurrence identity")
            return nil
        }
        return endAdmittedGesture(logicalVelocity: logicalVelocity)
    }

    @discardableResult
    func endGesture(
        logicalVelocity: CGFloat,
        in occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) -> GaryxHorizontalRevealPosition? {
        guard hostOwnership?.accepts(occurrenceID) == true else { return nil }
        return endAdmittedGesture(logicalVelocity: logicalVelocity)
    }

    private func endAdmittedGesture(
        logicalVelocity: CGFloat
    ) -> GaryxHorizontalRevealPosition? {
        guard let settle = state.release(
            logicalVelocity: logicalVelocity,
            extent: extent,
            projection: projection
        ) else { return nil }
        requestedPosition = settle.target
        publish()
        startSettle(settle, animated: true, curve: releaseCurve)
        return settle.target
    }

    @discardableResult
    func cancelGesture() -> GaryxHorizontalRevealPosition? {
        guard hostOwnership == nil else {
            assertionFailure("host-bound reveal gesture is missing its occurrence identity")
            return nil
        }
        return cancelAdmittedGesture()
    }

    @discardableResult
    func cancelGesture(
        in occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) -> GaryxHorizontalRevealPosition? {
        guard hostOwnership?.accepts(occurrenceID) == true else { return nil }
        return cancelAdmittedGesture()
    }

    private func cancelAdmittedGesture() -> GaryxHorizontalRevealPosition? {
        guard let settle = state.cancelDrag(extent: extent) else { return nil }
        requestedPosition = settle.target
        publish()
        startSettle(settle, animated: true, curve: nonMomentumCurve)
        return settle.target
    }

    func isGestureEligible(in occurrenceID: GaryxHorizontalRevealHostOccurrenceID) -> Bool {
        hostOwnership?.accepts(occurrenceID) == true && isGestureEligible
    }

    func applyRootSurfaceOccurrenceTransition(
        _ transition: GaryxRootSurfaceOccurrenceTransition,
        position: GaryxHorizontalRevealPosition
    ) {
        guard var hostOwnership else {
            assertionFailure("root transition applied to a surface-local reveal")
            return
        }
        let requiresTerminalization = hostOwnership.applyRootSurfaceTransition(transition)
        self.hostOwnership = hostOwnership
        if requiresTerminalization {
            forceTerminal(.hostOccurrenceEnded, position: position)
        } else if !hostOwnership.hasActiveHost {
            assertTerminalHasZeroResidue()
        }
    }

    func attachHostOccurrence(
        _ occurrenceID: GaryxHorizontalRevealHostOccurrenceID,
        position: GaryxHorizontalRevealPosition
    ) {
        guard var hostOwnership else {
            assertionFailure("host occurrence attached to a surface-local reveal")
            return
        }
        let result = hostOwnership.attachHost(occurrenceID)
        self.hostOwnership = hostOwnership
        switch result {
        case .attached:
            assertTerminalHasZeroResidue()
        case .alreadyAttached:
            break
        case .superseded:
            forceTerminal(.hostOccurrenceEnded, position: position)
        case .rejected:
            assertionFailure("reveal host attached outside its root surface occurrence")
        }
    }

    func detachHostOccurrence(
        _ occurrenceID: GaryxHorizontalRevealHostOccurrenceID,
        position: GaryxHorizontalRevealPosition,
        observableSettlement: GaryxObservableSettlementTiming = .immediate
    ) {
        guard var hostOwnership else {
            assertionFailure("host occurrence detached from a surface-local reveal")
            return
        }
        let detachedCurrentOwner = hostOwnership.detachHost(occurrenceID)
        self.hostOwnership = hostOwnership
        if detachedCurrentOwner {
            forceTerminal(
                .hostOccurrenceEnded,
                position: position,
                observableSettlement: observableSettlement
            )
        }
    }

    func invalidate(
        position: GaryxHorizontalRevealPosition,
        event: GaryxHorizontalRevealInvalidation = .routeInvalidated
    ) {
        requestedPosition = position
        forceTerminal(event, position: position)
    }

    func forceTerminal(
        _ event: GaryxHorizontalRevealInvalidation,
        position: GaryxHorizontalRevealPosition? = nil,
        observableSettlement: GaryxObservableSettlementTiming = .immediate
    ) {
        let terminalPosition = position ?? requestedPosition
        requestedPosition = terminalPosition
        settleDriver.invalidate()
        activeCurve = nil
        _ = state.forceTerminal(event, to: terminalPosition, extent: extent)
        publish(observableSettlement: observableSettlement)
        assertTerminalHasZeroResidue()
    }

    func assertTerminalHasZeroResidue(
        file: StaticString = #file,
        line: UInt = #line
    ) {
        assert(
            state.phase == .idle,
            "terminal reveal retained interaction ownership",
            file: file,
            line: line
        )
        assert(
            !settleDriver.isSettling,
            "terminal reveal retained display-link settle",
            file: file,
            line: line
        )
    }

    private func startSettle(
        _ settle: GaryxHorizontalRevealSettle,
        animated: Bool,
        curve: GaryxMotionPhysics.SpringCurve
    ) {
        guard animated, extent > 0 else {
            settleDriver.invalidate()
            state.synchronize(to: settle.target, extent: extent)
            activeCurve = nil
            publish()
            return
        }
        activeCurve = curve
        settleDriver.settle(
            from: settle.initialReveal,
            to: settle.target.reveal(for: extent),
            initialVelocity: settle.initialVelocity,
            curve: curve,
            onUpdate: { [weak self] sample in
                guard let self else { return }
                state.updateSettle(sampledReveal: sample.value, extent: extent)
                publish()
            },
            onCompletion: { [weak self] in
                guard let self else { return }
                activeCurve = nil
                _ = state.finishSettle(extent: extent)
                publish()
            }
        )
    }

    private func publish(
        observableSettlement: GaryxObservableSettlementTiming = .immediate
    ) {
        if hostOwnership?.hasActiveHost == false {
            assert(
                state.phase == .idle && !settleDriver.isSettling,
                "hostless reveal retained interaction ownership"
            )
        }
        presentationSettlement.settle(
            semanticPresentation,
            timing: observableSettlement
        )
    }
}

@MainActor
extension GaryxMobileModel {
    /// The drawer and task-tree stores outlive the SwiftUI surfaces that
    /// configure them. Lifecycle and canonical-owner invalidations therefore
    /// terminate both stores together at their model-owned endpoints.
    func forceTerminalGlobalRevealInteractions(
        _ event: GaryxHorizontalRevealInvalidation
    ) {
        drawerRevealInteraction.forceTerminal(
            event,
            position: sidebarVisible ? .open : .closed
        )
        taskTreeRevealInteraction.forceTerminal(
            event,
            position: isTaskTreeSidebarOpen ? .open : .closed
        )
    }

    /// Root branch publication and UIKit host attach/dismantle both converge on
    /// the same occurrence ledger. Ending either owner synchronously stops the
    /// display link and returns both shared reveal states to idle.
    func applyGlobalRevealRootSurfaceTransition(
        _ transition: GaryxRootSurfaceOccurrenceTransition
    ) {
        drawerRevealInteraction.applyRootSurfaceOccurrenceTransition(
            transition,
            position: sidebarVisible ? .open : .closed
        )
        taskTreeRevealInteraction.applyRootSurfaceOccurrenceTransition(
            transition,
            position: isTaskTreeSidebarOpen ? .open : .closed
        )
    }

    func attachGlobalRevealHostOccurrence(
        _ occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) {
        drawerRevealInteraction.attachHostOccurrence(
            occurrenceID,
            position: sidebarVisible ? .open : .closed
        )
        taskTreeRevealInteraction.attachHostOccurrence(
            occurrenceID,
            position: isTaskTreeSidebarOpen ? .open : .closed
        )
    }

    func detachGlobalRevealHostOccurrence(
        _ occurrenceID: GaryxHorizontalRevealHostOccurrenceID
    ) {
        drawerRevealInteraction.detachHostOccurrence(
            occurrenceID,
            position: sidebarVisible ? .open : .closed,
            observableSettlement: .afterViewGraphUpdate
        )
        taskTreeRevealInteraction.detachHostOccurrence(
            occurrenceID,
            position: isTaskTreeSidebarOpen ? .open : .closed,
            observableSettlement: .afterViewGraphUpdate
        )
    }

    func assertGlobalRevealInteractionsHaveZeroResidue(
        file: StaticString = #file,
        line: UInt = #line
    ) {
        drawerRevealInteraction.assertTerminalHasZeroResidue(file: file, line: line)
        taskTreeRevealInteraction.assertTerminalHasZeroResidue(file: file, line: line)
    }
}

/// UIKit pan delivery for row-local gestures. Unlike SwiftUI `DragGesture`,
/// this exposes `.cancelled` as a first-class event so the Core state machine
/// can settle deterministically after system takeover.
struct GaryxHorizontalPanGesture: UIGestureRecognizerRepresentable {
    var isEnabled = true
    let shouldBegin: (_ translation: CGSize, _ velocity: CGSize) -> Bool
    let onBegan: () -> Void
    let onChanged: (_ translation: CGSize, _ velocity: CGSize) -> Void
    let onEnded: (_ translation: CGSize, _ velocity: CGSize) -> Void
    let onCancelled: () -> Void

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var gesture: GaryxHorizontalPanGesture

        init(gesture: GaryxHorizontalPanGesture) {
            self.gesture = gesture
        }

        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard gesture.isEnabled,
                  let pan = gestureRecognizer as? UIPanGestureRecognizer,
                  let view = pan.view else { return false }
            let translation = pan.translation(in: view).size
            let velocity = pan.velocity(in: view).size
            return GaryxRouteEdgeGestureArbitrator.axis(
                translation: translation,
                velocity: velocity
            ) == .horizontal && gesture.shouldBegin(translation, velocity)
        }
    }

    func makeCoordinator(converter _: CoordinateSpaceConverter) -> Coordinator {
        Coordinator(gesture: self)
    }

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let recognizer = UIPanGestureRecognizer()
        recognizer.delegate = context.coordinator
        recognizer.maximumNumberOfTouches = 1
        recognizer.cancelsTouchesInView = true
        recognizer.delaysTouchesBegan = false
        recognizer.delaysTouchesEnded = false
        return recognizer
    }

    func updateUIGestureRecognizer(
        _ recognizer: UIPanGestureRecognizer,
        context: Context
    ) {
        context.coordinator.gesture = self
        recognizer.isEnabled = isEnabled
    }

    func handleUIGestureRecognizerAction(
        _ recognizer: UIPanGestureRecognizer,
        context: Context
    ) {
        guard let view = recognizer.view else { return }
        let translation = recognizer.translation(in: view).size
        let velocity = recognizer.velocity(in: view).size
        switch recognizer.state {
        case .began:
            context.coordinator.gesture.onBegan()
            context.coordinator.gesture.onChanged(translation, velocity)
        case .changed:
            context.coordinator.gesture.onChanged(translation, velocity)
        case .ended:
            context.coordinator.gesture.onEnded(translation, velocity)
        case .cancelled, .failed:
            context.coordinator.gesture.onCancelled()
        default:
            break
        }
    }
}

private extension CGPoint {
    var size: CGSize { CGSize(width: x, height: y) }
}
