import SwiftUI
import UIKit

/// A direction-filtered UIKit recognizer for paged previews. SwiftUI's
/// DragGesture claims the paged TabView touch stream on iOS 26 even when it is
/// attached simultaneously. This recognizer fails before beginning for every
/// non-downward drag and explicitly shares recognition with the pager.
struct GaryxImagePreviewDismissGestureBridge: UIViewRepresentable {
    let isEnabled: Bool
    let onChanged: (CGFloat) -> Void
    let onEnded: () -> Void
    let onDismiss: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> InstallerView {
        let view = InstallerView(coordinator: context.coordinator)
        view.isUserInteractionEnabled = false
        return view
    }

    func updateUIView(_ uiView: InstallerView, context: Context) {
        context.coordinator.onChanged = onChanged
        context.coordinator.onEnded = onEnded
        context.coordinator.onDismiss = onDismiss
        context.coordinator.isEnabled = isEnabled
        uiView.setEnabled(isEnabled)
        uiView.installIfNeeded()
    }

    static func dismantleUIView(_ uiView: InstallerView, coordinator: Coordinator) {
        uiView.uninstall()
    }

    final class InstallerView: UIView {
        private weak var installedView: UIView?
        private let recognizer: UIPanGestureRecognizer

        init(coordinator: Coordinator) {
            recognizer = UIPanGestureRecognizer(target: coordinator, action: #selector(Coordinator.handlePan(_:)))
            super.init(frame: .zero)
            recognizer.delegate = coordinator
            recognizer.cancelsTouchesInView = false
            recognizer.delaysTouchesBegan = false
            recognizer.delaysTouchesEnded = false
            recognizer.maximumNumberOfTouches = 1
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("init(coder:) has not been implemented")
        }

        override func didMoveToWindow() {
            super.didMoveToWindow()
            installIfNeeded()
        }

        func installIfNeeded() {
            guard let window, installedView !== window else { return }
            uninstall()
            window.addGestureRecognizer(recognizer)
            installedView = window
        }

        func uninstall() {
            installedView?.removeGestureRecognizer(recognizer)
            installedView = nil
        }

        func setEnabled(_ enabled: Bool) {
            recognizer.isEnabled = enabled
        }
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var isEnabled = true
        var onChanged: (CGFloat) -> Void = { _ in }
        var onEnded: () -> Void = {}
        var onDismiss: () -> Void = {}

        @objc func handlePan(_ recognizer: UIPanGestureRecognizer) {
            guard let view = recognizer.view else { return }
            let translation = recognizer.translation(in: view)
            switch recognizer.state {
            case .changed:
                onChanged(max(0, translation.y))
            case .ended:
                let dragTranslation = CGSize(width: translation.x, height: translation.y)
                let phase = GaryxImagePreviewDismissGesture.classify(translation: dragTranslation)
                if GaryxImagePreviewDismissGesture.shouldDismiss(
                    phase: phase,
                    translation: dragTranslation
                ) {
                    onDismiss()
                } else {
                    onEnded()
                }
            case .cancelled, .failed:
                onEnded()
            default:
                break
            }
        }

        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard isEnabled,
                  let pan = gestureRecognizer as? UIPanGestureRecognizer,
                  let view = pan.view else {
                return false
            }
            let velocity = pan.velocity(in: view)
            return GaryxImagePreviewDismissGesture.isDownwardIntent(
                CGSize(width: velocity.x, height: velocity.y)
            )
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }
}
