import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

/// Samples the velocity passed to Core at release. UIKit commonly emits a
/// stationary `touchesEnded` sample just after the final movement; preserving
/// a recent movement sample keeps a real flick from being flattened to zero,
/// while a deliberate hold still expires to zero.
struct GaryxCapsuleDismissVelocitySampler: Equatable {
    static let minimumMovement: CGFloat = 0.5
    static let maximumReleaseSampleAge: TimeInterval = 0.12

    private(set) var previousLocation = CGPoint.zero
    private(set) var previousTimestamp: TimeInterval = 0
    private(set) var velocity = CGSize.zero

    mutating func begin(location: CGPoint, timestamp: TimeInterval) {
        previousLocation = location
        previousTimestamp = timestamp
        velocity = .zero
    }

    @discardableResult
    mutating func sample(
        location: CGPoint,
        timestamp: TimeInterval,
        isRelease: Bool
    ) -> CGSize {
        let elapsed = timestamp - previousTimestamp
        let delta = CGSize(
            width: location.x - previousLocation.x,
            height: location.y - previousLocation.y
        )
        let moved = hypot(delta.width, delta.height) >= Self.minimumMovement

        if elapsed > 0, moved {
            velocity = CGSize(width: delta.width / elapsed, height: delta.height / elapsed)
            previousLocation = location
            previousTimestamp = timestamp
        } else if !isRelease || elapsed > Self.maximumReleaseSampleAge {
            velocity = .zero
            previousLocation = location
            previousTimestamp = timestamp
        }
        return velocity
    }

    mutating func reset() {
        previousLocation = .zero
        previousTimestamp = 0
        velocity = .zero
    }
}

/// Mutable bridge shared by the focused SwiftUI surface, its WKWebView, and the
/// one container-level recognizer. It intentionally owns no gesture decisions;
/// both recognizer ownership and visible state feed the same Core reducer.
@MainActor
final class GaryxCapsuleDismissGestureBridge: ObservableObject {
    weak var webViewPanGestureRecognizer: UIPanGestureRecognizer?
    var webAtTop = true
    var panelPresented = false
    var containerWidth: CGFloat = 0
    var onChanged: ((CGFloat, CGSize) -> Void)?
    var onReleased: ((CGSize) -> Void)?
    var onCancelled: (() -> Void)?
}

struct GaryxCapsuleDismissGestureInstaller: UIViewRepresentable {
    @ObservedObject var bridge: GaryxCapsuleDismissGestureBridge
    let webAtTop: Bool
    let panelPresented: Bool
    let containerWidth: CGFloat
    let onChanged: (CGFloat, CGSize) -> Void
    let onReleased: (CGSize) -> Void
    let onCancelled: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(bridge: bridge)
    }

    func makeUIView(context: Context) -> InstallerView {
        let view = InstallerView()
        view.isUserInteractionEnabled = false
        view.onWindowChange = { [weak coordinator = context.coordinator] window in
            coordinator?.install(on: window)
        }
        return view
    }

    func updateUIView(_ uiView: InstallerView, context: Context) {
        context.coordinator.bridge = bridge
        bridge.webAtTop = webAtTop
        bridge.panelPresented = panelPresented
        bridge.containerWidth = containerWidth
        bridge.onChanged = onChanged
        bridge.onReleased = onReleased
        bridge.onCancelled = onCancelled
        context.coordinator.install(on: uiView.window)
    }

    static func dismantleUIView(_ uiView: InstallerView, coordinator: Coordinator) {
        coordinator.uninstall()
        coordinator.bridge.onChanged = nil
        coordinator.bridge.onReleased = nil
        coordinator.bridge.onCancelled = nil
        uiView.onWindowChange = nil
    }

    final class InstallerView: UIView {
        var onWindowChange: ((UIWindow?) -> Void)?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            onWindowChange?(window)
        }
    }

    @MainActor
    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var bridge: GaryxCapsuleDismissGestureBridge
        private weak var installedWindow: UIWindow?
        private lazy var recognizer: GaryxCapsuleContinuousDismissGestureRecognizer = {
            let recognizer = GaryxCapsuleContinuousDismissGestureRecognizer()
            recognizer.delegate = self
            recognizer.cancelsTouchesInView = true
            recognizer.addTarget(self, action: #selector(handleRecognizer(_:)))
            recognizer.context = { [weak self] in
                guard let self else { return (true, false) }
                return (self.bridge.webAtTop, self.bridge.panelPresented)
            }
            return recognizer
        }()

        init(bridge: GaryxCapsuleDismissGestureBridge) {
            self.bridge = bridge
        }

        func install(on window: UIWindow?) {
            guard installedWindow !== window else { return }
            uninstall()
            guard let window else { return }
            window.addGestureRecognizer(recognizer)
            installedWindow = window
        }

        func uninstall() {
            if let installedWindow {
                installedWindow.removeGestureRecognizer(recognizer)
            }
            installedWindow = nil
            // Do not write SwiftUI state from `dismantleUIView`: teardown can
            // already hold the State location's exclusive update access. The
            // focused view resets in `onDisappear`, while real gesture/system
            // cancellation still arrives through the recognizer's `.cancelled`
            // state. Calling the bridge here causes a fatal access conflict when
            // shared routing unmounts the full-screen cover.
        }

        /// Web content pan must wait for the container recognizer to decide at
        /// 14pt. If Core says `.ignored`, this recognizer fails and WKWebView
        /// receives the complete pan; if Core owns either axis, WebKit never
        /// produces visible movement.
        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldBeRequiredToFailBy otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            otherGestureRecognizer === bridge.webViewPanGestureRecognizer
        }

        @objc private func handleRecognizer(
            _ recognizer: GaryxCapsuleContinuousDismissGestureRecognizer
        ) {
            switch recognizer.state {
            case .began, .changed:
                bridge.onChanged?(recognizer.startX, recognizer.translation)
            case .ended:
                bridge.onChanged?(recognizer.startX, recognizer.translation)
                bridge.onReleased?(recognizer.velocity)
            case .cancelled:
                bridge.onCancelled?()
            default:
                break
            }
        }
    }
}

