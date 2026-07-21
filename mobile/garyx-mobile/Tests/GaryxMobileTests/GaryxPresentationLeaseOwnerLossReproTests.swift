import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxPresentationLeaseOwnerLossReproTests: XCTestCase {
    func testRemovingPresentedLazyRowLeaksLeaseAndFreezesEdgesAndThreadOpen() async throws {
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

        defer {
            // Intentionally exercise the ordinary representable/window
            // teardown with the lease still unreleased. A forced release here
            // would hide any direct teardown crash caused by this state.
            window.rootViewController?.dismiss(animated: false)
            window.isHidden = true
            window.rootViewController = nil
            store.detach(container)
        }

        let presenterDidAppear = await waitUntil { probe.presenterRowAppearCount == 1 }
        XCTAssertTrue(presenterDidAppear)
        probe.requestPresentation()

        let didPresent = await waitUntil {
            probe.presentedContentAppearCount == 1
                && container.presentationLeaseRecordsForTesting.count == 1
                && container.hasPresentationBarrier
        }
        print(
            "PRESENTATION_LEASE_UI_SETUP os=\(UIDevice.current.systemVersion) "
                + "rowAppear=\(probe.presenterRowAppearCount) "
                + "contentAppear=\(probe.presentedContentAppearCount) "
                + "barrier=\(container.hasPresentationBarrier)"
        )
        XCTAssertTrue(didPresent)
        let token = try XCTUnwrap(container.presentationLeaseRecordsForTesting.keys.first)
        XCTAssertEqual(container.presentationLeaseRecord(token)?.joinState, .presented)
        XCTAssertNotNil(probe.rowLifetime)
        XCTAssertTrue(hasPresentedViewController(in: container))
        XCTAssertFalse(container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertFalse(container.trailingEdgePanGestureRecognizer.isEnabled)

        probe.removePresenterRow()
        let presenterDidDisappear = await waitUntil {
            probe.presenterRowDisappearCount == 1
        }
        XCTAssertTrue(presenterDidDisappear)
        XCTAssertTrue(probe.presenterRowIDs.isEmpty)
        let coverDidDisappear = await waitUntil {
            probe.presentedContentDisappearCount == 1
                && !self.hasPresentedViewController(in: container)
        }
        XCTAssertTrue(coverDidDisappear)

        // Give SwiftUI/UIKit ample time to perform any automatic teardown,
        // binding writeback, or onDismiss delivery caused by presenter loss.
        for _ in 0..<40 {
            await Task.yield()
            try await Task.sleep(for: .milliseconds(25))
            window.layoutIfNeeded()
        }

        let abandonedRecord = try XCTUnwrap(container.presentationLeaseRecord(token))
        print(
            "PRESENTATION_LEASE_UI_REPRO rowDisappear=\(probe.presenterRowDisappearCount) "
                + "rowLifetimeAlive=\(probe.rowLifetime != nil) "
                + "contentAppear=\(probe.presentedContentAppearCount) "
                + "contentDisappear=\(probe.presentedContentDisappearCount) "
                + "onDismiss=\(probe.onDismissCount) "
                + "selectionNilObservations=\(probe.selectionNilObservationCount) "
                + "joinState=\(abandonedRecord.joinState) "
                + "released=\(abandonedRecord.released) "
                + "barrier=\(container.hasPresentationBarrier) "
                + "presentedController=\(hasPresentedViewController(in: container))"
        )

        XCTAssertNotNil(
            probe.rowLifetime,
            "iOS 26 retains the removed presenter's StateObject storage after both views disappear"
        )
        XCTAssertEqual(probe.presentedContentDisappearCount, 1)
        XCTAssertFalse(hasPresentedViewController(in: container))
        XCTAssertEqual(
            probe.onDismissCount,
            0,
            "REPRO: iOS 26 does not deliver the cover's onDismiss after presenter removal"
        )
        XCTAssertEqual(probe.selectionNilObservationCount, 0)
        XCTAssertFalse(abandonedRecord.released)
        XCTAssertEqual(abandonedRecord.releaseCount, 0)
        XCTAssertTrue(container.hasPresentationBarrier)
        XCTAssertTrue(store.hasPresentationBarrier)
        XCTAssertEqual(container.reclaimReleasedPresentationLeases(), 0)
        XCTAssertNotNil(container.presentationLeaseRecord(token))
        XCTAssertFalse(container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertFalse(container.trailingEdgePanGestureRecognizer.isEnabled)

        let directOpen = store.open(
            .conversation(threadID: "synthetic-direct-open"),
            source: .replace,
            animated: false
        )
        XCTAssertTrue(store.path.isEmpty)
        XCTAssertTrue(container.path.isEmpty)
        XCTAssertEqual(directOpen.destination, .conversation(threadID: "synthetic-direct-open"))

        let preparation = store.beginNavigationPreparation(source: .replace, animated: false)
        let queuedOpen = store.completeNavigationPreparation(
            preparation,
            outcome: .ready([.conversation(threadID: "synthetic-queued-open")])
        )
        XCTAssertEqual(queuedOpen.result, .queued)

        for _ in 0..<16 {
            store.rendererBecameIdle()
            store.presentationBarrierDidChange()
            await Task.yield()
            XCTAssertTrue(store.path.isEmpty)
            XCTAssertTrue(container.path.isEmpty)
        }

        XCTAssertTrue(container.hasPresentationBarrier)
        XCTAssertFalse(container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertFalse(container.trailingEdgePanGestureRecognizer.isEnabled)
        print(
            "PRESENTATION_LEASE_UI_FREEZE_REPRO directOpenAdmitted=false "
                + "queuedResult=\(queuedOpen.result) drainCycles=16 "
                + "leadingEnabled=\(container.leadingEdgePanGestureRecognizer.isEnabled) "
                + "trailingEnabled=\(container.trailingEdgePanGestureRecognizer.isEnabled)"
        )
        print("PRESENTATION_LEASE_UI_EXIT_PROBE teardownWithUnreleasedLease=true")
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
        timeout: Duration = .seconds(3),
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
private final class GaryxPresentationLeaseOwnerLossProbe: ObservableObject {
    @Published private(set) var presenterRowIDs = [1]
    @Published private(set) var presentationRequestGeneration = 0
    weak var rowLifetime: GaryxPresentationLeaseRowLifetime?
    var presenterRowAppearCount = 0
    var presenterRowDisappearCount = 0
    var presentedContentAppearCount = 0
    var presentedContentDisappearCount = 0
    var onDismissCount = 0
    var selectionNilObservationCount = 0

    func requestPresentation() {
        presentationRequestGeneration += 1
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

    init(probe: GaryxPresentationLeaseOwnerLossProbe) {
        _probe = ObservedObject(wrappedValue: probe)
        _selection = State(initialValue: nil)
    }

    var body: some View {
        Text("Synthetic transcript preview row")
            .frame(maxWidth: .infinity, minHeight: 80)
            .onAppear {
                probe.rowLifetime = lifetime
                probe.presenterRowAppearCount += 1
            }
            .onDisappear { probe.presenterRowDisappearCount += 1 }
            .onChange(of: probe.presentationRequestGeneration) { oldValue, newValue in
                guard newValue > oldValue else { return }
                selection = GaryxPresentationLeaseReproSelection()
            }
            .onChange(of: selection) { oldValue, newValue in
                guard oldValue != nil, newValue == nil else { return }
                probe.selectionNilObservationCount += 1
            }
            .garyxFullScreenCover(
                item: $selection,
                onDismiss: { probe.onDismissCount += 1 }
            ) { _ in
                GaryxPresentationLeasePresentedContent(probe: probe)
            }
    }
}

private struct GaryxPresentationLeasePresentedContent: View {
    let probe: GaryxPresentationLeaseOwnerLossProbe

    var body: some View {
        Color.white
            .overlay(Text("Synthetic full-screen preview"))
            .onAppear {
                probe.presentedContentAppearCount += 1
            }
            .onDisappear { probe.presentedContentDisappearCount += 1 }
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
