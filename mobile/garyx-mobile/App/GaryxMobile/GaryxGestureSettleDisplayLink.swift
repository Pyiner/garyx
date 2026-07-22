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
    private(set) var latestFrameTimestamp: TimeInterval?

    private var displayLink: CADisplayLink?

    func start() {
        displayLink?.invalidate()
        latestFrameTimestamp = nil
        let displayLink = CADisplayLink(target: self, selector: #selector(handleFrame(_:)))
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
        latestFrameTimestamp = nil
    }

    deinit {
        displayLink?.invalidate()
    }

    @objc private func handleFrame(_ displayLink: CADisplayLink) {
        latestFrameTimestamp = displayLink.timestamp
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