/// A continuous recognizer whose begin timing is under our control. It stays in
/// `.possible` while accumulating movement, then at 14pt invokes the shared Core
/// classifier exactly once: owned axis -> `.began`; ignored -> `.failed`.
@MainActor
final class GaryxCapsuleContinuousDismissGestureRecognizer: UIGestureRecognizer {
    typealias Context = () -> (webAtTop: Bool, panelPresented: Bool)

    var context: Context = { (true, false) }
    private(set) var startX: CGFloat = 0
    private(set) var translation: CGSize = .zero
    private(set) var velocity: CGSize = .zero

    private var startLocation: CGPoint = .zero
    private var velocitySampler = GaryxCapsuleDismissVelocitySampler()

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard touches.count == 1, let touch = touches.first, let view else {
            state = .failed
            return
        }
        let location = touch.location(in: view)
        startLocation = location
        velocitySampler.begin(location: location, timestamp: touch.timestamp)
        startX = location.x
        translation = .zero
        velocity = .zero
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let touch = touches.first, let view else { return }
        updateMotion(touch: touch, in: view)

        if state == .possible {
            let currentContext = context()
            let phase = GaryxCapsuleDragDismiss.classify(
                startX: startX,
                translation: translation,
                webAtTop: currentContext.webAtTop,
                panelPresented: currentContext.panelPresented
            )
            switch phase {
            case .pending:
                break
            case .horizontalDismiss, .verticalDismiss:
                state = .began
            case .ignored:
                state = .failed
            }
        } else if state == .began || state == .changed {
            state = .changed
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        if let touch = touches.first, let view {
            updateMotion(touch: touch, in: view, isRelease: true)
        }
        switch state {
        case .began, .changed:
            state = .ended
        case .possible:
            state = .failed
        default:
            break
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .began || state == .changed {
            state = .cancelled
        } else if state == .possible {
            state = .failed
        }
    }

    override func reset() {
        super.reset()
        startX = 0
        translation = .zero
        velocity = .zero
        startLocation = .zero
        velocitySampler.reset()
    }

    private func updateMotion(touch: UITouch, in view: UIView, isRelease: Bool = false) {
        let location = touch.location(in: view)
        translation = CGSize(
            width: location.x - startLocation.x,
            height: location.y - startLocation.y
        )
        velocity = velocitySampler.sample(
            location: location,
            timestamp: touch.timestamp,
            isRelease: isRelease
        )
    }
}
