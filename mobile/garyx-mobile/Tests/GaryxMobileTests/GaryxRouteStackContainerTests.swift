import Combine
import SwiftUI
import UIKit
import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxRouteStackContainerTests: XCTestCase {
    func testSwiftUIUpdateRemovesPreviousThreadSnapshotFromVisibleHost() throws {
        let previousThreadID = "thread::swiftui-previous-\(UUID().uuidString)"
        let nextThreadID = "thread::swiftui-next-\(UUID().uuidString)"
        let hostSize = CGSize(width: 393, height: 180)
        cacheTranscriptSnapshot(
            threadID: previousThreadID,
            size: CGSize(width: 393, height: 600)
        )

        // Keep a direct reference to the cache's compositor view so the test
        // can distinguish a truly removed old snapshot from a new empty host.
        let probeContainer = UIView(frame: CGRect(origin: .zero, size: hostSize))
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: previousThreadID,
            in: probeContainer
        )
        let previousSnapshot = try XCTUnwrap(probeContainer.subviews.first)

        let previousRoot = GaryxConversationTranscriptSnapshotView(
            threadID: previousThreadID
        )
        .frame(width: hostSize.width, height: hostSize.height)
        let hostingController = UIHostingController(rootView: previousRoot)
        let hostWindow = makeTestWindow(frame: CGRect(origin: .zero, size: hostSize))
        hostWindow.rootViewController = hostingController
        hostWindow.isHidden = false
        defer {
            hostWindow.isHidden = true
            hostWindow.rootViewController = nil
        }
        hostWindow.layoutIfNeeded()
        pumpMainRunLoop(duration: 0.1)
        XCTAssertNotNil(previousSnapshot.window)

        hostingController.rootView = GaryxConversationTranscriptSnapshotView(
            threadID: nextThreadID
        )
        .frame(width: hostSize.width, height: hostSize.height)
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()
        pumpMainRunLoop(duration: 0.1)

        XCTAssertNil(
            previousSnapshot.window,
            "SwiftUI reused the representable host, and its update left the previous thread's cached pixels visible"
        )
    }

    func testSnapshotRepresentableUpdateDoesNotRetainPixelsFromPreviousThread() {
        let previousThreadID = "thread::cached-previous-\(UUID().uuidString)"
        let nextThreadID = "thread::uncached-next-\(UUID().uuidString)"
        cacheTranscriptSnapshot(
            threadID: previousThreadID,
            size: CGSize(width: 393, height: 600)
        )

        let reusedRepresentableContainer = UIView(
            frame: CGRect(x: 0, y: 0, width: 393, height: 180)
        )
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: previousThreadID,
            in: reusedRepresentableContainer
        )
        XCTAssertEqual(reusedRepresentableContainer.subviews.count, 1)

        // Mirror UIViewRepresentable.updateUIView when SwiftUI keeps the same
        // platform view while its value changes to another thread. That next
        // thread has no snapshot, so no pixels from the previous route may
        // remain in the container.
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: nextThreadID,
            in: reusedRepresentableContainer
        )

        XCTAssertTrue(
            reusedRepresentableContainer.subviews.isEmpty,
            "an uncached destination must clear the prior thread's compositor snapshot instead of showing stale cross-thread transcript pixels"
        )
    }

    func testCachedTranscriptSnapshotKeepsCapturedHeightWhenRepresentableStartsAtZeroSize() throws {
        let threadID = "thread::snapshot-vertical-scale-repro-\(UUID().uuidString)"
        let capturedSize = CGSize(width: 393, height: 600)
        let firstPresentedSize = CGSize(width: 393, height: 180)

        let sourceController = UIViewController()
        let sourceScrollView = UIScrollView(frame: CGRect(origin: .zero, size: capturedSize))
        sourceScrollView.backgroundColor = .white
        sourceScrollView.contentSize = capturedSize
        sourceController.view = sourceScrollView

        let sourceWindow = makeTestWindow(frame: CGRect(origin: .zero, size: capturedSize))
        sourceWindow.rootViewController = sourceController
        sourceWindow.isHidden = false
        defer {
            sourceWindow.isHidden = true
            sourceWindow.rootViewController = nil
        }

        let transcript = UILabel(frame: CGRect(x: 16, y: 16, width: 361, height: 568))
        transcript.numberOfLines = 0
        transcript.font = .systemFont(ofSize: 17)
        transcript.text = """
        Ran 3 commands
        会话恢复，逐面提取瞬态界面。

        Ran 2 commands
        五个页面已经全部提取完成。

        Ran 5 commands
        设计验证完成，等待后续处理。
        """
        sourceScrollView.addSubview(transcript)
        sourceWindow.layoutIfNeeded()

        GaryxConversationTranscriptSnapshotCache.shared.scheduleCapture(
            threadID: threadID,
            revision: "captured-600pt-transcript",
            scrollView: { sourceScrollView }
        )
        let snapshotCaptured = waitForTranscriptSnapshot(threadID: threadID)
        XCTAssertTrue(
            snapshotCaptured,
            "the production compositor capture must complete before exercising installation"
        )

        // Mirror UIViewRepresentable.makeUIView: the cache installs into a
        // zero-sized container before SwiftUI supplies the first real frame.
        let openingContainer = UIView(frame: .zero)
        GaryxConversationTranscriptSnapshotCache.shared.installSnapshot(
            for: threadID,
            in: openingContainer
        )
        openingContainer.frame = CGRect(origin: .zero, size: firstPresentedSize)
        openingContainer.layoutIfNeeded()

        XCTAssertEqual(openingContainer.subviews.count, 1)
        let installedSnapshot = try XCTUnwrap(openingContainer.subviews.first)
        XCTAssertTrue(openingContainer.clipsToBounds)
        XCTAssertEqual(installedSnapshot.transform, CGAffineTransform.identity)
        XCTAssertEqual(
            installedSnapshot.frame.minY,
            openingContainer.bounds.minY,
            accuracy: 0.001
        )
        XCTAssertEqual(
            installedSnapshot.frame.minX,
            openingContainer.bounds.minX,
            accuracy: 0.001
        )
        XCTAssertGreaterThan(installedSnapshot.frame.maxY, openingContainer.bounds.maxY)
        let horizontalRenderScale = installedSnapshot.bounds.width / capturedSize.width
        let verticalRenderScale = installedSnapshot.bounds.height / capturedSize.height
        XCTAssertEqual(horizontalRenderScale, 1, accuracy: 0.001)
        XCTAssertEqual(
            verticalRenderScale,
            1,
            accuracy: 0.001,
            "a compositor snapshot must retain its captured 1:1 pixel geometry; resizing its bounds to the transient opening height vertically squashes every text glyph"
        )

        openingContainer.frame = CGRect(
            origin: .zero,
            size: CGSize(width: 240, height: 300)
        )
        openingContainer.layoutIfNeeded()

        XCTAssertEqual(installedSnapshot.bounds.width, capturedSize.width, accuracy: 0.001)
        XCTAssertEqual(installedSnapshot.bounds.height, capturedSize.height, accuracy: 0.001)
        XCTAssertEqual(
            installedSnapshot.frame.minY,
            openingContainer.bounds.minY,
            accuracy: 0.001
        )
        XCTAssertEqual(
            installedSnapshot.frame.minX,
            openingContainer.bounds.minX,
            accuracy: 0.001
        )
        XCTAssertGreaterThan(installedSnapshot.frame.maxX, openingContainer.bounds.maxX)
        XCTAssertGreaterThan(installedSnapshot.frame.maxY, openingContainer.bounds.maxY)
    }

    func testWarmReentrySnapshotMapsFullPageCaptureIntoTranscriptContainer() throws {
        let threadID = "thread::snapshot-page-space-repro-\(UUID().uuidString)"
        let pageSize = CGSize(width: 402, height: 874)
        let sourceController = UIViewController()
        let sourceScrollView = UIScrollView(frame: CGRect(origin: .zero, size: pageSize))
        sourceScrollView.contentInsetAdjustmentBehavior = .never
        sourceScrollView.contentInset = UIEdgeInsets(top: 124, left: 0, bottom: 0, right: 0)
        sourceScrollView.contentOffset = CGPoint(x: 0, y: -124)
        sourceScrollView.contentSize = CGSize(width: 402, height: 249)
        sourceController.view.addSubview(sourceScrollView)

        let firstRow = UIView(frame: CGRect(x: 16, y: 34, width: 370, height: 80))
        firstRow.backgroundColor = .black
        sourceScrollView.addSubview(firstRow)

        let sourceWindow = makeTestWindow(frame: CGRect(origin: .zero, size: pageSize))
        sourceWindow.rootViewController = sourceController
        sourceWindow.isHidden = false
        defer {
            sourceWindow.isHidden = true
            sourceWindow.rootViewController = nil
        }
        sourceWindow.layoutIfNeeded()

        XCTAssertEqual(sourceScrollView.adjustedContentInset.top, 124, accuracy: 0.001)
        XCTAssertEqual(sourceScrollView.contentOffset.y, -124, accuracy: 0.001)
        GaryxConversationTranscriptSnapshotCache.shared.scheduleCapture(
            threadID: threadID,
            revision: "full-page-402x874-inset-124",
            scrollView: { sourceScrollView }
        )
        XCTAssertTrue(
            waitForTranscriptSnapshot(threadID: threadID),
            "the measured full-page scroll geometry must be captured before installation"
        )

        let destinationController = UIViewController()
        let openingContainer = GaryxConversationTranscriptSnapshotHostView(
            frame: CGRect(x: 0, y: 124, width: 402, height: 593)
        )
        destinationController.view.addSubview(openingContainer)
        let destinationWindow = makeTestWindow(frame: CGRect(origin: .zero, size: pageSize))
        destinationWindow.rootViewController = destinationController
        destinationWindow.isHidden = false
        defer {
            destinationWindow.isHidden = true
            destinationWindow.rootViewController = nil
        }
        destinationWindow.layoutIfNeeded()

        openingContainer.displaySnapshot(for: threadID)
        openingContainer.layoutIfNeeded()

        let installedSnapshot = try XCTUnwrap(openingContainer.subviews.first)
        XCTAssertEqual(installedSnapshot.frame.minX, 0, accuracy: 0.001)
        XCTAssertEqual(installedSnapshot.frame.minY, -124, accuracy: 0.001)
        XCTAssertEqual(installedSnapshot.frame.size, pageSize)

        let firstRowYInCapturedPixels = firstRow.frame.minY - sourceScrollView.contentOffset.y
        let liveFirstRowPageY = sourceScrollView.frame.minY + firstRowYInCapturedPixels
        let openingFirstRowPageY = openingContainer.frame.minY
            + installedSnapshot.frame.minY
            + firstRowYInCapturedPixels
        XCTAssertEqual(liveFirstRowPageY, 158, accuracy: 0.001)
        XCTAssertEqual(
            openingFirstRowPageY,
            liveFirstRowPageY,
            accuracy: 0.001,
            "the cached opening pixels must keep the live transcript's page position"
        )
    }

    func testTaskNotificationAndLongUserBubbleShareTrailingWidthOwnerAcrossDynamicType() throws {
        let viewportWidth = try XCTUnwrap(
            UIApplication.shared.connectedScenes
                .compactMap { ($0 as? UIWindowScene)?.screen }
                .first
        ).bounds.width
        let transcriptWidth = viewportWidth - 32
        let longBody = """
        阶段一已经完成，下面是用于稳定复现窄列换行的较长说明。This synthetic paragraph is intentionally long enough to fill the ordinary user-message cap at every tested size.

        - 第一项验证进入线程时的消息阅读宽度
        - 第二项验证长文本与列表共享同一个 trailing edge
        - 第三项验证 Dynamic Type 分支只由 user-role owner 决定
        """
        let taskMessage = GaryxMobileMessage(
            id: "task-notification-width-repro",
            role: .user,
            text: """
            <garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review" title="Width reproduction">
            \(longBody)
            </garyx_task_notification>

            View details: garyx task get #TASK-42
            """,
            timestamp: "00:00",
            isStreaming: false,
            renderPresentation: .taskNotification(
                event: "ready_for_review",
                status: "in_review",
                taskId: "#TASK-42",
                title: "Width reproduction"
            )
        )
        let userMessage = GaryxMobileMessage(
            id: "ordinary-user-width-repro",
            role: .user,
            text: longBody,
            timestamp: "00:00",
            isStreaming: false
        )

        for (size, fraction, leadingSpacer) in [
            (DynamicTypeSize.large, CGFloat(0.77), CGFloat(60)),
            (DynamicTypeSize.xxxLarge, CGFloat(0.94), CGFloat(12)),
        ] {
            let taskImage = try renderedMessageImage(
                message: taskMessage,
                transcriptWidth: transcriptWidth,
                dynamicTypeSize: size
            )
            let userImage = try renderedMessageImage(
                message: userMessage,
                transcriptWidth: transcriptWidth,
                dynamicTypeSize: size
            )
            let taskAttachment = XCTAttachment(image: taskImage)
            taskAttachment.name = "task-notification-\(size)"
            taskAttachment.lifetime = .keepAlways
            add(taskAttachment)
            let userAttachment = XCTAttachment(image: userImage)
            userAttachment.name = "ordinary-user-\(size)"
            userAttachment.lifetime = .keepAlways
            add(userAttachment)
            let taskBounds = try nonMagentaBounds(in: taskImage)
            let userBounds = try nonMagentaBounds(in: userImage)
            let sharedCap = min(viewportWidth * fraction, transcriptWidth - leadingSpacer)

            XCTAssertEqual(taskBounds.maxX, userBounds.maxX, accuracy: 2)
            XCTAssertEqual(taskBounds.maxX, transcriptWidth, accuracy: 2)
            XCTAssertEqual(taskBounds.width, userBounds.width, accuracy: 3)
            XCTAssertEqual(taskBounds.width, sharedCap, accuracy: 3)
        }
    }

    func testTaskNotificationClampUsesRealMarkdownLayoutAcrossWidthAndDynamicType() async throws {
        let short = try await taskNotificationMeasurement(
            body: "All focused tests pass.",
            width: 300,
            dynamicTypeSize: .large
        )
        XCTAssertFalse(short.overflows, "short measurement: \(short)")
        XCTAssertLessThan(
            short.naturalHeight,
            short.clampHeight,
            "short measurement: \(short)"
        )

        let wrappingBody = Array(repeating: "A single source line wraps through the shared card width.", count: 7)
            .joined(separator: " ")
        let narrow = try await taskNotificationMeasurement(
            body: wrappingBody,
            width: 250,
            dynamicTypeSize: .large
        )
        let wide = try await taskNotificationMeasurement(
            body: wrappingBody,
            width: 370,
            dynamicTypeSize: .large
        )
        XCTAssertTrue(narrow.overflows, "narrow measurement: \(narrow)")
        XCTAssertFalse(wide.overflows, "wide measurement: \(wide)")
        XCTAssertGreaterThan(narrow.naturalHeight, wide.naturalHeight)

        let explicitLines = (1...11).map { "Explicit line \($0)." }.joined(separator: "\n")
        let explicit = try await taskNotificationMeasurement(
            body: explicitLines,
            width: 300,
            dynamicTypeSize: .large
        )
        XCTAssertTrue(explicit.overflows, "explicit-lines measurement: \(explicit)")

        let richBody = """
        - Manifest discovery passed
        - Enable and disable passed
        - Login-state end-to-end path passed

        ```swift
        let manifest = await discoverTools()
        await verify(manifest)
        ```

        | Surface | Result |
        | --- | --- |
        | Desktop | pass |
        | iOS | pass |
        """
        let rich = try await taskNotificationMeasurement(
            body: richBody,
            width: 300,
            dynamicTypeSize: .large
        )
        XCTAssertTrue(rich.overflows, "rich measurement: \(rich)")

        let accessibility = try await taskNotificationMeasurement(
            body: wrappingBody,
            width: 370,
            dynamicTypeSize: .xxxLarge
        )
        XCTAssertGreaterThan(accessibility.clampHeight, wide.clampHeight)
        XCTAssertNotEqual(accessibility.naturalHeight, wide.naturalHeight)
    }

    func testTaskNotificationRemeasuresWhenMarkdownImageSettlesLate() async throws {
        let preview = try makeTaskNotificationImagePreview(size: CGSize(width: 100, height: 1_000))
        let sink = TaskNotificationMeasurementSink()
        let notification = taskNotification(body: "![Late intrinsic image](late-image.png)")
        let root = AnyView(
            GaryxTaskNotificationCard(
                notification: notification,
                onExpand: {},
                onFileLinkTap: { _ in },
                onImageFilePreview: { _ in
                    try? await Task.sleep(for: .milliseconds(500))
                    return preview
                },
                onMeasurement: { sink.values.append($0) }
            )
            .frame(width: 300)
            .environment(\.dynamicTypeSize, .large)
        )
        let controller = UIHostingController(rootView: root)
        let window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: 300, height: 800))
        window.rootViewController = controller
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }

        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(150))
        window.layoutIfNeeded()
        let loading = try XCTUnwrap(sink.values.last)
        XCTAssertFalse(loading.overflows)

        let deadline = ContinuousClock.now + .seconds(3)
        while ContinuousClock.now < deadline,
              !(sink.values.last?.naturalHeight ?? 0 > loading.naturalHeight + 1) {
            window.layoutIfNeeded()
            try await Task.sleep(for: .milliseconds(50))
        }
        let settled = try XCTUnwrap(sink.values.last)
        XCTAssertTrue(settled.overflows, "settled measurements: \(sink.values)")
        XCTAssertGreaterThan(
            settled.naturalHeight,
            loading.naturalHeight,
            "all measurements: \(sink.values)"
        )
        XCTAssertEqual(settled.clampHeight, loading.clampHeight, accuracy: 0.5)
    }

    func testInactiveConversationPreparationDoesNotMutatePathAndPushReusesHost() throws {
        let harness = Harness(path: [])
        let prepared = entry(
            1,
            destination: .conversation(threadID: "thread-prepared")
        )

        XCTAssertTrue(harness.container.prepareInactiveHost(prepared))
        harness.pumpUI()

        XCTAssertTrue(harness.container.path.isEmpty)
        XCTAssertEqual(harness.hostBuildProbe.buildCount(for: prepared.id), 1)
        let preparedWrapper = try XCTUnwrap(
            harness.wrapper(identity: .entry(prepared.id))
        )
        let preparedHostedView = try XCTUnwrap(preparedWrapper.contentView.subviews.first)
        XCTAssertFalse(preparedWrapper.isHidden)
        XCTAssertFalse(preparedWrapper.isUserInteractionEnabled)
        XCTAssertTrue(preparedWrapper.accessibilityElementsHidden)
        XCTAssertEqual(preparedWrapper.alpha, 0.01, accuracy: 0.001)
        XCTAssertEqual(preparedWrapper.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(preparedHostedView.bounds.width, harness.width, accuracy: 0.01)

        XCTAssertTrue(harness.container.push(prepared, animated: false))
        harness.pumpUI()

        XCTAssertEqual(harness.container.path, [prepared])
        XCTAssertEqual(
            harness.hostBuildProbe.buildCount(for: prepared.id),
            1,
            "the admitted push must reuse the touch-prepared host"
        )
        XCTAssertFalse(preparedWrapper.isHidden)
        XCTAssertTrue(preparedWrapper.isUserInteractionEnabled)
        XCTAssertFalse(preparedWrapper.accessibilityElementsHidden)
        XCTAssertEqual(preparedWrapper.alpha, 1, accuracy: 0.001)
        XCTAssertEqual(preparedWrapper.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(preparedHostedView.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testSpatialPushTranslatesFullWidthDestinationWithoutNarrowingItsHost() throws {
        let harness = Harness(path: [])
        let destination = entry(
            1,
            destination: .conversation(threadID: "thread-width-repro")
        )

        XCTAssertTrue(harness.container.push(destination, animated: true))
        harness.pumpUI()

        let wrapper = try XCTUnwrap(
            harness.wrapper(identity: .entry(destination.id))
        )
        let hostedView = try XCTUnwrap(wrapper.contentView.subviews.first)
        XCTAssertEqual(wrapper.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(wrapper.contentView.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(hostedView.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(wrapper.transform.tx, harness.width, accuracy: 0.01)

        harness.advance(
            by: GaryxRouteTransitionCalibration.programmaticSettleCurve.settlingDuration * 0.4
        )

        XCTAssertGreaterThan(wrapper.transform.tx, 0)
        XCTAssertLessThan(wrapper.transform.tx, harness.width)
        XCTAssertEqual(wrapper.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(wrapper.contentView.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(hostedView.bounds.width, harness.width, accuracy: 0.01)

        harness.completeDisplayLinkedSettle()

        XCTAssertEqual(wrapper.transform, .identity)
        XCTAssertEqual(wrapper.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(wrapper.contentView.bounds.width, harness.width, accuracy: 0.01)
        XCTAssertEqual(hostedView.bounds.width, harness.width, accuracy: 0.01)
    }

    func testRendererInfrastructureDoesNotOwnPageBackground() throws {
        let harness = Harness(path: [entry(1)])
        let wrapper = try XCTUnwrap(harness.visibleWrapper())
        let hostedView = try XCTUnwrap(wrapper.contentView.subviews.first)

        XCTAssertEqual(harness.container.view.backgroundColor, UIColor.clear)
        XCTAssertFalse(harness.container.view.isOpaque)
        XCTAssertEqual(wrapper.backgroundColor, UIColor.clear)
        XCTAssertFalse(wrapper.isOpaque)
        XCTAssertEqual(wrapper.contentView.backgroundColor, UIColor.clear)
        XCTAssertFalse(wrapper.contentView.isOpaque)
        XCTAssertEqual(hostedView.backgroundColor, UIColor.clear)
        XCTAssertFalse(hostedView.isOpaque)

        XCTAssertEqual(wrapper.scrimView.backgroundColor, UIColor.black)
        XCTAssertEqual(wrapper.scrimView.alpha, 0)
    }

    func testProductionRouteCanvasOccupiesTheFullWindowBeyondSafeAreas() throws {
        let suiteName = "GaryxRouteCanvasTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = GaryxProductionRouteStore()
        let model = GaryxMobileModel(defaults: defaults)
        let rootSurfaceOccurrenceID = GaryxRootSurfaceOccurrenceID(rawValue: 1)
        model.applyGlobalRevealRootSurfaceTransition(
            .navigationShellBegan(rootSurfaceOccurrenceID)
        )
        let root = UIHostingController(
            rootView: GaryxProductionRouteCanvas(
                rootSurfaceOccurrenceID: rootSurfaceOccurrenceID,
                store: store,
                model: model,
                homeContent: AnyView(Color.blue),
                routeContent: { _ in AnyView(Color.green) },
                onOpenDrawer: {}
            )
        )
        root.additionalSafeAreaInsets = UIEdgeInsets(top: 24, left: 0, bottom: 20, right: 0)
        let window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: 393, height: 852))
        window.rootViewController = root
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        root.view.frame = window.bounds
        root.view.setNeedsLayout()
        root.view.layoutIfNeeded()
        pumpMainRunLoop(duration: 0.1)

        let container = try XCTUnwrap(
            descendants(of: root).compactMap { $0 as? GaryxRouteStackContainer }.first
        )
        let canvasFrame = container.view.convert(container.view.bounds, to: root.view)
        XCTAssertGreaterThan(root.view.safeAreaInsets.top, 0)
        XCTAssertGreaterThan(root.view.safeAreaInsets.bottom, 0)
        XCTAssertEqual(canvasFrame.minY, root.view.bounds.minY, accuracy: 0.5)
        XCTAssertEqual(canvasFrame.maxY, root.view.bounds.maxY, accuracy: 0.5)
    }

    func testProductionRouteCanvasRebindsRevealOwnerWhenRootUpdatesInPlace() throws {
        let suiteName = "GaryxRouteCanvasRootOccurrenceTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = GaryxProductionRouteStore()
        let model = GaryxMobileModel(defaults: defaults)
        model.gatewayURL = "http://127.0.0.1:4000"
        model.connectionState = .ready(version: "first")
        guard case .navigationShell(let firstRootOccurrence) =
            model.homeObservationStore.rootSurface else {
            return XCTFail("first navigation Shell did not begin")
        }

        func canvas(
            _ occurrenceID: GaryxRootSurfaceOccurrenceID
        ) -> GaryxProductionRouteCanvas {
            GaryxProductionRouteCanvas(
                rootSurfaceOccurrenceID: occurrenceID,
                store: store,
                model: model,
                homeContent: AnyView(Color.blue),
                routeContent: { _ in AnyView(Color.green) },
                onOpenDrawer: {}
            )
        }

        let root = UIHostingController(rootView: canvas(firstRootOccurrence))
        let window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: 393, height: 852))
        window.rootViewController = root
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        root.view.frame = window.bounds
        root.view.layoutIfNeeded()
        pumpMainRunLoop(duration: 0.1)

        let initialContainer = try XCTUnwrap(
            descendants(of: root).compactMap { $0 as? GaryxRouteStackContainer }.first
        )
        model.drawerRevealInteraction.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: firstRootOccurrence
        )
        let initialInteraction = try XCTUnwrap(initialContainer.homeLeadingEdgeInteraction)
        XCTAssertTrue(initialInteraction.isEligible())
        initialInteraction.began()
        initialInteraction.changed(120, 0)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)

        // Publish an exit and re-entry before SwiftUI receives a replacement
        // canvas. Updating the same representable must rotate its host token,
        // clear the old touch-stream token, and retain the physical container.
        model.connectionState = .checking
        model.connectionState = .ready(version: "second")
        guard case .navigationShell(let secondRootOccurrence) =
            model.homeObservationStore.rootSurface else {
            return XCTFail("second navigation Shell did not begin")
        }
        XCTAssertNotEqual(secondRootOccurrence, firstRootOccurrence)
        root.rootView = canvas(secondRootOccurrence)
        root.view.layoutIfNeeded()
        pumpMainRunLoop(duration: 0.1)

        let updatedContainer = try XCTUnwrap(
            descendants(of: root).compactMap { $0 as? GaryxRouteStackContainer }.first
        )
        XCTAssertTrue(updatedContainer === initialContainer)
        let updatedInteraction = try XCTUnwrap(updatedContainer.homeLeadingEdgeInteraction)

        initialInteraction.changed(180, 0)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .idle)
        XCTAssertTrue(updatedInteraction.isEligible())
        updatedInteraction.began()
        updatedInteraction.changed(80, 0)
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)

        model.connectionState = .checking
        pumpMainRunLoop(duration: 0.1)
        XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
    }

    func testProductionRouteCanvasRootUpdateDefersRevealPublishOutsideUpdateWindow() async throws {
        let suiteName = "GaryxRouteCanvasUpdatePublishTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = GaryxProductionRouteStore()
        let model = GaryxMobileModel(defaults: defaults)
        model.gatewayURL = "http://127.0.0.1:4000"
        model.connectionState = .ready(version: "first")
        guard case .navigationShell(let firstRootOccurrence) =
            model.homeObservationStore.rootSurface else {
            return XCTFail("first navigation Shell did not begin")
        }

        func canvas(
            _ occurrenceID: GaryxRootSurfaceOccurrenceID
        ) -> GaryxProductionRouteCanvas {
            GaryxProductionRouteCanvas(
                rootSurfaceOccurrenceID: occurrenceID,
                store: store,
                model: model,
                homeContent: AnyView(Color.blue),
                routeContent: { _ in AnyView(Color.green) },
                onOpenDrawer: {}
            )
        }

        let root = UIHostingController(rootView: canvas(firstRootOccurrence))
        let window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: 393, height: 852))
        window.rootViewController = root
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        root.view.frame = window.bounds
        root.view.layoutIfNeeded()
        pumpMainRunLoop(duration: 0.1)

        let initialContainer = try XCTUnwrap(
            descendants(of: root).compactMap { $0 as? GaryxRouteStackContainer }.first
        )
        model.drawerRevealInteraction.configure(
            extent: 330,
            restingPosition: .closed,
            rootSurfaceOccurrenceID: firstRootOccurrence
        )

        model.connectionState = .checking
        model.connectionState = .ready(version: "second")
        guard case .navigationShell(let secondRootOccurrence) =
            model.homeObservationStore.rootSurface else {
            return XCTFail("second navigation Shell did not begin")
        }
        let foregroundHost = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: secondRootOccurrence,
            rawValue: "foreground-update-host"
        )
        model.attachGlobalRevealHostOccurrence(foregroundHost)
        model.drawerRevealInteraction.beginGesture(in: foregroundHost)
        model.drawerRevealInteraction.updateGesture(
            logicalTranslation: 120,
            in: foregroundHost
        )
        XCTAssertEqual(model.drawerRevealInteraction.presentation.phase, .dragging)

        var publicationCount = 0
        var synchronousUpdatePublicationCount = 0
        var isUpdatingRootView = false
        let publication = model.drawerRevealInteraction.objectWillChange.sink {
            publicationCount += 1
            if isUpdatingRootView {
                synchronousUpdatePublicationCount += 1
            }
        }

        isUpdatingRootView = true
        root.rootView = canvas(secondRootOccurrence)
        root.view.layoutIfNeeded()
        isUpdatingRootView = false

        XCTAssertEqual(
            synchronousUpdatePublicationCount,
            0,
            "UIViewControllerRepresentable.update must not synchronously publish"
        )
        for _ in 0..<20
            where model.drawerRevealInteraction.diagnostics.hasTerminalResidue {
            root.view.layoutIfNeeded()
            await Task.yield()
        }

        let updatedContainer = try XCTUnwrap(
            descendants(of: root).compactMap { $0 as? GaryxRouteStackContainer }.first
        )
        XCTAssertTrue(updatedContainer === initialContainer)
        XCTAssertGreaterThan(publicationCount, 0, "the deferred terminal publish must land")
        XCTAssertEqual(synchronousUpdatePublicationCount, 0)
        XCTAssertFalse(model.drawerRevealInteraction.diagnostics.hasTerminalResidue)
        XCTAssertTrue(try XCTUnwrap(updatedContainer.homeLeadingEdgeInteraction).isEligible())
        withExtendedLifetime(publication) {}
    }

    func testFakeRouteHostRequiresExplicitDebugEnvironmentOptIn() throws {
        XCTAssertNil(GaryxFluidFakeRouteDebugFixture.Configuration.load(environment: [:]))
        let configuration = try XCTUnwrap(
            GaryxFluidFakeRouteDebugFixture.Configuration.load(environment: [
                "GARYX_MOBILE_FLUID_FAKE_ROUTES": "1",
                "GARYX_MOBILE_FLUID_FAKE_DEPTH": "20",
                "GARYX_MOBILE_FLUID_FAKE_RTL": "1",
                "GARYX_MOBILE_FLUID_FAKE_VISUAL_POLICY": "crossFade",
            ])
        )
        XCTAssertEqual(configuration.initialDepth, 20)
        XCTAssertEqual(configuration.layoutDirection, .rightToLeft)
        XCTAssertEqual(configuration.preferences.resolvedPolicy, .crossFade)
    }

    func testCommitWritesCanonicalAtReleaseAndScreenChangedOnlyAtVisibleTerminal() {
        let harness = Harness(path: [entry(1), entry(2)])
        let bodyCountBeforeDrag = harness.bodyCounter.count

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.20)
        XCTAssertEqual(harness.container.path.count, 2)
        XCTAssertEqual(harness.probe.screenChangedCount, 0)
        XCTAssertEqual(harness.bodyCounter.count, bodyCountBeforeDrag)

        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 300), .committed)
        XCTAssertEqual(harness.container.path.count, 1, "canonical path changes at commit release")
        XCTAssertEqual(harness.probe.screenChangedCount, 0, "settle is not page terminal")
        XCTAssertEqual(harness.container.metrics.transitionPhase, .commitSettle)

        harness.completeDisplayLinkedSettle()

        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .visible)]
        )
        XCTAssertFalse(harness.container.hasTerminalResidue)
        XCTAssertLessThanOrEqual(
            harness.container.metrics.mountedHostCount,
            GaryxRouteStackContainer.maximumMountedHostCount
        )
        XCTAssertTrue(harness.container.children.allSatisfy { $0.view.transform == .identity })
    }

    func testA4cAssistiveTechnologyAcceptanceMatrix() throws {
        let cases: [(name: String, preferences: GaryxRouteVisualPreferences, policy: GaryxRouteVisualPolicy)] = [
            ("VoiceOver", .init(reduceMotion: false, prefersCrossFadeTransitions: false), .spatial),
            ("Switch Control", .init(reduceMotion: false, prefersCrossFadeTransitions: false), .spatial),
            ("Full Keyboard Access", .init(reduceMotion: false, prefersCrossFadeTransitions: false), .spatial),
            ("Reduce Motion + Cross-Fade", .init(reduceMotion: true, prefersCrossFadeTransitions: true), .crossFade),
        ]

        for item in cases {
            let harness = Harness(path: [entry(1)], preferences: item.preferences)
            XCTAssertTrue(harness.container.beginInteractivePop(), item.name)
            let staged = try XCTUnwrap(
                harness.wrapper(identity: .home),
                item.name
            )
            XCTAssertTrue(staged.accessibilityElementsHidden, item.name)
            XCTAssertFalse(staged.isUserInteractionEnabled, item.name)
            XCTAssertEqual(
                harness.container.visualPolicyForActiveTransaction,
                item.policy,
                item.name
            )
            XCTAssertEqual(harness.probe.screenChangedArguments.count, 0, item.name)

            harness.container.updateInteractivePop(
                logicalTranslation: harness.width * 0.8
            )
            XCTAssertEqual(
                harness.container.endInteractivePop(logicalVelocity: harness.width * 2),
                .committed,
                item.name
            )
            harness.completeDisplayLinkedSettle()

            XCTAssertEqual(harness.probe.screenChangedArguments.count, 1, item.name)
            let visibleWrapper = try XCTUnwrap(harness.visibleWrapper(), item.name)
            let screenChangedArgument = harness.probe.screenChangedArguments[0]
            XCTAssertTrue(
                screenChangedArgument === visibleWrapper
                    || screenChangedArgument.isDescendant(of: visibleWrapper),
                "screenChanged must carry the committed visible host for \(item.name)"
            )
            XCTAssertTrue(screenChangedArgument.window === harness.window, item.name)
            XCTAssertEqual(
                harness.probe.screenChangedHostWasVisible,
                [true],
                "screenChanged must run after the destination becomes interactive for \(item.name)"
            )
        }
    }

    func testCancelSettleCanRegrabAndCarryPhysicalProgressIntoCommit() throws {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.3947)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)
        XCTAssertEqual(harness.container.path.count, 1)
        XCTAssertEqual(harness.container.metrics.transitionPhase, .cancelSettle)

        harness.advance(by: 0.08)
        let regrab = try XCTUnwrap(harness.container.regrabCancelSettle())
        XCTAssertGreaterThan(regrab.value, 0)
        XCTAssertLessThan(regrab.value, 0.3947)
        XCTAssertEqual(harness.container.metrics.transitionPhase, .preCommit)
        XCTAssertFalse(harness.frames.isRunning)

        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.70)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertTrue(harness.container.path.isEmpty)
        harness.completeDisplayLinkedSettle()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
        XCTAssertTrue(
            harness.probe.phases.containsSubsequence([
                .preCommit,
                .cancelSettle,
                .preCommit,
                .commitSettle,
                .terminal,
            ])
        )
    }

    func testPromotionRebuildsMountedHostForSameOccurrenceWithThreadDestination() {
        let draftID = "draft-mounted"
        let threadID = "thread-promoted"
        let draft = entry(
            1,
            destination: .conversationDraft(draftID: draftID)
        )
        let harness = Harness(path: [draft])
        let initialBuildCount = harness.hostBuildProbe.buildCount(for: draft.id)
        let mountedIdentity = GaryxRoutePresentationIdentity.entry(draft.id)
        let initialMountCount = harness.probe.mounted.filter { $0 == mountedIdentity }.count

        XCTAssertGreaterThan(initialBuildCount, 0)
        XCTAssertTrue(
            harness.container.promoteVisibleDraft(
                instanceID: draft.id,
                draftID: draftID,
                threadID: threadID
            )
        )
        harness.pumpUI()

        XCTAssertEqual(harness.container.path.last?.id, draft.id)
        XCTAssertEqual(
            harness.container.path.last?.destination,
            .conversation(threadID: threadID)
        )
        XCTAssertGreaterThan(
            harness.hostBuildProbe.buildCount(for: draft.id),
            initialBuildCount,
            "promotion must rebuild the already-mounted host for the stable occurrence"
        )
        XCTAssertEqual(
            harness.hostBuildProbe.lastDestination(for: draft.id),
            .conversation(threadID: threadID)
        )
        XCTAssertEqual(
            harness.probe.mounted.filter { $0 == mountedIdentity }.count,
            initialMountCount,
            "root replacement must preserve occurrence and host mount identity"
        )
    }

    func testProductionDraftRouteNeverConnectsStagedDriverAcrossPromotion() throws {
        let suiteName = "GaryxDraftRouteWiringTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let model = GaryxMobileModel(defaults: defaults)
        let rootSurfaceOccurrenceID = GaryxRootSurfaceOccurrenceID(rawValue: 1)
        let revealHostOccurrenceID = GaryxHorizontalRevealHostOccurrenceID(
            rootSurfaceOccurrenceID: rootSurfaceOccurrenceID,
            rawValue: "draft-route-wiring-host"
        )
        model.applyGlobalRevealRootSurfaceTransition(
            .navigationShellBegan(rootSurfaceOccurrenceID)
        )
        model.attachGlobalRevealHostOccurrence(revealHostOccurrenceID)
        defer {
            model.detachGlobalRevealHostOccurrence(revealHostOccurrenceID)
            model.applyGlobalRevealRootSurfaceTransition(
                .navigationShellEnded(rootSurfaceOccurrenceID)
            )
        }
        let draft = entry(
            1,
            destination: .conversationDraft(draftID: "draft-direct")
        )
        let draftIdentity = GaryxRoutePresentationIdentity.entry(draft.id)
        let draftRegistry = GaryxRouteLifecycleRegistry()
        let draftHarness = Harness(
            path: [draft],
            routeLifecycleRegistry: draftRegistry,
            routeHostBuilder: { [model, draftRegistry] node in
                Self.productionConversationHost(
                    node: node,
                    model: model,
                    rootSurfaceOccurrenceID: rootSurfaceOccurrenceID,
                    routeLifecycleRegistry: draftRegistry
                )
            }
        )
        draftHarness.pumpUI(duration: 0.05)

        XCTAssertEqual(
            draftRegistry.observationCounts(for: draftIdentity),
            .init(),
            "a local draft must not own the staged lifecycle or frame observers"
        )
        XCTAssertFalse(draftRegistry.hasPresentedFrameDemand)

        XCTAssertTrue(
            draftHarness.container.promoteVisibleDraft(
                instanceID: draft.id,
                draftID: "draft-direct",
                threadID: "thread-promoted-direct"
            )
        )
        draftHarness.pumpUI(duration: 0.05)

        XCTAssertEqual(
            draftRegistry.observationCounts(for: draftIdentity),
            .init(),
            "in-place promotion must retain the draft occurrence's direct presentation plan"
        )
        XCTAssertFalse(draftRegistry.hasPresentedFrameDemand)

        let existing = entry(
            2,
            destination: .conversation(threadID: "thread-staged-control")
        )
        let existingIdentity = GaryxRoutePresentationIdentity.entry(existing.id)
        let existingRegistry = GaryxRouteLifecycleRegistry()
        let existingHarness = Harness(
            path: [existing],
            routeLifecycleRegistry: existingRegistry,
            routeHostBuilder: { [model, existingRegistry] node in
                Self.productionConversationHost(
                    node: node,
                    model: model,
                    rootSurfaceOccurrenceID: rootSurfaceOccurrenceID,
                    routeLifecycleRegistry: existingRegistry
                )
            }
        )
        existingHarness.pumpUI(duration: 0.05)

        XCTAssertEqual(
            existingRegistry.observationCounts(for: existingIdentity),
            .init(lifecycle: 1, presentedFrames: 1),
            "the sensitivity control must observe the staged gateway-thread driver"
        )
        XCTAssertTrue(existingRegistry.hasPresentedFrameDemand)

        withExtendedLifetime((draftHarness, existingHarness)) {}
    }

    func testPromotionDuringInteractivePopIsAppliedAfterCancellationWithoutInvalidatingGesture() {
        var draft = entry(2)
        draft.replacePayload(with: .conversationDraft(draftID: "draft-in-flight"))
        let harness = Harness(path: [entry(1), draft])
        let initialBuildCount = harness.hostBuildProbe.buildCount(for: draft.id)

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.20)
        XCTAssertTrue(
            harness.container.promoteVisibleDraft(
                instanceID: draft.id,
                draftID: "draft-in-flight",
                threadID: "thread-after-cancel"
            )
        )
        XCTAssertEqual(harness.container.path.last?.destination, draft.destination)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)

        harness.completeDisplayLinkedSettle()

        XCTAssertEqual(
            harness.container.path.last?.destination,
            .conversation(threadID: "thread-after-cancel")
        )
        XCTAssertGreaterThan(
            harness.hostBuildProbe.buildCount(for: draft.id),
            initialBuildCount,
            "queued promotion must rebuild the surviving mounted host after cancel settle"
        )
        XCTAssertEqual(
            harness.hostBuildProbe.lastDestination(for: draft.id),
            .conversation(threadID: "thread-after-cancel")
        )
        XCTAssertEqual(harness.probe.terminals.last, .init(outcome: .cancelled, visibility: .visible))
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testBatchPushCommitsIntermediatePredecessorInOneTransaction() {
        let first = entry(1)
        let harness = Harness(path: [first])
        let overview = entry(2, destination: .settingsDetail("manage"))
        let detail = entry(3, destination: .settingsDetail("gateway"))

        XCTAssertTrue(harness.container.push([overview, detail], animated: false))

        XCTAssertEqual(harness.container.path, [first, overview, detail])
        XCTAssertEqual(harness.probe.terminals, [
            .init(outcome: .committed, visibility: .visible),
        ])
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testPresentedContentTouchesDoNotReachUnderlyingRouteGestures() {
        let harness = Harness(path: [entry(1)])
        let modal = UIViewController()

        XCTAssertTrue(harness.container.routeOwnsGestureTouch(in: harness.container.view))
        harness.container.present(modal, animated: false)
        pumpMainRunLoop(duration: 0.05)

        XCTAssertFalse(harness.container.routeOwnsGestureTouch(in: modal.view))

        harness.container.dismiss(animated: false)
    }

    func testPublicEdgeRecognizersShareTheWindowFailureGraphWithDescendantPans() {
        let harness = Harness(path: [entry(1)])
        let descendantScroll = UIScrollView(frame: harness.container.view.bounds)
        harness.container.view.addSubview(descendantScroll)

        XCTAssertTrue(harness.container.leadingEdgePanGestureRecognizer.view === harness.window)
        XCTAssertTrue(harness.container.trailingEdgePanGestureRecognizer.view === harness.window)
        XCTAssertTrue(harness.container.gestureRecognizer(
            harness.container.leadingEdgePanGestureRecognizer,
            shouldBeRequiredToFailBy: descendantScroll.panGestureRecognizer
        ))
        XCTAssertTrue(harness.container.gestureRecognizer(
            harness.container.trailingEdgePanGestureRecognizer,
            shouldBeRequiredToFailBy: descendantScroll.panGestureRecognizer
        ))
    }

    func testHostedTouchDownSnapshotAndAxisLockUseRealLTRAndRTLCoordinates() {
        let harness = Harness(path: [entry(1)])

        harness.container.layoutDirectionOverride = .leftToRight
        harness.container.recordEdgeTouchDown(
            physicalX: 5,
            viewportWidth: harness.window.bounds.width,
            edge: .leading
        )
        XCTAssertTrue(
            harness.container.shouldBeginEdgePan(
                edge: .leading,
                translation: CGSize(width: 20, height: 0),
                velocity: .zero
            ),
            "a 5 pt touch remains navigation-owned after recognition at 25 pt"
        )

        harness.container.recordEdgeTouchDown(
            physicalX: 25,
            viewportWidth: harness.window.bounds.width,
            edge: .leading
        )
        XCTAssertFalse(
            harness.container.shouldBeginEdgePan(
                edge: .leading,
                translation: CGSize(width: -20, height: 0),
                velocity: .zero
            ),
            "moving backwards into the edge cannot rewrite touch-down ownership"
        )

        harness.container.recordEdgeTouchDown(
            physicalX: 5,
            viewportWidth: harness.window.bounds.width,
            edge: .leading
        )
        XCTAssertFalse(
            harness.container.shouldBeginEdgePan(
                edge: .leading,
                translation: CGSize(width: 20, height: 100),
                velocity: .zero
            ),
            "vertical intent must stay with the descendant scroll"
        )

        harness.container.layoutDirectionOverride = .rightToLeft
        harness.container.recordEdgeTouchDown(
            physicalX: harness.window.bounds.width - 5,
            viewportWidth: harness.window.bounds.width,
            edge: .leading
        )
        XCTAssertTrue(harness.container.shouldBeginEdgePan(
            edge: .leading,
            translation: CGSize(width: -20, height: 0),
            velocity: .zero
        ))
    }

    func testHomeDrawerAndTaskTreeUseNodeSpecificOwnersAndSettleInterruptSemantics() {
        let home = Harness(path: [])
        home.container.homeLeadingEdgeInteraction = edgeInteraction(
            requiresEdgeZone: true,
            direction: .positive
        )
        home.container.recordEdgeTouchDown(
            physicalX: 5,
            viewportWidth: home.window.bounds.width,
            edge: .leading
        )
        XCTAssertTrue(home.container.shouldBeginEdgePan(
            edge: .leading,
            translation: CGSize(width: 20, height: 0),
            velocity: .zero
        ))

        home.container.homeLeadingEdgeInteraction = edgeInteraction(
            requiresEdgeZone: false,
            direction: .either
        )
        home.container.recordEdgeTouchDown(
            physicalX: 180,
            viewportWidth: home.window.bounds.width,
            edge: .leading
        )
        XCTAssertTrue(
            home.container.shouldBeginEdgePan(
                edge: .leading,
                translation: CGSize(width: -20, height: 0),
                velocity: .zero
            ),
            "an in-flight drawer settle must be regrabbable in reverse"
        )

        let conversation = Harness(path: [entry(
            1,
            destination: .conversation(threadID: "synthetic-thread")
        )])
        conversation.container.trailingEdgeInteraction = edgeInteraction(
            requiresEdgeZone: true,
            direction: .positive
        )
        conversation.container.interactivePopEligible = { false }
        conversation.container.recordEdgeTouchDown(
            physicalX: conversation.window.bounds.width - 5,
            viewportWidth: conversation.window.bounds.width,
            edge: .trailing
        )
        XCTAssertTrue(conversation.container.shouldBeginEdgePan(
            edge: .trailing,
            translation: CGSize(width: -20, height: 0),
            velocity: .zero
        ))

        conversation.container.recordEdgeTouchDown(
            physicalX: 5,
            viewportWidth: conversation.window.bounds.width,
            edge: .leading
        )
        XCTAssertFalse(
            conversation.container.shouldBeginEdgePan(
                edge: .leading,
                translation: CGSize(width: 20, height: 0),
                velocity: .zero
            ),
            "an open task tree keeps route pop ineligible"
        )
    }

    func testPresentationBarrierDisablesBothPublicEdgeRecognizers() {
        let harness = Harness(path: [entry(1)])
        let token = GaryxPresentationLeaseToken(rawValue: "synthetic-edge-barrier")
        XCTAssertTrue(harness.container.acquirePresentationLease(token))
        XCTAssertFalse(harness.container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertFalse(harness.container.trailingEdgePanGestureRecognizer.isEnabled)
        harness.container.presentationDismissalCompleted(token)
        XCTAssertTrue(harness.container.leadingEdgePanGestureRecognizer.isEnabled)
        XCTAssertTrue(harness.container.trailingEdgePanGestureRecognizer.isEnabled)
    }

    func testHighVelocityRegrabSettleRemainsEndpointBoundedAndMonotonic() throws {
        let harness = Harness(path: [entry(1)])
        var commitProgress: [CGFloat] = []
        harness.container.transitionFrameObserver = { phase, progress, _ in
            if phase == .commitSettle {
                commitProgress.append(progress)
            }
        }
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.30)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)
        harness.advance(by: 0.08)
        _ = try XCTUnwrap(harness.container.regrabCancelSettle())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.70)
        XCTAssertEqual(
            harness.container.endInteractivePop(logicalVelocity: harness.width * 5),
            .committed
        )

        let frameInterval = 1.0 / 120.0
        for _ in 0...Int(GaryxRouteTransitionCalibration.settleCurve.settlingDuration / frameInterval) {
            harness.advance(by: frameInterval)
        }
        harness.completeDisplayLinkedSettle()

        XCTAssertGreaterThan(commitProgress.count, 2)
        XCTAssertTrue(commitProgress.allSatisfy { (0...1).contains($0) })
        XCTAssertTrue(zip(commitProgress, commitProgress.dropFirst()).allSatisfy { pair in
            pair.1 + 0.000_1 >= pair.0
        })
        XCTAssertEqual(commitProgress.last, 1)
    }

    func testCancelledInactiveRestoresWithoutAnnouncementAndSupersededHasNoEffects() {
        let inactive = Harness(path: [entry(1)])
        XCTAssertTrue(inactive.container.beginInteractivePop())
        inactive.container.updateInteractivePop(logicalTranslation: 80)
        inactive.container.sceneDidBecomeInactive()

        XCTAssertEqual(inactive.container.path.count, 1)
        XCTAssertEqual(
            inactive.probe.terminals,
            [.init(outcome: .cancelled, visibility: .inactive)]
        )
        XCTAssertEqual(inactive.probe.screenChangedCount, 0)
        XCTAssertFalse(try! XCTUnwrap(inactive.visibleWrapper()).isUserInteractionEnabled)

        inactive.container.sceneDidBecomeActive()
        XCTAssertEqual(inactive.probe.screenChangedCount, 0)
        XCTAssertTrue(try! XCTUnwrap(inactive.visibleWrapper()).isUserInteractionEnabled)

        let superseded = Harness(path: [entry(1)])
        XCTAssertTrue(superseded.container.beginInteractivePop())
        superseded.container.updateInteractivePop(logicalTranslation: 90)
        superseded.container.supersedeActiveTransition()

        XCTAssertEqual(superseded.container.path.count, 1)
        XCTAssertEqual(
            superseded.probe.terminals,
            [.init(outcome: .cancelled, visibility: .superseded)]
        )
        XCTAssertEqual(superseded.probe.screenChangedCount, 0)
        XCTAssertFalse(try! XCTUnwrap(superseded.visibleWrapper()).isUserInteractionEnabled)
    }

    func testCommittedInactiveDefersOneAnnouncementAndCommittedSupersededEmitsNone() {
        let inactive = Harness(path: [entry(1)])
        XCTAssertTrue(inactive.container.beginInteractivePop())
        inactive.container.updateInteractivePop(logicalTranslation: inactive.width * 0.7)
        XCTAssertEqual(inactive.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertTrue(inactive.container.path.isEmpty)
        inactive.container.sceneDidBecomeInactive()

        XCTAssertEqual(
            inactive.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(inactive.probe.screenChangedCount, 0)
        inactive.container.sceneDidBecomeActive()
        inactive.container.sceneDidBecomeActive()
        XCTAssertEqual(inactive.probe.screenChangedCount, 1)

        let superseded = Harness(path: [entry(1)])
        XCTAssertTrue(superseded.container.beginInteractivePop())
        superseded.container.updateInteractivePop(logicalTranslation: superseded.width * 0.7)
        XCTAssertEqual(superseded.container.endInteractivePop(logicalVelocity: 0), .committed)
        superseded.container.supersedeActiveTransition()

        XCTAssertTrue(superseded.container.path.isEmpty)
        XCTAssertEqual(
            superseded.probe.terminals,
            [.init(outcome: .committed, visibility: .superseded)]
        )
        XCTAssertEqual(superseded.probe.screenChangedCount, 0)
        XCTAssertFalse(try! XCTUnwrap(superseded.visibleWrapper()).isUserInteractionEnabled)
    }

    func testProgrammaticImmediateSettleWhileInactiveDefersVisibleEffects() {
        let harness = Harness(path: [entry(1)])
        harness.container.sceneDidBecomeInactive()

        XCTAssertTrue(harness.container.push(entry(2), animated: false))
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeInactive()
        XCTAssertEqual(harness.probe.terminals.count, 1)
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeActive()
        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testHardSnapWhileInactiveDefersVisibleEffectsExactlyOnce() throws {
        let harness = Harness(path: [entry(1)])
        harness.container.sceneDidBecomeInactive()

        XCTAssertTrue(harness.container.requestHardSnap(to: [entry(2)]))
        XCTAssertEqual(harness.container.path, [entry(2)])
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)
        let wrapper = try XCTUnwrap(harness.visibleWrapper())
        XCTAssertFalse(wrapper.isUserInteractionEnabled)
        XCTAssertTrue(wrapper.accessibilityElementsHidden)

        harness.container.sceneDidBecomeActive()
        harness.container.sceneDidBecomeActive()

        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertTrue(wrapper.isUserInteractionEnabled)
        XCTAssertFalse(wrapper.accessibilityElementsHidden)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testInteractiveImmediatePolicyWhileInactiveDefersVisibleEffects() {
        let harness = Harness(
            path: [entry(1)],
            preferences: .init(reduceMotion: true, prefersCrossFadeTransitions: false)
        )
        harness.container.sceneDidBecomeInactive()

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testDeferredCommittedDestinationCanStartNextCommitWithoutLifecycleViolation() {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        harness.container.sceneDidBecomeInactive()

        XCTAssertEqual(
            harness.probe.terminals,
            [.init(outcome: .committed, visibility: .inactive)]
        )
        XCTAssertTrue(harness.container.push(entry(2), animated: false))
        XCTAssertEqual(harness.container.path, [entry(2)])
        XCTAssertEqual(
            harness.probe.terminals,
            [
                .init(outcome: .committed, visibility: .inactive),
                .init(outcome: .committed, visibility: .inactive),
            ]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        harness.container.sceneDidBecomeActive()
        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(harness.probe.screenChangedCount, 1)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testNewTransitionPermanentlyCancelsDeferredVisibleEffects() {
        let harness = Harness(path: [entry(1), entry(2)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        harness.container.sceneDidBecomeInactive()
        XCTAssertEqual(harness.probe.screenChangedCount, 0)

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.sceneDidBecomeActive()
        XCTAssertEqual(
            harness.probe.screenChangedCount,
            0,
            "a superseded inactive terminal must never replay during the next transaction"
        )

        harness.container.cancelInteractivePop()
        harness.container.completeSettleImmediately()
        XCTAssertEqual(
            harness.probe.terminals,
            [
                .init(outcome: .committed, visibility: .inactive),
                .init(outcome: .cancelled, visibility: .visible),
            ]
        )
        XCTAssertEqual(harness.probe.screenChangedCount, 0)
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testStagedDestinationPerformsNoLifecycleWritesUntilCommittedVisible() {
        let harness = Harness(path: [entry(1)])
        let home = GaryxRoutePresentationIdentity.home
        let route = GaryxRoutePresentationIdentity.entry(entry(1).id)
        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [])
        XCTAssertEqual(harness.probe.lifecycle[route, default: []], [.appeared, .active])

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.3)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .cancelled)
        harness.container.completeSettleImmediately()
        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [])

        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.7)
        XCTAssertEqual(harness.container.endInteractivePop(logicalVelocity: 0), .committed)
        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [])
        harness.container.completeSettleImmediately()

        XCTAssertEqual(harness.probe.lifecycle[home, default: []], [.appeared, .active])
        XCTAssertEqual(
            harness.probe.lifecycle[route, default: []],
            [.appeared, .active, .inactive, .disappeared]
        )
    }

    func testAllVisualPoliciesWriteOnlyWrappers() throws {
        let policies: [(GaryxRouteVisualPreferences, GaryxRouteVisualPolicy)] = [
            (.init(reduceMotion: false, prefersCrossFadeTransitions: false), .spatial),
            (.init(reduceMotion: false, prefersCrossFadeTransitions: true), .crossFade),
            (.init(reduceMotion: true, prefersCrossFadeTransitions: false), .immediate),
        ]

        for (preferences, expectedPolicy) in policies {
            let harness = Harness(path: [entry(1)], preferences: preferences)
            XCTAssertTrue(harness.container.beginInteractivePop())
            harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.4)
            XCTAssertEqual(harness.container.visualPolicyForActiveTransaction, expectedPolicy)
            let wrappers = harness.wrappers()
            XCTAssertEqual(wrappers.count, 2)
            XCTAssertTrue(harness.container.children.allSatisfy { $0.view.transform == .identity })

            if expectedPolicy == .spatial {
                XCTAssertTrue(wrappers.contains { $0.transform.tx != 0 })
                XCTAssertTrue(wrappers.contains { $0.layer.shadowOpacity > 0 })
            } else {
                XCTAssertTrue(wrappers.allSatisfy { $0.transform == .identity })
                XCTAssertTrue(wrappers.allSatisfy { $0.layer.shadowOpacity == 0 })
                XCTAssertTrue(wrappers.allSatisfy { $0.scrimView.alpha == 0 })
            }
        }
    }

    func testPopZOrderAlwaysPlacesOutgoingWrapperAboveIncoming() throws {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        let source = try XCTUnwrap(
            harness.wrapper(identity: .entry(entry(1).id))
        )
        let destination = try XCTUnwrap(harness.wrapper(identity: .home))
        let sourceIndex = try XCTUnwrap(harness.container.view.subviews.firstIndex(of: source))
        let destinationIndex = try XCTUnwrap(
            harness.container.view.subviews.firstIndex(of: destination)
        )
        XCTAssertGreaterThan(sourceIndex, destinationIndex)
    }

    func testRotationRederivesWrapperGeometryAtCurrentProgress() throws {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.container.updateInteractivePop(logicalTranslation: harness.width * 0.4)
        let sourceIdentity = GaryxRoutePresentationIdentity.entry(entry(1).id)
        let sourceBefore = try XCTUnwrap(harness.wrapper(identity: sourceIdentity))
        XCTAssertEqual(sourceBefore.transform.tx, harness.width * 0.4, accuracy: 0.01)

        harness.container.view.frame = CGRect(x: 0, y: 0, width: 844, height: 393)
        harness.container.view.setNeedsLayout()
        harness.container.view.layoutIfNeeded()

        let sourceAfter = try XCTUnwrap(harness.wrapper(identity: sourceIdentity))
        XCTAssertEqual(sourceAfter.bounds.width, 844, accuracy: 0.01)
        XCTAssertEqual(sourceAfter.transform.tx, 844 * 0.4, accuracy: 0.01)
        XCTAssertEqual(sourceAfter.center.x, 422, accuracy: 0.01)
    }

    func testOneHundredTwentyHertzDragFramesCauseZeroSwiftUIBodyRecomputations() {
        let harness = Harness(path: [entry(1)])
        XCTAssertTrue(harness.container.beginInteractivePop())
        harness.pumpUI()
        let baseline = harness.bodyCounter.count

        for frame in 1...120 {
            harness.container.updateInteractivePop(
                logicalTranslation: harness.width * CGFloat(frame) / 240
            )
            harness.pumpUI(duration: 0.0001)
        }

        XCTAssertEqual(
            harness.bodyCounter.count,
            baseline,
            "display progress must never publish SwiftUI state"
        )
    }

    func testTwentyLayerStackAndFiveHundredChurnNeverExceedHostBudget() {
        let deep = Harness(path: (1...20).map { entry($0) })
        XCTAssertLessThanOrEqual(deep.container.metrics.mountedHostCount, 4)
        XCTAssertLessThanOrEqual(deep.container.metrics.peakMountedHostCount, 4)

        let churn = Harness(path: [])
        for index in 0..<500 {
            XCTAssertTrue(churn.container.push(entry(index + 1), animated: false))
            XCTAssertTrue(churn.container.pop(animated: false))
            XCTAssertFalse(churn.container.hasTerminalResidue)
        }
        XCTAssertEqual(churn.container.path, [])
        XCTAssertLessThanOrEqual(churn.container.metrics.mountedHostCount, 4)
        XCTAssertLessThanOrEqual(churn.container.metrics.peakMountedHostCount, 4)
        XCTAssertLessThanOrEqual(churn.container.metrics.stateStore.evictableEntryCount, 32)
        XCTAssertLessThanOrEqual(
            churn.container.metrics.stateStore.evictableCostBytes,
            2 * 1_024 * 1_024
        )
    }

    func testPopMultipleUnmountsEveryPermanentlyRemovedHostAtTerminal() {
        let harness = Harness(path: [entry(1), entry(2), entry(3)])
        let middle = GaryxRoutePresentationIdentity.entry(entry(2).id)
        let source = GaryxRoutePresentationIdentity.entry(entry(3).id)
        XCTAssertTrue(harness.container.mountedHostIdentities.contains(middle))
        XCTAssertTrue(harness.container.mountedHostIdentities.contains(source))

        XCTAssertTrue(harness.container.pop(count: 2, animated: false))

        XCTAssertEqual(harness.container.path, [entry(1)])
        XCTAssertFalse(harness.container.mountedHostIdentities.contains(middle))
        XCTAssertFalse(harness.container.mountedHostIdentities.contains(source))
        XCTAssertTrue(harness.probe.unmounted.contains(middle))
        XCTAssertTrue(harness.probe.unmounted.contains(source))
        XCTAssertEqual(harness.probe.lifecycle[middle, default: []], [])
        XCTAssertEqual(
            harness.probe.lifecycle[source, default: []],
            [.appeared, .active, .inactive, .disappeared]
        )
        XCTAssertFalse(harness.container.hasTerminalResidue)
    }

    func testPresentationLeaseJoinSameFrameRaceAndHardSnapBarrier() throws {
        let harness = Harness(path: [entry(1)])
        let parent = GaryxPresentationLeaseToken(rawValue: "synthetic-parent")
        let picker = GaryxPresentationLeaseToken(rawValue: "synthetic-picker")
        XCTAssertTrue(harness.container.acquirePresentationLease(parent))
        XCTAssertTrue(
            harness.container.acquirePresentationLease(
                picker,
                parent: parent,
                resultBearing: true
            )
        )
        XCTAssertFalse(harness.container.leadingEdgePanGestureRecognizer.isEnabled)

        let replacement = [entry(99)]
        XCTAssertFalse(harness.container.requestHardSnap(to: replacement))
        XCTAssertEqual(harness.container.path, [entry(1)])

        harness.container.presentationDismissalCompleted(picker)
        XCTAssertEqual(
            harness.container.presentationLeaseRecord(picker)?.joinState,
            .dismissedAwaitingResult
        )
        harness.container.recordPresentationResult(picker)
        XCTAssertEqual(harness.container.presentationLeaseRecord(picker)?.releaseCount, 1)
        XCTAssertTrue(harness.container.hasPresentationBarrier, "parent still blocks hard snap")

        harness.container.presentationDismissalCompleted(parent)
        harness.container.presentationDismissalCompleted(parent)
        XCTAssertEqual(harness.container.presentationLeaseRecord(parent)?.releaseCount, 1)
        XCTAssertFalse(harness.container.hasPresentationBarrier)
        XCTAssertEqual(harness.container.path, replacement)
        XCTAssertFalse(harness.container.hasTerminalResidue)
        XCTAssertEqual(harness.probe.screenChangedCount, 1)

        let secondReplacement = [entry(100)]
        XCTAssertTrue(harness.container.requestHardSnap(to: secondReplacement))
        XCTAssertEqual(harness.container.path, secondReplacement)
        XCTAssertEqual(
            harness.probe.screenChangedCount,
            2,
            "each committed-visible hard snap emits exactly one screen change"
        )

        let failed = GaryxPresentationLeaseToken(rawValue: "synthetic-failure")
        XCTAssertTrue(harness.container.acquirePresentationLease(failed, resultBearing: true))
        harness.container.presentationFailed(failed)
        XCTAssertEqual(harness.container.presentationLeaseRecord(failed)?.releaseCount, 1)
        XCTAssertFalse(harness.container.hasPresentationBarrier)
    }

    func testContainerDeinitReleasesAllHostingControllersAndRootViews() {
        let factory = LifetimeFactory()
        weak var weakContainer: GaryxRouteStackContainer?
        var window: UIWindow?
        autoreleasepool {
            var container: GaryxRouteStackContainer? = GaryxRouteStackContainer(
                initialPath: (1...20).map { entry($0) },
                preferencesProvider: {
                    .init(reduceMotion: false, prefersCrossFadeTransitions: false)
                },
                hostBuilder: { node in
                    AnyView(LifetimeRouteView(node: node, token: factory.make()))
                }
            )
            weakContainer = container
            window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: 393, height: 852))
            window?.rootViewController = container
            window?.isHidden = false
            container?.loadViewIfNeeded()
            container?.view.layoutIfNeeded()
            XCTAssertLessThanOrEqual(container?.children.count ?? .max, 4)
            window?.rootViewController = nil
            container = nil
        }
        window = nil
        pumpMainRunLoop(duration: 0.05)

        XCTAssertNil(weakContainer)
        XCTAssertGreaterThan(factory.weakTokens.count, 0)
        XCTAssertTrue(factory.weakTokens.allSatisfy { $0.value == nil })
    }

    // MARK: Fixtures

    private func edgeInteraction(
        requiresEdgeZone: Bool,
        direction: GaryxRouteGestureDirection
    ) -> GaryxRouteEdgePanInteraction {
        GaryxRouteEdgePanInteraction(
            isEligible: { true },
            requiresEdgeZone: { requiresEdgeZone },
            acceptedDirection: { direction },
            began: {},
            changed: { _, _ in },
            ended: { _ in },
            cancelled: {}
        )
    }

    private func entry(
        _ index: Int,
        destination: GaryxRouteDestination? = nil
    ) -> GaryxRouteEntry {
        GaryxRouteEntry(
            id: .init(rawValue: "synthetic-route-\(index)"),
            destination: destination ?? .panel("synthetic-panel-\(index)")
        )
    }

    private static func productionConversationHost(
        node: GaryxRoutePresentationNode,
        model: GaryxMobileModel,
        rootSurfaceOccurrenceID: GaryxRootSurfaceOccurrenceID,
        routeLifecycleRegistry: GaryxRouteLifecycleRegistry
    ) -> AnyView {
        guard case .entry(let entry) = node else {
            return AnyView(EmptyView())
        }
        return AnyView(
            GaryxConversationRouteView(
                destination: entry.destination,
                occurrenceID: entry.id
            )
            .environmentObject(model)
            .environment(model.homeObservationStore)
            .environment(\.garyxAvatarImageProvider, model.avatarImageProvider)
            .environment(\.garyxAvatarScopeId, model.currentGatewayScopeId)
            // The production route host always supplies this through its root
            // shell; tests thread the harness occurrence through so
            // occurrence-scoped assertions can vary it.
            .environment(\.garyxRootSurfaceOccurrenceID, rootSurfaceOccurrenceID)
            .environment(\.garyxRouteLifecycleRegistry, routeLifecycleRegistry)
            .environment(
                \.garyxPresentationLeaseCoordinator,
                model.productionRouteStore.presentationCoordinator
            )
        )
    }

    private final class Probe {
        var mounted: [GaryxRoutePresentationIdentity] = []
        var unmounted: [GaryxRoutePresentationIdentity] = []
        var lifecycle: [GaryxRoutePresentationIdentity: [GaryxRouteHostLifecyclePhase]] = [:]
        var phases: [GaryxPresentationTransactionPhase] = []
        var paths: [[GaryxRouteEntry]] = []
        var terminals: [GaryxPresentationTerminalState] = []
        var screenChangedCount = 0
        var screenChangedArguments: [UIView] = []
        var screenChangedHostWasVisible: [Bool] = []
    }

    private final class BodyCounter {
        private(set) var count = 0
        func record() { count += 1 }
    }

    private final class HostBuildProbe {
        private(set) var nodes: [GaryxRoutePresentationNode] = []

        func record(_ node: GaryxRoutePresentationNode) {
            nodes.append(node)
        }

        func buildCount(for instanceID: GaryxRouteInstanceID) -> Int {
            nodes.reduce(into: 0) { count, node in
                guard case .entry(let entry) = node, entry.id == instanceID else { return }
                count += 1
            }
        }

        func lastDestination(
            for instanceID: GaryxRouteInstanceID
        ) -> GaryxRouteDestination? {
            nodes.reversed().compactMap { node in
                guard case .entry(let entry) = node, entry.id == instanceID else { return nil }
                return entry.destination
            }.first
        }
    }

    private struct CountingRouteView: View {
        let node: GaryxRoutePresentationNode
        let counter: BodyCounter

        var body: some View {
            counter.record()
            return VStack {
                Text(label)
                    .accessibilityIdentifier("synthetic route label")
                ScrollView(.horizontal) {
                    HStack {
                        ForEach(0..<8, id: \.self) { index in
                            Text("Item \(index)")
                        }
                    }
                }
            }
        }

        private var label: String {
            switch node {
            case .home:
                "Synthetic home"
            case .entry(let entry):
                "Synthetic \(entry.id.rawValue)"
            }
        }
    }

    private final class ManualTimeSource: GaryxGestureSettleTimeSource {
        var now: TimeInterval = 10
    }

    private final class ManualFrameSource: GaryxGestureSettleFrameSource {
        var onFrame: (() -> Void)?
        private(set) var isRunning = false

        func start() { isRunning = true }
        func invalidate() { isRunning = false }
        func fire() {
            guard isRunning else { return }
            onFrame?()
        }
    }

    @MainActor
    private final class Harness {
        let width: CGFloat = 393
        let probe = Probe()
        let bodyCounter = BodyCounter()
        let hostBuildProbe = HostBuildProbe()
        let clock = ManualTimeSource()
        let frames = ManualFrameSource()
        let container: GaryxRouteStackContainer
        let window: UIWindow

        init(
            path: [GaryxRouteEntry],
            preferences: GaryxRouteVisualPreferences = .init(
                reduceMotion: false,
                prefersCrossFadeTransitions: false
            ),
            routeLifecycleRegistry: GaryxRouteLifecycleRegistry? = nil,
            routeHostBuilder: (@MainActor (GaryxRoutePresentationNode) -> AnyView)? = nil
        ) {
            var callbacks = GaryxRouteStackContainerCallbacks()
            callbacks.hostMounted = { [probe, routeLifecycleRegistry] identity in
                probe.mounted.append(identity)
                routeLifecycleRegistry?.hostMounted(identity)
            }
            callbacks.hostUnmounted = { [probe, routeLifecycleRegistry] identity in
                probe.unmounted.append(identity)
                routeLifecycleRegistry?.hostUnmounted(identity)
            }
            callbacks.hostLifecycleChanged = { [probe] identity, phase in
                probe.lifecycle[identity, default: []].append(phase)
                routeLifecycleRegistry?.update(identity, lifecycle: phase)
            }
            callbacks.hasPresentedFrameDemand = { [routeLifecycleRegistry] in
                routeLifecycleRegistry?.hasPresentedFrameDemand ?? false
            }
            callbacks.presentedFrame = { [routeLifecycleRegistry] in
                routeLifecycleRegistry?.presentedFrame()
            }
            callbacks.phaseChanged = { [probe] in probe.phases.append($0) }
            callbacks.canonicalPathChanged = { [probe] in probe.paths.append($0) }
            callbacks.terminalReached = { [probe] in probe.terminals.append($0) }
            callbacks.screenChanged = { [probe] view in
                probe.screenChangedCount += 1
                probe.screenChangedArguments.append(view)
                var ancestor: UIView? = view
                while ancestor != nil,
                      !(ancestor is GaryxRouteTransitionWrapperView) {
                    ancestor = ancestor?.superview
                }
                let wrapper = ancestor as? GaryxRouteTransitionWrapperView
                probe.screenChangedHostWasVisible.append(
                    wrapper?.isHidden == false
                        && wrapper?.isUserInteractionEnabled == true
                        && wrapper?.accessibilityElementsHidden == false
                )
            }

            container = GaryxRouteStackContainer(
                initialPath: path,
                settleDriver: GaryxGestureSettleDriver(
                    timeSource: clock,
                    frameSource: frames
                ),
                callbacks: callbacks,
                preferencesProvider: { preferences },
                hostBuilder: { [bodyCounter, hostBuildProbe, routeHostBuilder] node in
                    hostBuildProbe.record(node)
                    if let routeHostBuilder {
                        return routeHostBuilder(node)
                    }
                    return AnyView(CountingRouteView(node: node, counter: bodyCounter))
                }
            )
            window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: width, height: 852))
            window.rootViewController = container
            window.isHidden = false
            container.loadViewIfNeeded()
            container.view.frame = window.bounds
            container.view.setNeedsLayout()
            container.view.layoutIfNeeded()
            pumpUI()
        }

        func advance(by delta: TimeInterval) {
            clock.now += delta
            frames.fire()
            pumpUI(duration: 0.001)
        }

        func completeDisplayLinkedSettle() {
            advance(by: GaryxRouteTransitionCalibration.settleCurve.settlingDuration + 0.001)
        }

        func wrappers() -> [GaryxRouteTransitionWrapperView] {
            container.view.subviews.compactMap { $0 as? GaryxRouteTransitionWrapperView }
        }

        func wrapper(
            identity: GaryxRoutePresentationIdentity
        ) -> GaryxRouteTransitionWrapperView? {
            wrappers().first { $0.representedIdentity == identity }
        }

        func visibleWrapper() -> GaryxRouteTransitionWrapperView? {
            wrappers().first { !$0.isHidden }
        }

        func pumpUI(duration: TimeInterval = 0.01) {
            pumpMainRunLoop(duration: duration)
        }
    }

    private final class LifetimeToken {}

    private final class WeakLifetimeToken {
        weak var value: LifetimeToken?
        init(_ value: LifetimeToken) { self.value = value }
    }

    private final class LifetimeFactory {
        private(set) var weakTokens: [WeakLifetimeToken] = []

        func make() -> LifetimeToken {
            let token = LifetimeToken()
            weakTokens.append(WeakLifetimeToken(token))
            return token
        }
    }

    private struct LifetimeRouteView: View {
        let node: GaryxRoutePresentationNode
        let token: LifetimeToken

        var body: some View {
            Text(String(describing: node))
        }
    }
}

