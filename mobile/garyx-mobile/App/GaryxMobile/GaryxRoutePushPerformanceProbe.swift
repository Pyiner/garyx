import QuartzCore
import UIKit

/// Opt-in, Release-capable frame probe for the production list-to-route path.
///
/// The A4a route diagnostics observe settle callbacks, which means they cannot
/// see a main-thread stall before the first callback or between the terminal
/// callback and the next presented frame. This probe owns an always-running
/// `CADisplayLink` while enabled and brackets the whole push transaction, from
/// before destination mounting through post-terminal presentation.
///
/// Enable with `GARYX_MOBILE_ROUTE_PUSH_PROBE=1`. The final machine-readable
/// report is exposed through accessibility and written to the app cache as
/// `garyx-route-push-probe.txt` for Release simulator/device captures.
@MainActor
final class GaryxRoutePushPerformanceProbe: NSObject {
    enum Stage: String {
        case idle
        case destinationMount = "destination_mount"
        case preCommit = "pre_commit"
        case commitSettle = "commit_settle"
        case canonicalProjection = "canonical_projection"
        case terminalActivation = "terminal_activation"
        case liveContentPreparation = "live_content_preparation"
        case contentPresentation = "content_presentation"
        case placeholderReveal = "placeholder_reveal"
        case postTerminal = "post_terminal"
    }

    private struct FrameMetrics {
        var frameCount = 0
        var hitchCount = 0
        var maximumInterval: CFTimeInterval = 0
        var maximumOverBudget: CFTimeInterval = 0

        mutating func record(interval: CFTimeInterval, budget: CFTimeInterval) {
            frameCount += 1
            if interval > maximumInterval {
                maximumInterval = interval
                maximumOverBudget = max(0, interval - budget)
            }
            if interval > budget * 1.5 {
                hitchCount += 1
            }
        }
    }

    static let shared: GaryxRoutePushPerformanceProbe? = {
        guard ProcessInfo.processInfo.environment["GARYX_MOBILE_ROUTE_PUSH_PROBE"] == "1" else {
            return nil
        }
        return GaryxRoutePushPerformanceProbe()
    }()

    private let statusLabel = UILabel()
    private var displayLink: CADisplayLink?
    private weak var container: GaryxRouteStackContainer?
    private var currentStage: Stage = .idle
    private var isRecording = false
    private var transactionCount = 0
    private var beginTimestamp: CFTimeInterval?
    private var mountCompletedTimestamp: CFTimeInterval?
    private var canonicalProjectionTimestamp: CFTimeInterval?
    private var terminalTimestamp: CFTimeInterval?
    private var livePreparationTimestamp: CFTimeInterval?
    private var contentTimestamp: CFTimeInterval?
    private var placeholderRevealTimestamp: CFTimeInterval?
    private var previousDisplayTimestamp: CFTimeInterval?
    private var firstDisplayTimestamp: CFTimeInterval?
    private var frameBudget: CFTimeInterval = 1.0 / 120.0
    private var frameCount = 0
    private var hitchCount = 0
    private var maximumFrameInterval: CFTimeInterval = 0
    private var maximumOverBudget: CFTimeInterval = 0
    private var worstStage: Stage = .idle
    private var postRevealFrameCount = 0
    private var revealTimedOut = false
    private var contentRowCount = 0
    private var transitionWindowOpen = false
    private var transitionMetrics = FrameMetrics()
    private var maskedPreparationMetrics = FrameMetrics()
    private var postRevealMetrics = FrameMetrics()
    private var hitchEvents: [String] = []

