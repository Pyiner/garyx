import QuartzCore
import SwiftUI
import UIKit

/// Short-lived visual cache of the production transcript viewport. A cached
/// view is created by UIKit's compositor from the real conversation scroll
/// view; no message text or loading shape is reimplemented here. Reopening a
/// complex local thread can therefore move the already-rendered pixels during
/// the push and hand off to the live SwiftUI rows after navigation settles.
@MainActor
final class GaryxConversationTranscriptSnapshotCache {
    static let shared = GaryxConversationTranscriptSnapshotCache()

    private struct Entry {
        let view: UIView
        let geometry: GaryxConversationTranscriptSnapshotCaptureGeometry
    }

    private var entries: [String: Entry] = [:]
    private var insertionOrder: [String] = []
    private var scheduledRevisionByThreadID: [String: String] = [:]
    private var captureDrivers: [String: GaryxConversationTranscriptSnapshotDriver] = [:]
    private let capacity = 12

    private init() {}

    func hasSnapshot(for threadID: String) -> Bool {
        entries[threadID] != nil
    }

    func scheduleCapture(
        threadID: String,
        revision: String,
        scrollView: @escaping @MainActor () -> UIScrollView?
    ) {
        guard !revision.isEmpty,
              scheduledRevisionByThreadID[threadID] != revision
        else { return }

        scheduledRevisionByThreadID[threadID] = revision
        captureDrivers[threadID]?.stop()
        let driver = GaryxConversationTranscriptSnapshotDriver(
            scrollView: scrollView
        ) { [weak self] snapshot, geometry in
            self?.captureDrivers[threadID] = nil
            self?.store(snapshot, geometry: geometry, for: threadID)
        }
        captureDrivers[threadID] = driver
        driver.start()
    }

    func installSnapshot(for threadID: String, in container: UIView) {
        container.clipsToBounds = true
        guard let entry = entries[threadID] else {
            container.subviews.forEach { $0.removeFromSuperview() }
            return
        }

        let snapshot = entry.view
        for staleSubview in container.subviews where staleSubview !== snapshot {
            staleSubview.removeFromSuperview()
        }
        if snapshot.superview !== container {
            snapshot.removeFromSuperview()
            container.addSubview(snapshot)
        }

        // A compositor snapshot is already-rendered pixels, not relayoutable
        // content. Keep its captured geometry and let the dedicated host clip
        // overflow. Resizing this view to a transient opening bound
        // non-uniformly scales every glyph.
        snapshot.transform = .identity
        snapshot.translatesAutoresizingMaskIntoConstraints = true
        snapshot.autoresizingMask = [
            .flexibleRightMargin,
            .flexibleBottomMargin,
        ]
        layoutSnapshot(for: threadID, in: container)
    }

    func layoutSnapshot(for threadID: String, in container: UIView) {
        guard let entry = entries[threadID], entry.view.superview === container else { return }
        let containerFrameInPage = container.garyxFrameInOwningRoutePage
            ?? CGRect(origin: .zero, size: container.bounds.size)
        entry.view.frame = GaryxConversationTranscriptSnapshotGeometry.installationFrame(
            capture: entry.geometry,
            containerFrameInPage: containerFrameInPage
        )
    }

    private func store(
        _ snapshot: UIView,
        geometry: GaryxConversationTranscriptSnapshotCaptureGeometry,
        for threadID: String
    ) {
        guard geometry.viewportFrameInPage.width > 0,
              geometry.viewportFrameInPage.height > 0
        else { return }
        if entries[threadID] == nil {
            insertionOrder.append(threadID)
        }
        entries[threadID] = Entry(view: snapshot, geometry: geometry)
        while insertionOrder.count > capacity {
            let evictedThreadID = insertionOrder.removeFirst()
            entries[evictedThreadID] = nil
            scheduledRevisionByThreadID[evictedThreadID] = nil
        }
    }
}

/// Hosts the cached compositor snapshot in exactly the viewport reserved for
/// the transcript. Header loading and composer chrome remain live SwiftUI, so
/// their pre-refactor animation and interaction language stays unchanged.
struct GaryxConversationTranscriptSnapshotView: UIViewRepresentable {
    let threadID: String