@MainActor
private final class TaskNotificationMeasurementSink {
    var values: [GaryxTaskNotificationCardMeasurement] = []
}

@MainActor
private func taskNotification(body: String) -> GaryxTaskNotification {
    GaryxTaskNotification(
        event: "ready_for_review",
        status: "in_review",
        taskId: "#TASK-42",
        title: "Layout validation",
        finalMessage: body
    )
}

@MainActor
private func taskNotificationMeasurement(
    body: String,
    width: CGFloat,
    dynamicTypeSize: DynamicTypeSize
) async throws -> GaryxTaskNotificationCardMeasurement {
    let sink = TaskNotificationMeasurementSink()
    let root = AnyView(
        GaryxTaskNotificationCard(
            notification: taskNotification(body: body),
            onExpand: {},
            onFileLinkTap: { _ in },
            onImageFilePreview: { _ in nil },
            onMeasurement: { sink.values.append($0) }
        )
        .frame(width: width)
        .environment(\.dynamicTypeSize, dynamicTypeSize)
    )
    let controller = UIHostingController(rootView: root)
    let window = makeTestWindow(frame: CGRect(x: 0, y: 0, width: width, height: 1_200))
    window.rootViewController = controller
    window.isHidden = false
    defer {
        window.isHidden = true
        window.rootViewController = nil
    }

    for _ in 0..<5 {
        window.layoutIfNeeded()
        try await Task.sleep(for: .milliseconds(40))
    }
    return try XCTUnwrap(sink.values.last)
}

