import UIKit
import WebKit

/// Renders a capsule's HTML once into a fixed-ratio thumbnail PNG.
///
/// This is the live-render step that the cached-image gallery defers to only on
/// a cache miss (first sight or after a `revision` bump). It renders the capsule
/// exactly as it would appear full-screen on a phone — at the device logical
/// width (`plan.layoutWidth`) with a device-width viewport — so author content
/// fills the frame instead of sitting centered with white side gutters (the
/// #TASK-1458 root cause: the old 760pt render viewport let a `max-width`
/// container center). After layout settles, `GaryxCapsuleThumbnailFill` measures
/// the content and uniformly scales the page so it fills the width even when the
/// author's `max-width` is narrower than the device — never by painting a
/// backing color behind the content. The top `16:rendition` band is captured
/// (cover, top-anchored): taller content is cropped at the bottom.
///
/// Concurrent renders are capped so a fresh, all-miss gallery does not spin up
/// many `WKWebView`s at once — but unlike the old display planner this never
/// starves a card: every card shows its image once its one-shot render drains
/// the queue.
@MainActor
final class GaryxCapsuleThumbnailRenderer {
    private let gate: RenderGate

    init(maxConcurrent: Int = 2) {
        self.gate = RenderGate(limit: maxConcurrent)
    }

    func renderPNG(html: String, plan: GaryxCapsuleThumbnailSnapshotPlan) async -> Data? {
        await gate.acquire()
        defer { gate.release() }
        let layout = CGSize(width: plan.layoutWidth, height: plan.layoutHeight)
        guard let snapshot = await renderSnapshot(html: html, layout: layout) else { return nil }
        return encodePNG(snapshot, pixelSize: CGSize(width: plan.pixelWidth, height: plan.pixelHeight))
    }

    private func renderSnapshot(html: String, layout: CGSize) async -> UIImage? {
        guard let host = Self.hostWindow() else { return nil }

        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false

        let webView = WKWebView(frame: CGRect(origin: .zero, size: layout), configuration: configuration)
        // Opaque so the snapshot has no alpha; the page itself paints the frame
        // (content fills via the fill transform) — no injected backing color.
        webView.isOpaque = true
        webView.scrollView.isScrollEnabled = false
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        webView.isUserInteractionEnabled = false
        let coordinator = NavigationCoordinator()
        webView.navigationDelegate = coordinator

        // Host offscreen in a live window so layout and paint actually run —
        // snapshots of an unhosted web view can come back blank.
        webView.frame.origin = CGPoint(x: -(layout.width + 64), y: 0)
        host.addSubview(webView)
        defer { webView.removeFromSuperview() }

        webView.loadHTMLString(GaryxCapsuleViewport.ensuringMobileViewport(in: html), baseURL: nil)
        await coordinator.waitUntilDone(timeout: 6.0)
        // Brief settle for final layout / inline JS paint before measuring.
        try? await Task.sleep(nanoseconds: 140_000_000)
        // Make the content fill the width (scale-to-fill for narrow content);
        // a no-op when it already fills. Then let the transform paint.
        _ = try? await webView.evaluateJavaScript(GaryxCapsuleThumbnailFill.fillScript)
        try? await Task.sleep(nanoseconds: 60_000_000)

        let snapConfig = WKSnapshotConfiguration()
        snapConfig.rect = CGRect(origin: .zero, size: layout)
        snapConfig.afterScreenUpdates = true
        return await withCheckedContinuation { (continuation: CheckedContinuation<UIImage?, Never>) in
            webView.takeSnapshot(with: snapConfig) { image, _ in
                continuation.resume(returning: image)
            }
        }
    }

    /// Redraw the captured band into a deterministic pixel size. Source and
    /// target share the rendition aspect, so this is a pure scale (no crop). The
    /// captured snapshot is opaque (content fills the frame), so there is no
    /// backing to fill — just the scale.
    private func encodePNG(_ image: UIImage, pixelSize: CGSize) -> Data? {
        guard pixelSize.width >= 1, pixelSize.height >= 1 else { return nil }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let renderer = UIGraphicsImageRenderer(size: pixelSize, format: format)
        let output = renderer.image { _ in
            image.draw(in: CGRect(origin: .zero, size: pixelSize))
        }
        return output.pngData()
    }

    private static func hostWindow() -> UIWindow? {
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap { $0.windows }
        return windows.first(where: { $0.isKeyWindow }) ?? windows.first
    }
}

/// Resolves once the web view finishes (or fails) its first load, with a timeout
/// safety net. Capsule HTML is self-contained inline (CSP blocks external
/// fetches), so the load is fast; the timeout only guards a pathological page.
private final class NavigationCoordinator: NSObject, WKNavigationDelegate {
    private var continuation: CheckedContinuation<Void, Never>?
    private var finished = false

    func waitUntilDone(timeout: TimeInterval) async {
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            if finished {
                continuation.resume()
                return
            }
            self.continuation = continuation
            Task { @MainActor [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(timeout * 1_000_000_000))
                self?.finish()
            }
        }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { finish() }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { finish() }

    func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: Error
    ) {
        finish()
    }

    private func finish() {
        guard !finished else { return }
        finished = true
        continuation?.resume()
        continuation = nil
    }
}

/// Caps concurrent one-shot renders. Releasing hands the slot directly to the
/// next waiter (FIFO) so the active count never exceeds `limit`.
@MainActor
private final class RenderGate {
    private let limit: Int
    private var active = 0
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(limit: Int) { self.limit = max(1, limit) }

    func acquire() async {
        if active < limit {
            active += 1
            return
        }
        await withCheckedContinuation { waiters.append($0) }
    }

    func release() {
        if !waiters.isEmpty {
            waiters.removeFirst().resume()
        } else {
            active = max(0, active - 1)
        }
    }
}
