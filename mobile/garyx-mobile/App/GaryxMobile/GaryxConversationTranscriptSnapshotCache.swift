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
        let size: CGSize
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
        ) { [weak self] snapshot, size in
            self?.captureDrivers[threadID] = nil
            self?.store(snapshot, size: size, for: threadID)
        }
        captureDrivers[threadID] = driver
        driver.start()
    }

    func installSnapshot(for threadID: String, in container: UIView) {
        guard let entry = entries[threadID] else { return }
        let snapshot = entry.view
        guard snapshot.superview !== container else { return }
        snapshot.removeFromSuperview()
        snapshot.frame = container.bounds
        snapshot.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        container.addSubview(snapshot)
    }

    private func store(_ snapshot: UIView, size: CGSize, for threadID: String) {
        guard size.width > 0, size.height > 0 else { return }
        if entries[threadID] == nil {
            insertionOrder.append(threadID)
        }
        entries[threadID] = Entry(view: snapshot, size: size)
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

    func makeUIView(context: Context) -> UIView {
        let container = UIView(frame: .zero)
        container.backgroundColor = .clear
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: threadID,
            in: container
        )
        return container
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: threadID,
            in: uiView
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
    private let completion: @MainActor (UIView, CGSize) -> Void
    private var displayLink: CADisplayLink?
    private var stableFrameCount = 0
    private var attempts = 0

    init(
        scrollView: @escaping @MainActor () -> UIScrollView?,
        completion: @escaping @MainActor (UIView, CGSize) -> Void
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
        let size = scrollView.bounds.size
        guard let snapshot = scrollView.snapshotView(afterScreenUpdates: false) else {
            stableFrameCount = 0
            if attempts >= 120 {
                stop()
            }
            return
        }

        stop()
        snapshot.frame = CGRect(origin: .zero, size: size)
        completion(snapshot, size)
    }
}
