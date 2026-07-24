import QuartzCore
import SwiftUI
import UIKit

/// Exact layout-bearing input behind one transcript snapshot.
///
/// This deliberately retains complete Equatable production values instead of
/// compressing them into the former first/last/count/tail-length heuristic.
/// Copy-on-write storage keeps the 12-entry cache bounded without introducing
/// a collision-prone local hash.
struct GaryxConversationTranscriptSnapshotRevision: Equatable {
    let renderSnapshot: GaryxRenderSnapshot?
    let messages: [GaryxMobileMessage]
    let turnRows: [GaryxMobileTurnRow]
    let treatment: GaryxConversationTranscriptTreatment
    let showsTailThinking: Bool
    let hasMoreRenderableHistory: Bool
    let isLoadingOlderHistory: Bool
    let capsuleHTMLCacheEpoch: Int
    let dynamicTypeSize: DynamicTypeSize
    let horizontalSizeClass: UserInterfaceSizeClass?
    let displayScale: CGFloat
}

struct GaryxConversationTranscriptSnapshotHandle: Equatable {
    let threadID: String
    fileprivate let entryID: UUID
    let openingViewportContract: GaryxConversationOpeningViewportContract

    var openingViewportContractID: String {
        entryID.uuidString
    }
}

/// Short-lived visual cache of the production transcript viewport. A cached
/// view is created by UIKit's compositor from the real conversation scroll
/// view; no message text or loading shape is reimplemented here. Reopening a
/// complex local thread can therefore move the already-rendered pixels during
/// the push and hand off to the live SwiftUI rows after navigation settles.
@MainActor
final class GaryxConversationTranscriptSnapshotCache {
    static let shared = GaryxConversationTranscriptSnapshotCache()

    private struct Entry {
        let id: UUID
        let view: UIView
        let sourceID: UUID
        let revision: GaryxConversationTranscriptSnapshotRevision
        let contract: GaryxConversationOpeningViewportContract
    }

    private struct CaptureRequest: Equatable {
        let sourceID: UUID
        let revision: GaryxConversationTranscriptSnapshotRevision
        let visibleViewportFrameInPage: CGRect
        let layoutEpoch: UInt64
    }

    private struct CaptureDriverSlot {
        let id: UUID
        let request: CaptureRequest
        let driver: GaryxConversationTranscriptSnapshotDriver
    }

    private var entries: [String: Entry] = [:]
    private var insertionOrder: [String] = []
    private var captureDrivers: [String: CaptureDriverSlot] = [:]
    private let capacity = 12

    private init() {}

    func qualifiedSnapshot(
        for threadID: String,
        revision: GaryxConversationTranscriptSnapshotRevision,
        visibleViewportFrameInPage: CGRect
    ) -> GaryxConversationTranscriptSnapshotHandle? {
        guard let entry = entries[threadID],
              GaryxConversationOpeningViewportContractPolicy.canServe(
                  entry.contract,
                  revisionMatches: entry.revision == revision,
                  visibleViewportFrameInPage: visibleViewportFrameInPage
              )
        else {
            return nil
        }
        return GaryxConversationTranscriptSnapshotHandle(
            threadID: threadID,
            entryID: entry.id,
            openingViewportContract: entry.contract
        )
    }