@MainActor
private func makeTaskNotificationImagePreview(
    size: CGSize
) throws -> GaryxWorkspaceFilePreview {
    let image = UIGraphicsImageRenderer(size: size).image { context in
        UIColor.systemBlue.setFill()
        context.cgContext.fill(CGRect(origin: .zero, size: size))
    }
    let encoded = try XCTUnwrap(image.pngData()).base64EncodedString()
    let payload: [String: Any] = [
        "workspace_dir": "/Users/test/workspace",
        "path": "late-image.png",
        "name": "late-image.png",
        "media_type": "image/png",
        "preview_kind": "image",
        "size": encoded.count,
        "truncated": false,
        "data_base64": encoded,
    ]
    return try JSONDecoder().decode(
        GaryxWorkspaceFilePreview.self,
        from: JSONSerialization.data(withJSONObject: payload)
    )
}

@MainActor
private func pumpMainRunLoop(duration: TimeInterval) {
    RunLoop.main.run(until: Date().addingTimeInterval(duration))
}

@MainActor
private func waitForTranscriptSnapshot(
    threadID: String,
    timeout: TimeInterval = 3
) -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if GaryxConversationTranscriptSnapshotCache.shared.hasSnapshot(for: threadID) {
            return true
        }
        pumpMainRunLoop(duration: 0.02)
    }
    return GaryxConversationTranscriptSnapshotCache.shared.hasSnapshot(for: threadID)
}

