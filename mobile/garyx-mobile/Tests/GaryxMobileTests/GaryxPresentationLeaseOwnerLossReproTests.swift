import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxPresentationLeaseOwnerLossReproTests: XCTestCase {
    func testRemovingPresentedLazyRowReleasesLeaseAndUnblocksNavigation() async throws {
        let harness = try makeHarness()
        defer { tearDown(harness) }

        let token = try await presentParent(in: harness)
        let queuedEntry = harness.store.open(
            .conversation(threadID: "synthetic-owner-loss-open"),
            source: .replace,
            animated: false
        )
        XCTAssertTrue(harness.store.path.isEmpty)
        XCTAssertTrue(harness.container.path.isEmpty)

        harness.probe.removePresenterRow()
        let recovered = await waitUntil {
            harness.window.layoutIfNeeded()
            return harness.probe.presenterRowDisappearCount == 1
                && harness.probe.parentContentDisappearCount == 1
                && !self.hasPresentedViewController(in: harness.container)
                && harness.container.presentationLeaseRecord(token)?.released == true
                && !harness.container.hasPresentationBarrier
                && !harness.store.hasPresentationBarrier
                && harness.store.path == [queuedEntry]
                && harness.container.path == [queuedEntry]
                && harness.container.leadingEdgePanGestureRecognizer.isEnabled
                && harness.container.trailingEdgePanGestureRecognizer.isEnabled
        }
        XCTAssertTrue(recovered)

        let record = try XCTUnwrap(harness.container.presentationLeaseRecord(token))
        XCTAssertNotNil(
            harness.probe.rowLifetime,
            "iOS 26 retains the removed presenter's StateObject after both views disappear"
        )
        XCTAssertTrue(harness.probe.presenterRowIDs.isEmpty)
        XCTAssertEqual(harness.probe.onDismissCount, 0)
        XCTAssertEqual(harness.probe.selectionNilObservationCount, 0)
        XCTAssertEqual(record.joinState, .released)
        XCTAssertEqual(record.releaseCount, 1)
        XCTAssertEqual(record.terminalCause, .ownerLoss)
        XCTAssertFalse(hasPresentedViewController(in: harness.container))

        print(
            "PRESENTATION_LEASE_UI_HEALTH os=\(UIDevice.current.systemVersion) "
                + "onDismiss=\(harness.probe.onDismissCount) "
                + "selectionNil=\(harness.probe.selectionNilObservationCount) "
                + "terminalCause=ownerLoss released=\(record.released) "
                + "barrier=\(harness.container.hasPresentationBarrier) "
                + "openPushed=\(harness.store.path == [queuedEntry]) "
                + "leadingEnabled=\(harness.container.leadingEdgePanGestureRecognizer.isEnabled) "
                + "trailingEnabled=\(harness.container.trailingEdgePanGestureRecognizer.isEnabled)"
        )

        XCTAssertEqual(harness.container.reclaimReleasedPresentationLeases(), 1)
        XCTAssertNil(harness.container.presentationLeaseRecord(token))
    }

    func testNormalDismissalKeepsNormalTerminalHistory() async throws {
        let harness = try makeHarness()
        defer { tearDown(harness) }

        let token = try await presentParent(in: harness)
        harness.probe.requestParentDismissal()

        let dismissed = await waitUntil {
            harness.window.layoutIfNeeded()
            return harness.probe.onDismissCount == 1
                && harness.probe.selectionNilObservationCount == 1
                && harness.container.presentationLeaseRecord(token)?.released == true
                && !harness.container.hasPresentationBarrier
                && !self.hasPresentedViewController(in: harness.container)
        }
        XCTAssertTrue(dismissed)

        let record = try XCTUnwrap(harness.container.presentationLeaseRecord(token))
        XCTAssertEqual(record.joinState, .released)
        XCTAssertEqual(record.releaseCount, 1)
        XCTAssertEqual(record.terminalCause, .normalDismissal)
        XCTAssertFalse(harness.store.hasPresentationBarrier)
        XCTAssertTrue(harness.container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertTrue(harness.container.trailingEdgePanGestureRecognizer.isEnabled)
    }

    func testNestedCoverDoesNotSettleStillPresentedAncestor() async throws {
        let harness = try makeHarness()
        defer { tearDown(harness) }

        let parentToken = try await presentParent(in: harness)
        harness.probe.requestNestedPresentation()

        let nestedDidPresent = await waitUntil {
            harness.window.layoutIfNeeded()
            let active = harness.container.presentationLeaseRecordsForTesting.values
                .filter { !$0.released }
            return harness.probe.nestedContentAppearCount == 1
                && active.count == 2
                && active.allSatisfy { $0.joinState == .presented }
                && harness.probe.parentController != nil
        }
        print(
            "PRESENTATION_LEASE_NESTED_SETUP nestedAppear="
                + "\(harness.probe.nestedContentAppearCount) parentDisappear="
                + "\(harness.probe.parentContentDisappearCount) records="
                + "\(harness.container.presentationLeaseRecordsForTesting.values.map(\.joinState))"
        )
        XCTAssertTrue(nestedDidPresent)
        let childToken = try XCTUnwrap(
            harness.container.presentationLeaseRecordsForTesting.keys.first {
                $0 != parentToken
            }
        )

        for _ in 0..<20 { await Task.yield() }

        let parentController = try XCTUnwrap(harness.probe.parentController)
        let parentWhileNested = try XCTUnwrap(
            harness.container.presentationLeaseRecord(parentToken)
        )
        XCTAssertFalse(parentWhileNested.released)
        XCTAssertNil(parentWhileNested.terminalCause)
        XCTAssertEqual(parentWhileNested.joinState, .presented)
        XCTAssertEqual(
            harness.container.presentationLeaseRecord(childToken)?.parent,
            parentToken
        )
        XCTAssertFalse(
            harness.container.presentationLeaseRecord(childToken)?.released == true
        )
        XCTAssertTrue(harness.container.hasPresentationBarrier)
        XCTAssertTrue(hasPresentedViewController(in: harness.container))
        XCTAssertTrue(
            harness.container.containsControllerInPresentedHierarchy(parentController),
            "the ancestor controller remains UIKit presentation ground truth"
        )

        XCTAssertEqual(harness.probe.nestedContentDisappearCount, 0)
    }

    private func presentParent(
        in harness: GaryxPresentationLeaseHarness
    ) async throws -> GaryxPresentationLeaseToken {
        let presenterDidAppear = await waitUntil {
            harness.probe.presenterRowAppearCount == 1
        }
        XCTAssertTrue(presenterDidAppear)
        harness.probe.requestParentPresentation()

        let didPresent = await waitUntil {
            harness.window.layoutIfNeeded()
            return harness.probe.parentContentAppearCount == 1
                && harness.container.presentationLeaseRecordsForTesting.values.count == 1
                && harness.container.presentationLeaseRecordsForTesting.values.first?.joinState
                    == .presented
                && harness.container.hasPresentationBarrier
                && self.hasPresentedViewController(in: harness.container)
        }
        XCTAssertTrue(didPresent)
        let token = try XCTUnwrap(
            harness.container.presentationLeaseRecordsForTesting.keys.first
        )
        XCTAssertNotNil(harness.probe.rowLifetime)
        XCTAssertFalse(harness.container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertFalse(harness.container.trailingEdgePanGestureRecognizer.isEnabled)
        return token
    }

    private func makeHarness() throws -> GaryxPresentationLeaseHarness {
        let probe = GaryxPresentationLeaseOwnerLossProbe()
        let store = GaryxProductionRouteStore()
        let container = makeRouteContainer(store: store, probe: probe)
        store.attach(container)

        let window = try makePresentationLeaseTestWindow()
        window.rootViewController = container
        window.isHidden = false
        container.loadViewIfNeeded()
        container.view.frame = window.bounds
        container.view.setNeedsLayout()
        container.view.layoutIfNeeded()
        return GaryxPresentationLeaseHarness(
            probe: probe,
            store: store,
            container: container,
            window: window
        )
    }

    private func tearDown(_ harness: GaryxPresentationLeaseHarness) {
        harness.window.rootViewController?.dismiss(animated: false)
        harness.window.isHidden = true
        harness.window.rootViewController = nil
        harness.store.detach(harness.container)
    }

    private func makeRouteContainer(
        store: GaryxProductionRouteStore,
        probe: GaryxPresentationLeaseOwnerLossProbe
    ) -> GaryxRouteStackContainer {
        var callbacks = GaryxRouteStackContainerCallbacks()
        callbacks.phaseChanged = { [weak store] phase in
            store?.routePhaseChanged(phase)
        }
        callbacks.canonicalPathChanged = { [weak store] path in
            store?.applyCanonicalPath(path)
        }
        callbacks.visibleRouteActivated = { [weak store] node in
            store?.visibleRouteActivated(node)
        }
        callbacks.rendererBecameIdle = { [weak store] in
            store?.rendererBecameIdle()
        }
        return GaryxRouteStackContainer(
            callbacks: callbacks,
            preferencesProvider: {
                .init(reduceMotion: true, prefersCrossFadeTransitions: false)
            },
            hostBuilder: { _ in
                AnyView(
                    GaryxPresentationLeaseLazyList(probe: probe)
                        .environment(
                            \.garyxPresentationLeaseCoordinator,
                            store.presentationCoordinator
                        )
                        .preferredColorScheme(.light)
                )
            }
        )
    }

    private func waitUntil(
        timeout: Duration = .seconds(5),
        condition: @escaping @MainActor () -> Bool
    ) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            if condition() { return true }
            await Task.yield()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return condition()
    }

    private func hasPresentedViewController(in controller: UIViewController) -> Bool {
        if controller.presentedViewController != nil { return true }
        return controller.children.contains { hasPresentedViewController(in: $0) }
    }
}

@MainActor
private struct GaryxPresentationLeaseHarness {
    let probe: GaryxPresentationLeaseOwnerLossProbe
    let store: GaryxProductionRouteStore
    let container: GaryxRouteStackContainer
    let window: UIWindow
}

@MainActor
private final class GaryxPresentationLeaseOwnerLossProbe: ObservableObject {
    @Published private(set) var presenterRowIDs = [1]
    @Published private(set) var parentPresentationGeneration = 0
    @Published private(set) var parentDismissalGeneration = 0
    @Published var nestedPresented = false
    weak var rowLifetime: GaryxPresentationLeaseRowLifetime?
    weak var parentController: UIViewController?
    var presenterRowAppearCount = 0
    var presenterRowDisappearCount = 0
    var parentContentAppearCount = 0
    var parentContentDisappearCount = 0
    var nestedContentAppearCount = 0
    var nestedContentDisappearCount = 0
    var onDismissCount = 0
    var selectionNilObservationCount = 0

    func requestParentPresentation() {
        parentPresentationGeneration += 1
    }

    func requestParentDismissal() {
        parentDismissalGeneration += 1
    }

    func requestNestedPresentation() {
        nestedPresented = true
    }

    func removePresenterRow() {
        presenterRowIDs.removeAll()
    }
}

private final class GaryxPresentationLeaseRowLifetime: ObservableObject {}

private struct GaryxPresentationLeaseReproSelection: Identifiable, Equatable {
    let id = "presented-row-preview"
}

private struct GaryxPresentationLeaseLazyList: View {
    @ObservedObject var probe: GaryxPresentationLeaseOwnerLossProbe

    var body: some View {
        ScrollView {
            LazyVStack {
                ForEach(probe.presenterRowIDs, id: \.self) { _ in
                    GaryxPresentationLeasePresenterRow(probe: probe)
                }
            }
        }
    }
}

private struct GaryxPresentationLeasePresenterRow: View {
    @ObservedObject var probe: GaryxPresentationLeaseOwnerLossProbe
    @StateObject private var lifetime = GaryxPresentationLeaseRowLifetime()
    @State private var selection: GaryxPresentationLeaseReproSelection?

    var body: some View {
        Text("Synthetic transcript preview row")
            .frame(maxWidth: .infinity, minHeight: 80)
            .onAppear {
                probe.rowLifetime = lifetime
                probe.presenterRowAppearCount += 1
            }
            .onDisappear { probe.presenterRowDisappearCount += 1 }
            .onChange(of: probe.parentPresentationGeneration) { oldValue, newValue in
                guard newValue > oldValue else { return }
                selection = GaryxPresentationLeaseReproSelection()
            }
            .onChange(of: probe.parentDismissalGeneration) { oldValue, newValue in
                guard newValue > oldValue else { return }
                selection = nil
            }
            .onChange(of: selection) { oldValue, newValue in
                guard oldValue != nil, newValue == nil else { return }
                probe.selectionNilObservationCount += 1
            }
            .garyxFullScreenCover(
                item: $selection,
                onDismiss: { probe.onDismissCount += 1 }
            ) { _ in
                GaryxPresentationLeaseParentContent(probe: probe)
            }
    }
}

private struct GaryxPresentationLeaseParentContent: View {
    @ObservedObject var probe: GaryxPresentationLeaseOwnerLossProbe

    var body: some View {
        Color.white
            .overlay(Text("Synthetic full-screen preview"))
            .background {
                GaryxPresentationLeaseControllerReader { controller in
                    probe.parentController = controller
                }
                .frame(width: 0, height: 0)
            }
            .onAppear {
                probe.parentContentAppearCount += 1
            }
            .onDisappear { probe.parentContentDisappearCount += 1 }
            .garyxFullScreenCover(
                isPresented: $probe.nestedPresented
            ) {
                GaryxPresentationLeaseNestedContent(probe: probe)
            }
    }
}

private struct GaryxPresentationLeaseNestedContent: View {
    let probe: GaryxPresentationLeaseOwnerLossProbe

    var body: some View {
        Color.white
            .overlay(Text("Synthetic nested full-screen preview"))
            .onAppear {
                probe.nestedContentAppearCount += 1
            }
            .onDisappear { probe.nestedContentDisappearCount += 1 }
    }
}

private struct GaryxPresentationLeaseControllerReader: UIViewRepresentable {
    let resolve: @MainActor (UIViewController) -> Void

    func makeUIView(context: Context) -> ResolverView {
        let view = ResolverView()
        view.isUserInteractionEnabled = false
        view.resolve = resolve
        return view
    }

    func updateUIView(_ uiView: ResolverView, context: Context) {
        uiView.resolve = resolve
    }

    final class ResolverView: UIView {
        var resolve: (@MainActor (UIViewController) -> Void)?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            resolveController()
        }

        override func didMoveToSuperview() {
            super.didMoveToSuperview()
            resolveController()
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            resolveController()
        }

        private func resolveController() {
            var responder: UIResponder? = self
            while let next = responder?.next {
                if let controller = next as? UIViewController {
                    resolve?(controller)
                    return
                }
                responder = next
            }
        }
    }
}

@MainActor
private func makePresentationLeaseTestWindow() throws -> UIWindow {
    let scene = try XCTUnwrap(
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first
    )
    let window = UIWindow(windowScene: scene)
    window.frame = CGRect(x: 0, y: 0, width: 393, height: 852)
    return window
}
