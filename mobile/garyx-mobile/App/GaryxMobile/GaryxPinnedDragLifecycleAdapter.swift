import Combine
import SwiftUI
import UIKit

enum GaryxPinnedThreadReorderRuntimeGate {
    /// The adapter is intentionally enabled only on runtime families exercised
    /// by the architecture gate. Adding another runtime requires rerunning the
    /// lifecycle, arbitration, animation, and hitch suites first.
    static var isVerifiedRuntime: Bool {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        return version.majorVersion == 26 && version.minorVersion == 5
    }

    static var isFeatureEnabled: Bool {
        guard isVerifiedRuntime else { return false }
        #if DEBUG
        return ProcessInfo.processInfo.environment["GARYX_MOBILE_PIN_REORDER_DISABLED"] != "1"
        #else
        return true
        #endif
    }

    #if DEBUG
    static var isArchitectureSpikeEnabled: Bool {
        isFeatureEnabled
            && ProcessInfo.processInfo.environment["GARYX_MOBILE_PIN_REORDER_SPIKE"] == "1"
    }
    #endif
}

@MainActor
final class GaryxPinnedDragLifecycleController: NSObject, ObservableObject {
    struct Callbacks {
        var began: () -> Void
        var moved: () -> Void
        var accepted: (_ previewOrder: [String]) -> Void
        var cancelled: () -> Void

        static let none = Callbacks(began: {}, moved: {}, accepted: { _ in }, cancelled: {})
    }

    @Published private(set) var isDragging = false
    @Published private(set) var beganCount = 0
    @Published private(set) var acceptedCount = 0
    @Published private(set) var cancelledCount = 0
    @Published private(set) var previewCallbackCount = 0
    @Published private(set) var movementCount = 0
    @Published private(set) var delegatesUnchanged = true
    @Published private(set) var lastClassification = "idle"
    @Published private(set) var observedRecognizerNames = "none"

    private weak var collectionView: UICollectionView?
    private var callbacks = Callbacks.none
    private var displayLink: CADisplayLink?
    private var observedRecognizers: [UIGestureRecognizer] = []
    private var expectedDelegateIdentity: ObjectIdentifier?
    private var expectedDragDelegateIdentity: ObjectIdentifier?
    private var expectedDropDelegateIdentity: ObjectIdentifier?
    private var wasActiveDrag = false
    private var sawActiveDrop = false
    private var previewOrder: [String]?
    private var candidateRecognizers = Set<ObjectIdentifier>()
    private var terminalPoint: CGPoint?
    private var initialPoints: [ObjectIdentifier: CGPoint] = [:]
    private var notifiedMovement = false
    private var sawTerminalEnd = false
    private var sawTerminalCancellation = false
    private var pendingClassification = UUID()

    deinit {
        displayLink?.invalidate()
    }

    func configure(callbacks: Callbacks) {
        self.callbacks = callbacks
    }