@MainActor
private func makeTestWindow(frame: CGRect) -> UIWindow {
    guard let scene = UIApplication.shared.connectedScenes
        .compactMap({ $0 as? UIWindowScene })
        .first
    else { preconditionFailure("hosted iOS tests require an active UIWindowScene") }
    let window = UIWindow(windowScene: scene)
    window.frame = frame
    return window
}

@MainActor
private func cacheTranscriptSnapshot(threadID: String, size: CGSize) {
    let sourceController = UIViewController()
    let sourceScrollView = UIScrollView(frame: CGRect(origin: .zero, size: size))
    sourceScrollView.backgroundColor = .white
    sourceScrollView.contentSize = size
    sourceController.view = sourceScrollView

    let transcript = UILabel(frame: sourceScrollView.bounds.insetBy(dx: 16, dy: 16))
    transcript.numberOfLines = 0
    transcript.text = "Ran 3 commands\n上一线程的缓存文字"
    sourceScrollView.addSubview(transcript)

    let sourceWindow = makeTestWindow(frame: CGRect(origin: .zero, size: size))
    sourceWindow.rootViewController = sourceController
    sourceWindow.isHidden = false
    sourceWindow.layoutIfNeeded()
    defer {
        sourceWindow.isHidden = true
        sourceWindow.rootViewController = nil
    }

    GaryxConversationTranscriptSnapshotCache.shared.scheduleCapture(
        threadID: threadID,
        revision: "cross-thread-snapshot-repro",
        scrollView: { sourceScrollView }
    )
    let snapshotCaptured = waitForTranscriptSnapshot(threadID: threadID)
    XCTAssertTrue(
        snapshotCaptured,
        "the production compositor capture must complete before exercising reuse"
    )
}