    func makeUIView(context: Context) -> GaryxConversationTranscriptSnapshotHostView {
        let container = GaryxConversationTranscriptSnapshotHostView(frame: .zero)
        container.backgroundColor = .clear
        container.displaySnapshot(for: threadID)
        return container
    }

    func updateUIView(
        _ uiView: GaryxConversationTranscriptSnapshotHostView,
        context: Context
    ) {
        uiView.displaySnapshot(for: threadID)
    }
}

/// Repositions cached pixels after SwiftUI has attached and laid out the
/// transcript-only viewport. This keeps the conversion in route-page space
/// while the outer route wrapper remains free to own horizontal push motion.
@MainActor
final class GaryxConversationTranscriptSnapshotHostView: UIView {
    private var threadID: String?

    func displaySnapshot(for threadID: String) {
        self.threadID = threadID
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: threadID,
            in: self
        )
        setNeedsLayout()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard let threadID else { return }
        GaryxConversationTranscriptSnapshotCache.shared.layoutSnapshot(
            for: threadID,
            in: self
        )
    }
}

/// Waits for consecutive display frames before asking UIKit for its native
/// snapshot. This keeps capture out of row layout and ensures the scroll view
/// has reached its final tail offset. `snapshotView` reuses compositor output
/// instead of synchronously rasterizing the Markdown hierarchy.
@MainActor
private final class GaryxConversationTranscriptSnapshotDriver: NSObject {
    private let scrollViewProvider: @MainActor () -> UIScrollView?
    private let completion: @MainActor (
        UIView,
        GaryxConversationTranscriptSnapshotCaptureGeometry
    ) -> Void
    private var displayLink: CADisplayLink?
    private var stableFrameCount = 0
    private var attempts = 0

    init(
        scrollView: @escaping @MainActor () -> UIScrollView?,
        completion: @escaping @MainActor (
            UIView,
            GaryxConversationTranscriptSnapshotCaptureGeometry
        ) -> Void
    ) {
        scrollViewProvider = scrollView
        self.completion = completion
    }

    func start() {
        guard displayLink == nil else { return }
        let link = CADisplayLink(target: self, selector: #selector(framePresented(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(
            minimum: 60,
            maximum: 120,
            preferred: 120
        )
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    func stop() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func framePresented(_ link: CADisplayLink) {
        attempts += 1
        guard let scrollView = scrollViewProvider(),
              scrollView.window != nil,
              scrollView.bounds.width > 0,
              scrollView.bounds.height > 0
        else {
            if attempts >= 120 {
                stop()
            }
            return
        }

        stableFrameCount += 1
        guard stableFrameCount >= 60 else { return }
        guard let viewportFrameInPage = scrollView.garyxFrameInOwningRoutePage else {
            stableFrameCount = 0
            if attempts >= 120 {
                stop()
            }
            return
        }
        guard let snapshot = scrollView.snapshotView(afterScreenUpdates: false) else {
            stableFrameCount = 0
            if attempts >= 120 {
                stop()
            }
            return
        }

        stop()
        let adjustedInsets = scrollView.adjustedContentInset
        let geometry = GaryxConversationTranscriptSnapshotCaptureGeometry(
            viewportFrameInPage: viewportFrameInPage,
            adjustedContentInsets: .init(
                top: adjustedInsets.top,
                left: adjustedInsets.left,
                bottom: adjustedInsets.bottom,
                right: adjustedInsets.right
            ),
            contentOffset: scrollView.contentOffset
        )
        snapshot.frame = CGRect(origin: .zero, size: viewportFrameInPage.size)
        completion(snapshot, geometry)
    }
}

private extension UIView {
    /// The viewport rect normalized to its nearest owning controller's page.
    /// Converting to this ancestor excludes the route wrapper's outer
    /// transition transform while retaining safe-area/container placement.
    var garyxFrameInOwningRoutePage: CGRect? {
        var responder: UIResponder? = self
        while let current = responder {
            if let controller = current as? UIViewController {
                let pageBounds = controller.view.bounds
                let frame = convert(bounds, to: controller.view)
                return frame.offsetBy(dx: -pageBounds.minX, dy: -pageBounds.minY)
            }
            responder = current.next
        }
        return nil
    }
}
