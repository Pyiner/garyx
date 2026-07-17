import QuartzCore
import UIKit

@MainActor
final class GaryxGestureSystemTimeSource: GaryxGestureSettleTimeSource {
    var now: TimeInterval {
        CACurrentMediaTime()
    }
}

/// The only platform-specific part of gesture settling. Core owns the
/// trajectory, lifecycle decisions, and interruption semantics.
final class GaryxGestureDisplayLinkFrameSource: NSObject, GaryxGestureSettleFrameSource {
    var onFrame: (() -> Void)?

    private var displayLink: CADisplayLink?

    func start() {
        displayLink?.invalidate()
        let displayLink = CADisplayLink(target: self, selector: #selector(handleFrame))
        displayLink.preferredFrameRateRange = CAFrameRateRange(
            minimum: 30,
            maximum: 120,
            preferred: 120
        )
        displayLink.add(to: .main, forMode: .common)
        self.displayLink = displayLink
    }

    func invalidate() {
        displayLink?.invalidate()
        displayLink = nil
    }

    deinit {
        displayLink?.invalidate()
    }

    @objc private func handleFrame() {
        onFrame?()
    }
}

@MainActor
extension GaryxGestureSettleDriver {
    static func displayLinked() -> GaryxGestureSettleDriver {
        GaryxGestureSettleDriver(
            timeSource: GaryxGestureSystemTimeSource(),
            frameSource: GaryxGestureDisplayLinkFrameSource()
        )
    }
}