@MainActor
private func renderedMessageImage(
    message: GaryxMobileMessage,
    transcriptWidth: CGFloat,
    dynamicTypeSize: DynamicTypeSize
) throws -> UIImage {
    let renderer = ImageRenderer(
        content: GaryxMessageBubble(message: message)
            .frame(width: transcriptWidth)
            .background(Color(red: 1, green: 0, blue: 1))
            .environment(\.dynamicTypeSize, dynamicTypeSize)
    )
    renderer.scale = 1
    return try XCTUnwrap(renderer.uiImage)
}

private func nonMagentaBounds(in image: UIImage) throws -> CGRect {
    let cgImage = try XCTUnwrap(image.cgImage)
    let width = cgImage.width
    let height = cgImage.height
    var pixels = [UInt8](repeating: 0, count: width * height * 4)
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    let context = try XCTUnwrap(
        CGContext(
            data: &pixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width * 4,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        )
    )
    context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
    let background = Array(pixels[0..<4])

    var minimumX = width
    var maximumX = -1
    for y in 0..<height {
        var first: Int?
        var last: Int?
        for x in 0..<width {
            let index = (y * width + x) * 4
            let red = pixels[index]
            let green = pixels[index + 1]
            let blue = pixels[index + 2]
            let alpha = pixels[index + 3]
            let isBackground = zip([red, green, blue, alpha], background).allSatisfy {
                abs(Int($0.0) - Int($0.1)) <= 2
            }
            guard alpha > 2, !isBackground else { continue }
            first = first ?? x
            last = x
        }
        if let first, let last {
            minimumX = min(minimumX, first)
            maximumX = max(maximumX, last)
        }
    }
    guard maximumX >= minimumX else {
        return .zero
    }
    return CGRect(
        x: CGFloat(minimumX) / image.scale,
        y: 0,
        width: CGFloat(maximumX - minimumX + 1) / image.scale,
        height: image.size.height
    )
}

@MainActor
private func descendants(of controller: UIViewController) -> [UIViewController] {
    controller.children.flatMap { child in
        [child] + descendants(of: child)
    }
}

private extension Array where Element: Equatable {
    func containsSubsequence(_ subsequence: [Element]) -> Bool {
        guard !subsequence.isEmpty else { return true }
        var index = subsequence.startIndex
        for element in self where element == subsequence[index] {
            index = subsequence.index(after: index)
            if index == subsequence.endIndex { return true }
        }
        return false
    }
}
