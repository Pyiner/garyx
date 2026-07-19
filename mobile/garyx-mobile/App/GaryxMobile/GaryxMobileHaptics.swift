import UIKit

/// The app's only UIKit haptic adapter. Generators stay cached so a touch-down,
/// gesture-begin, or operation-begin preparation warms the exact instance that
/// emits at commit. Every emission re-arms that instance for repeated actions.
@MainActor
final class GaryxMobileHaptics {
    static let shared = GaryxMobileHaptics()

    private lazy var lightImpact = UIImpactFeedbackGenerator(style: .light)
    private lazy var mediumImpact = UIImpactFeedbackGenerator(style: .medium)
    private lazy var notification = UINotificationFeedbackGenerator()
    private lazy var selection = UISelectionFeedbackGenerator()

    private init() {}

    func prepare(_ event: GaryxHapticEvent) {
        prepare(event.specification.pattern)
    }

    func play(_ event: GaryxHapticEvent) {
        let pattern = event.specification.pattern
        switch pattern {
        case .impact(.light):
            lightImpact.impactOccurred()
        case .impact(.medium):
            mediumImpact.impactOccurred()
        case .notification(.success):
            notification.notificationOccurred(.success)
        case .notification(.error):
            notification.notificationOccurred(.error)
        case .selection:
            selection.selectionChanged()
        }
        prepare(pattern)
    }

    private func prepare(_ pattern: GaryxHapticPattern) {
        switch pattern {
        case .impact(.light):
            lightImpact.prepare()
        case .impact(.medium):
            mediumImpact.prepare()
        case .notification:
            notification.prepare()
        case .selection:
            selection.prepare()
        }
    }
}