    func attach(to collectionView: UICollectionView) {
        guard self.collectionView !== collectionView else {
            observeNewRecognizers()
            return
        }
        detach()
        self.collectionView = collectionView
        expectedDelegateIdentity = identity(of: collectionView.delegate)
        expectedDragDelegateIdentity = identity(of: collectionView.dragDelegate)
        expectedDropDelegateIdentity = identity(of: collectionView.dropDelegate)
        delegatesUnchanged = true
        observeNewRecognizers()

        let link = CADisplayLink(target: self, selector: #selector(sampleCollectionView))
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    func notePreviewMove(_ order: [String]) {
        if !isDragging, let collectionView {
            beginSession(in: collectionView)
            wasActiveDrag = collectionView.hasActiveDrag
        }
        guard isDragging else { return }
        notifyMovementIfNeeded()
        previewOrder = order
        previewCallbackCount += 1
    }

    var debugReport: String {
        [
            "dragging=\(isDragging ? 1 : 0)",
            "began=\(beganCount)",
            "accepted=\(acceptedCount)",
            "cancelled=\(cancelledCount)",
            "preview_callbacks=\(previewCallbackCount)",
            "movements=\(movementCount)",
            "delegates_unchanged=\(delegatesUnchanged ? 1 : 0)",
            "classification=\(lastClassification)",
        ].joined(separator: " ")
    }

    private func detach() {
        displayLink?.invalidate()
        displayLink = nil
        for recognizer in observedRecognizers {
            recognizer.removeTarget(self, action: #selector(observeGesture(_:)))
        }
        observedRecognizers.removeAll()
        collectionView = nil
        wasActiveDrag = false
    }

    private func observeNewRecognizers() {
        guard let collectionView else { return }
        let existing = Set(observedRecognizers.map(ObjectIdentifier.init))
        let recognizers = allRecognizers(in: collectionView)
        for recognizer in recognizers where !existing.contains(ObjectIdentifier(recognizer)) {
            // `addTarget` is observation-only: UIKit supports multiple targets
            // on one recognizer, and SwiftUI's delegates remain untouched.
            recognizer.addTarget(self, action: #selector(observeGesture(_:)))
            observedRecognizers.append(recognizer)
        }
        let names = Set(recognizers.map { String(describing: type(of: $0)) })
            .sorted()
            .joined(separator: ",")
        if observedRecognizerNames != names {
            observedRecognizerNames = names
        }
    }

    @objc private func sampleCollectionView() {
        guard let collectionView else { return }
        verifyDelegateIdentity(collectionView)

        let hasActiveDrag = collectionView.hasActiveDrag
        if hasActiveDrag && !wasActiveDrag && !isDragging {
            beginSession(in: collectionView)
        }
        if hasActiveDrag {
            sawActiveDrop = sawActiveDrop || collectionView.hasActiveDrop
        }
        if !hasActiveDrag && wasActiveDrag {
            scheduleClassification(in: collectionView)
        }
        wasActiveDrag = hasActiveDrag
    }

    private func beginSession(in collectionView: UICollectionView) {
        pendingClassification = UUID()
        previewOrder = nil
        candidateRecognizers.removeAll()
        terminalPoint = nil
        initialPoints.removeAll()
        notifiedMovement = false
        sawTerminalEnd = false
        sawTerminalCancellation = false
        sawActiveDrop = collectionView.hasActiveDrop
        for recognizer in observedRecognizers where recognizer !== collectionView.panGestureRecognizer {
            if recognizer.state == .began || recognizer.state == .changed {
                let identity = ObjectIdentifier(recognizer)
                candidateRecognizers.insert(identity)
                terminalPoint = recognizer.location(in: collectionView)
                initialPoints[identity] = terminalPoint
            }
        }
        isDragging = true
        beganCount += 1
        lastClassification = "active"
        callbacks.began()
    }

    @objc private func observeGesture(_ recognizer: UIGestureRecognizer) {
        guard isDragging, let collectionView else { return }
        let identity = ObjectIdentifier(recognizer)
        let typeName = String(describing: type(of: recognizer)).lowercased()
        let isDragNamed = typeName.contains("drag") || typeName.contains("lift")
        guard candidateRecognizers.contains(identity) || isDragNamed else { return }

        candidateRecognizers.insert(identity)
        terminalPoint = recognizer.location(in: collectionView)
        if initialPoints[identity] == nil {
            initialPoints[identity] = terminalPoint
        }
        if typeName.contains("draglift"),
           let initialPoint = initialPoints[identity],
           let terminalPoint {
            let distance = hypot(
                terminalPoint.x - initialPoint.x,
                terminalPoint.y - initialPoint.y
            )
            if distance >= 8 {
                notifyMovementIfNeeded()
            }
        }
        switch recognizer.state {
        case .ended:
            sawTerminalEnd = true
        case .cancelled, .failed:
            sawTerminalCancellation = true
        default:
            break
        }
    }

    private func scheduleClassification(in collectionView: UICollectionView) {
        let token = UUID()
        pendingClassification = token
        // SwiftUI can publish the final `onMove` callback one run-loop turn
        // after UIKit ends the drag. Defer classification so the callback is
        // part of the same session without delaying visible settle animation.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) { [weak self, weak collectionView] in
            guard let self, let collectionView, self.pendingClassification == token else { return }
            self.classifyEndedSession(in: collectionView)
        }
    }

    private func notifyMovementIfNeeded() {
        guard !notifiedMovement else { return }
        notifiedMovement = true
        movementCount += 1
        callbacks.moved()
    }

    private func classifyEndedSession(in collectionView: UICollectionView) {
        let pointIsInside = terminalPoint.map(collectionView.bounds.contains) ?? false
        let hasPreview = previewOrder != nil
        let endedNormally = sawTerminalEnd || sawActiveDrop
        let accepted = hasPreview
            && endedNormally
            && !sawTerminalCancellation
            && pointIsInside

        isDragging = false
        if accepted, let previewOrder {
            acceptedCount += 1
            lastClassification = "accepted"
            callbacks.accepted(previewOrder)
        } else {
            cancelledCount += 1
            lastClassification = "cancelled"
            callbacks.cancelled()
        }
        self.previewOrder = nil
        candidateRecognizers.removeAll()
    }

    private func verifyDelegateIdentity(_ collectionView: UICollectionView) {
        let unchanged = delegatesUnchanged
            && identity(of: collectionView.delegate) == expectedDelegateIdentity
            && identity(of: collectionView.dragDelegate) == expectedDragDelegateIdentity
            && identity(of: collectionView.dropDelegate) == expectedDropDelegateIdentity
        if delegatesUnchanged != unchanged {
            delegatesUnchanged = unchanged
        }
    }

    private func identity(of object: AnyObject?) -> ObjectIdentifier? {
        object.map(ObjectIdentifier.init)
    }

    private func allRecognizers(in root: UIView) -> [UIGestureRecognizer] {
        // Native List drag/drop recognizers are owned by the collection view.
        // Do not walk recycled cells: retaining their row recognizers would
        // grow observation work during long scroll runs and is unnecessary for
        // lifecycle classification.
        root.gestureRecognizers ?? []
    }
}

struct GaryxPinnedDragLifecycleAdapter: UIViewRepresentable {
    @ObservedObject var controller: GaryxPinnedDragLifecycleController

    func makeCoordinator() -> Coordinator {
        Coordinator(controller: controller)
    }

    func makeUIView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.isUserInteractionEnabled = false
        view.onWindowChange = { [weak coordinator = context.coordinator, weak view] in
            coordinator?.attachNearestCollection(to: view)
        }
        context.coordinator.attachNearestCollection(to: view)
        return view
    }

    func updateUIView(_ uiView: ProbeView, context: Context) {
        context.coordinator.controller = controller
        context.coordinator.attachNearestCollection(to: uiView)
    }

    final class ProbeView: UIView {
        var onWindowChange: (() -> Void)?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            onWindowChange?()
        }

        override func didMoveToSuperview() {
            super.didMoveToSuperview()
            onWindowChange?()
        }
    }

    @MainActor
    final class Coordinator {
        var controller: GaryxPinnedDragLifecycleController
        private var scheduledAttach = false

        init(controller: GaryxPinnedDragLifecycleController) {
            self.controller = controller
        }

        func attachNearestCollection(to probe: UIView?) {
            guard let probe else { return }
            if let collection = nearestCollection(to: probe) {
                scheduledAttach = false
                controller.attach(to: collection)
                return
            }
            guard !scheduledAttach else { return }
            scheduledAttach = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [weak self, weak probe] in
                self?.scheduledAttach = false
                self?.attachNearestCollection(to: probe)
            }
        }

        private func nearestCollection(to probe: UIView) -> UICollectionView? {
            var ancestor = probe.superview
            while let view = ancestor {
                if let collection = view as? UICollectionView { return collection }
                ancestor = view.superview
            }
            guard let window = probe.window else { return nil }
            let probeFrame = probe.convert(probe.bounds, to: window)
            return collections(in: window)
                .filter { !$0.isHidden && $0.alpha > 0 }
                .max { lhs, rhs in
                    intersectionArea(lhs, with: probeFrame, in: window)
                        < intersectionArea(rhs, with: probeFrame, in: window)
                }
        }

        private func collections(in view: UIView) -> [UICollectionView] {
            var result = (view as? UICollectionView).map { [$0] } ?? []
            for subview in view.subviews {
                result.append(contentsOf: collections(in: subview))
            }
            return result
        }

        private func intersectionArea(
            _ collection: UICollectionView,
            with probeFrame: CGRect,
            in window: UIWindow
        ) -> CGFloat {
            let frame = collection.convert(collection.bounds, to: window)
            let intersection = frame.intersection(probeFrame)
            guard !intersection.isNull else { return 0 }
            return intersection.width * intersection.height
        }
    }
}