    func scheduleCapture(
        threadID: String,
        sourceID: UUID,
        revision: GaryxConversationTranscriptSnapshotRevision,
        visibleViewportFrameInPage: CGRect,
        layoutEpoch: UInt64,
        captureStatus: @escaping @MainActor () -> (
            isFollowingTail: Bool,
            isUserInteracting: Bool,
            layoutEpoch: UInt64
        ),
        scrollView: @escaping @MainActor () -> UIScrollView?
    ) {
        guard visibleViewportFrameInPage.width > 0,
              visibleViewportFrameInPage.height > 0
        else {
            return
        }
        let request = CaptureRequest(
            sourceID: sourceID,
            revision: revision,
            visibleViewportFrameInPage: visibleViewportFrameInPage,
            layoutEpoch: layoutEpoch
        )
        if captureDrivers[threadID]?.request == request {
            return
        }
        if let entry = entries[threadID],
           entry.sourceID == sourceID,
           entry.revision == revision,
           GaryxConversationOpeningViewportContractPolicy.canServe(
               entry.contract,
               revisionMatches: true,
               visibleViewportFrameInPage: visibleViewportFrameInPage
           ),
           entry.contract.layoutEpoch == layoutEpoch {
            return
        }

        // A layout-bearing input changed in the same mounted source. Its old
        // pixels stop being serviceable immediately; a later stable capture
        // may replace them. A new route occurrence does not compare monotonic
        // layout epochs with its predecessor.
        if let entry = entries[threadID], entry.sourceID == sourceID {
            entries[threadID] = nil
        }

        captureDrivers[threadID]?.driver.stop()
        let driverID = UUID()
        let driver = GaryxConversationTranscriptSnapshotDriver(
            visibleViewportFrameInPage: visibleViewportFrameInPage,
            captureStatus: captureStatus,
            scrollView: scrollView,
            completion: { [weak self] snapshot, contract in
                guard let self,
                      self.captureDrivers[threadID]?.id == driverID
                else {
                    return
                }
                self.captureDrivers[threadID] = nil
                guard contract.layoutEpoch == request.layoutEpoch else {
                    return
                }
                self.store(
                    snapshot,
                    request: request,
                    contract: contract,
                    for: threadID
                )
            },
            stopped: { [weak self] in
                guard self?.captureDrivers[threadID]?.id == driverID else {
                    return
                }
                self?.captureDrivers[threadID] = nil
            }
        )
        captureDrivers[threadID] = CaptureDriverSlot(
            id: driverID,
            request: request,
            driver: driver
        )
        driver.start()
    }

    func revealReadiness(
        for handle: GaryxConversationTranscriptSnapshotHandle,
        revision: GaryxConversationTranscriptSnapshotRevision,
        visibleViewportFrameInPage: CGRect,
        layoutEpoch: UInt64,
        isFollowingTail: Bool,
        isUserInteracting: Bool,
        scrollView: UIScrollView
    ) -> GaryxConversationOpeningViewportRevealReadiness {
        guard let entry = entries[handle.threadID],
              entry.id == handle.entryID,
              let viewportFrameInPage = scrollView.garyxFrameInOwningRoutePage
        else {
            return .pending
        }
        let adjustedInsets = scrollView.adjustedContentInset
        let sample = GaryxConversationOpeningViewportSample(
            captureGeometry: GaryxConversationTranscriptSnapshotCaptureGeometry(
                viewportFrameInPage: viewportFrameInPage,
                adjustedContentInsets: .init(
                    top: adjustedInsets.top,
                    left: adjustedInsets.left,
                    bottom: adjustedInsets.bottom,
                    right: adjustedInsets.right
                ),
                contentOffset: scrollView.contentOffset
            ),
            visibleViewportFrameInPage: visibleViewportFrameInPage,
            contentSize: scrollView.contentSize,
            displayScale: scrollView.traitCollection.displayScale,
            layoutEpoch: layoutEpoch,
            isFollowingTail: isFollowingTail,
            isUserInteracting: isUserInteracting
        )
        return GaryxConversationOpeningViewportContractPolicy.revealReadiness(
            for: entry.contract,
            live: sample,
            revisionMatches: entry.revision == revision
        )
    }