    func install(in container: GaryxRouteStackContainer) {
        guard self.container == nil else { return }
        self.container = container

        statusLabel.translatesAutoresizingMaskIntoConstraints = false
        statusLabel.font = .monospacedSystemFont(ofSize: 8, weight: .regular)
        statusLabel.textColor = .secondaryLabel
        statusLabel.backgroundColor = UIColor.systemBackground.withAlphaComponent(0.94)
        statusLabel.numberOfLines = 2
        statusLabel.isUserInteractionEnabled = false
        statusLabel.accessibilityIdentifier = "route-push-probe-report"
        statusLabel.text = "GARYX_ROUTE_PUSH_PROBE state=ready"
        statusLabel.accessibilityValue = statusLabel.text
        container.view.addSubview(statusLabel)
        NSLayoutConstraint.activate([
            statusLabel.leadingAnchor.constraint(equalTo: container.view.leadingAnchor, constant: 8),
            statusLabel.trailingAnchor.constraint(equalTo: container.view.trailingAnchor, constant: -8),
            statusLabel.bottomAnchor.constraint(
                equalTo: container.view.safeAreaLayoutGuide.bottomAnchor,
                constant: -4
            ),
        ])

        let link = CADisplayLink(target: self, selector: #selector(stepDisplayLink(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(
            minimum: 80,
            maximum: 120,
            preferred: 120
        )
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    func transitionWillBegin(kind: GaryxRouteTransitionKind) {
        guard kind == .push else { return }
        transactionCount += 1
        isRecording = true
        currentStage = .destinationMount
        beginTimestamp = CACurrentMediaTime()
        mountCompletedTimestamp = nil
        canonicalProjectionTimestamp = nil
        terminalTimestamp = nil
        livePreparationTimestamp = nil
        contentTimestamp = nil
        placeholderRevealTimestamp = nil
        // The idle tick before the tap is outside this transaction. Start a
        // fresh interval series at the first delivered push tick and report
        // begin-to-first-tick latency separately; otherwise XCTest/idb input
        // scheduling can be misclassified as an in-transition dropped frame.
        previousDisplayTimestamp = nil
        firstDisplayTimestamp = nil
        frameCount = 0
        hitchCount = 0
        maximumFrameInterval = 0
        maximumOverBudget = 0
        worstStage = .idle
        postRevealFrameCount = 0
        revealTimedOut = false
        contentRowCount = 0
        transitionWindowOpen = true
        transitionMetrics = FrameMetrics()
        maskedPreparationMetrics = FrameMetrics()
        postRevealMetrics = FrameMetrics()
        hitchEvents = []
        // Do not mutate the visible/accessibility report element while the
        // transition is moving. With an XCUI accessibility session attached,
        // that write itself can trigger AX-tree and layout work on the first
        // animated frame. The completed report is published after sampling.
    }

    func transitionHostsMounted() {
        guard isRecording else { return }
        mountCompletedTimestamp = CACurrentMediaTime()
        currentStage = .preCommit
    }

    func transitionPhaseChanged(_ phase: GaryxPresentationTransactionPhase) {
        guard isRecording else { return }
        switch phase {
        case .active:
            break
        case .preCommit:
            currentStage = .preCommit
        case .commitSettle:
            currentStage = .commitSettle
        case .cancelSettle:
            // A list push cannot cancel; retain the closest route-stage label
            // if a future caller exposes cancellation through this path.
            currentStage = .commitSettle
        case .terminal:
            terminalTimestamp = CACurrentMediaTime()
            currentStage = .terminalActivation
            postRevealFrameCount = 0
        }
    }

    func canonicalProjectionWillApply() {
        guard isRecording else { return }
        currentStage = .canonicalProjection
    }

    func canonicalProjectionDidApply() {
        guard isRecording else { return }
        canonicalProjectionTimestamp = CACurrentMediaTime()
        currentStage = .commitSettle
    }

    func visibleRouteActivated() {
        guard isRecording else { return }
        currentStage = contentTimestamp == nil ? .postTerminal : .contentPresentation
    }

    func markConversationContent(rowCount: Int) {
        guard isRecording, rowCount > 0 else { return }
        contentRowCount = max(contentRowCount, rowCount)
        if contentTimestamp == nil {
            contentTimestamp = CACurrentMediaTime()
        }
        currentStage = .contentPresentation
    }

    func liveContentPreparationBegan() {
        guard isRecording else { return }
        if livePreparationTimestamp == nil {
            livePreparationTimestamp = CACurrentMediaTime()
        }
        currentStage = .liveContentPreparation
    }

    func placeholderRevealBegan() {
        guard isRecording else { return }
        if placeholderRevealTimestamp == nil {
            placeholderRevealTimestamp = CACurrentMediaTime()
        }
        currentStage = .placeholderReveal
    }

    @objc private func stepDisplayLink(_ link: CADisplayLink) {
        let scheduledInterval = max(0, link.targetTimestamp - link.timestamp)
        if scheduledInterval > 0 {
            frameBudget = min(max(scheduledInterval, 1.0 / 120.0), 1.0 / 30.0)
        }

        guard isRecording, let beginTimestamp else { return }
        // A tap may begin after the current display-link callback was queued.
        // In that case `link.timestamp` still names the pre-transaction VSync;
        // using it as the first sample would fabricate a 33 ms transition
        // interval even when the first animated presentation arrives one
        // cadence after the tap.
        guard link.timestamp >= beginTimestamp else {
            previousDisplayTimestamp = nil
            return
        }
        if firstDisplayTimestamp == nil {
            firstDisplayTimestamp = link.timestamp
        }
        guard let previousDisplayTimestamp else {
            self.previousDisplayTimestamp = link.timestamp
            return
        }

        let interval = max(0, link.timestamp - previousDisplayTimestamp)
        self.previousDisplayTimestamp = link.timestamp
        frameCount += 1
        if interval > maximumFrameInterval {
            maximumFrameInterval = interval
            maximumOverBudget = max(0, interval - frameBudget)
            worstStage = currentStage
        }
        if interval > frameBudget * 1.5 {
            hitchCount += 1
            let elapsed = max(0, link.timestamp - beginTimestamp)
            hitchEvents.append(
                "\(milliseconds(elapsed)):\(currentStage.rawValue):\(milliseconds(interval))"
            )
        }

        if transitionWindowOpen {
            // Include the first delivered tick after terminal so the final
            // animated frame cannot disappear between marker and sampling.
            transitionMetrics.record(interval: interval, budget: frameBudget)
            if terminalTimestamp != nil {
                transitionWindowOpen = false
            }
        } else if placeholderRevealTimestamp == nil {
            // Live content is being prepared behind an opaque, non-animated
            // placeholder. These intervals are measured but not perceptible
            // motion hitches.
            maskedPreparationMetrics.record(interval: interval, budget: frameBudget)
        } else {
            postRevealMetrics.record(interval: interval, budget: frameBudget)
        }

        if terminalTimestamp != nil, placeholderRevealTimestamp != nil {
            postRevealFrameCount += 1
            // Keep sampling after the actual reveal, not merely after route
            // terminal. Network/disk preparation may legitimately outlive the
            // first twelve terminal frames.
            if postRevealFrameCount >= 12 {
                finish()
            }
        } else if link.timestamp - beginTimestamp >= 5 {
            revealTimedOut = true
            finish()
        }
    }

    private func finish() {
        guard isRecording, let beginTimestamp else { return }
        let reportTimestamp = CACurrentMediaTime()
        let hitchEventReport = hitchEvents.isEmpty
            ? "none"
            : hitchEvents.joined(separator: ",")
        let line = [
            "GARYX_ROUTE_PUSH_PROBE",
            "transaction=\(transactionCount)",
            "frame_budget_ms=\(milliseconds(frameBudget))",
            "frame_count=\(frameCount)",
            "hitch_count=\(hitchCount)",
            "max_interval_ms=\(milliseconds(maximumFrameInterval))",
            "max_over_budget_ms=\(milliseconds(maximumOverBudget))",
            "worst_stage=\(worstStage.rawValue)",
            "transition_frame_count=\(transitionMetrics.frameCount)",
            "transition_hitch_count=\(transitionMetrics.hitchCount)",
            "transition_max_interval_ms=\(milliseconds(transitionMetrics.maximumInterval))",
            "transition_max_over_budget_ms=\(milliseconds(transitionMetrics.maximumOverBudget))",
            "masked_hitch_count=\(maskedPreparationMetrics.hitchCount)",
            "masked_max_interval_ms=\(milliseconds(maskedPreparationMetrics.maximumInterval))",
            "post_reveal_hitch_count=\(postRevealMetrics.hitchCount)",
            "post_reveal_max_interval_ms=\(milliseconds(postRevealMetrics.maximumInterval))",
            "perceptible_hitch_count=\(transitionMetrics.hitchCount + postRevealMetrics.hitchCount)",
            "reveal_observed=\(placeholderRevealTimestamp == nil ? 0 : 1)",
            "reveal_timed_out=\(revealTimedOut ? 1 : 0)",
            "hitch_events=\(hitchEventReport)",
            "begin_to_first_tick_ms=\(milliseconds(delta(firstDisplayTimestamp, beginTimestamp)))",
            "mount_ms=\(milliseconds(delta(mountCompletedTimestamp, beginTimestamp)))",
            "begin_to_projection_ms=\(milliseconds(delta(canonicalProjectionTimestamp, beginTimestamp)))",
            "begin_to_terminal_ms=\(milliseconds(delta(terminalTimestamp, beginTimestamp)))",
            "terminal_to_preparation_ms=\(milliseconds(delta(livePreparationTimestamp, terminalTimestamp)))",
            "terminal_to_content_ms=\(milliseconds(delta(contentTimestamp, terminalTimestamp)))",
            "terminal_to_reveal_ms=\(milliseconds(delta(placeholderRevealTimestamp, terminalTimestamp)))",
            "duration_ms=\(milliseconds(reportTimestamp - beginTimestamp))",
            "content_rows=\(contentRowCount)",
        ].joined(separator: " ")
        isRecording = false
        currentStage = .idle
        publish(line)
        writeReport(line)
        print(line)
    }

    private func publish(_ line: String) {
        statusLabel.text = line
        statusLabel.accessibilityValue = line
        if let container {
            container.view.bringSubviewToFront(statusLabel)
        }
    }

    private func writeReport(_ line: String) {
        guard let cacheURL = FileManager.default.urls(
            for: .cachesDirectory,
            in: .userDomainMask
        ).first else { return }
        let reportURL = cacheURL.appendingPathComponent(
            "garyx-route-push-probe.txt",
            isDirectory: false
        )
        try? line.appending("\n").write(to: reportURL, atomically: true, encoding: .utf8)
    }

    private func delta(
        _ later: CFTimeInterval?,
        _ earlier: CFTimeInterval?
    ) -> CFTimeInterval {
        guard let later, let earlier else { return -1 }
        return max(0, later - earlier)
    }

    private func milliseconds(_ value: CFTimeInterval) -> String {
        guard value >= 0 else { return "-1" }
        return String(format: "%.3f", value * 1_000)
    }
}
