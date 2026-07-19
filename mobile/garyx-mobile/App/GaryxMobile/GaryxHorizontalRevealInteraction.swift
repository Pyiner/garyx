import SwiftUI
import UIKit

struct GaryxHorizontalRevealPresentation: Equatable {
    var reveal: CGFloat
    var phase: GaryxHorizontalRevealPhase
    var target: GaryxHorizontalRevealPosition
}

/// Main-actor adapter around the pure Core reveal state. It owns only display
/// link orchestration and publishes the one presentation value SwiftUI draws.
@MainActor
final class GaryxHorizontalRevealInteractionStore: ObservableObject {
    @Published private(set) var presentation: GaryxHorizontalRevealPresentation

    private let projection: GaryxMotionPhysics.ProjectionPolicy
    private let curve: GaryxMotionPhysics.SpringCurve
    private let settleDriver: GaryxGestureSettleDriver
    private var state: GaryxHorizontalRevealState
    private var extent: CGFloat = 0
    private var requestedPosition: GaryxHorizontalRevealPosition
    private var isConfigured = false

    init(
        initialPosition: GaryxHorizontalRevealPosition = .closed,
        projection: GaryxMotionPhysics.ProjectionPolicy,
        curve: GaryxMotionPhysics.SpringCurve = GaryxRouteTransitionCalibration.settleCurve,
        settleDriver: GaryxGestureSettleDriver? = nil
    ) {
        requestedPosition = initialPosition
        state = GaryxHorizontalRevealState(position: initialPosition)
        self.projection = projection
        self.curve = curve
        self.settleDriver = settleDriver ?? .displayLinked()
        presentation = GaryxHorizontalRevealPresentation(
            reveal: 0,
            phase: .idle,
            target: initialPosition
        )
    }

    var reveal: CGFloat { presentation.reveal }
    var progress: CGFloat {
        guard extent > 0 else { return requestedPosition == .open ? 1 : 0 }
        return min(max(reveal / extent, 0), 1)
    }
    var isDragging: Bool { presentation.phase == .dragging }
    var isSettling: Bool {
        if case .settling = presentation.phase { return true }
        return false
    }
    var isGestureEligible: Bool { isConfigured && extent > 0 }
    var requiresEdgeZone: Bool {
        presentation.phase == .idle && presentation.target == .closed
    }
    var acceptedDirection: GaryxRouteGestureDirection {
        switch presentation.phase {
        case .settling, .dragging:
            .either
        case .idle:
            presentation.target == .open ? .negative : .positive
        }
    }

    func configure(extent newExtent: CGFloat, restingPosition: GaryxHorizontalRevealPosition) {
        let newExtent = max(0, newExtent)
        requestedPosition = restingPosition
        guard !isConfigured else {
            guard newExtent != extent else {
                if state.phase == .idle, state.settledPosition != restingPosition {
                    state.synchronize(to: restingPosition, extent: extent)
                    publish()
                }
                return
            }
            let oldExtent = extent
            let interrupted = settleDriver.interrupt()
            state.rederiveExtent(from: oldExtent, to: newExtent)
            extent = newExtent
            publish()
            if case .settling(let target) = state.phase, newExtent > 0 {
                let scale = oldExtent > 0 ? newExtent / oldExtent : 1
                startSettle(
                    .init(
                        target: target,
                        initialReveal: state.reveal,
                        initialVelocity: (interrupted?.velocity ?? 0) * scale
                    ),
                    animated: true
                )
            }
            return
        }

        extent = newExtent
        isConfigured = true
        state.synchronize(to: restingPosition, extent: newExtent)
        publish()
    }

    func setTarget(
        _ position: GaryxHorizontalRevealPosition,
        animated: Bool,
        initialVelocity: CGFloat = 0
    ) {
        requestedPosition = position
        guard isConfigured, extent > 0 else {
            settleDriver.invalidate()
            state.synchronize(to: position, extent: extent)
            publish()
            return
        }
        if !animated {
            settleDriver.invalidate()
            state.synchronize(to: position, extent: extent)
            publish()
            return
        }
        if case .settling(let target) = state.phase, target == position {
            return
        }
        _ = settleDriver.interrupt()
        guard let settle = state.beginProgrammaticSettle(
            to: position,
            initialVelocity: initialVelocity,
            extent: extent
        ) else {
            publish()
            return
        }
        publish()
        startSettle(settle, animated: true)
    }

    func beginGesture() {
        guard isGestureEligible else { return }
        let interrupted = settleDriver.interrupt()
        // `reveal` is the exact value drawn on the last frame. Adopt it at the
        // seam even if the underlying spring sample was outside an endpoint
        // and therefore rubber-banded for presentation.
        state.beginDrag(
            interruptedReveal: interrupted == nil ? nil : presentation.reveal,
            extent: extent
        )
        publish()
    }

    func updateGesture(logicalTranslation: CGFloat) {
        state.updateDrag(logicalTranslation: logicalTranslation, extent: extent)
        publish()
    }

    @discardableResult
    func endGesture(logicalVelocity: CGFloat) -> GaryxHorizontalRevealPosition? {
        guard let settle = state.release(
            logicalVelocity: logicalVelocity,
            extent: extent,
            projection: projection
        ) else { return nil }
        requestedPosition = settle.target
        publish()
        startSettle(settle, animated: true)
        return settle.target
    }

    @discardableResult
    func cancelGesture() -> GaryxHorizontalRevealPosition? {
        guard let settle = state.cancelDrag(extent: extent) else { return nil }
        requestedPosition = settle.target
        publish()
        startSettle(settle, animated: true)
        return settle.target
    }

    func invalidate(position: GaryxHorizontalRevealPosition) {
        requestedPosition = position
        settleDriver.invalidate()
        state.synchronize(to: position, extent: extent)
        publish()
    }

    private func startSettle(
        _ settle: GaryxHorizontalRevealSettle,
        animated: Bool
    ) {
        guard animated, extent > 0 else {
            state.synchronize(to: settle.target, extent: extent)
            publish()
            return
        }
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
                _ = state.finishSettle(extent: extent)
                publish()
            }
        )
    }

    private func publish() {
        let next = GaryxHorizontalRevealPresentation(
            reveal: state.reveal,
            phase: state.phase,
            target: state.targetPosition
        )
        if presentation != next {
            presentation = next
        }
    }
}

/// UIKit pan delivery for row-local gestures. Unlike SwiftUI `DragGesture`,
/// this exposes `.cancelled` as a first-class event so the Core state machine
/// can settle deterministically after system takeover.
@available(iOS 18.0, *)
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