    func installSnapshot(
        for handle: GaryxConversationTranscriptSnapshotHandle?,
        in container: UIView
    ) {
        container.clipsToBounds = true
        guard let handle,
              let entry = entries[handle.threadID],
              entry.id == handle.entryID else {
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
        layoutSnapshot(for: handle, in: container)
    }

    func layoutSnapshot(
        for handle: GaryxConversationTranscriptSnapshotHandle,
        in container: UIView
    ) {
        guard let entry = entries[handle.threadID],
              entry.id == handle.entryID,
              entry.view.superview === container else {
            return
        }
        let containerFrameInPage = container.garyxFrameInOwningRoutePage
            ?? CGRect(origin: .zero, size: container.bounds.size)
        entry.view.frame = GaryxConversationTranscriptSnapshotGeometry.installationFrame(
            capture: entry.contract.captureGeometry,
            containerFrameInPage: containerFrameInPage
        )
    }

    private func store(
        _ snapshot: UIView,
        request: CaptureRequest,
        contract: GaryxConversationOpeningViewportContract,
        for threadID: String
    ) {
        guard contract.captureGeometry.viewportFrameInPage.width > 0,
              contract.captureGeometry.viewportFrameInPage.height > 0
        else { return }
        if !insertionOrder.contains(threadID) {
            insertionOrder.append(threadID)
        }
        entries[threadID] = Entry(
            id: UUID(),
            view: snapshot,
            sourceID: request.sourceID,
            revision: request.revision,
            contract: contract
        )
        while insertionOrder.count > capacity {
            let evictedThreadID = insertionOrder.removeFirst()
            entries[evictedThreadID] = nil
            captureDrivers[evictedThreadID]?.driver.stop()
            captureDrivers[evictedThreadID] = nil
        }
    }

#if DEBUG
    /// Additive UIKit test seam. Production callers must supply the complete
    /// render revision and service geometry through `scheduleCapture`.
    func testScheduleCapture(
        threadID: String,
        scrollView: UIScrollView
    ) {
        let visibleFrame = scrollView.garyxFrameInOwningRoutePage
            ?? CGRect(origin: .zero, size: scrollView.bounds.size)
        scheduleCapture(
            threadID: threadID,
            sourceID: UUID(),
            revision: GaryxConversationTranscriptSnapshotRevision(
                renderSnapshot: nil,
                messages: [],
                turnRows: [],
                treatment: .content,
                showsTailThinking: false,
                hasMoreRenderableHistory: false,
                isLoadingOlderHistory: false,
                capsuleHTMLCacheEpoch: 0,
                dynamicTypeSize: .large,
                horizontalSizeClass: nil,
                displayScale: scrollView.traitCollection.displayScale
            ),
            visibleViewportFrameInPage: visibleFrame,
            layoutEpoch: 0,
            captureStatus: {
                (
                    isFollowingTail: true,
                    isUserInteracting: false,
                    layoutEpoch: 0
                )
            },
            scrollView: { scrollView }
        )
    }

    func testSnapshotHandle(
        for threadID: String
    ) -> GaryxConversationTranscriptSnapshotHandle? {
        guard let entry = entries[threadID] else { return nil }
        return GaryxConversationTranscriptSnapshotHandle(
            threadID: threadID,
            entryID: entry.id,
            openingViewportContract: entry.contract
        )
    }
#endif
}

/// Hosts the cached compositor snapshot in exactly the viewport reserved for
/// the transcript. Header loading and composer chrome remain live SwiftUI, so
/// their pre-refactor animation and interaction language stays unchanged.
struct GaryxConversationTranscriptSnapshotView: UIViewRepresentable {
    let handle: GaryxConversationTranscriptSnapshotHandle?

    func makeUIView(context: Context) -> GaryxConversationTranscriptSnapshotHostView {
        let container = GaryxConversationTranscriptSnapshotHostView(frame: .zero)
        container.backgroundColor = .clear
        container.displaySnapshot(for: handle)
        return container
    }

    func updateUIView(
        _ uiView: GaryxConversationTranscriptSnapshotHostView,
        context: Context
    ) {
        uiView.displaySnapshot(for: handle)
    }
}

/// Repositions cached pixels after SwiftUI has attached and laid out the
/// transcript-only viewport. This keeps the conversion in route-page space
/// while the outer route wrapper remains free to own horizontal push motion.
@MainActor
final class GaryxConversationTranscriptSnapshotHostView: UIView {
    private var handle: GaryxConversationTranscriptSnapshotHandle?

    func displaySnapshot(for handle: GaryxConversationTranscriptSnapshotHandle?) {
        self.handle = handle
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: handle,
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
        guard let handle else { return }
        GaryxConversationTranscriptSnapshotCache.shared.layoutSnapshot(
            for: handle,
            in: self
        )
    }
}

/// Samples the mounted live viewport on every presented frame while an opaque
/// snapshot cover is still installed. Scroll-to-tail settles independently of
/// SwiftUI geometry preferences, so a preference-only handshake can miss the
/// one state change that completes the contract and wait for its timeout.
@MainActor
final class GaryxConversationOpeningViewportReadinessSampler: NSObject {
    private var displayLink: CADisplayLink?
    private var sample: (@MainActor () -> Void)?

    func start(sample: @escaping @MainActor () -> Void) {
        self.sample = sample
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
        sample = nil
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func framePresented(_ link: CADisplayLink) {
        sample?()
    }
}

/// Waits for consecutive display frames before asking UIKit for its native
/// snapshot. This keeps capture out of row layout and ensures the scroll view
/// has reached its final tail offset. `snapshotView` reuses compositor output
/// instead of synchronously rasterizing the Markdown hierarchy.
@MainActor
private final class GaryxConversationTranscriptSnapshotDriver: NSObject {
    private let visibleViewportFrameInPage: CGRect
    private let captureStatus: @MainActor () -> (
        isFollowingTail: Bool,
        isUserInteracting: Bool,
        layoutEpoch: UInt64
    )
    private let scrollViewProvider: @MainActor () -> UIScrollView?
    private let completion: @MainActor (
        UIView,
        GaryxConversationOpeningViewportContract
    ) -> Void
    private let stopped: @MainActor () -> Void
    private var displayLink: CADisplayLink?
    private var captureState = GaryxConversationOpeningViewportCaptureState()
    private var attempts = 0
    private var hasFinished = false

    init(
        visibleViewportFrameInPage: CGRect,
        captureStatus: @escaping @MainActor () -> (
            isFollowingTail: Bool,
            isUserInteracting: Bool,
            layoutEpoch: UInt64
        ),
        scrollView: @escaping @MainActor () -> UIScrollView?,
        completion: @escaping @MainActor (
            UIView,
            GaryxConversationOpeningViewportContract
        ) -> Void,
        stopped: @escaping @MainActor () -> Void
    ) {
        self.visibleViewportFrameInPage = visibleViewportFrameInPage
        self.captureStatus = captureStatus
        scrollViewProvider = scrollView
        self.completion = completion
        self.stopped = stopped
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
        guard !hasFinished else { return }
        hasFinished = true
        displayLink?.invalidate()
        displayLink = nil
        stopped()
    }

    @objc private func framePresented(_ link: CADisplayLink) {
        attempts += 1
        guard let scrollView = scrollViewProvider(),
              scrollView.window != nil,
              scrollView.bounds.width > 0,
              scrollView.bounds.height > 0
        else {
            if attempts >= 240 {
                stop()
            }
            return
        }

        guard let viewportFrameInPage = scrollView.garyxFrameInOwningRoutePage else {
            if attempts >= 240 {
                stop()
            }
            return
        }

        let adjustedInsets = scrollView.adjustedContentInset
        let captureGeometry = GaryxConversationTranscriptSnapshotCaptureGeometry(
            viewportFrameInPage: viewportFrameInPage,
            adjustedContentInsets: .init(
                top: adjustedInsets.top,
                left: adjustedInsets.left,
                bottom: adjustedInsets.bottom,
                right: adjustedInsets.right
            ),
            contentOffset: scrollView.contentOffset
        )
        let status = captureStatus()
        let sample = GaryxConversationOpeningViewportSample(
            captureGeometry: captureGeometry,
            visibleViewportFrameInPage: visibleViewportFrameInPage,
            contentSize: scrollView.contentSize,
            displayScale: scrollView.traitCollection.displayScale,
            layoutEpoch: status.layoutEpoch,
            isFollowingTail: status.isFollowingTail,
            isUserInteracting: status.isUserInteracting
        )
        guard let contract = captureState.observe(sample) else {
            if attempts >= 240 {
                stop()
            }
            return
        }
        guard let snapshot = scrollView.snapshotView(afterScreenUpdates: false) else {
            if attempts >= 240 {
                stop()
            }
            return
        }

        hasFinished = true
        displayLink?.invalidate()
        displayLink = nil
        snapshot.frame = CGRect(origin: .zero, size: viewportFrameInPage.size)
        completion(snapshot, contract)
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
